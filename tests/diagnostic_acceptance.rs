// SPDX-License-Identifier: MIT OR Apache-2.0
//! Incident collection and maintenance failures exercised through public APIs.
#![cfg(feature = "cli")]

mod common;

use std::path::{Path, PathBuf};
use std::time::Duration;

use common::script::{self, ShimStep};
use ginary::{cache, diagnose, doctor, strip};

/// Environment-dependent APIs run in their own native test process. This also
/// lets instrumented runs observe library code without changing global state.
fn isolated(name: &str, host_tools: bool, body: impl FnOnce(&Path)) {
    const CASE: &str = "GINARY_DIAGNOSTIC_ACCEPTANCE_CASE";
    const ROOT: &str = "GINARY_DIAGNOSTIC_ACCEPTANCE_ROOT";
    if std::env::var(CASE).ok().as_deref() == Some(name) {
        let root = PathBuf::from(std::env::var_os(ROOT).expect("isolated root"));
        body(&root);
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let bin = dir.path().join("tools");
    std::fs::create_dir(&bin).expect("tool directory");
    let mut paths = vec![bin];
    if host_tools && let Some(existing) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&existing));
    }
    let output = common::bounded::run_bounded(
        std::process::Command::new(std::env::current_exe().expect("test program"))
            .args(["--exact", name, "--nocapture"])
            .env(CASE, name)
            .env(ROOT, dir.path())
            .env("PATH", std::env::join_paths(paths).expect("isolated PATH"))
            .env("GINARY_CACHE_DIR", dir.path().join("cache"))
            .env_remove("ERL_FLAGS")
            .env_remove("ERL_AFLAGS")
            .env_remove("ERL_ZFLAGS")
            .current_dir(dir.path()),
        Duration::from_secs(60),
        name,
    );
    assert!(
        output.status.success(),
        "{name}: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
}

#[test]
fn gathered_diagnosis_reports_unusable_environment_without_copying_local_secrets() {
    isolated(
        "gathered_diagnosis_reports_unusable_environment_without_copying_local_secrets",
        false,
        |root| {
            std::fs::write(root.join("cache"), b"user data").expect("cache path occupied by file");
            std::fs::write(root.join("gleam.toml"), b"\xff").expect("unreadable UTF-8 project");
            script::program(
                &root.join("tools"),
                "gleam",
                &[
                    ShimStep::Print(vec!["private-tool-response".into()]),
                    ShimStep::Exit(7),
                ],
            );
            let report = doctor::DetailedReport::gather();
            let codes: Vec<_> = report
                .findings
                .iter()
                .map(|finding| finding.code.as_str())
                .collect();
            assert!(
                codes.contains(&"cache_unusable") && codes.contains(&"project_unreadable"),
                "{report:?}"
            );
            let text = report.render_text();
            assert!(
                text.contains("private-tool-response") && text.contains("remedy:"),
                "{text}"
            );
            assert_eq!(
                report.tool_probes[0].outcome,
                doctor::ProbeOutcome::NonzeroExit
            );
            let summary = diagnose::gather(None, None, None);
            assert!(
                summary.complete,
                "all requested evidence was collected; unavailable tools remain separate readiness outcomes"
            );
            let environment = summary.environment.as_ref().expect("environment probes");
            assert!(!environment.cache_writable && !environment.cache_executable);
            assert_eq!(
                environment.tools[0].outcome,
                doctor::ProbeOutcome::NonzeroExit
            );
            let json = serde_json::to_string(&summary).expect("summary JSON");
            let text = summary.render_text();
            for output in [&json, &text] {
                assert!(!output.contains("private-tool-response"));
                assert!(!output.contains(&root.display().to_string()));
            }
            assert!(
                text.contains("environment: OTP false") && text.contains("NonzeroExit"),
                "{text}"
            );
            assert_eq!(
                std::fs::read(root.join("cache")).expect("user file"),
                b"user data"
            );
        },
    );
}

