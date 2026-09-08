// SPDX-License-Identifier: MIT OR Apache-2.0
//! Exercise complete cross-target packaging, sidecars and partial failure through the CLI.
use crate::common::{
    fake_otp::{FakeOtp, FakeShipment},
    native,
    project::TempProject,
    stubfile,
};
use ginary::target::Target;

const TARGETS: [&str; 3] = ["linux-x86_64-gnu", "linux-aarch64-gnu", "linux-x86_64-musl"];

fn project() -> TempProject {
    let project = TempProject::named("hello");
    FakeShipment::new()
        .app("hello", "1.0.0", &["kernel", "stdlib"])
        .build_in(project.root().join(ginary::gleam::SHIPMENT_DIR));
    let stubs = project.subdir("stubs");
    let mut config = "name = \"hello\"\nversion = \"1.0.0\"\n".to_owned();
    for (index, name) in TARGETS.iter().enumerate() {
        let target: Target = name.parse().unwrap();
        let machine = if index == 1 { 183 } else { 62 };
        let interp = if index == 2 {
            "/lib/ld-musl-x86_64.so.1"
        } else if index == 1 {
            "/lib/ld-linux-aarch64.so.1"
        } else {
            "/lib64/ld-linux-x86-64.so.2"
        };
        let runtime = FakeOtp::new().build_in(project.root().join(format!("otp-{index}")));
        std::fs::write(
            runtime.erts_bin().join("beam.smp"),
            native::elf_bytes(machine, native::ET_EXEC, Some(interp)),
        )
        .unwrap();
        let mut stub = native::elf_bytes(machine, native::ET_EXEC, Some(interp));
        stub.extend_from_slice(&stubfile::Marker::for_target(&target).bytes());
        stubfile::write_executable(
            &stubs,
            &stubfile::stub_file_name(stubfile::VERSION, &target),
            &stub,
        );
        config.push_str(&format!(
            "\n[tools.ginary.target.{name}]\nerts = 'dir:{}'\n",
            runtime.root.display()
        ));
    }
    std::fs::write(project.manifest(), config).unwrap();
    project
}

fn command(project: &TempProject) -> assert_cmd::Command {
    let mut command = assert_cmd::Command::cargo_bin("ginary").unwrap();
    command
        .current_dir(project.root())
        .env("GINARY_STUB_DIR", project.root().join("stubs"))
        .env("GINARY_CACHE_DIR", project.root().join("cache"))
        .args([
            "build",
            "--skip-export",
            "--no-strip",
            "--keep-staging",
            "--report",
            "json",
            "--sbom",
        ]);
    command
}

#[test]
fn no_strip_cross_builds_need_no_host_erlang_and_write_every_sbom() {
    let project = project();
    let output = command(&project)
        .args([
            "--target",
            TARGETS[0],
            "--target",
            TARGETS[1],
            "--sbom-out",
            "boms",
        ])
        .assert()
        .success()
        .get_output()
        .clone();
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["format_version"], 2);
    assert_eq!(report["status"], "success");
    assert_eq!(report["targets"].as_array().unwrap().len(), 2);
    assert_eq!(report["sboms"].as_array().unwrap().len(), 2);
    for target in &TARGETS[..2] {
        assert!(
            project
                .root()
                .join(format!("boms/hello-{target}.spdx.json"))
                .is_file()
        );
        let artifact = project.root().join(format!("build/ginary/hello-{target}"));
        assert!(ginary::inspect::open(&artifact).is_ok());
    }
}

#[test]
fn second_target_failure_retains_first_artifact_and_sbom_and_names_unattempted_target() {
    let project = project();
    std::fs::remove_file(project.root().join("otp-1/bin/no_dot_erlang.boot")).unwrap();
    let output = command(&project)
        .args(TARGETS.iter().flat_map(|t| ["--target", *t]))
        .assert()
        .code(1)
        .get_output()
        .clone();
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("one complete partial report");
    assert_eq!(report["status"], "failed");
    assert_eq!(report["targets"].as_array().unwrap().len(), 1, "{report}");
    assert_eq!(report["failed_target"], TARGETS[1]);
    assert_eq!(report["unattempted"], serde_json::json!([TARGETS[2]]));
    assert_eq!(report["sboms"].as_array().unwrap().len(), 1);
    assert!(std::path::Path::new(report["staging"].as_str().unwrap()).is_dir());
    let first = project
        .root()
        .join(format!("build/ginary/hello-{}", TARGETS[0]));
    assert!(ginary::inspect::open(&first).is_ok());
}

#[test]
fn sbom_alias_is_refused_before_any_artifact_is_replaced() {
    let project = project();
    let output = project.root().join("existing");
    std::fs::write(&output, b"previous artifact").unwrap();
    let result = command(&project)
        .args(["--out", "existing", "--sbom-out", "existing"])
        .assert()
        .code(1)
        .get_output()
        .clone();
    assert!(String::from_utf8_lossy(&result.stderr).contains("output"));
    assert_eq!(std::fs::read(&output).unwrap(), b"previous artifact");
}

#[test]
fn sbom_cannot_replace_an_implicitly_located_cross_target_stub() {
    let project = project();
    let target: Target = TARGETS
        .iter()
        .map(|name| name.parse::<Target>().unwrap())
        .find(|target| *target != Target::host())
        .unwrap();
    let stub = project
        .root()
        .join("stubs")
        .join(stubfile::stub_file_name(stubfile::VERSION, &target));
    let before = std::fs::read(&stub).unwrap();
    let output = command(&project)
        .args(["--target", &target.name(), "--sbom-out"])
        .arg(&stub)
        .assert()
        .code(1)
        .get_output()
        .clone();
    assert!(String::from_utf8_lossy(&output.stderr).contains("stub"));
    assert_eq!(std::fs::read(&stub).unwrap(), before);
    assert!(
        !project
            .root()
            .join(format!("build/ginary/hello-{}", target.name()))
            .exists()
    );
}
