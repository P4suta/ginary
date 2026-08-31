// SPDX-License-Identifier: MIT OR Apache-2.0
//! End-to-end smoke tests for the `ginary` command line interface.
//!
//! These tests drive the real binary through `assert_cmd` and only assert on the
//! user-visible contract: exit codes, human output shape and the JSON schemas.
// The command line half of the suite: every claim in this file is about a
// module the `cli` feature carries, so a `--no-default-features` build has
// nothing here to run. See `docs/dev/log/C2.md`.
#![cfg(feature = "cli")]

use assert_cmd::Command;
use predicates::prelude::*;
use predicates::str::is_match;
use serde_json::Value;

/// Builds a `Command` for the `ginary` binary under test.
fn ginary() -> Command {
    Command::cargo_bin("ginary").expect("the `ginary` binary should be built for tests")
}

#[test]
fn help_succeeds_and_mentions_build() {
    let assert = ginary().arg("--help").assert().success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8 help output");
    assert!(
        stdout.contains("build"),
        "`--help` should mention the `build` command, got:\n{stdout}"
    );
}

#[test]
fn version_prints_a_semver_line() {
    ginary()
        .arg("version")
        .assert()
        .success()
        .stdout(is_match(r"(?m)^ginary \d+\.\d+\.\d+").expect("valid regex"));
}

#[test]
fn version_json_carries_version_target_and_format_version() {
    let assert = ginary().args(["version", "--json"]).assert().success();
    let value: Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("`version --json` prints JSON");

    assert!(
        is_match(r"^\d+\.\d+\.\d+")
            .expect("valid regex")
            .eval(value["version"].as_str().expect("version is a string")),
        "unexpected version field: {value:?}"
    );
    assert!(
        is_match(r"^(linux|macos|windows)-(x86_64|aarch64)(-(gnu|musl))?$")
            .expect("valid regex")
            .eval(value["target"].as_str().expect("target is a string")),
        "unexpected target field: {value:?}"
    );
    assert_eq!(value["format_version"], Value::from(1));
}

#[test]
fn no_arguments_is_a_usage_error() {
    ginary().assert().code(2);
}

#[test]
fn doctor_reports_every_probed_subject() {
    let assert = ginary().arg("doctor").assert().success();
    let stdout =
        String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8 doctor output");

    for pattern in [
        r"(?m)^host target: (linux|macos|windows)-(x86_64|aarch64)(-(gnu|musl))?$",
        r"(?m)^rustc/cargo: not required",
        r"(?m)^gleam: \S",
        r"(?m)^erl: \S",
        r"(?m)^strip: \S",
        r"(?m)^docker: \S",
        r"(?m)^cache dir: \S",
    ] {
        assert!(
            is_match(pattern).expect("valid regex").eval(&stdout),
            "doctor output does not match {pattern}:\n{stdout}"
        );
    }
}

#[test]
fn doctor_json_has_the_documented_schema() {
    let assert = ginary().args(["doctor", "--json"]).assert().success();
    let value: Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("`doctor --json` prints JSON");

    assert_eq!(value["format_version"], Value::from(1));
    assert_eq!(value["rustc_required"], Value::from(false));
    assert!(value["host_target"].is_string(), "host_target: {value:?}");
    assert!(
        value["cache_dir"].is_string() || value["cache_dir"].is_null(),
        "cache_dir: {value:?}"
    );

    let tools = value["tools"].as_array().expect("tools is an array");
    let names: Vec<&str> = tools
        .iter()
        .map(|tool| tool["name"].as_str().expect("tool name is a string"))
        .collect();
    assert_eq!(names, ["gleam", "erl", "strip", "docker"]);

    for tool in tools {
        assert!(tool["found"].is_boolean(), "found: {tool:?}");
        assert!(
            tool["version"].is_string() || tool["version"].is_null(),
            "version: {tool:?}"
        );
        assert!(
            tool["path"].is_string() || tool["path"].is_null(),
            "path: {tool:?}"
        );
    }
}
