// SPDX-License-Identifier: MIT OR Apache-2.0
//! Summarising an `erl_crash.dump`.
//!
//! Three kinds of input. `tests/fixtures/crashdump/synthetic.dump` is
//! hand-written and small, so every assertion can name a field and a value;
//! `truncated.dump` is the same file cut off inside a `=proc:` section, which
//! is what a runtime killed while writing its dump leaves; and the gated test
//! at the end runs a real `erl` and reads what it wrote, because a parser that
//! only handles files written by its own author is not a parser.
//!
//! Nothing here reads a dump into memory. `parse` takes a
//! [`std::io::BufRead`], and the bound on a single line is asserted rather
//! than assumed.
// The command line half of the suite: every claim in this file is about a
// module the `cli` feature carries, so a `--no-default-features` build has
// nothing here to run. See `docs/dev/log/C2.md`.
#![cfg(feature = "cli")]

mod common;

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::time::Duration;

use assert_cmd::Command;
use ginary::crashdump::{
    self, CRASHDUMP_FORMAT_VERSION, CrashDump, CrashdumpError, MAX_LINE_BYTES, ProcessSummary,
    TOP_PROCESSES,
};
use serde_json::Value;

use crate::common::bounded::run_bounded;
use crate::common::tools::require_tools;

/// A `Command` for the `ginary` binary, run from the crate root so the fixture
/// paths it is given are relative and stable.
fn ginary() -> Command {
    let mut command = Command::cargo_bin("ginary").expect("the `ginary` binary is built for tests");
    command.current_dir(env!("CARGO_MANIFEST_DIR"));
    command
}

/// The absolute path of one crash dump fixture.
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/crashdump")
        .join(name)
}

/// The whole synthetic fixture, summarised.
fn synthetic() -> CrashDump {
    crashdump::read(&fixture("synthetic.dump")).expect("the synthetic fixture is a crash dump")
}

// ------------------------------------------------------- the preamble --

#[test]
fn the_preamble_is_read_field_by_field() {
    let dump = synthetic();

    assert_eq!(dump.format_version, CRASHDUMP_FORMAT_VERSION);
    assert_eq!(dump.dump_version, "0.5");
    assert_eq!(dump.date.as_deref(), Some("Mon Aug 31 11:52:30 2026"));
    assert_eq!(dump.slogan.as_deref(), Some("kaboom"));
    assert_eq!(
        dump.system_version.as_deref(),
        Some(
            "Erlang/OTP 29 [erts-17.0.5] [source] [64-bit] [smp:8:8] [ds:8:8:10] \
             [async-threads:1] [jit:ns]"
        )
    );
    assert_eq!(dump.taints, ["crypto", "asn1rt_nif"]);
    assert!(!dump.truncated, "the fixture ends with `=end`");
}

#[test]
fn every_proc_section_is_counted_even_though_only_five_are_listed() {
    let dump = synthetic();
    assert_eq!(dump.processes, 7);
    assert_eq!(dump.top_processes.len(), TOP_PROCESSES);
}

#[test]
fn the_top_processes_are_the_largest_heaps_first() {
    let dump = synthetic();

    // The largest is in the middle of the file, so a reader that took the
    // first five in file order would produce a different list.
    assert_eq!(
        dump.top_processes,
        vec![
            ProcessSummary {
                pid: "<0.44.0>".to_owned(),
                name: None,
                initial_call: Some("erlang:apply/2".to_owned()),
                heap: 6772,
            },
            ProcessSummary {
                pid: "<0.45.0>".to_owned(),
                name: Some("application_controller".to_owned()),
                initial_call: Some("application_controller:start/1".to_owned()),
                heap: 4185,
            },
            ProcessSummary {
                pid: "<0.47.0>".to_owned(),
                name: Some("code_server".to_owned()),
                initial_call: Some("erlang:apply/2".to_owned()),
                heap: 2586,
            },
            ProcessSummary {
                pid: "<0.46.0>".to_owned(),
                name: None,
                initial_call: Some("proc_lib:init_p/5".to_owned()),
                heap: 1598,
            },
            ProcessSummary {
                pid: "<0.0.0>".to_owned(),
                name: Some("init".to_owned()),
                initial_call: Some("erl_init:start/2".to_owned()),
                heap: 987,
            },
        ]
    );
}

#[test]
fn a_truncated_dump_reports_what_was_readable() {
    let dump = crashdump::read(&fixture("truncated.dump"))
        .expect("a dump that stops mid-section is still summarised");

    assert!(dump.truncated, "the fixture does not end with `=end`");
    assert_eq!(dump.slogan.as_deref(), Some("kaboom"));
    assert_eq!(dump.processes, 3, "three `=proc:` sections were begun");
    assert_eq!(
        dump.top_processes
            .iter()
            .map(|process| process.pid.as_str())
            .collect::<Vec<_>>(),
        ["<0.44.0>", "<0.0.0>", "<0.1.0>"],
        "the process whose section was cut had already given its heap"
    );
}

