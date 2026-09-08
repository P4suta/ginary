// SPDX-License-Identifier: MIT OR Apache-2.0
//! Diagnosis must distinguish broken tools and incomplete local evidence.

use crate::common::script::{self, ShimStep};

fn version(text: &str) -> Option<String> {
    text.trim().strip_prefix("version ").map(str::to_owned)
}

#[test]
fn f1_doctor_cache_probe_preserves_preexisting_files_and_hardlinks() {
    const CASE: &str = "GINARY_TEST_DOCTOR_PROBE_COLLISION";
    if let Some(case) = std::env::var_os(CASE) {
        let dir = tempfile::tempdir().expect("tempdir");
        let cache = dir.path().join("cache");
        std::fs::create_dir(&cache).expect("cache directory");
        let existing = cache.join(format!(
            ".ginary-doctor-probe-{}-0{}",
            std::process::id(),
            ginary::platform::probe_suffix(ginary::platform::HOST)
        ));
        let original = b"user evidence must survive";
        let source = dir.path().join("important.txt");
        std::fs::write(&source, original).expect("user file");
        if case == "hardlink" {
            std::fs::hard_link(&source, &existing).expect("existing hard link");
        } else {
            std::fs::write(&existing, original).expect("existing file");
        }
        let report = ginary::doctor::probe_cache_dir(&cache);
        assert_eq!(std::fs::read(&source).expect("source survives"), original);
        assert_eq!(
            std::fs::read(&existing).expect("preexisting name survives"),
            original,
            "a diagnostic must not overwrite a preexisting file: {report:?}"
        );
        assert!(report.writable && report.executable, "{report:?}");
        assert_eq!(
            std::fs::read_dir(&cache).expect("cache contents").count(),
            1
        );
        return;
    }
    for case in ["file", "hardlink"] {
        // A fresh test process gives the original deterministic probe sequence
        // zero without racing other tests or changing the parent environment.
        let test = concat!(
            "f1_diagnosis_lost_failure_evidence::",
            "f1_doctor_cache_probe_preserves_preexisting_files_and_hardlinks"
        );
        let output = crate::common::bounded::run_bounded(
            std::process::Command::new(std::env::current_exe().expect("test binary"))
                .args(["--exact", test, "--nocapture"])
                .env(CASE, case),
            std::time::Duration::from_secs(30),
            "isolated doctor cache collision",
        );
        assert!(
            output.status.success(),
            "{case}: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
    }
}

#[test]
fn f1_build_preflight_failure_still_produces_the_requested_json_report() {
    let dir = tempfile::tempdir().expect("tempdir");
    let manifest = dir.path().join("gleam.toml");
    let original = b"name = \"hello\"\nversion = \"1.0.0\"\n";
    std::fs::write(&manifest, original).expect("project manifest");
    let trace = dir.path().join("preflight.jsonl");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_ginary"))
        .args([
            "build",
            "--skip-export",
            "--no-strip",
            "--report",
            "json",
            "--sbom-out",
            "gleam.toml",
        ])
        .env("GINARY_CACHE_DIR", dir.path().join("cache"))
        .env("GINARY_TRACE", &trace)
        .current_dir(dir.path())
        .output()
        .expect("build");
    assert!(!output.status.success());
    assert_eq!(
        std::fs::read(&manifest).expect("manifest survives"),
        original
    );
    assert!(
        !output.stdout.is_empty(),
        "preflight discarded the requested JSON failure report: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("failure JSON");
    assert_eq!(report["status"], "failed");
    assert_eq!(report["format_version"], 2);
    assert_eq!(
        report["targets"]
            .as_array()
            .expect("completed targets")
            .len(),
        0
    );
    assert_build_failure_trace(&trace);
}

fn assert_build_failure_trace(path: &std::path::Path) {
    let trace = std::fs::read_to_string(path).expect("build failure trace");
    assert!(
        trace
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("JSON event"))
            .any(|event| event["phase"] == "build" && event["event"] == "failure"),
        "preflight has no failed build outcome: {trace}"
    );
}

#[test]
fn f1_build_reports_missing_and_invalid_projects_as_json_failures() {
    for invalid in [false, true] {
        let dir = tempfile::tempdir().expect("tempdir");
        if invalid {
            std::fs::write(dir.path().join("gleam.toml"), b"name = [").expect("invalid manifest");
        }
        let trace = dir.path().join("failure.jsonl");
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_ginary"))
            .args(["build", "--report", "json"])
            .env("GINARY_TRACE", &trace)
            .current_dir(dir.path())
            .output()
            .expect("build");
        assert!(!output.status.success());
        assert!(
            !output.stdout.is_empty(),
            "missing preflight report; invalid={invalid}"
        );
        let report: serde_json::Value =
            serde_json::from_slice(&output.stdout).expect("failure JSON");
        assert_eq!(report["status"], "failed");
        assert_eq!(
            report["targets"]
                .as_array()
                .expect("completed targets")
                .len(),
            0
        );
        assert_build_failure_trace(&trace);
    }
}

#[cfg(windows)]
#[test]
fn f1_opening_a_trace_retries_another_recorders_short_write_lock() {
    use std::os::windows::fs::OpenOptionsExt;
    let dir = tempfile::tempdir().expect("tempdir");
    let trace = dir.path().join("shared.jsonl");
    let locked = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .share_mode(0)
        .open(&trace)
        .expect("exclusive trace writer");
    let (started, ready) = std::sync::mpsc::channel();
    let release = std::thread::spawn(move || {
        started.send(()).expect("writer ready");
        std::thread::sleep(std::time::Duration::from_millis(20));
        drop(locked);
    });
    ready.recv().expect("writer ready");
    let diag = ginary::diag::Diag::from_env(&ginary::diag::EnvSnapshot {
        ginary_debug: None,
        ginary_trace: Some(trace.as_os_str().to_owned()),
    });
    release.join().expect("release writer");
    assert!(
        diag.is_enabled(),
        "a brief competing write disabled the whole recorder: {:?}",
        diag.health()
    );
    diag.kv("after_contention", &[]);
    assert_eq!(
        std::fs::read_to_string(&trace)
            .expect("trace")
            .lines()
            .count(),
        1
    );
}

#[test]
fn f1_adjacent_urls_do_not_hide_a_second_credential() {
    let value = ginary::diag::redact(
        "error",
        "mirrors=[https://a.invalid/x,https://user:private-secret@b.invalid/y]",
    );
    assert!(
        !value.contains("private-secret"),
        "a second URL credential survived default redaction: {value}"
    );
}

#[test]
fn f1_relative_trace_remains_at_its_original_location_after_changing_directory() {
    const CASE: &str = "GINARY_F1_TRACE_CWD_CASE";
    if let Some(moved) = std::env::var_os(CASE) {
        let diag = ginary::diag::Diag::from_env(&ginary::diag::EnvSnapshot {
            ginary_debug: None,
            ginary_trace: Some("relative.jsonl".into()),
        });
        diag.kv("before", &[]);
        std::env::set_current_dir(moved).expect("change child working directory");
        diag.kv("after", &[]);
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let moved = dir.path().join("moved");
    std::fs::create_dir(&moved).expect("second directory");
    let output = std::process::Command::new(std::env::current_exe().expect("test executable"))
        .args(["--exact", "f1_diagnosis_lost_failure_evidence::f1_relative_trace_remains_at_its_original_location_after_changing_directory", "--nocapture"])
        .env(CASE, &moved).current_dir(dir.path()).output().expect("isolated test child");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let original =
        std::fs::read_to_string(dir.path().join("relative.jsonl")).expect("original trace");
    assert_eq!(
        original.lines().count(),
        2,
        "the trace moved with the working directory: {original}"
    );
    assert!(!moved.join("relative.jsonl").exists());
}

#[test]
fn f1_diagnose_accepts_events_emitted_by_the_current_recorder() {
    let dir = tempfile::tempdir().expect("tempdir");
    let trace = dir.path().join("recorded.jsonl");
    let diag = ginary::diag::Diag::with_sinks(
        None,
        Some(Box::new(std::fs::File::create(&trace).expect("trace"))),
    );
    diag.operation("probe").finish(false, &[]);
    drop(diag);
    let report = ginary::diagnose::summarize(None, Some(&trace), None);
    let trace = report.trace.expect("trace summary");
    assert_eq!(trace.events, 2, "{trace:?}");
    assert_eq!(trace.invalid_lines, 0);
    assert_eq!(trace.outcomes.get("start"), Some(&1));
}

#[test]
fn f1_doctor_keeps_timeout_output_and_rejects_incomplete_version_output() {
    use ginary::doctor::{self, ProbeOutcome};
    let dir = tempfile::tempdir().expect("tempdir");
    let slow = script::program(
        dir.path(),
        "slow",
        &[
            ShimStep::Print(vec!["version 1.0".into()]),
            ShimStep::Sleep(10000),
        ],
    );
    let timed = doctor::probe_version(
        "slow",
        Some(&slow),
        &[],
        std::time::Duration::from_secs(2),
        version,
    );
    assert_eq!(timed.outcome, ProbeOutcome::TimedOut);
    assert!(timed.stdout.text.contains("version 1.0"));
    assert!(timed.tool.version.is_none());
    let loud = script::program(
        dir.path(),
        "loud",
        &[ShimStep::Print(vec![
            "a".repeat(doctor::PROBE_CAPTURE_LIMIT + 100),
            "version 1.0".into(),
        ])],
    );
    let capped = doctor::probe_version(
        "loud",
        Some(&loud),
        &[],
        std::time::Duration::from_secs(5),
        version,
    );
    assert_eq!(capped.outcome, ProbeOutcome::IncompleteOutput);
    assert!(capped.stdout.omitted_bytes > 0);
    assert!(capped.stdout.text.len() <= doctor::PROBE_CAPTURE_LIMIT);
    assert!(capped.tool.version.is_none());
}

#[test]
fn f1_doctor_distinguishes_missing_spawn_failure_and_invalid_output() {
    use ginary::doctor::{self, ProbeOutcome};
    let dir = tempfile::tempdir().expect("tempdir");
    let missing = doctor::probe_version(
        "absent",
        None,
        &[],
        std::time::Duration::from_secs(5),
        version,
    );
    assert_eq!(missing.outcome, ProbeOutcome::Missing);
    let broken = doctor::probe_version(
        "broken",
        Some(&dir.path().join("missing.exe")),
        &[],
        std::time::Duration::from_secs(5),
        version,
    );
    assert_eq!(broken.outcome, ProbeOutcome::SpawnFailed);
    let wrong = script::program(
        dir.path(),
        "wrong",
        &[ShimStep::Print(vec!["not a version".into()])],
    );
    let invalid = doctor::probe_version(
        "wrong",
        Some(&wrong),
        &[],
        std::time::Duration::from_secs(5),
        version,
    );
    assert_eq!(invalid.outcome, ProbeOutcome::InvalidOutput);
}

#[test]
fn f1_diagnose_bounds_trace_lines_and_does_not_copy_dump_terms() {
    let dir = tempfile::tempdir().expect("tempdir");
    let trace = dir.path().join("trace");
    std::fs::write(
        &trace,
        format!(
            "{}\n{{\"t_us\":1,\"phase\":\"exec\",\"kv\":{{}}}}\n",
            "x".repeat(ginary::diagnose::MAX_TRACE_LINE_BYTES + 1)
        ),
    )
    .expect("trace");
    let dump = dir.path().join("dump");
    std::fs::write(&dump, "=erl_crash_dump:0.5\ndate\nSlogan: private-secret\n=proc:<0.1.0>\nName: private-secret\nStack+heap: 123\n=end\n").expect("dump");
    let report = ginary::diagnose::summarize(None, Some(&trace), Some(&dump));
    assert_eq!(report.trace.as_ref().expect("trace").oversized_lines, 1);
    assert_eq!(report.trace.as_ref().expect("trace").events, 1);
    assert!(!report.complete);
    assert_eq!(
        report.crashdump.as_ref().expect("dump").largest_heap_words,
        123
    );
    assert!(
        !serde_json::to_string(&report)
            .expect("json")
            .contains("private-secret")
    );
}

#[test]
fn f1_diagnose_never_executes_the_supplied_artifact_and_keeps_other_evidence() {
    let dir = tempfile::tempdir().expect("tempdir");
    let program = script::program(
        dir.path(),
        "untrusted",
        &[ShimStep::RecordArgv, ShimStep::Exit(9)],
    );
    let report = ginary::diagnose::summarize(
        Some(&program),
        Some(&dir.path().join("missing-trace")),
        None,
    );
    assert!(
        !script::shim_sidecar(&program, "argv").exists(),
        "diagnose executed its input"
    );
    assert!(!report.complete);
    assert_eq!(report.findings.len(), 2);
    let value = serde_json::to_string(&report).expect("JSON");
    assert!(!value.contains(&dir.path().display().to_string()));
}

#[test]
fn f1_diagnose_verifies_a_packaged_runtime_without_extracting_or_running_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = crate::common::artifact::SyntheticArtifact::build_with_runtime_steps(
        dir.path(),
        &[ShimStep::RecordArgv, ShimStep::Exit(9)],
    );
    let report = ginary::diagnose::summarize(Some(artifact.path()), None, None);
    assert!(
        report.artifact.is_some(),
        "artifact was not inspected: {report:?}"
    );
    assert!(
        !artifact.cache_root().exists(),
        "diagnose extracted or ran its artifact"
    );
}

#[test]
fn f1_diagnose_marks_the_trace_byte_bound_incomplete() {
    let dir = tempfile::tempdir().expect("tempdir");
    let trace = dir.path().join("large-trace");
    let file = std::fs::File::create(&trace).expect("trace");
    file.set_len(ginary::diagnose::MAX_TRACE_BYTES + 1)
        .expect("bounded fixture");
    let report = ginary::diagnose::summarize(None, Some(&trace), None);
    let summary = report.trace.expect("trace");
    assert!(!summary.complete);
    assert!(summary.limit_reached);
    assert_eq!(summary.bytes_read, ginary::diagnose::MAX_TRACE_BYTES);
    assert_eq!(summary.oversized_lines, 1);
}

#[test]
fn f1_doctor_reports_a_malformed_native_file_even_without_declared_targets() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("gleam.toml"),
        "name = \"probe\"\nversion = \"1.0.0\"\n",
    )
    .expect("manifest");
    let native = dir
        .path()
        .join("build/erlang-shipment/probe/priv/broken.so");
    std::fs::create_dir_all(native.parent().expect("parent")).expect("priv");
    std::fs::write(&native, b"\x7fELFbroken").expect("native");
    let project =
        ginary::doctor::project_context(dir.path(), std::time::SystemTime::now()).expect("project");
    assert!(
        !project.native_notes.is_empty(),
        "a malformed native file was silently omitted: {project:?}"
    );
}

