// SPDX-License-Identifier: MIT OR Apache-2.0
//! The final stderr emitted before a timeout is often the only diagnosis.
use crate::common::script::{self, ShimStep};
use std::time::Duration;

#[test]
fn a_timeout_keeps_the_output_emitted_before_the_child_stalled() {
    let dir = tempfile::tempdir().expect("tempdir");
    let program = script::program(
        dir.path(),
        "stall",
        &[
            ShimStep::Print(vec!["before-timeout".into()]),
            ShimStep::Sleep(10000),
        ],
    );
    let result = ginary::process::run_with_timeout(&program, &[], Duration::from_secs(2));
    let error = result.expect_err("the child outlives its deadline");
    assert!(
        error.to_string().contains("before-timeout"),
        "timeout must retain partial output: {error}"
    );
}

#[test]
fn parsing_cannot_accept_a_silently_truncated_output() {
    let dir = tempfile::tempdir().expect("tempdir");
    let program = script::program(
        dir.path(),
        "chatty",
        &[
            ShimStep::Print(vec!["x".repeat(1024 * 1024 + 1)]),
            ShimStep::Exit(0),
        ],
    );
    let result = ginary::process::run_with_timeout(&program, &[], Duration::from_secs(10));
    assert!(
        result.is_err(),
        "an oversized parse input must be an explicit error"
    );
}

#[test]
fn a_timeout_reports_the_status_observed_when_the_child_is_reaped() {
    let dir = tempfile::tempdir().expect("tempdir");
    let program = script::program(dir.path(), "status", &[ShimStep::Sleep(3000)]);
    let report = ginary::process::run_command(
        &mut std::process::Command::new(program),
        Duration::from_millis(100),
        1024,
    );
    assert!(
        report.error.is_some(),
        "the deadline must remain the primary failure"
    );
    assert!(
        report.status.is_some(),
        "the status obtained during cleanup must not be lost"
    );
    assert!(!report.status.expect("reaped status").success());
    let cleanup = report
        .cleanup
        .expect("a spawned child has cleanup evidence");
    assert!(cleanup.kill_requested);
    assert!(cleanup.reaped);
    assert!(!cleanup.background_reaper);
    assert!(cleanup.error.is_none());
}

#[test]
fn the_test_harness_saves_exact_timeout_evidence_when_configured() {
    const MARKER: &str = "GINARY_EVIDENCE_REGRESSION_CHILD";
    if std::env::var_os(MARKER).is_some() {
        let dir = tempfile::tempdir().expect("tempdir");
        let program = script::program(
            dir.path(),
            "retained",
            &[
                ShimStep::Print(vec!["retained-progress".into()]),
                ShimStep::Sleep(10000),
            ],
        );
        crate::common::bounded::run_bounded(
            &mut std::process::Command::new(program),
            Duration::from_secs(2),
            "evidence fixture",
        );
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let mut command = std::process::Command::new(std::env::current_exe().expect("test executable"));
    command.args(["--exact", "f1_process_timeouts_lost_their_output::the_test_harness_saves_exact_timeout_evidence_when_configured", "--nocapture"])
        .env(MARKER, "1").env("GINARY_TEST_EVIDENCE_DIR", dir.path());
    let observed = ginary::process::run_command(&mut command, Duration::from_secs(20), 128 * 1024);
    assert!(
        observed.error.is_none(),
        "the nested test completed: {observed:?}"
    );
    assert!(!observed.success(), "the nested test must fail on timeout");
    let directories: Vec<_> = std::fs::read_dir(dir.path())
        .expect("evidence directory")
        .map(|entry| entry.expect("evidence entry").path())
        .collect();
    assert_eq!(
        directories.len(),
        1,
        "one unique directory per failed child"
    );
    assert!(
        std::fs::read(directories[0].join("stdout.bin"))
            .expect("raw output")
            .starts_with(b"retained-progress")
    );
    let metadata: serde_json::Value = serde_json::from_slice(
        &std::fs::read(directories[0].join("report.json")).expect("metadata"),
    )
    .expect("JSON");
    assert_eq!(metadata["schema_version"], 1);
    assert_eq!(metadata["cleanup"]["reaped"], true);
    assert_eq!(metadata["stdout"]["omitted_bytes"], 0);
    assert!(
        metadata["cause"]
            .as_str()
            .expect("cause")
            .contains("2000ms")
    );
    assert!(metadata["elapsed_ms"].as_u64().expect("elapsed") >= 2000);
}

#[test]
fn the_test_harness_preserves_output_when_its_child_times_out() {
    let dir = tempfile::tempdir().expect("tempdir");
    let program = script::program(
        dir.path(),
        "harness",
        &[
            ShimStep::Print(vec!["last-progress-before-hang".into()]),
            ShimStep::Sleep(10000),
        ],
    );
    let failure = std::panic::catch_unwind(|| {
        crate::common::bounded::run_bounded(
            &mut std::process::Command::new(program),
            Duration::from_secs(2),
            "fixture command",
        );
    })
    .expect_err("the bounded harness reports timeout as a test failure");
    let message = failure
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| failure.downcast_ref::<&str>().copied())
        .expect("panic text");
    assert!(
        message.contains("last-progress-before-hang"),
        "the timeout must preserve evidence: {message}"
    );
}
