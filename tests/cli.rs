// SPDX-License-Identifier: MIT OR Apache-2.0
//! The command line surface added in A1a and A1b: `ginary appfile parse`, the
//! OTP installation `ginary doctor` now reports, and `ginary closure`.
//!
//! `tests/smoke_cli.rs` covers the A0 commands. This file drives the same real
//! binary and asserts only on the user-visible contract: exit codes, the table,
//! and the JSON schema.

mod common;

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use serde_json::Value;

use crate::common::fake_otp::{FakeOtp, FakeShipment};
use crate::common::tools::require_tools;

/// A `Command` for the `ginary` binary, run from the crate root so that the
/// fixture paths it is given — and prints back — are relative and stable.
fn ginary() -> Command {
    let mut command = Command::cargo_bin("ginary").expect("the `ginary` binary is built for tests");
    command.current_dir(env!("CARGO_MANIFEST_DIR"));
    command
}

/// A fixture path relative to the crate root, as it appears in the output.
fn fixture(relative: &str) -> String {
    format!("tests/fixtures/app/{relative}")
}

/// The absolute path of a fixture.
fn fixture_path(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(fixture(relative))
}

#[test]
fn the_help_lists_the_appfile_command() {
    let assert = ginary().arg("--help").assert().success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8");
    assert!(stdout.contains("appfile"), "{stdout}");
}

#[test]
fn appfile_parse_prints_one_labelled_block_per_file() {
    let assert = ginary()
        .args([
            "appfile",
            "parse",
            &fixture("nested.app"),
            &fixture("quoted.app"),
        ])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8");

    insta::assert_snapshot!("appfile_parse_table", stdout);
}

#[test]
fn appfile_parse_json_carries_every_field_of_the_resource() {
    let assert = ginary()
        .args(["appfile", "parse", "--json", &fixture("included.app")])
        .assert()
        .success();
    let value: Value = serde_json::from_slice(&assert.get_output().stdout).expect("JSON");

    assert_eq!(value["format_version"], Value::from(1));
    let apps = value["apps"].as_array().expect("apps is an array");
    assert_eq!(apps.len(), 1);
    let app = &apps[0];
    assert_eq!(app["path"], Value::from(fixture("included.app")));
    assert_eq!(app["name"], Value::from("included"));
    assert_eq!(app["vsn"], Value::from("2.5.0"));
    assert_eq!(app["description"], Value::from("included applications"));
    assert_eq!(
        app["applications"],
        Value::from(vec!["kernel", "stdlib", "crypto"])
    );
    assert_eq!(
        app["included_applications"],
        Value::from(vec!["sasl", "runtime_tools"])
    );
    assert_eq!(app["modules"], Value::from(vec!["included"]));
    assert_eq!(app["registered"], Value::from(vec!["included_sup"]));
    assert_eq!(app["has_mod"], Value::from(false));
    assert_eq!(app["env_keys"], Value::from(Vec::<String>::new()));
    assert_eq!(app["warnings"], Value::from(Vec::<String>::new()));
}

#[test]
fn appfile_parse_keeps_the_files_in_the_order_they_were_given() {
    let assert = ginary()
        .args([
            "appfile",
            "parse",
            "--json",
            &fixture("shipment/mist.app"),
            &fixture("otp/crypto.app"),
        ])
        .assert()
        .success();
    let value: Value = serde_json::from_slice(&assert.get_output().stdout).expect("JSON");

    let names: Vec<&str> = value["apps"]
        .as_array()
        .expect("apps is an array")
        .iter()
        .map(|app| app["name"].as_str().expect("a name"))
        .collect();
    assert_eq!(names, ["mist", "crypto"]);
}

#[test]
fn appfile_parse_without_a_path_is_a_usage_error() {
    ginary().args(["appfile", "parse"]).assert().code(2);
}

#[test]
fn appfile_without_a_subcommand_is_a_usage_error() {
    ginary().arg("appfile").assert().code(2);
}

#[test]
fn appfile_parse_reports_a_malformed_file_and_exits_one() {
    let assert = ginary()
        .args(["appfile", "parse", &fixture("malformed.app")])
        .assert()
        .code(1);
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf-8");

    assert!(stderr.contains("malformed.app"), "{stderr}");
    assert!(
        stderr.contains("line 5, column 3"),
        "the message must locate the problem: {stderr}"
    );
    assert!(
        assert.get_output().stdout.is_empty(),
        "no partial table may be printed: {:?}",
        assert.get_output().stdout
    );
}