#[test]
fn f1_doctor_reports_a_depth_limited_shipment_scan() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("gleam.toml"),
        "name = \"probe\"\nversion = \"1.0.0\"\n",
    )
    .expect("manifest");
    let mut deep = dir.path().join("build/erlang-shipment");
    for _ in 0..14 {
        deep.push("d");
    }
    std::fs::create_dir_all(&deep).expect("deep shipment");
    let project =
        ginary::doctor::project_context(dir.path(), std::time::SystemTime::now()).expect("project");
    assert!(
        project
            .native_notes
            .iter()
            .any(|note| note.contains("depth")),
        "scan silently stopped: {project:?}"
    );
}

#[test]
fn f1_doctor_reports_nonzero_exit_and_a_remedy() {
    let dir = tempfile::tempdir().expect("tempdir");
    script::program(
        dir.path(),
        "gleam",
        &[
            ShimStep::Print(vec!["gleam 1.2.3".into()]),
            ShimStep::Exit(7),
        ],
    );
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_ginary"))
        .args(["doctor", "--json"])
        .env("PATH", dir.path())
        .env("GINARY_CACHE_DIR", dir.path().join("cache"))
        .current_dir(dir.path())
        .output()
        .expect("doctor");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("doctor JSON");
    assert_eq!(
        report["tool_probes"][0]["outcome"], "nonzero_exit",
        "{report}"
    );
    assert_eq!(report["tool_probes"][0]["exit_code"], 7);
    assert!(
        !report["tool_probes"][0]["remedy"]
            .as_str()
            .unwrap_or_default()
            .is_empty()
    );
}