#[test]
fn a_file_that_is_not_a_crash_dump_is_refused() {
    let error = crashdump::parse(Cursor::new(b"#!/bin/sh\necho hello\n".to_vec()))
        .expect_err("a shell script is not a crash dump");

    match error {
        CrashdumpError::NotACrashDump { found } => assert_eq!(found, "#!/bin/sh"),
        other => panic!("expected NotACrashDump, got {other:?}"),
    }
}

#[test]
fn a_line_longer_than_the_bound_is_cut_rather_than_held() {
    let slogan = "b".repeat(MAX_LINE_BYTES * 2);
    let text = format!(
        "=erl_crash_dump:0.5\nMon Aug 31 11:52:30 2026\nSlogan: {slogan}\nTaints: \n=end\n"
    );

    let dump = crashdump::parse(Cursor::new(text.into_bytes()))
        .expect("a dump with one enormous line is still summarised");

    let held = dump.slogan.expect("the slogan was read");
    assert_eq!(
        held.len(),
        MAX_LINE_BYTES,
        "the value is bounded by MAX_LINE_BYTES"
    );
    assert!(held.bytes().all(|byte| byte == b'b'), "{held:.40}");
}

#[test]
fn the_summary_is_this() {
    insta::assert_snapshot!("crashdump_text", synthetic().render_text());
}

// ------------------------------------------------------- the command --

#[test]
fn crashdump_prints_the_summary() {
    let assert = ginary()
        .args(["crashdump", "tests/fixtures/crashdump/synthetic.dump"])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8");

    assert!(stdout.contains("slogan:"), "{stdout}");
    assert!(stdout.contains("kaboom"), "{stdout}");
    assert!(stdout.contains("<0.44.0>"), "{stdout}");
}

#[test]
fn crashdump_json_carries_the_documented_keys() {
    let assert = ginary()
        .args([
            "crashdump",
            "tests/fixtures/crashdump/synthetic.dump",
            "--json",
        ])
        .assert()
        .success();
    let value: Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("the output is JSON");

    assert_eq!(value["format_version"], CRASHDUMP_FORMAT_VERSION);
    assert_eq!(value["dump_version"], "0.5");
    assert_eq!(value["slogan"], "kaboom");
    assert_eq!(value["processes"], 7);
    assert_eq!(value["truncated"], false);
    assert_eq!(
        value["top_processes"]
            .as_array()
            .expect("top_processes")
            .len(),
        TOP_PROCESSES
    );
    assert_eq!(value["top_processes"][0]["pid"], "<0.44.0>");
    assert_eq!(value["top_processes"][0]["heap"], 6772);
}

// -------------------------------------------------------- a real dump --

/// The one-liner that makes a real runtime write a real dump.
///
/// `erlang:halt/2` with a string is the only reliable way: `halt(abort)` dumps
/// core and writes nothing, and an ordinary `halt(1)` exits cleanly. The
/// `{flush, true}` is what makes the dump complete before the process is gone.
/// How long the runtime gets to start, die and flush its dump.
///
/// Every other real-tool test in the suite spawns through
/// `tests/common/bounded.rs`, and this one may not be the exception: an `erl`
/// that stalls — a busy machine, a dump written onto a full disk — would hang
/// the target with no diagnosis at all.
const DUMP_BUDGET: Duration = Duration::from_secs(60);

const DUMP_RECIPE: &str =
    "spawn(fun() -> exit(kaboom) end), timer:sleep(100), erlang:halt(\"kaboom\", [{flush,true}]).";

#[test]
fn a_dump_a_real_runtime_wrote_is_summarised() {
    let Some(tools) = require_tools(&["erl"]) else {
        return;
    };
    let dir = tempfile::tempdir().expect("a temporary directory");
    let dump = dir.path().join("erl_crash.dump");

    let mut command = std::process::Command::new(tools.path("erl"));
    command
        .args([
            "-noshell",
            "-env",
            "ERL_CRASH_DUMP",
            &dump.display().to_string(),
            "-eval",
            DUMP_RECIPE,
        ])
        .current_dir(dir.path());
    let output = run_bounded(&mut command, DUMP_BUDGET, "`erl` writing a crash dump");

    assert!(
        !output.status.success(),
        "the recipe halts with a slogan: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(dump.is_file(), "the recipe writes {}", dump.display());

    let summary = crashdump::read(&dump).expect("a real dump is summarised");

    assert_eq!(summary.slogan.as_deref(), Some("kaboom"));
    assert!(
        summary
            .system_version
            .as_deref()
            .is_some_and(|version| version.starts_with("Erlang/OTP")),
        "{:?}",
        summary.system_version
    );
    assert!(!summary.truncated, "a whole dump ends with `=end`");
    assert!(summary.processes > TOP_PROCESSES, "{}", summary.processes);
    assert_eq!(summary.top_processes.len(), TOP_PROCESSES);
    assert!(
        summary
            .top_processes
            .windows(2)
            .all(|pair| pair[0].heap >= pair[1].heap),
        "{:?}",
        summary.top_processes
    );
}