#[test]
fn appfile_parse_reports_a_missing_file_and_exits_one() {
    let assert = ginary()
        .args(["appfile", "parse", &fixture("does_not_exist.app")])
        .assert()
        .code(1);
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf-8");

    assert!(stderr.contains("does_not_exist.app"), "{stderr}");
    assert!(
        stderr.contains("cannot read"),
        "the cause must be the failed read, not a generic message: {stderr}"
    );
    assert!(
        stderr.contains("No such file or directory") || stderr.contains("cannot find the file"),
        "the operating system's own reason must survive: {stderr}"
    );
    assert!(
        fixture_path("does_not_exist.app")
            .symlink_metadata()
            .is_err(),
        "the fixture must stay absent for this test to mean anything"
    );
}

#[test]
fn doctor_json_reports_the_otp_installation() {
    let Some(_tools) = require_tools(&["erl"]) else {
        return;
    };

    let assert = ginary().args(["doctor", "--json"]).assert().success();
    let value: Value = serde_json::from_slice(&assert.get_output().stdout).expect("JSON");

    let otp = &value["otp"];
    assert!(
        otp.is_object(),
        "`erl` is on PATH, so `otp` must not be null: {value}"
    );
    let root = otp["root"].as_str().expect("otp.root is a string");
    assert!(
        Path::new(root).is_dir(),
        "otp.root {root} is not a directory"
    );
    assert!(otp["release"].as_u64().is_some_and(|r| r >= 26), "{otp}");
    assert!(otp["erts_vsn"].is_string(), "{otp}");
    assert!(otp["otp_version"].is_string(), "{otp}");
    assert_eq!(
        value["otp_error"],
        Value::Null,
        "a successful discovery records no reason: {value}"
    );
}

#[test]
fn doctor_text_names_the_otp_root_and_version() {
    let Some(_tools) = require_tools(&["erl"]) else {
        return;
    };

    let assert = ginary().arg("doctor").assert().success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8");

    assert!(
        !stdout.contains("otp: not found"),
        "`erl` is on PATH, so doctor must report the installation:\n{stdout}"
    );
    assert!(
        stdout.lines().any(|line| line.starts_with("otp root: /")),
        "no absolute `otp root:` line in:\n{stdout}"
    );
    assert!(
        stdout
            .lines()
            .any(|line| line.starts_with("otp: ") && line.contains("release ")),
        "no `otp:` version line in:\n{stdout}"
    );
}

/// A shipment and an OTP root in one temporary directory, for `closure`.
///
/// The same six-application scenario `tests/closure.rs` uses, plus one
/// unresolvable `optional_applications` entry, so the footer the command
/// prints under the table is exercised too. The temporary directory is
/// returned because dropping it deletes both trees.
fn closure_trees() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let shipment = FakeShipment::new()
        .app("notify", "1.0.0", &["gleam_crypto"])
        .app_with("gleam_crypto", "0.4.0", |app| {
            app.applications(&["crypto"]).optional(&["observer"])
        })
        .build_in(dir.path().join("shipment"));
    std::fs::create_dir_all(dir.path().join("otp")).expect("the otp root");
    let otp = FakeOtp::new()
        .app("crypto", "5.9.2", &["kernel", "stdlib"])
        .app("sasl", "4.3.1", &["kernel", "stdlib"])
        .build_in(dir.path().join("otp"));
    let shipment_root = shipment.root.clone();
    let otp_root = otp.root.clone();
    (dir, shipment_root, otp_root)
}

#[test]
fn the_help_lists_the_closure_command() {
    let assert = ginary().arg("--help").assert().success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8");
    assert!(stdout.contains("closure"), "{stdout}");
}

#[test]
fn closure_without_a_root_is_a_usage_error() {
    let (_dir, shipment, otp) = closure_trees();

    ginary()
        .arg("closure")
        .arg(&shipment)
        .arg("--otp-root")
        .arg(&otp)
        .assert()
        .code(2);
}

#[test]
fn closure_with_both_json_and_explain_is_a_usage_error() {
    let (_dir, shipment, otp) = closure_trees();

    ginary()
        .arg("closure")
        .arg(&shipment)
        .arg("--otp-root")
        .arg(&otp)
        .args(["--root", "notify", "--json", "--explain"])
        .assert()
        .code(2);
}

#[test]
fn closure_prints_a_table_of_every_application_and_its_ebin() {
    let (_dir, shipment, otp) = closure_trees();

    let assert = ginary()
        .arg("closure")
        .arg(&shipment)
        .arg("--otp-root")
        .arg(&otp)
        .args(["--root", "notify", "--extra", "sasl"])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8");

    let names: Vec<&str> = stdout
        .lines()
        .skip(1)
        .take_while(|line| !line.is_empty())
        .filter_map(|line| line.split_whitespace().next())
        .collect();
    assert_eq!(
        names,
        [
            "crypto",
            "gleam_crypto",
            "kernel",
            "notify",
            "sasl",
            "stdlib"
        ],
        "{stdout}"
    );
    assert!(
        stdout.starts_with("name "),
        "the first line is the header: {stdout}"
    );
    assert!(
        stdout.contains(
            &otp.join("lib")
                .join("crypto-5.9.2")
                .join("ebin")
                .display()
                .to_string()
        ),
        "the default table names the directory each application is read from:\n{stdout}"
    );
    assert!(
        stdout.contains("skipped optional applications:"),
        "an unresolvable optional application is reported, not swallowed:\n{stdout}"
    );
}

