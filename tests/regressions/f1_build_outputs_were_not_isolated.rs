// SPDX-License-Identifier: MIT OR Apache-2.0
//! Build outputs and workspaces must belong to the invocation that writes them.
//!
//! Previously two target SBOMs shared the application-only name, JSON writes
//! truncated existing hard links, and a second build ignored the project lock.
//! These inputs exercise the actual filesystem and the public build entry point.

use std::path::{Path, PathBuf};
use std::time::Duration;

use ginary::bundle;
use ginary::config::{BuildFlags, BuildOptions, ProjectConfig};
use ginary::diag::Diag;
use ginary::sbom;
use ginary::target::Target;

use crate::common::artifact::SyntheticArtifact;
use crate::common::project::TempProject;

/// A build whose shipment is deliberately absent, so preflight runs first.
fn options(project: &TempProject) -> BuildOptions {
    let config = ProjectConfig::read(&project.manifest()).expect("fixture manifest");
    BuildOptions::merge(
        project.root(),
        &config,
        &BuildFlags {
            skip_export: true,
            ..BuildFlags::default()
        },
    )
    .expect("build options")
}

#[test]
fn target_sboms_have_their_artifacts_own_names() {
    let linux = Path::new("dist/hello-linux-x86_64-gnu");
    let windows = Path::new("dist/hello-windows-x86_64.exe");
    assert_ne!(
        sbom::out_path(linux, "hello"),
        sbom::out_path(windows, "hello"),
        "each target needs its own SBOM instead of overwriting another target's"
    );
    assert_eq!(
        sbom::out_path(windows, "hello"),
        Path::new("dist/hello-windows-x86_64.exe.spdx.json")
    );
}

#[cfg(windows)]
#[test]
fn ambiguous_windows_build_and_sbom_paths_fail_before_build_or_replacement() {
    for existing in [false, true] {
        for (artifact, sidecar) in [
            ("artifact", "artifact."),
            ("artifact", "artifact "),
            ("artifact.", "bill.json"),
            ("parent./artifact", "bill.json"),
            ("artifact", "NUL.json"),
            ("artifact", "artifact:metadata"),
        ] {
            let project = TempProject::named("hello");
            let original = project.root().join("artifact");
            if existing {
                std::fs::write(&original, b"previous artifact").unwrap();
            }
            let output = assert_cmd::Command::cargo_bin("ginary")
                .unwrap()
                .current_dir(project.root())
                .args([
                    "build",
                    "--skip-export",
                    "--no-strip",
                    "--report",
                    "json",
                    "--out",
                    artifact,
                    "--sbom-out",
                    sidecar,
                    "--otp-root",
                    "unused-runtime",
                ])
                .assert()
                .code(1)
                .get_output()
                .clone();
            let report: serde_json::Value = serde_json::from_slice(&output.stdout)
                .expect("unsafe destinations still receive a structured report");
            assert_eq!(report["stage"], "preflight", "{report}");
            assert_eq!(report["targets"], serde_json::json!([]), "{report}");
            assert!(
                String::from_utf8_lossy(&output.stderr).contains("Windows output destination"),
                "artifact={artifact:?}, sidecar={sidecar:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(!project.root().join("build/ginary/.build-lock").exists());
            if existing {
                assert_eq!(std::fs::read(original).unwrap(), b"previous artifact");
            } else {
                assert!(!original.exists());
            }
        }
    }
}

#[test]
fn publishing_an_sbom_replaces_the_name_without_truncating_a_hardlink_peer() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let artifact = SyntheticArtifact::build(dir.path());
    let document = sbom::for_artifact(artifact.path(), None).expect("SBOM");
    let original = dir.path().join("previous.json");
    let destination = dir.path().join("next.json");
    std::fs::write(&original, b"previous document").expect("previous file");
    std::fs::hard_link(&original, &destination).expect("hardlink");

    sbom::write(&document, &destination).expect("publish document");

    assert!(
        std::fs::read(&original).expect("read peer") == b"previous document",
        "publishing a name must not rewrite the inode shared by another name"
    );
    let written = std::fs::read(&destination).expect("new document");
    assert!(serde_json::from_slice::<serde_json::Value>(&written).is_ok());
}

