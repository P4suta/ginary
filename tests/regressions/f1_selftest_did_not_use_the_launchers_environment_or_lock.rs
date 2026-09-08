// SPDX-License-Identifier: MIT OR Apache-2.0
//! Selftest planned against an empty environment and did not hold its cache.
//!
//! A caller's HOME and ERL_OTP flags therefore behaved differently in selftest
//! from a real launch, and maintenance could remove the runtime being tested.
//! A native fixture runtime makes both contracts observable on every host.

use std::time::{Duration, Instant};

use crate::common::artifact::{SyntheticArtifact, read_trace};
use crate::common::bounded::wait_bounded;
use crate::common::script::ShimStep;

#[test]
fn f1_launcher_records_maintenance_completion_and_failure() {
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = SyntheticArtifact::build_with_runtime_steps(dir.path(), &[ShimStep::Exit(0)]);
    for (command, outcome, code) in [("directory", "end", 0), ("unknown-command", "failure", 2)] {
        let trace = dir.path().join(format!("{command}.jsonl"));
        let output = artifact
            .run()
            .env("GINARY_CACHE_DIR", artifact.cache_root())
            .env("GINARY_CMD", command)
            .env("GINARY_TRACE", &trace)
            .output();
        assert_eq!(output.code(), code, "{}", output.stderr_text());
        let records: Vec<serde_json::Value> = std::fs::read_to_string(&trace)
            .expect("trace")
            .lines()
            .map(|line| serde_json::from_str(line).expect("JSON event"))
            .collect();
        assert!(
            records
                .iter()
                .any(|event| event["phase"] == "launcher" && event["event"] == "start"),
            "missing launcher start: {records:?}"
        );
        assert!(
            records
                .iter()
                .any(|event| event["phase"] == "launcher" && event["event"] == outcome),
            "missing launcher {outcome}: {records:?}"
        );
    }
}

#[test]
fn f1_selftest_uses_the_real_environment_for_defaults_and_scrubbing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = SyntheticArtifact::build_with_runtime_steps(dir.path(), &[ShimStep::Exit(0)]);
    let trace = dir.path().join("selftest.jsonl");
    let run = artifact
        .run()
        .env("GINARY_CACHE_DIR", artifact.cache_root())
        .env("GINARY_CMD", "selftest")
        .env("GINARY_TRACE", &trace)
        .env("GINARY_TRACE_SENSITIVE", "1")
        .env("ERL_OTP29_FLAGS", "unwanted")
        .env("GINARY_ERL_FLAGS", "+S 2")
        .output();
    assert_eq!(run.code(), 0, "{}", run.stderr_text());
    let records = read_trace(&trace);
    let exec = records
        .iter()
        .find(|record| record.phase == "exec")
        .expect("exec event");
    let removed: Vec<String> = serde_json::from_str(&exec.kv["env_remove"]).expect("removed vars");
    let set: Vec<String> = serde_json::from_str(&exec.kv["env_set"]).expect("set vars");
    let argv: Vec<String> = serde_json::from_str(&exec.kv["argv"]).expect("arguments");
    assert!(
        removed.iter().any(|name| name == "ERL_OTP29_FLAGS"),
        "selftest failed to scrub the real environment"
    );
    assert!(
        !set.iter().any(|pair| pair.starts_with("HOME=")),
        "selftest overwrote the caller's HOME"
    );
    assert!(
        argv.windows(2).any(|pair| pair == ["+S", "2"]),
        "selftest omitted the caller's runtime flags"
    );
}

#[test]
fn f1_selftest_holds_the_runtime_entry_until_the_child_exits() {
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = SyntheticArtifact::build_with_runtime_steps(
        dir.path(),
        &[
            ShimStep::RecordArgv,
            ShimStep::Sleep(2_000),
            ShimStep::Exit(0),
        ],
    );
    let marker = artifact
        .key_dir()
        .join(&artifact.manifest().launch.bindir)
        .join(format!("{}.argv", artifact.manifest().launch.program));
    let mut child = artifact
        .run()
        .env("GINARY_CACHE_DIR", artifact.cache_root())
        .env("GINARY_CMD", "selftest")
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .spawn();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !marker.is_file() && Instant::now() < deadline {
        if child.try_wait().expect("probe child status").is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let started = marker.is_file();
    let locked = ginary::cache_lock::try_exclusive(&artifact.key_dir()).is_none();
    let output = wait_bounded(child, Duration::from_secs(10), "selftest fixture runtime");
    assert!(
        started,
        "runtime did not announce itself: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.status.success(),
        "selftest failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(locked, "selftest ran without protecting its cache entry");
    assert!(
        ginary::cache_lock::try_exclusive(&artifact.key_dir()).is_some(),
        "selftest did not release its lock after exit"
    );
}
