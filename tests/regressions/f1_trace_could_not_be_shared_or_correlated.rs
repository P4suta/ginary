// SPDX-License-Identifier: MIT OR Apache-2.0
//! Trace records need a session identity and must not expose arbitrary application secrets.
use crate::common::payload::SharedSink;
use ginary::diag::Diag;

#[test]
fn traces_are_versioned_and_identify_separate_runs() {
    let sink = SharedSink::new();
    for _ in 0..2 {
        Diag::with_sinks(None, Some(Box::new(sink.clone()))).kv("test", &[("value", "ok")]);
    }
    let rows: Vec<serde_json::Value> = sink
        .lines()
        .iter()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    assert_eq!(rows[0]["schema_version"], 2);
    assert!(rows[0]["run_id"].is_string());
    assert_ne!(rows[0]["run_id"], rows[1]["run_id"]);
}

#[test]
fn default_traces_do_not_record_arguments_environment_values_or_signed_urls() {
    let sink = SharedSink::new();
    let diag = Diag::with_sinks(None, Some(Box::new(sink.clone())));
    diag.kv(
        "exec",
        &[
            ("argv", "[\"secret-argument-123\"]"),
            ("env_set", "[\"TOKEN=secret-env-456\"]"),
        ],
    );
    diag.kv(
        "fetch",
        &[(
            "url",
            "https://user:password@example.com/file?token=secret-url-789",
        )],
    );
    let text = sink.lines().join("\n");
    for secret in [
        "secret-argument-123",
        "secret-env-456",
        "password",
        "secret-url-789",
    ] {
        assert!(!text.contains(secret), "trace exposed {secret}: {text}");
    }
}

#[test]
fn concurrent_events_have_one_order_in_both_sinks_and_complete_operations() {
    let trace = SharedSink::new();
    let debug = SharedSink::new();
    let diag = Diag::with_sinks(Some(Box::new(debug.clone())), Some(Box::new(trace.clone())));
    std::thread::scope(|scope| {
        for worker in 0..8 {
            let diag = &diag;
            scope.spawn(move || {
                for event in 0..20 {
                    diag.kv("worker", &[("id", &format!("{worker}:{event}"))]);
                }
            });
        }
    });
    let rows: Vec<serde_json::Value> = trace
        .lines()
        .iter()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    let human = debug.lines();
    for (index, row) in rows.iter().enumerate() {
        assert_eq!(row["sequence"].as_u64(), Some(index as u64 + 1));
        assert!(human[index].contains(row["kv"]["id"].as_str().unwrap()));
    }
    assert!(
        rows.windows(2)
            .all(|pair| pair[0]["t_us"].as_u64() <= pair[1]["t_us"].as_u64())
    );
    diag.operation("ok").finish(true, &[]);
    diag.operation("failed")
        .finish(false, &[("cause", "disk full")]);
    drop(diag.operation("abandoned"));
    let rows: Vec<serde_json::Value> = trace
        .lines()
        .iter()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    let events: Vec<_> = rows[160..]
        .iter()
        .map(|row| row["event"].as_str().unwrap())
        .collect();
    assert_eq!(
        events,
        ["start", "end", "start", "failure", "start", "interrupted"]
    );
}

