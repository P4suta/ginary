// SPDX-License-Identifier: MIT OR Apache-2.0
//! The launcher's only observability.
//!
//! The recorder is asserted through injected sinks rather than by spawning a
//! process and reading its standard error: `Diag::with_sinks` exists for that
//! reason, and it is the pattern `docs/dev/testing.md` records. What the tests
//! pin is the exact shape of both outputs — a debug line a person reads and a
//! JSON object a tool reads — because both are a contract the moment anyone
//! greps them.

mod common;

use std::ffi::OsString;

use common::payload::SharedSink;
use ginary::diag::{Diag, EnvSnapshot};

/// The prefix every human-readable line carries.
const PREFIX: &str = "ginary[debug]: ";

fn trace_objects(sink: &SharedSink) -> Vec<serde_json::Value> {
    sink.lines()
        .iter()
        .map(|line| {
            serde_json::from_str(line).unwrap_or_else(|error| {
                panic!("a trace line must be one JSON object: {line:?}: {error}")
            })
        })
        .collect()
}

#[test]
fn a_recorder_with_no_sinks_is_not_enabled_and_writes_nothing() {
    let diag = Diag::disabled();

    assert!(!diag.is_enabled());
    {
        let _phase = diag.phase("open_self");
    }
    diag.kv("cache", &[("key", "0123456789abcdef")]);
}

#[test]
fn a_recorder_with_a_sink_is_enabled() {
    let diag = Diag::with_sinks(Some(Box::new(SharedSink::new())), None);

    assert!(diag.is_enabled());
}

#[test]
fn a_phase_reaches_the_debug_sink_as_one_line_with_its_elapsed_time() {
    let debug = SharedSink::new();
    let diag = Diag::with_sinks(Some(Box::new(debug.clone())), None);

    {
        let _phase = diag.phase("open_self");
    }

    let lines = debug.lines();
    assert_eq!(lines.len(), 1, "one phase, one line: {lines:?}");
    let line = &lines[0];
    assert!(
        line.starts_with(&format!("{PREFIX}open_self (")),
        "the phase name follows the prefix: {line:?}"
    );
    assert!(
        line.ends_with("us)"),
        "the elapsed time closes the line: {line:?}"
    );
}

#[test]
fn key_values_reach_the_debug_sink_in_the_order_they_were_given() {
    let debug = SharedSink::new();
    let diag = Diag::with_sinks(Some(Box::new(debug.clone())), None);

    diag.kv(
        "cache",
        &[
            ("key", "0123456789abcdef"),
            ("hit", "true"),
            ("entries", "42"),
        ],
    );

    assert_eq!(
        debug.lines(),
        [format!(
            "{PREFIX}cache key=0123456789abcdef hit=true entries=42"
        )],
        "a fact that is not a phase carries no elapsed time"
    );
}

#[test]
fn the_trace_sink_holds_one_json_object_per_line() {
    let trace = SharedSink::new();
    let diag = Diag::with_sinks(None, Some(Box::new(trace.clone())));

    diag.kv("cache", &[("key", "0123456789abcdef")]);
    {
        let _phase = diag.phase("extract");
    }

    let objects = trace_objects(&trace);
    assert_eq!(objects.len(), 2, "two events, two lines");

    assert_eq!(objects[0]["phase"], "cache");
    assert_eq!(objects[0]["kv"]["key"], "0123456789abcdef");
    assert!(
        objects[0]["t_us"].is_u64(),
        "t_us is a number of microseconds: {}",
        objects[0]
    );
    assert!(
        objects[0].get("elapsed_us").is_none(),
        "a fact that is not a phase has no duration: {}",
        objects[0]
    );

    assert_eq!(objects[1]["phase"], "extract");
    assert_eq!(objects[1]["kv"], serde_json::json!({}));
    assert!(
        objects[1]["elapsed_us"].is_u64(),
        "a phase records how long it took: {}",
        objects[1]
    );
}

#[test]
fn trace_lines_are_written_in_the_order_the_events_happened() {
    let trace = SharedSink::new();
    let diag = Diag::with_sinks(None, Some(Box::new(trace.clone())));

    for name in ["open_self", "read_trailer", "resolve_cache", "exec"] {
        let _phase = diag.phase(name);
    }

    let objects = trace_objects(&trace);
    let phases: Vec<&str> = objects
        .iter()
        .map(|object| object["phase"].as_str().expect("a phase name"))
        .collect();
    assert_eq!(
        phases,
        ["open_self", "read_trailer", "resolve_cache", "exec"]
    );

    let stamps: Vec<u64> = objects
        .iter()
        .map(|object| object["t_us"].as_u64().expect("a timestamp"))
        .collect();
    assert!(
        stamps.windows(2).all(|pair| pair[0] <= pair[1]),
        "the monotonic clock does not go backwards: {stamps:?}"
    );
}