#[test]
fn gathered_doctor_explains_invalid_configuration_and_incomplete_native_scans() {
    isolated(
        "gathered_doctor_explains_invalid_configuration_and_incomplete_native_scans",
        false,
        |root| {
            std::fs::write(
                root.join("gleam.toml"),
                "name = \"hello\"\nversion = \"1.0.0\"\n[tools.ginary]\nunknown_option = true\n",
            )
            .expect("invalid ginary configuration");
            let native = root.join("build/erlang-shipment/hello/priv/broken.so");
            std::fs::create_dir_all(native.parent().expect("parent")).expect("shipment");
            std::fs::write(native, b"\x7fELF").expect("malformed native object");
            let report = doctor::DetailedReport::gather();
            let codes: Vec<_> = report
                .findings
                .iter()
                .map(|finding| finding.code.as_str())
                .collect();
            assert!(codes.contains(&"configuration_invalid"), "{report:?}");
            assert!(codes.contains(&"native_scan_incomplete"), "{report:?}");
            let text = report.render_text();
            assert!(
                text.contains("unknown_option") && text.contains("broken.so"),
                "{text}"
            );
        },
    );
}

#[test]
fn gathered_doctor_describes_catalog_readiness_without_claiming_validation() {
    isolated(
        "gathered_doctor_describes_catalog_readiness_without_claiming_validation",
        false,
        |root| {
            std::fs::write(root.join("gleam.toml"),
            "name = \"hello\"\nversion = \"1.0.0\"\n[tools.ginary]\ntargets = [\"linux-x86_64-musl\"]\n[tools.ginary.target.linux-x86_64-musl]\nerts = \"catalog\"\n")
            .expect("catalog configuration");
            let report = doctor::DetailedReport::gather();
            assert!(
                report
                    .findings
                    .iter()
                    .any(|finding| finding.code == "runtime_unchecked"),
                "{report:?}"
            );
            assert!(
                report
                    .render_text()
                    .contains("not been downloaded or validated")
            );
            assert!(
                !root.join("dist").exists(),
                "diagnosis must not fetch a runtime"
            );
        },
    );
}

#[test]
fn library_doctor_refuses_invalid_runtime_sources_before_calling_a_resolver() {
    let target = ginary::target::Target::host();
    let config = std::collections::BTreeMap::from([(
        target.name(),
        ginary::config::TargetConfig {
            erts: Some("not-a-runtime-source".into()),
            ..Default::default()
        },
    )]);
    let rows = doctor::probe_targets_with(&[target], &config, |_, _| {
        panic!("invalid configuration must never reach runtime resolution")
    });
    assert_eq!(rows.len(), 1);
    assert!(!rows[0].resolvable);
    assert_eq!(rows[0].erts, "not-a-runtime-source");
    assert!(
        rows[0]
            .detail
            .as_ref()
            .is_some_and(|detail| !detail.is_empty())
    );
}

#[test]
fn broken_project_text_keeps_a_usable_identity_and_reports_the_configuration_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("gleam.toml"), b"name = [").expect("broken manifest");
    let report =
        doctor::project_context(dir.path(), std::time::SystemTime::now()).expect("project context");
    assert_eq!(
        report.name,
        dir.path()
            .file_name()
            .expect("directory name")
            .to_string_lossy()
    );
    assert!(report.version.is_none());
    assert!(matches!(report.config, doctor::ConfigStatus::Error { .. }));
    let text = report.render();
    assert!(text.contains("shipment: none exported yet") && text.contains("[tools.ginary]:"));
}