#[test]
fn closure_explain_prints_the_origin_of_every_application() {
    let (_dir, shipment, otp) = closure_trees();

    let assert = ginary()
        .arg("closure")
        .arg(&shipment)
        .arg("--otp-root")
        .arg(&otp)
        .args(["--root", "notify", "--extra", "sasl", "--explain"])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8");

    insta::assert_snapshot!("closure_explain_table", stdout);
}

#[test]
fn closure_json_carries_the_documented_keys() {
    let (_dir, shipment, otp) = closure_trees();

    let assert = ginary()
        .arg("closure")
        .arg(&shipment)
        .arg("--otp-root")
        .arg(&otp)
        .args(["--root", "notify", "--extra", "sasl", "--json"])
        .assert()
        .success();
    let value: Value = serde_json::from_slice(&assert.get_output().stdout).expect("JSON");

    assert_eq!(value["format_version"], Value::from(1));
    assert_eq!(
        value["otp_lib"],
        Value::from(otp.join("lib").display().to_string())
    );
    let apps = value["apps"].as_object().expect("apps is an object");
    let names: Vec<&String> = apps.keys().collect();
    assert_eq!(
        names,
        [
            "crypto",
            "gleam_crypto",
            "kernel",
            "notify",
            "sasl",
            "stdlib"
        ]
    );
    assert_eq!(apps["notify"]["seed"], Value::from("root"));
    assert_eq!(apps["sasl"]["seed"], Value::from("extra"));
    assert_eq!(apps["crypto"]["source"]["kind"], Value::from("otp"));
    assert_eq!(apps["crypto"]["source"]["vsn"], Value::from("5.9.2"));
    assert_eq!(value["warnings"], Value::from(Vec::<String>::new()));
    assert_eq!(
        value["skipped_optional"],
        serde_json::json!([["observer", "gleam_crypto"]])
    );
}

#[test]
fn closure_reports_an_application_taken_from_the_shipment_instead_of_otp() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let shipment = FakeShipment::new()
        .app("notify", "1.0.0", &["crypto"])
        .app("crypto", "9.9.9", &[])
        .build_in(dir.path().join("shipment"));
    std::fs::create_dir_all(dir.path().join("otp")).expect("the otp root");
    let otp = FakeOtp::new()
        .app("crypto", "5.9.2", &["kernel", "stdlib"])
        .build_in(dir.path().join("otp"));

    let assert = ginary()
        .arg("closure")
        .arg(&shipment.root)
        .arg("--otp-root")
        .arg(&otp.root)
        .args(["--root", "notify"])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8");

    assert!(
        stdout.contains("\nwarnings:\n"),
        "an application taken from the shipment instead of OTP is reported, not swallowed:\n{stdout}"
    );
    assert!(
        stdout.contains(
            &shipment
                .app_dir("crypto")
                .join("ebin")
                .display()
                .to_string()
        ) && stdout.contains(
            &otp.lib()
                .join("crypto-5.9.2")
                .join("ebin")
                .display()
                .to_string()
        ),
        "the warning names the copy that was used and the one that was dropped:\n{stdout}"
    );
}

#[test]
fn closure_reports_a_missing_application_and_exits_one() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let shipment = FakeShipment::new()
        .app("notify", "1.0.0", &["gleam_crypto"])
        .app("gleam_crypto", "0.4.0", &["crypto"])
        .build_in(dir.path().join("shipment"));
    std::fs::create_dir_all(dir.path().join("otp")).expect("the otp root");
    let otp = FakeOtp::new().build_in(dir.path().join("otp"));

    let assert = ginary()
        .arg("closure")
        .arg(&shipment.root)
        .arg("--otp-root")
        .arg(&otp.root)
        .args(["--root", "notify"])
        .assert()
        .code(1);
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf-8");

    assert!(
        stderr.contains("application `crypto` was not found"),
        "{stderr}"
    );
    assert!(
        stderr.contains("required by: notify -> gleam_crypto -> crypto"),
        "the chain must name every step from the root:\n{stderr}"
    );
    assert!(
        stderr.contains("[erlang] extra_applications")
            && stderr.contains("[tools.ginary] otp_applications"),
        "the hint must name both gleam.toml keys:\n{stderr}"
    );
    assert!(
        assert.get_output().stdout.is_empty(),
        "no partial table may be printed: {:?}",
        assert.get_output().stdout
    );
}