#[test]
fn duplicate_artifact_destinations_are_refused_before_obtaining_the_shipment() {
    let project = TempProject::named("hello");
    let mut opts = options(&project);
    opts.targets = vec![Target::host(), Target::host()];
    let error = bundle::build_with_stub(
        &opts,
        &PathBuf::from(env!("CARGO_BIN_EXE_ginary")),
        &Diag::disabled(),
    )
    .expect_err("duplicate output names must be refused");
    assert!(
        error.to_string().contains("output"),
        "the refusal must identify the output collision before export: {error}"
    );
}

#[test]
fn an_embedding_call_cannot_package_a_file_that_is_not_a_ginary_stub() {
    let project = TempProject::named("hello");
    let foreign = project.root().join("another-program");
    std::fs::write(&foreign, b"not a ginary executable").expect("unrelated program");
    let error = bundle::build_with_stub(&options(&project), &foreign, &Diag::disabled())
        .expect_err("an embedding application must supply a ginary stub");
    assert!(
        error.to_string().contains("stub"),
        "stub identity must be checked before the missing shipment: {error}"
    );
}

#[test]
fn another_build_cannot_obtain_a_shipment_while_the_project_is_locked() {
    let project = TempProject::named("hello");
    let lock_dir = project.root().join("build/ginary/.build-lock");
    std::fs::create_dir_all(&lock_dir).expect("lock directory");
    let _lock = ginary::cache_lock::wait_exclusive(&lock_dir, Duration::ZERO)
        .expect("first build holds the project lock");
    let error = bundle::build_with_stub(
        &options(&project),
        &PathBuf::from(env!("CARGO_BIN_EXE_ginary")),
        &Diag::disabled(),
    )
    .expect_err("the second build cannot start");
    assert!(
        error.to_string().contains("build lock"),
        "the competing build must stop at the project lock: {error}"
    );
}

/// A complete synthetic shipment and a runtime whose header names this host.
/// No external compiler or emulator is executed by these packaging checks.
fn complete_options(project: &TempProject) -> BuildOptions {
    use crate::common::fake_otp::{FakeOtp, FakeShipment};
    use ginary::target::{Arch, Os};
    let host = Target::host();
    let fake = match host.os {
        Os::Windows => FakeOtp::new().windows(),
        Os::Macos => FakeOtp::new().macos().macho_cpu_type(match host.arch {
            Arch::X86_64 => crate::common::native::MACHO_CPU_X86_64,
            Arch::Aarch64 => crate::common::native::MACHO_CPU_ARM64,
        }),
        Os::Linux => FakeOtp::new(),
    };
    let runtime = fake.build_in(project.root().join("otp"));
    if host.os == Os::Linux {
        let machine = match host.arch {
            Arch::X86_64 => crate::common::repack::EM_X86_64,
            Arch::Aarch64 => crate::common::repack::EM_AARCH64,
        };
        std::fs::write(
            runtime.erts_bin().join("beam.smp"),
            crate::common::native::elf_bytes(machine, crate::common::native::ET_EXEC, None),
        )
        .expect("host emulator header");
    }
    FakeShipment::new()
        .app("hello", "1.0.0", &["kernel", "stdlib"])
        .build_in(project.root().join(ginary::gleam::SHIPMENT_DIR));
    let mut opts = options(project);
    opts.otp_root = Some(runtime.root);
    opts.strip = ginary::strip::StripOptions {
        elf: false,
        beams: false,
    };
    opts.compression_level = 1;
    opts.keep_staging = true;
    opts
}