#[test]
fn doctor_renders_truncated_tool_evidence_and_rejects_an_empty_parsed_version() {
    isolated(
        "doctor_renders_truncated_tool_evidence_and_rejects_an_empty_parsed_version",
        false,
        |root| {
            let program = script::program(
                &root.join("tools"),
                "noisy",
                &[ShimStep::Print(vec![
                    "x".repeat(doctor::PROBE_CAPTURE_LIMIT + 31),
                ])],
            );
            let probe = doctor::probe_version(
                "noisy",
                Some(&program),
                &[],
                Duration::from_secs(10),
                |_| Some("1.0".into()),
            );
            assert_eq!(probe.outcome, doctor::ProbeOutcome::IncompleteOutput);
            assert!(probe.stdout.omitted_bytes > 0);
            let mut report = doctor::DetailedReport::gather();
            report.tool_probes = vec![probe];
            let text = report.render_text();
            assert!(
                text.contains("stdout: incomplete (") && text.contains("bytes omitted"),
                "missing bounded evidence"
            );
            let quiet = script::program(
                &root.join("tools"),
                "quiet",
                &[ShimStep::Print(vec!["version".into()])],
            );
            let empty =
                doctor::probe_version("quiet", Some(&quiet), &[], Duration::from_secs(10), |_| {
                    Some("   ".into())
                });
            assert_eq!(empty.outcome, doctor::ProbeOutcome::InvalidOutput);
            assert!(empty.tool.version.is_none());
        },
    );
}

#[test]
fn diagnosis_distinguishes_non_files_invalid_records_and_run_limit_exhaustion() {
    let dir = tempfile::tempdir().expect("tempdir");
    let absent = dir.path().join("absent");
    let missing = diagnose::summarize(None, Some(&absent), Some(dir.path()));
    assert!(!missing.complete);
    assert!(
        missing
            .findings
            .iter()
            .any(|finding| finding.code == "unreadable_trace")
    );
    assert!(
        missing
            .findings
            .iter()
            .any(|finding| finding.code == "unreadable_crashdump")
    );
    let trace = dir.path().join("trace.jsonl");
    let mut lines = vec!["[]".to_owned(), "not-json".into(), "{\"schema_version\":99}".into(),
        "{\"schema_version\":\"2\"}".into(),
        "{\"schema_version\":2,\"t_us\":0,\"phase\":\"x\",\"kv\":{},\"event\":\"unknown\",\"run_id\":\"private\",\"sequence\":0}".into()];
    for run in 0..=diagnose::MAX_TRACE_RUNS {
        lines.push(
            serde_json::json!({"schema_version":2,"t_us":0,"phase":"test","kv":{},
            "event":"interrupted","run_id":format!("private-run-{run}"),"sequence":0})
            .to_string(),
        );
    }
    std::fs::write(&trace, lines.join("\n")).expect("trace");
    let report = diagnose::summarize(None, Some(&trace), None);
    let summary = report.trace.as_ref().expect("trace summary");
    assert_eq!(summary.invalid_lines, 5);
    assert_eq!(summary.unsupported_lines, 2);
    assert_eq!(summary.run_count, diagnose::MAX_TRACE_RUNS);
    assert!(summary.limit_reached && !summary.complete && !report.complete);
    assert!(summary.runs.iter().all(|run| run.interrupted == 1));
    assert!(!report.render_text().contains("private-run-"));
}

#[test]
fn diagnosis_reports_crash_prefix_limits_and_completed_artifact_findings_honestly() {
    let dir = tempfile::tempdir().expect("tempdir");
    let dump = dir.path().join("erl_crash.dump");
    std::fs::write(
        &dump,
        b"=erl_crash_dump:0.5\nMon\n=proc:<0.1.0>\nStack+heap: 123\n",
    )
    .expect("truncated dump");
    let partial = diagnose::summarize(None, None, Some(&dump));
    assert!(!partial.complete);
    assert!(partial.render_text().contains("123 words; incomplete"));
    let file = std::fs::OpenOptions::new()
        .write(true)
        .open(&dump)
        .expect("dump file");
    file.set_len(diagnose::MAX_CRASHDUMP_BYTES + 1)
        .expect("oversized dump");
    drop(file);
    let bounded = diagnose::summarize(None, None, Some(&dump));
    assert!(
        bounded
            .crashdump
            .as_ref()
            .expect("bounded summary")
            .limit_reached
    );
    assert!(!bounded.complete);
    std::fs::write(&dump, b"not a crash dump").expect("invalid dump");
    let invalid = diagnose::summarize(None, None, Some(&dump));
    assert!(
        invalid
            .findings
            .iter()
            .any(|finding| finding.code == "unreadable_crashdump")
    );

    let artifact = common::repack::build(
        dir.path(),
        &common::repack::RepackOptions {
            ghost_index_rows: vec!["missing-file.txt".into()],
            ..Default::default()
        },
    );
    let bad = diagnose::summarize(Some(artifact.path()), None, None);
    assert!(
        bad.artifact
            .as_ref()
            .expect("artifact summary")
            .issues
            .is_some_and(|count| count > 0)
    );
    assert!(bad.render_text().contains("verification failed"));
    let good_dir = tempfile::tempdir().expect("valid artifact directory");
    let artifact = common::repack::build(good_dir.path(), &Default::default());
    let good = diagnose::summarize(Some(artifact.path()), None, None);
    assert!(good.artifact.as_ref().expect("artifact summary").verified);
    assert!(good.render_text().contains("artifact: verified"));
}