#[test]
fn a_phase_that_takes_time_records_it() {
    let trace = SharedSink::new();
    let diag = Diag::with_sinks(None, Some(Box::new(trace.clone())));

    {
        let _phase = diag.phase("extract");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }

    let objects = trace_objects(&trace);
    let object = objects.first().expect("a phase writes one trace line");
    let elapsed = object["elapsed_us"].as_u64().expect("an elapsed time");
    assert!(
        elapsed >= 1_000,
        "a phase that slept five milliseconds reported {elapsed}us"
    );
}

#[test]
fn nothing_set_records_nothing_and_creates_no_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("traces/run.jsonl");

    // The complement first, over the same path: with the variable set, the
    // recorder does create the file and its parent. Without this half the
    // assertion below is over a path nothing was ever told about, and it would
    // hold whatever the recorder did.
    {
        let asked = Diag::from_env(&EnvSnapshot {
            ginary_debug: None,
            ginary_trace: Some(OsString::from(&path)),
        });
        {
            let _phase = asked.phase("open_self");
        }
        assert!(
            path.is_file(),
            "a recorder that was asked for a file makes one"
        );
    }
    std::fs::remove_dir_all(dir.path().join("traces")).expect("clear the trace directory");

    let diag = Diag::from_env(&EnvSnapshot::default());

    assert!(!diag.is_enabled());
    {
        let _phase = diag.phase("open_self");
    }
    diag.kv("cache", &[("key", "0123456789abcdef")]);
    assert!(
        !path.exists() && !path.parent().expect("a parent").exists(),
        "a recorder nobody asked for touches no disk"
    );
    assert_eq!(
        std::fs::read_dir(dir.path()).expect("read_dir").count(),
        0,
        "and writes nothing anywhere else either"
    );
}

#[test]
fn an_empty_trace_variable_is_an_unset_one() {
    let dir = tempfile::tempdir().expect("tempdir");
    let env = EnvSnapshot {
        ginary_debug: None,
        ginary_trace: Some(OsString::new()),
    };

    let diag = Diag::from_env(&env);

    assert!(
        !diag.is_enabled(),
        "an exported-but-empty `GINARY_TRACE` did not ask for a trace"
    );
    {
        let _phase = diag.phase("open_self");
    }
    assert_eq!(
        std::fs::read_dir(dir.path()).expect("read_dir").count(),
        0,
        "and no file was opened for the empty name"
    );
}

#[test]
fn ginary_trace_writes_json_lines_to_the_file_it_names_and_creates_its_parents() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("traces/today/run.jsonl");
    let env = EnvSnapshot {
        ginary_debug: None,
        ginary_trace: Some(OsString::from(&path)),
    };

    let diag = Diag::from_env(&env);
    assert!(diag.is_enabled(), "a trace path turns the recorder on");
    {
        let _phase = diag.phase("open_self");
    }
    drop(diag);

    let text = std::fs::read_to_string(&path).expect("the trace file was created, parents and all");
    let lines: Vec<&str> = text.lines().filter(|line| !line.is_empty()).collect();
    assert_eq!(lines.len(), 1, "one phase, one line: {text:?}");
    let object: serde_json::Value = serde_json::from_str(lines[0]).expect("one JSON object");
    assert_eq!(object["phase"], "open_self");
}

#[test]
fn ginary_debug_is_on_for_one_and_off_for_anything_else() {
    let on = Diag::from_env(&EnvSnapshot {
        ginary_debug: Some(OsString::from("1")),
        ginary_trace: None,
    });
    assert!(on.is_enabled(), "`GINARY_DEBUG=1` turns the recorder on");

    for value in ["0", "", "true", "yes"] {
        let off = Diag::from_env(&EnvSnapshot {
            ginary_debug: Some(OsString::from(value)),
            ginary_trace: None,
        });
        assert!(
            !off.is_enabled(),
            "`GINARY_DEBUG={value}` is not `1` and does not turn the recorder on"
        );
    }
}

#[test]
fn a_trace_file_that_cannot_be_opened_leaves_the_run_working() {
    let dir = tempfile::tempdir().expect("tempdir");
    let blocker = dir.path().join("not-a-directory");
    std::fs::write(&blocker, b"a regular file").expect("write");
    let env = EnvSnapshot {
        ginary_debug: None,
        ginary_trace: Some(OsString::from(blocker.join("run.jsonl"))),
    };

    let diag = Diag::from_env(&env);

    assert!(
        !diag.is_enabled(),
        "a sink that could not be opened is not a sink"
    );
    {
        let _phase = diag.phase("open_self");
    }
    assert_eq!(
        std::fs::read(&blocker).expect("read"),
        b"a regular file",
        "the recorder did not write over the thing that was in the way"
    );
}

#[test]
fn both_sinks_get_the_same_events() {
    let debug = SharedSink::new();
    let trace = SharedSink::new();
    let diag = Diag::with_sinks(Some(Box::new(debug.clone())), Some(Box::new(trace.clone())));

    diag.kv("trailer", &[("offset", "4096"), ("len", "2048")]);
    {
        let _phase = diag.phase("extract");
    }

    assert_eq!(debug.lines().len(), 2);
    assert_eq!(trace_objects(&trace).len(), 2);
    assert_eq!(
        debug.lines()[0],
        format!("{PREFIX}trailer offset=4096 len=2048")
    );
}