#[cfg(windows)]
#[test]
fn a_windows_build_consumes_verified_local_catalog_and_tarball_runtimes() {
    use crate::common::script::{ShimStep, program};
    use ginary::catalog::{RepackOptions, RepackSelector};

    let project = TempProject::named("hello");
    let opts = complete_options(&project);
    let runtime = opts.otp_root.as_ref().unwrap();
    let target = Target::host().name();
    let repacked = ginary::catalog::repack_from_root(
        &RepackOptions {
            upstream_tag: "OTP-29.0.5".into(),
            selectors: vec![RepackSelector::parse(&target).unwrap()],
            out: project.root().join("catalog"),
            upstream_dir: None,
            source_date_epoch: Some(0),
        },
        runtime,
        &Diag::disabled(),
    )
    .expect("a verified native runtime produces a local catalog");
    let probe = project.subdir("probe");
    program(
        &probe,
        "erl",
        &[ShimStep::Print(vec![
            runtime.display().to_string(),
            "29".into(),
            "17.0.5".into(),
        ])],
    );
    let run = |source: &str| {
        std::fs::write(
            project.manifest(),
            format!(
                "name = 'hello'\nversion = '1.0.0'\n[tools.ginary.target.'{target}']\nerts = '{source}'\n"
            ),
        )
        .unwrap();
        assert_cmd::Command::cargo_bin("ginary")
            .unwrap()
            .current_dir(project.root())
            .env("PATH", &probe)
            .env("GINARY_CACHE_DIR", project.root().join("cache"))
            .env("GINARY_CATALOG", &repacked.catalog)
            .env("GINARY_OFFLINE", "1")
            .args([
                "build",
                "--skip-export",
                "--no-strip",
                "--target",
                &target,
                "--sbom",
                "--report",
                "json",
            ])
            .timeout(Duration::from_secs(60))
            .output()
            .expect("bounded local-catalog build")
    };
    let mut artifact = None;
    for source in [
        "catalog".to_owned(),
        format!("tarball:{}", repacked.outcomes[0].tarball.display()),
    ] {
        let output = run(&source);
        assert!(
            output.status.success(),
            "{source}: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report["targets"].as_array().unwrap().len(), 1);
        assert_eq!(report["sboms"].as_array().unwrap().len(), 1);
        let path = PathBuf::from(report["targets"][0]["out"].as_str().unwrap());
        let info = ginary::inspect::open(&path).unwrap();
        assert!(ginary::inspect::verify(&info).unwrap().ok());
        artifact = Some(path);
    }

    let artifact = artifact.unwrap();
    let previous = std::fs::read(&artifact).unwrap();
    let foreign =
        crate::common::fake_otp::FakeOtp::new().build_in(project.root().join("foreign-runtime"));
    std::fs::write(
        foreign.erts_bin().join("beam.smp"),
        crate::common::native::elf_bytes(
            crate::common::repack::EM_X86_64,
            crate::common::native::ET_EXEC,
            Some("/lib64/ld-linux-x86-64.so.2"),
        ),
    )
    .unwrap();
    let incomplete = crate::common::fake_otp::FakeOtp::new()
        .windows()
        .build_in(project.root().join("incomplete-runtime"));
    std::fs::remove_file(incomplete.erts_bin().join("beam.smp.dll")).unwrap();
    for (name, root) in [
        ("wrong-platform", &foreign.root),
        ("missing-binary", &incomplete.root),
    ] {
        let archive = project.root().join(format!("{name}.tar.zst"));
        std::fs::write(&archive, crate::common::catalog::runtime_tarball(root)).unwrap();
        let output = run(&format!("tarball:{}", archive.display()));
        assert!(
            !output.status.success(),
            "an invalid runtime was accepted: {name}"
        );
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report["targets"], serde_json::json!([]), "{report}");
        if name == "wrong-platform" {
            assert!(
                report.to_string().contains("linux-x86_64-gnu"),
                "the actual emulator must contradict the Windows request: {report}"
            );
        }
        assert_eq!(std::fs::read(&artifact).unwrap(), previous);
    }
}