fn owned_entry(root: &Path) -> PathBuf {
    let app = root.join("hello");
    common::cachefs::plant_entry(&app, "0123456789abcdef", Duration::from_secs(3600))
}

#[test]
fn maintenance_preserves_non_directory_applications_and_invalid_marker_shapes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("hello");
    std::fs::write(&file, b"user data").expect("user application name");
    assert!(
        cache::uninstall(&file)
            .kept
            .contains(&(file.clone(), cache::KeptReason::Unowned))
    );
    assert!(
        cache::prune_app(
            &file,
            None,
            cache::PruneOptions { all: true, days: 0 },
            std::time::SystemTime::now(),
            &ginary::diag::Diag::disabled()
        )
        .kept
        .contains(&(file.clone(), cache::KeptReason::Unowned))
    );
    assert!(
        cache::sweep(&file, std::process::id(), &ginary::diag::Diag::disabled())
            .expect("sweep")
            .kept
            .contains(&file)
    );
    assert_eq!(std::fs::read(&file).expect("preserved file"), b"user data");
    std::fs::remove_file(file).expect("remove owned test file");
    let entry = owned_entry(dir.path());
    let marker = entry.join("ginary.json");
    std::fs::remove_file(&marker).expect("remove fixture manifest");
    std::fs::create_dir(&marker).expect("foreign marker directory");
    assert!(
        cache::clean_detailed(dir.path(), Some("hello"))
            .expect("clean")
            .kept
            .contains(&(entry.clone(), cache::KeptReason::Unowned))
    );
    std::fs::remove_dir(&marker).expect("remove test directory");
    let marker_file = std::fs::File::create(&marker).expect("oversized marker");
    marker_file
        .set_len(ginary::payload::MAX_FRONT_ENTRY_BYTES + 1)
        .expect("marker bound");
    drop(marker_file);
    assert!(
        cache::clean_detailed(dir.path(), Some("hello"))
            .expect("clean")
            .kept
            .contains(&(entry, cache::KeptReason::Unowned))
    );
}

#[cfg(windows)]
#[test]
fn maintenance_reports_windows_read_and_delete_refusals_then_recovers() {
    use std::os::windows::fs::OpenOptionsExt as _;
    let dir = tempfile::tempdir().expect("tempdir");
    let entry = owned_entry(dir.path());
    let marker = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(entry.join("ginary.json"))
        .expect("deny marker readers");
    let report = cache::clean_detailed(dir.path(), Some("hello")).expect("clean unreadable marker");
    assert!(
        report
            .kept
            .contains(&(entry.clone(), cache::KeptReason::Unowned)),
        "{report:?}"
    );
    drop(marker);
    let note = entry.join("open-file");
    std::fs::write(&note, b"retained while open").expect("open file");
    let held = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&note)
        .expect("deny removal");
    let report = cache::clean_detailed(dir.path(), Some("hello")).expect("clean held file");
    assert!(
        report
            .kept
            .contains(&(entry.clone(), cache::KeptReason::Unremovable)),
        "{report:?}"
    );
    assert!(report.removed.is_empty());
    drop(held);
    assert_eq!(
        std::fs::read(&note).expect("file retained"),
        b"retained while open"
    );
    assert_eq!(
        cache::clean_detailed(dir.path(), Some("hello"))
            .expect("clean after close")
            .removed,
        [entry]
    );
}