#[test]
fn failed_sink_is_reported_without_losing_the_other_sink() {
    struct Broken;
    impl std::io::Write for Broken {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("injected disk full"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let healthy = SharedSink::new();
    let diag = Diag::with_sinks(Some(Box::new(healthy.clone())), Some(Box::new(Broken)));
    diag.kv("launch", &[("value", "still works")]);
    assert_eq!(healthy.lines().len(), 1);
    assert_eq!(diag.health().trace_failures, 1);
    assert!(diag.health().last_error.unwrap().contains("disk full"));
}

#[test]
fn explicitly_requested_sensitive_capture_preserves_values_and_human_controls_are_escaped() {
    let sink = SharedSink::new();
    let human = SharedSink::new();
    let diag = Diag::with_sinks(Some(Box::new(human.clone())), Some(Box::new(sink.clone())))
        .with_sensitive(true);
    diag.kv("exec", &[("argv", "secret\n\u{1b}[2J")]);
    let row: serde_json::Value = serde_json::from_str(&sink.lines()[0]).unwrap();
    assert_eq!(row["kv"]["argv"], "secret\n\u{1b}[2J");
    assert_eq!(human.lines().len(), 1);
    assert!(!human.lines()[0].contains('\u{1b}'));
}

#[test]
fn every_url_in_one_error_is_redacted() {
    let value = ginary::diag::redact(
        "error",
        "fetch https://a.invalid/x then https://user:credential@b.invalid/y?token=hidden",
    );
    assert!(
        !value.contains("credential") && !value.contains("hidden"),
        "{value}"
    );
}

#[test]
fn sensitive_argument_opt_in_still_hides_url_credentials() {
    let sink = SharedSink::new();
    let diag = Diag::with_sinks(None, Some(Box::new(sink.clone()))).with_sensitive(true);
    diag.kv(
        "request",
        &[(
            "argv",
            "get https://user:credential@example.invalid/path?token=hidden",
        )],
    );
    let output = sink.lines().join("\n");
    assert!(
        output.contains("get https://example.invalid/path"),
        "{output}"
    );
    assert!(
        !output.contains("credential") && !output.contains("hidden"),
        "{output}"
    );
}

#[test]
fn trace_files_preserve_unrelated_content_and_detect_destination_replacement() {
    let directory = tempfile::tempdir().unwrap();
    let original = directory.path().join("original");
    let alias = directory.path().join("trace.jsonl");
    std::fs::write(&original, b"existing artifact bytes").unwrap();
    std::fs::hard_link(&original, &alias).unwrap();
    let env = ginary::diag::EnvSnapshot {
        ginary_debug: None,
        ginary_trace: Some(alias.clone().into_os_string()),
    };
    let refused = Diag::from_env(&env);
    refused.kv("must_not_write", &[]);
    assert!(!refused.is_enabled());
    assert_eq!(refused.health().trace_failures, 1);
    assert_eq!(
        std::fs::read(&original).unwrap(),
        b"existing artifact bytes"
    );
    std::fs::remove_file(&alias).unwrap();
    let active = Diag::from_env(&env);
    active.kv("first", &[]);
    std::fs::write(&alias, b"a file replaced during this run").unwrap();
    active.kv("must_not_append", &[]);
    assert_eq!(active.health().trace_failures, 1);
    assert_eq!(
        std::fs::read(&alias).unwrap(),
        b"a file replaced during this run"
    );
}

#[test]
fn existing_trace_ownership_checks_are_bounded_and_keep_legacy_logs_readable() {
    let directory = tempfile::tempdir().unwrap();
    let trace = directory.path().join("trace.jsonl");
    let env = ginary::diag::EnvSnapshot {
        ginary_debug: None,
        ginary_trace: Some(trace.clone().into_os_string()),
    };
    let legacy = b"{\"t_us\":0,\"phase\":\"old\",\"kv\":{}}\n";
    std::fs::write(&trace, legacy).unwrap();
    let recorder = Diag::from_env(&env);
    recorder.kv("current", &[]);
    assert_eq!(std::fs::read_to_string(&trace).unwrap().lines().count(), 2);
    for invalid in [
        b"{\"t_us\":0,\"phase\":\"old\",\"kv\":{}}".to_vec(),
        vec![b'x'; 1024 * 1024 + 1],
    ] {
        std::fs::write(&trace, &invalid).unwrap();
        let recorder = Diag::from_env(&env);
        recorder.kv("refused", &[]);
        assert!(!recorder.is_enabled());
        assert_eq!(std::fs::read(&trace).unwrap(), invalid);
    }
}

#[test]
fn overlapping_operations_can_be_paired_without_guessing() {
    let sink = SharedSink::new();
    let diag = Diag::with_sinks(None, Some(Box::new(sink.clone())));
    let first = diag.operation("build");
    let second = diag.operation("build");
    first.finish(true, &[]);
    second.finish(false, &[]);
    let rows: Vec<serde_json::Value> = sink
        .lines()
        .iter()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    assert!(rows[0]["operation_id"].is_u64());
    assert_ne!(rows[0]["operation_id"], rows[1]["operation_id"]);
    assert_eq!(rows[0]["operation_id"], rows[2]["operation_id"]);
    assert_eq!(rows[1]["operation_id"], rows[3]["operation_id"]);
}

#[test]
fn independent_recorders_append_complete_records_to_one_file() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("shared.jsonl");
    let recorders: Vec<_> = (0..4)
        .map(|_| {
            Diag::from_env(&ginary::diag::EnvSnapshot {
                ginary_debug: None,
                ginary_trace: Some(path.clone().into_os_string()),
            })
        })
        .collect();
    std::thread::scope(|scope| {
        for recorder in &recorders {
            scope.spawn(move || {
                for _ in 0..20 {
                    recorder.kv("writer", &[("value", &"x".repeat(16 * 1024))]);
                }
            });
        }
    });
    for recorder in &recorders {
        assert_eq!(recorder.health().trace_failures, 0);
    }
    let text = std::fs::read_to_string(&path).unwrap();
    let rows: Vec<serde_json::Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).expect("no interleaved JSON fragments"))
        .collect();
    assert_eq!(rows.len(), 80);
    let runs: std::collections::BTreeSet<_> = rows
        .iter()
        .map(|row| row["run_id"].as_str().unwrap())
        .collect();
    assert_eq!(runs.len(), 4);
}