#[test]
fn output_hardlinks_to_shipment_or_runtime_inputs_are_refused() {
    let project = TempProject::named("hello");
    let mut opts = complete_options(&project);
    let shipment_file = project
        .root()
        .join("build/erlang-shipment/hello/ebin/hello.app");
    let runtime_file = opts
        .otp_root
        .as_ref()
        .expect("runtime")
        .join("bin/no_dot_erlang.boot");
    for (index, input) in [shipment_file, runtime_file].iter().enumerate() {
        let output = project.root().join(format!("aliased-artifact-{index}"));
        std::fs::hard_link(input, &output).expect("input alias");
        opts.out = output;
        let error =
            bundle::validate_output_paths(&opts, &PathBuf::from(env!("CARGO_BIN_EXE_ginary")), &[])
                .expect_err("an output cannot be an existing input under another name");
        assert!(
            error.to_string().contains("input"),
            "the conflict must name its role: {error}"
        );
    }
}

#[test]
fn exporting_a_project_cannot_replace_its_sources_or_dependency_manifest() {
    let project = TempProject::named("hello");
    let mut opts = options(&project);
    for relative in [
        "src/hello.gleam",
        "test/hello_test.gleam",
        "priv/data.txt",
        "manifest.toml",
    ] {
        let input = project.root().join(relative);
        std::fs::create_dir_all(input.parent().expect("parent")).expect("input parent");
        std::fs::write(&input, b"project input").expect("input");
        opts.out = input.clone();
        bundle::validate_output_paths(&opts, Path::new(env!("CARGO_BIN_EXE_ginary")), &[])
            .expect_err("outputs must not replace export inputs");
        let alias = project.root().join("outside-source-tree");
        std::fs::hard_link(&input, &alias).expect("input hard link");
        opts.out = alias.clone();
        bundle::validate_output_paths(&opts, Path::new(env!("CARGO_BIN_EXE_ginary")), &[])
            .expect_err("moving an input alias outside its tree does not make it an output");
        std::fs::remove_file(alias).expect("remove only the test alias");
        assert_eq!(
            std::fs::read(input).expect("input preserved"),
            b"project input"
        );
    }
}

#[test]
fn retained_staging_is_unique_across_builds_in_one_process() {
    let project = TempProject::named("hello");
    let opts = complete_options(&project);
    let stub = PathBuf::from(env!("CARGO_BIN_EXE_ginary"));
    let first =
        bundle::build_with_stub_detailed(&opts, &stub, &Diag::disabled()).expect("first build");
    let second =
        bundle::build_with_stub_detailed(&opts, &stub, &Diag::disabled()).expect("second build");
    let first = first.staging.expect("first staging retained");
    let second = second.staging.expect("second staging retained");
    assert_ne!(
        first, second,
        "a retained build must not be reused by this process"
    );
    for work in [first, second] {
        assert!(
            work.join(Target::host().name())
                .join("root/bin/no_dot_erlang.boot")
                .is_file(),
            "{} must hold its target's complete staging tree",
            work.display()
        );
    }
}

#[test]
fn auxiliary_publication_keeps_the_project_locked_after_building_the_artifact() {
    let project = TempProject::named("hello");
    let mut opts = complete_options(&project);
    opts.keep_staging = false;
    let stub = Path::new(env!("CARGO_BIN_EXE_ginary"));
    let mut calls = 0;
    bundle::build_with_stub_finalized(&opts, stub, &Diag::disabled(), |targets| {
        calls += 1;
        assert_eq!(targets.len(), 1);
        assert!(targets[0].out.is_file());
        let competing = bundle::build_with_stub_detailed(&opts, stub, &Diag::disabled())
            .expect_err(
                "a second build cannot replace an artifact before its auxiliary outputs finish",
            );
        assert!(competing.to_string().contains("build lock"), "{competing}");
    })
    .expect("first build and finalization");
    assert_eq!(calls, 1);
    bundle::build_with_stub_detailed(&opts, stub, &Diag::disabled())
        .expect("the lock is released after finalization");
}