#[cfg(windows)]
#[test]
fn cache_preparation_reports_a_file_collision_without_overwriting_or_falling_back() {
    let dir = tempfile::tempdir().expect("tempdir");
    let occupied = dir.path().join("cache");
    std::fs::write(&occupied, b"user data").expect("occupied cache");
    let environment = cache::Env::from_pairs([
        ("GINARY_CACHE_DIR".into(), occupied.as_os_str().to_owned()),
        ("TEMP".into(), dir.path().join("fallback").into_os_string()),
    ]);
    let mut warning = Vec::new();
    assert!(cache::prepare_windows(&environment, &mut warning).is_err());
    assert!(warning.is_empty());
    assert!(!dir.path().join("fallback").exists());
    assert_eq!(std::fs::read(occupied).expect("user bytes"), b"user data");
}

#[cfg(windows)]
#[test]
fn an_unreadable_native_file_is_reported_by_doctor_and_refused_by_strip() {
    use std::os::windows::fs::OpenOptionsExt as _;
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("gleam.toml"), b"name = \"hello\"\n").expect("project");
    let shipment = dir.path().join("build/erlang-shipment");
    let native = shipment.join("hello/priv/held.so");
    std::fs::create_dir_all(native.parent().expect("parent")).expect("shipment");
    let original = host_elf();
    std::fs::write(&native, &original).expect("native object");
    let held = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&native)
        .expect("deny native readers");
    let report = doctor::project_context(dir.path(), std::time::SystemTime::now())
        .expect("project diagnosis");
    assert!(report.native.is_empty());
    assert!(
        report
            .native_notes
            .iter()
            .any(|note| note.contains("held.so") && note.contains("incomplete")),
        "{report:?}"
    );
    let otp = common::fake_otp::FakeOtp::new().build_in(dir.path().join("otp"));
    let info = ginary::otp::inspect_root(&otp.root).expect("OTP");
    assert!(
        matches!(strip::strip(&shipment, &info, &strip::StripOptions { elf: true, beams: false }),
        Err(strip::StripError::Io { path, .. }) if path == native)
    );
    drop(held);
    assert_eq!(std::fs::read(native).expect("source preserved"), original);
}

fn host_elf() -> Vec<u8> {
    let machine = match ginary::target::Target::host().arch {
        ginary::target::Arch::X86_64 => 62,
        ginary::target::Arch::Aarch64 => 183,
    };
    common::repack::patch_elf_machine(&common::repack::test_binary(), machine)
}