#[test]
fn f1_doctor_does_not_accept_an_unrelated_programs_greeting_as_a_version() {
    let dir = tempfile::tempdir().expect("tempdir");
    script::program(
        dir.path(),
        "gleam",
        &[ShimStep::Print(vec!["hello stranger".into()])],
    );
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_ginary"))
        .args(["doctor", "--json"])
        .env("PATH", dir.path())
        .env("GINARY_CACHE_DIR", dir.path().join("cache"))
        .current_dir(dir.path())
        .output()
        .expect("doctor");
    assert!(output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("JSON");
    assert_eq!(
        report["tool_probes"][0]["outcome"], "invalid_output",
        "{report}"
    );
}

#[test]
fn f1_diagnose_reads_both_trace_schemas_without_copying_secrets() {
    let dir = tempfile::tempdir().expect("tempdir");
    let trace = dir.path().join("trace.jsonl");
    std::fs::write(&trace, concat!(
        "{\"t_us\":1,\"phase\":\"exec\",\"kv\":{\"argv\":\"private-secret\"}}\n",
        "{\"schema_version\":2,\"run_id\":\"run-a\",\"sequence\":1,\"event\":\"failure\",\"t_us\":2,\"phase\":\"exec\",\"kv\":{\"token\":\"private-secret\"}}\n",
        "broken JSON\n"
    )).expect("trace");
    let report = ginary::diagnose::summarize(None, Some(&trace), None);
    let value = serde_json::to_value(&report).expect("JSON");
    assert_eq!(value["trace"]["events"], 2, "{value}");
    assert_eq!(value["trace"]["invalid_lines"], 1);
    assert_eq!(value["trace"]["outcomes"]["failure"], 1);
    assert_eq!(value["trace"]["run_count"], 1);
    assert_eq!(value["trace"]["complete"], false);
    assert!(!value.to_string().contains("private-secret"));
    assert!(!report.render_text().contains("private-secret"));
}