#[test]
fn an_early_build_failure_finalizes_once_with_no_artifacts() {
    let project = TempProject::named("hello");
    let mut calls = 0;
    bundle::build_with_stub_finalized(
        &options(&project),
        Path::new(env!("CARGO_BIN_EXE_ginary")),
        &Diag::disabled(),
        |targets| {
            calls += 1;
            assert!(targets.is_empty());
        },
    )
    .expect_err("missing shipment");
    assert_eq!(calls, 1);
}

#[cfg(feature = "fault-injection")]
#[test]
fn atomic_publication_keeps_previous_sbom_on_write_or_replace_failure() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let artifact = SyntheticArtifact::build(dir.path());
    let destination = dir.path().join("document.json");
    for fault in ["output-write", "output-persist"] {
        std::fs::write(&destination, b"previous SBOM").expect("previous output");
        let before_entries = std::fs::read_dir(dir.path()).expect("directory").count();
        let mut command = assert_cmd::Command::cargo_bin("ginary").expect("CLI");
        command
            .args(["sbom"])
            .arg(artifact.path())
            .arg("--out")
            .arg(&destination)
            .env("GINARY_FAULT", format!("{fault}:fail"))
            .assert()
            .code(1);
        assert_eq!(
            std::fs::read(&destination).expect("retained output"),
            b"previous SBOM"
        );
        assert_eq!(
            std::fs::read_dir(dir.path()).expect("directory").count(),
            before_entries,
            "failed publication cleans its temporary file"
        );
        let mut retry = assert_cmd::Command::cargo_bin("ginary").expect("CLI");
        retry
            .args(["sbom"])
            .arg(artifact.path())
            .arg("--out")
            .arg(&destination)
            .env_remove("GINARY_FAULT")
            .assert()
            .success();
        serde_json::from_slice::<serde_json::Value>(
            &std::fs::read(&destination).expect("new document"),
        )
        .expect("complete JSON after retry");
    }
}

#[cfg(feature = "fault-injection")]
#[test]
fn atomic_publication_keeps_previous_manifest_after_an_artifact_was_published() {
    let project = TempProject::named("hello");
    let mut opts = complete_options(&project);
    opts.named_targets = true;
    let target_name = Target::host().name();
    let manifest = opts
        .manifest_copy_path(Target::host())
        .expect("target manifest");
    std::fs::create_dir_all(manifest.parent().expect("parent")).expect("output directory");
    for point in ["output-write", "output-persist"] {
        std::fs::write(&manifest, b"previous manifest").expect("previous output");
        let mut command = assert_cmd::Command::cargo_bin("ginary").expect("CLI");
        let result = command
            .current_dir(project.root())
            .args([
                "build",
                "--skip-export",
                "--no-strip",
                "--target",
                target_name.as_str(),
                "--report",
                "json",
                "--otp-root",
            ])
            .arg(opts.otp_root.as_ref().expect("runtime"))
            .env("GINARY_FAULT", format!("{point}:fail-document"))
            .assert()
            .code(1)
            .get_output()
            .clone();
        assert_eq!(
            std::fs::read(&manifest).expect("retained manifest"),
            b"previous manifest"
        );
        let report: serde_json::Value =
            serde_json::from_slice(&result.stdout).expect("partial result JSON");
        assert_eq!(report["status"], "failed");
        assert_eq!(
            report["targets"].as_array().expect("completed rows").len(),
            1
        );
        assert!(report["targets"][0]["manifest_copy"].is_null());
        let artifact =
            ginary::inspect::open(&opts.artifact_path(Target::host())).expect("published artifact");
        assert!(
            ginary::inspect::verify(&artifact)
                .expect("payload verification")
                .ok()
        );
        let mut retry = assert_cmd::Command::cargo_bin("ginary").expect("CLI");
        retry
            .current_dir(project.root())
            .args([
                "build",
                "--skip-export",
                "--no-strip",
                "--target",
                target_name.as_str(),
                "--otp-root",
            ])
            .arg(opts.otp_root.as_ref().expect("runtime"))
            .env_remove("GINARY_FAULT")
            .assert()
            .success();
        serde_json::from_slice::<serde_json::Value>(
            &std::fs::read(&manifest).expect("new manifest"),
        )
        .expect("complete manifest");
    }
}