#[test]
fn native_stripping_rechecks_tool_results_and_retains_failure_context_on_every_host() {
    isolated(
        "native_stripping_rechecks_tool_results_and_retains_failure_context_on_every_host",
        true,
        |root| {
            let otp = common::fake_otp::FakeOtp::new().build_in(root.join("otp"));
            let info = ginary::otp::inspect_root(&otp.root).expect("OTP fixture");
            let original = host_elf();
            for case in [
                "success",
                "not_elf",
                "malformed",
                "machine",
                "stderr",
                "stdout",
                "spawn",
            ] {
                let staged = root.join(case);
                std::fs::create_dir(&staged).expect("staged directory");
                let file = staged.join("native.beam");
                let mut input = original.clone();
                input.extend_from_slice(&[0; 512]);
                std::fs::write(&file, &input).expect("real ELF input");
                let steps = match case {
                    "stderr" => vec![ShimStep::PrintStderrFile, ShimStep::Exit(7)],
                    "stdout" => vec![
                        ShimStep::Print(vec!["rewrite refused on stdout".into()]),
                        ShimStep::Exit(7),
                    ],
                    _ => vec![ShimStep::RecordArgv, ShimStep::ReplaceBeamArguments],
                };
                let program = script::program(&root.join("tools"), "strip", &steps);
                std::fs::write(
                    script::shim_sidecar(&program, "stderr"),
                    b"rewrite refused on stderr",
                )
                .expect("tool error");
                let replacement = match case {
                    "not_elf" => b"not an executable".to_vec(),
                    "malformed" => b"\x7fELF".to_vec(),
                    "machine" => common::repack::patch_elf_machine(
                        &original,
                        if ginary::target::Target::host().arch == ginary::target::Arch::X86_64 {
                            183
                        } else {
                            62
                        },
                    ),
                    _ => original.clone(),
                };
                std::fs::write(script::shim_sidecar(&program, "module"), replacement)
                    .expect("tool replacement");
                if case == "spawn" {
                    let invalid: &[u8] = if cfg!(windows) {
                        b"invalid native program"
                    } else {
                        b"#!/ginary-test-nonexistent-interpreter\n"
                    };
                    std::fs::write(&program, invalid).expect("unstartable strip");
                }
                if case == "success" {
                    let foreign = common::repack::patch_elf_machine(
                        &original,
                        if ginary::target::Target::host().arch == ginary::target::Arch::X86_64 {
                            183
                        } else {
                            62
                        },
                    );
                    std::fs::write(staged.join("foreign-object"), foreign)
                        .expect("foreign native object");
                    std::fs::write(staged.join("windows-object"), b"MZ\0\0")
                        .expect("other container");
                }
                let result = strip::strip(
                    &staged,
                    &info,
                    &strip::StripOptions {
                        elf: true,
                        beams: false,
                    },
                );
                match case {
                    "success" => {
                        let report = result.expect("native strip succeeds");
                        assert!(matches!(
                            report.elf,
                            strip::ElfOutcome::Stripped { files: 1, .. }
                        ));
                        assert_eq!(report.saved(), 512);
                        assert_eq!(report.warnings.len(), 2);
                        assert_eq!(std::fs::read(&file).expect("stripped ELF"), original);
                    }
                    "not_elf" => assert!(
                        matches!(result, Err(strip::StripError::NotElfAfterStrip { path }) if path == file)
                    ),
                    "malformed" => assert!(
                        matches!(result, Err(strip::StripError::Elf { path, .. }) if path == file)
                    ),
                    "machine" => assert!(
                        matches!(result, Err(strip::StripError::ElfChanged { path, .. }) if path == file)
                    ),
                    "stderr" | "stdout" => {
                        let error = result.expect_err("tool refused rewrite");
                        assert!(
                            error
                                .to_string()
                                .contains(&format!("rewrite refused on {case}")),
                            "{error}"
                        );
                        assert_eq!(std::fs::read(&file).expect("unchanged on refusal"), input);
                    }
                    "spawn" => assert!(
                        matches!(result, Err(strip::StripError::StripProcess { path, .. }) if path == file)
                    ),
                    _ => unreachable!(),
                }
            }
        },
    );
}

#[test]
fn absent_native_stripper_is_a_reported_skip_and_an_empty_beam_set_needs_no_process() {
    isolated(
        "absent_native_stripper_is_a_reported_skip_and_an_empty_beam_set_needs_no_process",
        false,
        |root| {
            let otp = common::fake_otp::FakeOtp::new()
                .with_erl_script()
                .build_in(root.join("otp"));
            let info = ginary::otp::inspect_root(&otp.root).expect("OTP fixture");
            let staged = root.join("staged");
            std::fs::create_dir(&staged).expect("staged root");
            let report = strip::strip(
                &staged,
                &info,
                &strip::StripOptions {
                    elf: false,
                    beams: true,
                },
            )
            .expect("empty modules");
            assert!(matches!(
                report.beams,
                strip::BeamOutcome::Stripped {
                    files: 0,
                    before: 0,
                    after: 0
                }
            ));
            assert!(otp.erl_argv().is_empty());
            let file = staged.join("native");
            std::fs::write(&file, host_elf()).expect("real ELF");
            let report = strip::strip(
                &staged,
                &info,
                &strip::StripOptions {
                    elf: true,
                    beams: false,
                },
            )
            .expect("reported tool absence");
            assert!(
                matches!(report.elf, strip::ElfOutcome::Skipped { reason } if reason.contains("not on PATH") && reason.contains("kept their debug information"))
            );
            assert!(file.is_file());
        },
    );
}
