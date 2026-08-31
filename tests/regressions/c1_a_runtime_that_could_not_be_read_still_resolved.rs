// SPDX-License-Identifier: MIT OR Apache-2.0
//! The targets table said `resolves: yes` for a runtime nothing could read.
//!
//! **What went wrong.** `probe_target` inspected the host's own runtime and,
//! when the inspection failed, still wrote `resolvable: true` with the reason
//! in the `detail` column. The report then contradicted itself: `otp: unusable
//! (<reason>)` three lines above, and a row below it claiming the host target
//! resolves today. The column is headed `resolves` and its other unhappy
//! answers — a source this ginary cannot fetch yet, `host` on a target this
//! machine is not — print `not yet`, so `yes` beside a reason is the one row
//! that sends a user to run a build to find out what `doctor` already knew.
//!
//! **The input.** An `erl` on `PATH` that reports a code root with no `erts-*`
//! in it, which is what `a1a_doctor_dropped_the_otp_error` uses: discovery
//! succeeds as a probe and fails as an installation.
//!
//! **The correct behaviour.** The row answers `not yet` and `--json` carries
//! `"resolvable": false`, with the reason still in `detail`. `resolves`
//! answers one question — can this ginary get that runtime today — and every
//! arm answers it the same way.

#![cfg(unix)]

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use ginary::target::Target;
use serde_json::Value;

use crate::common::script::script;

/// Writes an `erl` that reports a directory that is not an OTP installation,
/// and returns the `PATH` directory holding it.
///
/// The three lines are what `otp::DISCOVER_EVAL` prints: the code root, the
/// release and the ERTS version. The root exists and has no `erts-*` under it,
/// so discovery gets an answer and then refuses it.
fn stub_erl_on_a_broken_root(dir: &Path) -> (PathBuf, PathBuf) {
    let root = dir.join("brokenroot");
    std::fs::create_dir_all(&root).expect("creates the broken root");
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).expect("creates the stub PATH directory");
    script(
        &bin,
        "erl",
        &format!("printf '%s\\n29\\n17.0.5\\n' '{}'", root.display()),
    );
    (bin, root)
}

/// A `doctor` invocation whose `PATH` is `bin` and whose working directory is
/// `cwd`, so no project above the repository can add a target to the table.
fn doctor_in(bin: &Path, cwd: &Path) -> Command {
    let mut command = Command::cargo_bin("ginary").expect("the `ginary` binary is built for tests");
    command.current_dir(cwd);
    command.env("PATH", bin);
    command.arg("doctor");
    command
}

#[test]
fn the_host_row_of_an_unusable_installation_does_not_claim_it_resolves() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (bin, root) = stub_erl_on_a_broken_root(dir.path());

    let assert = doctor_in(&bin, dir.path()).assert().success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8");

    let host = Target::host().name();
    let row = stdout
        .lines()
        .find(|line| line.split_whitespace().next() == Some(host.as_str()))
        .unwrap_or_else(|| panic!("the targets table must hold a row for {host}:\n{stdout}"));
    let resolves = row
        .split_whitespace()
        .nth(2)
        .unwrap_or_else(|| panic!("the row must have a `resolves` cell: {row}"));

    assert_ne!(
        resolves, "yes",
        "a runtime `doctor` could not read does not resolve:\n{stdout}"
    );
    assert!(
        row.contains("not yet"),
        "the row answers `resolves` the way every other unhappy row does: {row}"
    );
    assert!(
        row.contains(&root.display().to_string()),
        "and the row still says which root was refused: {row}"
    );
}

#[test]
fn the_json_row_carries_the_reason_beside_a_false_resolvable() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (bin, root) = stub_erl_on_a_broken_root(dir.path());

    let assert = doctor_in(&bin, dir.path()).arg("--json").assert().success();
    let value: Value = serde_json::from_slice(&assert.get_output().stdout).expect("JSON");

    let host = Target::host().name();
    let row = value["targets"]
        .as_array()
        .and_then(|rows| {
            rows.iter()
                .find(|row| row["name"].as_str() == Some(host.as_str()))
        })
        .unwrap_or_else(|| panic!("the JSON must hold a row for {host}: {value}"));

    assert_eq!(row["resolvable"], Value::Bool(false), "{row}");
    let detail = row["detail"]
        .as_str()
        .unwrap_or_else(|| panic!("the row must say why: {row}"));
    assert!(detail.contains(&root.display().to_string()), "{detail}");
}