#[test]
fn catalog_cache_inputs_are_refused_as_sbom_destinations_before_runtime_lookup() {
    let project = TempProject::named("hello");
    let target = Target::host().name();
    std::fs::write(
        project.manifest(),
        format!(
            "name = 'hello'\nversion = '1.0.0'\n[tools.ginary.target.{target}]\nerts = 'catalog'\n"
        ),
    )
    .expect("catalog options");
    let cache = project.root().join("cache");
    let input = ginary::catalog::cache_root(&cache).join("existing-runtime/bin/emulator");
    std::fs::create_dir_all(input.parent().expect("parent")).expect("runtime directory");
    std::fs::write(&input, b"runtime input").expect("existing cached runtime");
    let alias = project.root().join("outside-cache");
    std::fs::hard_link(&input, &alias).expect("input alias");
    for destination in [&input, &alias] {
        let mut command = assert_cmd::Command::cargo_bin("ginary").expect("CLI");
        let output = command
            .current_dir(project.root())
            .args([
                "build",
                "--skip-export",
                "--no-strip",
                "--sbom",
                "--report",
                "json",
                "--sbom-out",
            ])
            .arg(destination)
            .env("GINARY_CACHE_DIR", &cache)
            .env("PATH", "")
            .assert()
            .code(1)
            .get_output()
            .clone();
        let report: serde_json::Value =
            serde_json::from_slice(&output.stdout).expect("failure report");
        assert_eq!(
            report["stage"], "preflight",
            "an existing runtime input must be refused before trying to discover Erlang: {report}"
        );
        let reason = String::from_utf8_lossy(&output.stderr);
        assert!(
            reason.contains("output") && reason.contains("input"),
            "{reason}"
        );
        assert_eq!(
            std::fs::read(&input).expect("preserved runtime"),
            b"runtime input"
        );
    }
}

