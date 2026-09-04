// SPDX-License-Identifier: MIT OR Apache-2.0
//! `doctor` threw away the reason OTP discovery failed.
//!
//! **What went wrong.** `Report::gather` called `otp::discover(None).ok()`, so
//! every carefully worded `OtpError` became an unexplained `otp: not found`
//! line and a `null` JSON field. A machine with no Erlang at all and a machine
//! whose Erlang is present but unusable — a root with no `erts-*`, a
//! non-executable `beam.smp`, a release older than the minimum — produced
//! byte-identical output, and every actionable message `otp` was built around
//! was unreachable by any user. CLAUDE.md forbids exactly that: skipping is a
//! reported decision or an error, never a default.
//!
//! **The input.** An `erl` on `PATH` that answers the discovery probe with a
//! code root that is not an OTP installation.
//!
//! **The correct behaviour.** The reason survives into both renderings: the
//! text report says `otp: unusable (<reason>)` and the JSON carries the
//! sentence in `otp_error` beside the `null` `otp`.

#![cfg(unix)]

use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;

use crate::common::script::{ShimStep, program};

/// A `Command` for the `ginary` binary whose `PATH` holds only `dir`.
fn ginary_with_path(dir: &Path) -> Command {
    let mut command = Command::cargo_bin("ginary").expect("the `ginary` binary is built for tests");
    command.current_dir(env!("CARGO_MANIFEST_DIR"));
    command.env("PATH", dir);
    command
}

/// Writes an `erl` that reports `root` as its code root, and returns its
/// directory.
///
/// The three lines are what `otp::DISCOVER_EVAL` prints: the code root, the
/// release and the ERTS version. Only the root leads anywhere here.
fn stub_erl_reporting(dir: &Path, root: &Path) -> std::path::PathBuf {
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).expect("creates the stub PATH directory");
    program(
        &bin,
        "erl",
        &[ShimStep::Print(vec![
            root.display().to_string(),
            "29".to_owned(),
            "17.0.5".to_owned(),
        ])],
    );
    bin
}

#[test]
fn the_text_report_says_why_an_unusable_installation_was_rejected() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("brokenroot");
    std::fs::create_dir_all(&root).expect("creates the broken root");
    let bin = stub_erl_reporting(dir.path(), &root);

    let assert = ginary_with_path(&bin).arg("doctor").assert().success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8");

    assert!(
        stdout.contains("erts-"),
        "the report must name what was wrong with the root:\n{stdout}"
    );
    assert!(
        stdout.contains(&root.display().to_string()),
        "the report must name the root that was rejected:\n{stdout}"
    );
    assert!(
        !stdout.contains("otp: not found"),
        "an `erl` that answered is not `not found`:\n{stdout}"
    );
}

#[test]
fn the_json_report_carries_the_reason_beside_the_null_installation() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("brokenroot");
    std::fs::create_dir_all(&root).expect("creates the broken root");
    let bin = stub_erl_reporting(dir.path(), &root);

    let assert = ginary_with_path(&bin)
        .args(["doctor", "--json"])
        .assert()
        .success();
    let value: Value = serde_json::from_slice(&assert.get_output().stdout).expect("JSON");

    assert_eq!(value["otp"], Value::Null, "{value}");
    let reason = value["otp_error"]
        .as_str()
        .unwrap_or_else(|| panic!("otp_error must hold the reason: {value}"));
    assert!(reason.contains("erts-"), "{reason}");
    assert!(reason.contains(&root.display().to_string()), "{reason}");
}

#[test]
fn a_machine_without_erl_is_told_that_that_is_the_reason() {
    let dir = tempfile::tempdir().expect("tempdir");
    let empty = dir.path().join("empty");
    std::fs::create_dir_all(&empty).expect("creates an empty PATH directory");

    let assert = ginary_with_path(&empty)
        .args(["doctor", "--json"])
        .assert()
        .success();
    let value: Value = serde_json::from_slice(&assert.get_output().stdout).expect("JSON");

    assert_eq!(value["otp"], Value::Null, "{value}");
    let reason = value["otp_error"]
        .as_str()
        .unwrap_or_else(|| panic!("otp_error must hold the reason: {value}"));
    assert!(
        reason.contains("`erl` is not on PATH"),
        "an absent `erl` must be named as the reason: {reason}"
    );
}