#[test]
fn selecting_the_project_manifest_as_a_trace_cannot_modify_it_before_preflight() {
    let project = TempProject::named("hello");
    let opts = complete_options(&project);
    let before = std::fs::read(project.manifest()).expect("project config");
    let mut command = assert_cmd::Command::cargo_bin("ginary").expect("CLI");
    let output = command
        .current_dir(project.root())
        .args(["build", "--skip-export", "--no-strip", "--otp-root"])
        .arg(opts.otp_root.as_ref().expect("runtime"))
        .env("GINARY_TRACE", project.manifest())
        .output()
        .expect("CLI output");
    assert_eq!(
        std::fs::read(project.manifest()).expect("preserved config"),
        before,
        "tracing must refuse a user input before appending the initial operation"
    );
    assert!(
        output.status.success(),
        "safe tracing refusal must let the valid build continue: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(feature = "fault-injection")]
#[test]
fn atomic_publication_keeps_previous_artifact_on_write_or_replace_failure() {
    let project = TempProject::named("hello");
    let opts = complete_options(&project);
    let destination = project.root().join("artifact");
    for fault in ["output-write", "output-persist"] {
        std::fs::write(&destination, b"previous artifact").expect("previous output");
        let mut command = assert_cmd::Command::cargo_bin("ginary").expect("CLI");
        command
            .current_dir(project.root())
            .args(["build", "--skip-export", "--no-strip", "--out"])
            .arg(&destination)
            .arg("--otp-root")
            .arg(opts.otp_root.as_ref().expect("runtime"))
            .env("GINARY_FAULT", format!("{fault}:fail"))
            .assert()
            .code(1);
        assert_eq!(
            std::fs::read(&destination).expect("retained output"),
            b"previous artifact"
        );
        assert!(
            std::fs::read_dir(project.root().join("build/ginary"))
                .expect("build directory")
                .all(|entry| !entry
                    .expect("entry")
                    .file_name()
                    .to_string_lossy()
                    .starts_with(bundle::WORK_DIR_PREFIX)),
            "failed build cleans its work directory"
        );
        let mut retry = assert_cmd::Command::cargo_bin("ginary").expect("CLI");
        retry
            .current_dir(project.root())
            .args(["build", "--skip-export", "--no-strip", "--out"])
            .arg(&destination)
            .arg("--otp-root")
            .arg(opts.otp_root.as_ref().expect("runtime"))
            .env_remove("GINARY_FAULT")
            .assert()
            .success();
        let artifact = ginary::inspect::open(&destination).expect("new complete artifact");
        assert!(ginary::inspect::verify(&artifact).expect("digest").ok());
    }
}

#[cfg(feature = "fault-injection")]
#[test]
fn atomic_publication_keeps_previous_macos_artifact_on_signing_failure() {
    let project = TempProject::named("hello");
    crate::common::fake_otp::FakeShipment::new()
        .app("hello", "1.0.0", &["kernel", "stdlib"])
        .build_in(project.root().join(ginary::gleam::SHIPMENT_DIR));
    let runtime = crate::common::fake_otp::FakeOtp::new()
        .macos()
        .macho_cpu_type(crate::common::native::MACHO_CPU_ARM64)
        .build_in(project.root().join("otp"));
    let target: Target = "macos-aarch64".parse().expect("target");
    let mut stub = crate::common::macho::real_fixture_bytes();
    stub.extend_from_slice(&crate::common::stubfile::Marker::for_target(&target).bytes());
    let stub_path = project.root().join("macos-stub");
    std::fs::write(&stub_path, stub).expect("marked fixture stub");
    let destination = project.root().join("artifact-macos-aarch64");
    for fault in [
        "artifact-sign:corrupt",
        "artifact-sign:fail",
        "output-write:fail",
        "output-persist:fail",
    ] {
        std::fs::write(&destination, b"previous macOS artifact").expect("previous output");
        let mut command = assert_cmd::Command::cargo_bin("ginary").expect("CLI");
        command
            .current_dir(project.root())
            .args([
                "build",
                "--skip-export",
                "--no-strip",
                "--target",
                "macos-aarch64",
                "--out",
                "artifact",
                "--stub",
            ])
            .arg(&stub_path)
            .arg("--otp-root")
            .arg(&runtime.root)
            .env("GINARY_FAULT", fault)
            .assert()
            .code(1);
        assert_eq!(
            std::fs::read(&destination).expect("retained output"),
            b"previous macOS artifact"
        );
        let mut retry = assert_cmd::Command::cargo_bin("ginary").expect("CLI");
        let completed = retry
            .current_dir(project.root())
            .args([
                "build",
                "--skip-export",
                "--no-strip",
                "--target",
                "macos-aarch64",
                "--out",
                "artifact",
                "--stub",
            ])
            .arg(&stub_path)
            .arg("--otp-root")
            .arg(&runtime.root)
            .env_remove("GINARY_FAULT")
            .assert()
            .success()
            .get_output()
            .clone();
        let artifact = ginary::inspect::open(&destination).expect("signed artifact");
        assert!(ginary::inspect::verify(&artifact).expect("digest").ok());
        assert!(
            String::from_utf8_lossy(&completed.stdout)
                .contains(&format!("{} bytes", artifact.total_len)),
            "the CLI must print the measured macOS artifact size including signature overhead: {}",
            String::from_utf8_lossy(&completed.stdout)
        );
    }
}

#[test]
fn a_manifest_copy_failure_reports_the_executable_already_published() {
    let project = TempProject::named("hello");
    let mut opts = complete_options(&project);
    opts.named_targets = true;
    opts.distribution = true;
    let runtime = ginary::otp::inspect_root(opts.otp_root.as_ref().unwrap()).unwrap();
    std::fs::write(
        runtime
            .erts_bin
            .join(format!("epmd{}", Target::host().exe_suffix())),
        b"fixture epmd",
    )
    .unwrap();
    let copy = opts
        .manifest_copy_path(Target::host())
        .expect("manifest copy");
    std::fs::create_dir_all(&copy).expect("a directory obstructs publication");
    let failure = bundle::build_with_stub_detailed(
        &opts,
        &PathBuf::from(env!("CARGO_BIN_EXE_ginary")),
        &Diag::disabled(),
    )
    .expect_err("manifest publication must fail");
    assert_eq!(failure.failed_target, Some(Target::host()));
    assert_eq!(
        failure.completed.len(),
        1,
        "the executable was already published"
    );
    assert!(failure.completed[0].manifest_copy.is_none());
    assert!(
        failure
            .warnings
            .iter()
            .any(|warning| warning == bundle::DISTRIBUTION_NO_NAME),
        "warnings about the published artifact must survive sidecar failure: {failure:?}"
    );
    let artifact = &failure.completed[0].out;
    assert!(
        ginary::inspect::open(artifact).is_ok(),
        "the retained executable is an artifact"
    );
    assert_eq!(
        failure.completed[0].total_len,
        std::fs::metadata(artifact)
            .expect("artifact metadata")
            .len(),
        "the reported length is measured from the finished file"
    );
    assert!(
        failure
            .staging
            .as_ref()
            .expect("staging retained on failure")
            .is_dir()
    );
    assert!(failure.unattempted.is_empty());
    assert!(
        failure
            .to_string()
            .contains(&artifact.display().to_string())
    );
}

#[test]
fn a_failed_retained_build_names_its_staging_and_releases_the_project_lock() {
    let project = TempProject::named("hello");
    let mut opts = complete_options(&project);
    opts.vm_args = Some(project.root().join("missing.args"));
    let stub = PathBuf::from(env!("CARGO_BIN_EXE_ginary"));
    let failure = bundle::build_with_stub_detailed(&opts, &stub, &Diag::disabled())
        .expect_err("missing runtime configuration");
    let retained = failure
        .staging
        .as_ref()
        .expect("failure retains its staging path");
    assert!(retained.join(Target::host().name()).join("root").is_dir());
    assert!(
        failure
            .to_string()
            .contains(&retained.display().to_string())
    );
    opts.vm_args = None;
    let next = bundle::build_with_stub_detailed(&opts, &stub, &Diag::disabled())
        .expect("failure released its project lock");
    assert_ne!(next.staging.as_ref(), Some(retained));
    assert!(
        retained.is_dir(),
        "the next call must preserve earlier evidence"
    );
}

#[test]
fn a_legacy_library_call_preserves_the_published_artifact_and_its_original_failure() {
    let project = TempProject::named("hello");
    let mut opts = complete_options(&project);
    opts.named_targets = true;
    let manifest = opts.manifest_copy_path(Target::host()).unwrap();
    std::fs::create_dir_all(&manifest).unwrap();
    let error = bundle::build_with_stub(
        &opts,
        Path::new(env!("CARGO_BIN_EXE_ginary")),
        &Diag::disabled(),
    )
    .expect_err("manifest publication is blocked after artifact publication");
    let bundle::BundleError::Incomplete(failure) = error else {
        panic!("legacy callers must retain partial evidence: {error}");
    };
    assert_eq!(failure.completed.len(), 1);
    assert_eq!(failure.failed_target, Some(Target::host()));
    assert!(failure.completed[0].manifest_copy.is_none());
    assert!(failure.staging.as_ref().unwrap().is_dir());
    assert!(std::error::Error::source(failure.as_ref()).is_some());
    let executable = &failure.completed[0];
    let artifact = ginary::inspect::open(&executable.out).unwrap();
    assert!(ginary::inspect::verify(&artifact).unwrap().ok());
    assert!(
        executable
            .artifact_line()
            .contains(&executable.out.display().to_string())
    );
    let report = serde_json::to_value(&failure.completed).unwrap();
    assert!(report[0]["manifest_copy"].is_null());
    assert!(
        report[0]["out"]
            .as_str()
            .unwrap()
            .ends_with(Target::host().exe_suffix())
    );
}
