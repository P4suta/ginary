// SPDX-License-Identifier: MIT OR Apache-2.0
//! The command line surface added in A1a, A1b, A1c and A2: `ginary appfile
//! parse`, the OTP installation `ginary doctor` now reports, `ginary closure`,
//! `ginary stage` with its stripping flags, and the two developer windows onto
//! the binaries an artifact is made of, `ginary beam chunks` and
//! `ginary elf deps`.
//!
//! `tests/smoke_cli.rs` covers the A0 commands. This file drives the same real
//! binary and asserts only on the user-visible contract: exit codes, the table,
//! and the JSON schema.
// The command line half of the suite: every claim in this file is about a
// module the `cli` feature carries, so a `--no-default-features` build has
// nothing here to run. See `docs/dev/log/C2.md`.
#![cfg(feature = "cli")]

mod common;

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use serde_json::Value;

use crate::common::fake_otp::{DUMMY_BEAM, FakeOtp, FakeShipment};
use crate::common::hostpath::is_absolute_for;
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
    // Absolute as *this* platform spells it: the Windows runtime prints
    // `otp root: d:/a/_temp/.setup-beam/otp`, which is absolute and does not
    // begin with a slash. See
    // `tests/regressions/e10_a_test_asked_posix_whether_a_windows_path_was_absolute.rs`.
    assert!(
        stdout.lines().any(|line| line
            .strip_prefix("otp root: ")
            .is_some_and(|root| is_absolute_for(ginary::platform::HOST, root))),
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

/// A shipment, an OTP root and an unused output path, for `stage`.
///
/// A smaller scenario than `closure_trees`: `stage` is covered in depth by
/// `tests/assemble.rs`, and what is left for the command line is that the
/// flags reach the function, the report is printed, and a failure is an exit
/// code rather than a panic. The runtime carries two spare programs so that
/// `--extra-bin` and the exclusion list have something to work on, `crypto`
/// carries one piece of junk so that `--keep-junk` has something to keep, and
/// `sasl` is in the library and reachable from nothing, so that `--extra` is
/// the only thing that can put it in the tree.
fn stage_trees() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let shipment = FakeShipment::new()
        .app_with("notify", "1.0.0", |app| {
            app.applications(&["gleam_crypto"])
                .priv_file("greeting.txt", b"hello from priv\n")
        })
        .app("gleam_crypto", "0.4.0", &["crypto"])
        .build_in(dir.path().join("shipment"));
    std::fs::create_dir_all(dir.path().join("otp")).expect("the otp root");
    let otp = FakeOtp::new()
        .extra_erts_bins(&["epmd", "heart"])
        .app_with("crypto", "5.9.2", |app| {
            app.applications(&["kernel", "stdlib"])
                .priv_file("lib/crypto.so", b"a fake NIF")
                .priv_file("lib/libcrypto_static.a", b"a fake static archive")
        })
        .app("sasl", "4.3", &["kernel", "stdlib"])
        .build_in(dir.path().join("otp"));
    let shipment_root = shipment.root.clone();
    let otp_root = otp.root.clone();
    let out = dir.path().join("out");
    (dir, shipment_root, otp_root, out)
}

/// `ginary stage <shipment> --otp-root <otp> --root notify --out <out>`.
fn stage_command(shipment: &Path, otp: &Path, out: &Path) -> Command {
    let mut command = ginary();
    command
        .arg("stage")
        .arg(shipment)
        .arg("--otp-root")
        .arg(otp)
        .args(["--root", "notify"])
        .arg("--out")
        .arg(out);
    command
}

#[test]
fn the_help_lists_the_stage_command() {
    let assert = ginary().arg("--help").assert().success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8");
    assert!(stdout.contains("stage"), "{stdout}");
}

#[test]
fn stage_without_an_out_directory_is_a_usage_error() {
    let (_dir, shipment, otp, _out) = stage_trees();

    ginary()
        .arg("stage")
        .arg(&shipment)
        .arg("--otp-root")
        .arg(&otp)
        .args(["--root", "notify"])
        .assert()
        .code(2);
}

#[test]
fn stage_writes_the_tree_and_prints_the_totals() {
    let (_dir, shipment, otp, out) = stage_trees();

    let assert = stage_command(&shipment, &otp, &out).assert().success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8");

    assert_eq!(
        stdout
            .lines()
            .next()
            .map(|line| line.split_whitespace().collect::<Vec<_>>()),
        Some(vec!["category", "bytes", "files"]),
        "the default output is the per-category table:\n{stdout}"
    );
    assert!(
        stdout.contains("erts_binary") && stdout.contains("gleam_beam"),
        "{stdout}"
    );
    assert!(
        stdout.lines().any(|line| line.starts_with("staged ")
            && line.contains(" files, ")
            && line.contains(&out.display().to_string())),
        "the last line names the file count, the byte count and the directory:\n{stdout}"
    );
    assert!(
        out.join("ginary.stage.json").is_file(),
        "no listing written"
    );
    assert!(out.join("bin/no_dot_erlang.boot").is_file());
    assert!(out.join("lib/notify/ebin/notify.app").is_file());
}

#[test]
fn stage_json_carries_the_documented_keys() {
    let (_dir, shipment, otp, out) = stage_trees();

    let assert = stage_command(&shipment, &otp, &out)
        .arg("--json")
        .assert()
        .success();
    let value: Value = serde_json::from_slice(&assert.get_output().stdout).expect("JSON");

    assert_eq!(value["format_version"], Value::from(2));
    assert_eq!(value["erts_vsn"], Value::from("17.0.5"));
    assert_eq!(value["otp_release"], Value::from(29));
    assert_eq!(value["otp_version"], Value::from("29.0.5"));
    assert_eq!(value["root"], Value::from(out.display().to_string()));
    let apps = value["apps"].as_array().expect("apps is an array");
    let names: Vec<&str> = apps.iter().filter_map(|app| app["name"].as_str()).collect();
    assert_eq!(
        names,
        ["crypto", "gleam_crypto", "kernel", "notify", "stdlib"]
    );
    assert!(
        value["files"]
            .as_array()
            .is_some_and(|files| !files.is_empty()),
        "{value}"
    );
}

#[test]
fn stage_explain_names_the_binaries_it_left_out() {
    let (_dir, shipment, otp, out) = stage_trees();

    let assert = stage_command(&shipment, &otp, &out)
        .args(["--extra-bin", "heart", "--explain"])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8");

    assert!(stdout.contains("excluded erts binaries:"), "{stdout}");
    assert!(
        stdout.contains("epmd") && !stdout.contains("\n  heart "),
        "`heart` was staged with --extra-bin, so it is not an exclusion:\n{stdout}"
    );
    assert!(stdout.contains("boot references checked:"), "{stdout}");
    assert!(out.join("erts-17.0.5/bin/heart").is_file());
}

#[test]
fn stage_refuses_a_non_empty_output_directory_and_exits_one() {
    let (_dir, shipment, otp, out) = stage_trees();
    std::fs::create_dir_all(&out).expect("the output directory");
    std::fs::write(out.join("occupied.txt"), b"not mine\n").expect("a file in the way");

    let assert = stage_command(&shipment, &otp, &out).assert().code(1);
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf-8");

    assert!(
        stderr.contains("already exists and is not empty"),
        "{stderr}"
    );
    assert!(stderr.contains("--force"), "the fix is named: {stderr}");
    assert!(
        assert.get_output().stdout.is_empty(),
        "no partial report may be printed"
    );
}

#[test]
fn stage_with_force_replaces_a_non_empty_output_directory() {
    let (_dir, shipment, otp, out) = stage_trees();
    std::fs::create_dir_all(&out).expect("the output directory");
    std::fs::write(out.join("occupied.txt"), b"not mine\n").expect("a file in the way");

    stage_command(&shipment, &otp, &out)
        .arg("--force")
        .assert()
        .success();

    assert!(!out.join("occupied.txt").exists());
    assert!(out.join("ginary.stage.json").is_file());
}

#[test]
fn stage_with_keep_junk_keeps_the_files_the_default_deletes() {
    let (_dir, shipment, otp, out) = stage_trees();

    stage_command(&shipment, &otp, &out)
        .arg("--keep-junk")
        .assert()
        .success();

    assert!(
        out.join("lib/crypto-5.9.2/priv/lib/libcrypto_static.a")
            .is_file(),
        "--keep-junk has to reach StageOptions::remove_junk, or the file is gone"
    );
    assert!(out.join("lib/crypto-5.9.2/priv/lib/crypto.so").is_file());
}

#[test]
fn stage_without_keep_junk_removes_the_same_files_and_says_so() {
    let (_dir, shipment, otp, out) = stage_trees();

    let assert = stage_command(&shipment, &otp, &out)
        .arg("--explain")
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8");

    assert!(
        !out.join("lib/crypto-5.9.2/priv/lib/libcrypto_static.a")
            .exists(),
        "the default is to remove the junk"
    );
    assert!(
        stdout.contains("junk removed:") && stdout.contains("libcrypto_static.a"),
        "a removal is a reported decision, not a silent one:\n{stdout}"
    );
}

#[test]
fn stage_with_an_extra_application_stages_it_beside_the_closure() {
    let (_dir, shipment, otp, out) = stage_trees();

    let assert = stage_command(&shipment, &otp, &out)
        .args(["--extra", "sasl"])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8");

    assert!(
        out.join("lib/sasl-4.3/ebin/sasl.app").is_file(),
        "--extra has to reach the closure the staging is built from:\n{stdout}"
    );
}

#[test]
fn stage_without_the_extra_application_leaves_it_out() {
    let (_dir, shipment, otp, out) = stage_trees();

    stage_command(&shipment, &otp, &out).assert().success();

    assert!(
        !out.join("lib/sasl-4.3").exists(),
        "nothing reaches the tree that the closure did not ask for"
    );
}

// ---------------------------------------------------------------------------
// A2: stripping, the size report, and the two binary windows.
// ---------------------------------------------------------------------------

/// The same trees as [`stage_trees`], with a stub `bin/erl` in the runtime.
///
/// `src/strip.rs` runs the OTP installation's own `erl` by absolute path, so
/// without one the beam step can only be skipped. The stub writes its argument
/// vector to `<otp>/bin/erl.argv` and exits, which is how a command line test
/// asserts on the one-liner ginary passes without an Erlang installed.
fn stage_trees_with_erl() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
    stage_trees_with(FakeOtp::with_erl_script)
}

/// The same trees again, with a stub `bin/erl` that shrinks every module it is
/// given.
///
/// The difference matters to exactly one test: a stub that changes no bytes
/// cannot show whether `ginary.stage.json` was rewritten after stripping,
/// because nothing on disk moved.
fn stage_trees_with_shrinking_erl() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
    stage_trees_with(FakeOtp::with_shrinking_erl_script)
}

/// Builds the trees, letting the caller choose which stub `erl` the runtime
/// carries.
fn stage_trees_with(erl: fn(FakeOtp) -> FakeOtp) -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let shipment = FakeShipment::new()
        .app_with("notify", "1.0.0", |app| {
            app.applications(&["gleam_crypto"])
                .priv_file("greeting.txt", b"hello from priv\n")
        })
        .app("gleam_crypto", "0.4.0", &["crypto"])
        .build_in(dir.path().join("shipment"));
    std::fs::create_dir_all(dir.path().join("otp")).expect("the otp root");
    let otp = erl(FakeOtp::new())
        // The same two programs `stage_trees` leaves in the runtime's `bin`, so
        // that `--explain` has an exclusion to print and the assertion that it
        // still prints one under the strip table is not vacuous.
        .extra_erts_bins(&["epmd", "heart"])
        .app("crypto", "5.9.2", &["kernel", "stdlib"])
        .build_in(dir.path().join("otp"));
    let shipment_root = shipment.root.clone();
    let otp_root = otp.root.clone();
    let out = dir.path().join("out");
    (dir, shipment_root, otp_root, out)
}

/// The argument vector the stub `bin/erl` under `otp` was called with.
fn erl_argv(otp: &Path) -> Vec<String> {
    match std::fs::read_to_string(otp.join("bin/erl.argv")) {
        Ok(text) => text.lines().map(str::to_owned).collect(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => panic!("cannot read the argv log: {error}"),
    }
}

/// Every `.beam` under `root`, as absolute paths, in staged-tree path order.
///
/// The list ginary has to hand `beam_lib:strip_files/1`, derived here by this
/// file's own walk rather than from the code's, so that the assertion is about
/// the tree and not about the implementation agreeing with itself.
fn staged_modules(root: &Path) -> Vec<String> {
    fn collect(root: &Path, dir: &Path, into: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).expect("a readable directory") {
            let path = entry.expect("a readable entry").path();
            if path.is_dir() {
                collect(root, &path, into);
            } else if path
                .extension()
                .is_some_and(|extension| extension == "beam")
            {
                into.push(
                    path.strip_prefix(root)
                        .expect("a path under the root")
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
        }
    }

    let mut found = Vec::new();
    collect(root, root, &mut found);
    found.sort();
    found
        .iter()
        .map(|relative| root.join(relative).display().to_string())
        .collect()
}

/// A `.beam` fixture path as it appears in the command's own output.
fn beam_fixture(name: &str) -> String {
    format!("tests/fixtures/beam/{name}")
}

#[test]
fn the_help_lists_the_beam_and_elf_commands() {
    let assert = ginary().arg("--help").assert().success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8");

    assert!(stdout.contains("beam"), "{stdout}");
    assert!(stdout.contains("elf"), "{stdout}");
}

#[test]
fn beam_chunks_prints_the_whole_table_and_the_debug_info_line() {
    let assert = ginary()
        .args(["beam", "chunks", &beam_fixture("gleam@bool.beam")])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8");

    insta::assert_snapshot!("beam_chunks_table", stdout);
}

#[test]
fn beam_chunks_json_carries_every_chunk_of_every_file_in_order() {
    let assert = ginary()
        .args([
            "beam",
            "chunks",
            "--json",
            &beam_fixture("gleam@bool.beam"),
            &beam_fixture("gleam@string.beam"),
        ])
        .assert()
        .success();
    let value: Value = serde_json::from_slice(&assert.get_output().stdout).expect("JSON");

    assert_eq!(value["format_version"], Value::from(1));
    let files = value["files"].as_array().expect("files is an array");
    assert_eq!(files.len(), 2);
    assert_eq!(
        files[0]["path"],
        Value::from(beam_fixture("gleam@bool.beam")),
        "the files keep the order they were given in"
    );
    assert_eq!(files[0]["debug_info"], Value::from(true));
    let first = &files[0]["chunks"][0];
    assert_eq!(first["id"], Value::from("AtU8"));
    assert_eq!(first["offset"], Value::from(20));
    assert_eq!(first["len"], Value::from(162));
    let ids: Vec<&str> = files[1]["chunks"]
        .as_array()
        .expect("chunks is an array")
        .iter()
        .filter_map(|chunk| chunk["id"].as_str())
        .collect();
    assert!(ids.contains(&"Code") && ids.contains(&"Dbgi"), "{ids:?}");
}

#[test]
fn beam_chunks_reports_a_file_that_is_not_a_module_and_exits_one() {
    let assert = ginary()
        .args(["beam", "chunks", &fixture("nested.app")])
        .assert()
        .code(1);
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf-8");

    assert!(
        stderr.contains(&fixture("nested.app")),
        "the file that could not be read is named: {stderr}"
    );
    assert!(
        assert.get_output().stdout.is_empty(),
        "no partial table may be printed"
    );
}

#[test]
fn beam_chunks_without_a_path_is_a_usage_error() {
    ginary().args(["beam", "chunks"]).assert().code(2);
}

// `ginary elf deps` reads an ELF, and the only file a test can point it at
// without a toolchain or a checked-in blob is the binary this run built. That
// is an ELF on Linux, a Mach-O on macOS and a PE on Windows, so the three
// claims about what it *contains* — its `libc.so.6`, its glibc floor, its
// `ET_DYN` — are claims only a host whose linker writes ELF can be asked. The
// format-blind half of the command, `elf_deps_reports_a_file_that_is_not_an_elf_
// and_exits_one`, is ungated and runs everywhere. This is the same scoping
// `tests/elf.rs` applies to `current_exe` and E8's Fix round 1 applied to
// `tests/erts_source.rs`; see `docs/dev/log/E8.md` section 14.
#[cfg(target_os = "linux")]
#[test]
fn elf_deps_prints_what_the_binary_needs() {
    let binary = assert_cmd::cargo::cargo_bin("ginary");
    let assert = ginary()
        .args(["elf", "deps"])
        .arg(&binary)
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8");

    assert!(
        stdout.lines().next() == Some(binary.display().to_string().as_str()),
        "the first line names the file: {stdout}"
    );
    for label in [
        "class",
        "machine",
        "interp",
        "pie",
        "stripped",
        "glibc_max",
        "needed",
    ] {
        assert!(stdout.contains(label), "no `{label}` line:\n{stdout}");
    }
    assert!(stdout.contains("libc.so.6"), "{stdout}");
}

// A host whose linker writes ELF, for the reason above.
#[cfg(target_os = "linux")]
#[test]
fn elf_deps_text_lists_each_named_binary_under_its_own_path() {
    // Two files, so the text form's per-file separator and its whole
    // block-per-binary layout are exercised rather than only the single-file
    // case the JSON test drives.
    let binary = assert_cmd::cargo::cargo_bin("ginary");
    let assert = ginary()
        .args(["elf", "deps"])
        .arg(&binary)
        .arg(&binary)
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8");

    let path = binary.display().to_string();
    assert_eq!(
        stdout.matches(&path).count(),
        2,
        "each of the two named binaries heads its own block:\n{stdout}"
    );
    for label in ["class", "machine", "interp", "pie", "stripped"] {
        assert!(stdout.contains(label), "no `{label}` line:\n{stdout}");
    }
    // The two blocks are separated by a blank line: the second path is preceded
    // by an empty line the first is not.
    assert!(
        stdout.contains(&format!("\n\n{path}")),
        "a blank line separates the two file blocks:\n{stdout}"
    );
}

// A host whose linker writes ELF, for the reason above.
#[cfg(target_os = "linux")]
#[test]
fn elf_deps_json_carries_the_documented_keys() {
    let binary = assert_cmd::cargo::cargo_bin("ginary");
    let assert = ginary()
        .args(["elf", "deps", "--json"])
        .arg(&binary)
        .assert()
        .success();
    let value: Value = serde_json::from_slice(&assert.get_output().stdout).expect("JSON");

    assert_eq!(value["format_version"], Value::from(1));
    let file = &value["files"][0];
    assert_eq!(file["path"], Value::from(binary.display().to_string()));
    assert_eq!(file["class"], Value::from(64));
    assert_eq!(file["machine"], Value::from(std::env::consts::ARCH));
    assert!(
        file["needed"]
            .as_array()
            .is_some_and(|needed| needed.iter().any(|name| name == "libc.so.6")),
        "{value}"
    );
    assert!(file["interp"].is_string(), "{value}");
    assert!(file["glibc_max"].is_string(), "{value}");
    assert_eq!(file["is_pie"], Value::from(true));
    assert!(file["stripped"].is_boolean(), "{value}");
    assert_eq!(
        file["kind"],
        Value::from("shared_object"),
        "a cargo binary is a position-independent executable, and `e_type` \
         calls that an `ET_DYN` like any other shared object: {value}"
    );

    // The other half of that mapping: the same file with `e_type` patched to
    // `ET_EXEC` is the one thing `kind` distinguishes and `interp` does not.
    let dir = tempfile::tempdir().expect("a temporary directory");
    let patched = dir.path().join("non-pie");
    std::fs::write(
        &patched,
        et_exec(&std::fs::read(&binary).expect("the test binary")),
    )
    .expect("write the patched binary");
    let assert = ginary()
        .args(["elf", "deps", "--json"])
        .arg(&patched)
        .assert()
        .success();
    let value: Value = serde_json::from_slice(&assert.get_output().stdout).expect("JSON");
    let file = &value["files"][0];

    assert_eq!(file["kind"], Value::from("executable"), "{value}");
    assert_eq!(file["is_pie"], Value::from(false), "{value}");
}

/// The same ELF with `e_type` set to `ET_EXEC`.
///
/// Reachable only from `elf_deps_json_carries_the_documented_keys`, which is
/// gated on a host whose linker writes ELF, so this is too.
///
/// `e_type` is the two bytes at offset 16 of the header, in the file's own
/// byte order; every target ginary builds for is little-endian, and the
/// assertion below says so rather than assuming it.
#[cfg(target_os = "linux")]
fn et_exec(elf: &[u8]) -> Vec<u8> {
    const ELFDATA2LSB: u8 = 1;
    const ET_EXEC: u16 = 2;

    let mut bytes = elf.to_vec();
    assert_eq!(bytes[5], ELFDATA2LSB, "a little-endian ELF");
    bytes[16..18].copy_from_slice(&ET_EXEC.to_le_bytes());
    bytes
}

#[test]
fn elf_deps_reports_a_file_that_is_not_an_elf_and_exits_one() {
    let assert = ginary()
        .args(["elf", "deps", &fixture("nested.app")])
        .assert()
        .code(1);
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf-8");

    assert!(stderr.contains(&fixture("nested.app")), "{stderr}");
    assert!(stderr.contains("not an ELF file"), "{stderr}");
}

#[test]
fn stage_strips_by_default_and_prints_the_strip_table() {
    let (_dir, shipment, otp, out) = stage_trees_with_erl();

    let assert = stage_command(&shipment, &otp, &out).assert().success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8");

    assert!(
        stdout.contains("elf:   nothing to strip"),
        "a fake runtime holds no native code, and the table says so:\n{stdout}"
    );
    assert!(
        stdout.lines().any(|line| line.starts_with("beams: ")
            && line.contains(" files, ")
            && line.contains(" saved")),
        "the beam step ran and reported its files:\n{stdout}"
    );
    assert!(
        stdout.lines().any(|line| line.starts_with("total: ")),
        "{stdout}"
    );
    assert!(
        !erl_argv(&otp).is_empty(),
        "stripping is on by default, so the runtime was started"
    );
}

#[test]
fn stage_runs_the_otp_roots_own_erl_with_the_beam_lib_one_liner() {
    let (_dir, shipment, otp, out) = stage_trees_with_erl();

    stage_command(&shipment, &otp, &out).assert().success();

    let mut expected = vec![
        "-noshell".to_owned(),
        "-env".to_owned(),
        "ERL_CRASH_DUMP".to_owned(),
        "/dev/null".to_owned(),
        "-eval".to_owned(),
        "Files=init:get_plain_arguments(), case beam_lib:strip_files(Files) of {ok,_} -> \
         halt(0); Err -> io:format(standard_error,\"~p~n\",[Err]), halt(1) end."
            .to_owned(),
        "-extra".to_owned(),
    ];
    let modules = staged_modules(&out);
    assert!(!modules.is_empty(), "the command stages modules");
    expected.extend(modules);
    assert_eq!(erl_argv(&otp), expected);
}

#[test]
fn stage_skips_the_beam_step_when_the_otp_root_holds_no_erl() {
    // `stage_trees` builds a runtime without the stub. A missing `erl` is a
    // reported skip and not a failure: the tree still stages.
    let (_dir, shipment, otp, out) = stage_trees();

    let assert = stage_command(&shipment, &otp, &out).assert().success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8");

    assert!(
        stdout.contains("beams: skipped:")
            && stdout.contains(&otp.join("bin/erl").display().to_string()),
        "the skip names the `erl` that was looked for:\n{stdout}"
    );
    assert!(
        out.join("ginary.stage.json").is_file(),
        "the tree still staged"
    );
}

#[test]
fn stage_with_no_strip_reports_both_halves_as_not_asked_for() {
    let (_dir, shipment, otp, out) = stage_trees_with_erl();

    let assert = stage_command(&shipment, &otp, &out)
        .arg("--no-strip")
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8");

    assert!(stdout.contains("elf:   not asked for"), "{stdout}");
    assert!(stdout.contains("beams: not asked for"), "{stdout}");
    assert!(
        erl_argv(&otp).is_empty(),
        "--no-strip must not start the runtime"
    );
}

#[test]
fn stage_with_strip_elf_only_leaves_the_modules_alone() {
    let (_dir, shipment, otp, out) = stage_trees_with_erl();

    let assert = stage_command(&shipment, &otp, &out)
        .arg("--strip-elf-only")
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8");

    assert!(stdout.contains("beams: not asked for"), "{stdout}");
    assert!(erl_argv(&otp).is_empty());
}

#[test]
fn stage_with_strip_beams_only_leaves_the_native_binaries_alone() {
    let (_dir, shipment, otp, out) = stage_trees_with_erl();

    let assert = stage_command(&shipment, &otp, &out)
        .arg("--strip-beams-only")
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8");

    assert!(stdout.contains("elf:   not asked for"), "{stdout}");
    assert!(!erl_argv(&otp).is_empty());
}

#[test]
fn stage_with_no_strip_and_a_strip_only_flag_is_a_usage_error() {
    let (_dir, shipment, otp, out) = stage_trees_with_erl();

    stage_command(&shipment, &otp, &out)
        .args(["--no-strip", "--strip-elf-only"])
        .assert()
        .code(2);
}

#[test]
fn stage_with_both_strip_only_flags_is_a_usage_error() {
    let (_dir, shipment, otp, out) = stage_trees_with_erl();

    stage_command(&shipment, &otp, &out)
        .args(["--strip-elf-only", "--strip-beams-only"])
        .assert()
        .code(2);
}

#[test]
fn stage_prints_the_needs_line_under_the_tables() {
    let (_dir, shipment, otp, out) = stage_trees_with_erl();

    let assert = stage_command(&shipment, &otp, &out).assert().success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8");

    assert!(
        stdout.contains("needs: (none)"),
        "a fake tree needs nothing, and the line still has to be there:\n{stdout}"
    );
}

#[test]
fn stage_explain_includes_the_strip_table_and_the_needs_line() {
    let (_dir, shipment, otp, out) = stage_trees_with_erl();

    let assert = stage_command(&shipment, &otp, &out)
        .arg("--explain")
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8");

    assert!(stdout.contains("excluded erts binaries:"), "{stdout}");
    assert!(stdout.contains("elf:   nothing to strip"), "{stdout}");
    assert!(stdout.contains("needs: "), "{stdout}");
}

#[test]
fn stage_with_report_json_prints_the_report_and_nothing_else() {
    let (_dir, shipment, otp, out) = stage_trees_with_erl();

    let assert = stage_command(&shipment, &otp, &out)
        .args(["--report", "json"])
        .assert()
        .success();
    let value: Value = serde_json::from_slice(&assert.get_output().stdout)
        .expect("the whole of standard output is one JSON object");

    assert_eq!(value["format_version"], Value::from(1));
    assert_eq!(
        value["strip"]["elf"]["status"],
        Value::from("nothing_to_strip")
    );
    assert_eq!(value["strip"]["beams"]["status"], Value::from("stripped"));
    assert!(
        value["categories"]["gleam_beam"]["files"].is_number(),
        "{value}"
    );
    assert!(value["total_before"].is_number(), "{value}");
    assert!(value["total_after"].is_number(), "{value}");
    assert_eq!(
        value["needs_summary"]["needed"],
        Value::from(Vec::<String>::new())
    );
}

#[test]
fn stage_with_report_json_and_json_is_a_usage_error() {
    let (_dir, shipment, otp, out) = stage_trees_with_erl();

    stage_command(&shipment, &otp, &out)
        .args(["--report", "json", "--json"])
        .assert()
        .code(2);
}

#[test]
fn stage_with_report_json_and_explain_is_a_usage_error() {
    // `--report json` prints the report alone, so an `--explain` beside it
    // would be silently dropped. Refusing it says so instead.
    let (_dir, shipment, otp, out) = stage_trees_with_erl();

    stage_command(&shipment, &otp, &out)
        .args(["--report", "json"])
        .arg("--explain")
        .assert()
        .code(2);
}

#[test]
fn stage_with_report_text_and_json_asks_for_the_default_and_is_accepted() {
    // The conflict belongs to the *value* `json`, not to the `--report` flag:
    // `--report text` is the default, and spelling a default out loud must
    // never turn a working command line into a usage error.
    let (_dir, shipment, otp, out) = stage_trees_with_erl();

    let assert = stage_command(&shipment, &otp, &out)
        .args(["--report", "text"])
        .arg("--json")
        .assert()
        .success();
    let value: Value = serde_json::from_slice(&assert.get_output().stdout)
        .expect("--json still prints the staging object");

    assert!(value["report"]["categories"].is_object(), "{value}");
}

#[test]
fn stage_json_carries_the_strip_and_report_members() {
    let (_dir, shipment, otp, out) = stage_trees_with_erl();

    let assert = stage_command(&shipment, &otp, &out)
        .arg("--json")
        .assert()
        .success();
    let value: Value = serde_json::from_slice(&assert.get_output().stdout).expect("JSON");

    assert_eq!(value["format_version"], Value::from(2));
    assert_eq!(
        value["strip"]["elf"]["status"],
        Value::from("nothing_to_strip")
    );
    assert!(value["strip"]["per_file"].is_array(), "{value}");
    assert!(value["report"]["categories"].is_object(), "{value}");
    assert!(value["report"]["needs_summary"].is_object(), "{value}");
}

#[test]
fn stage_rewrites_the_listing_so_its_sizes_match_the_stripped_tree() {
    // The listing is the tree's description of itself, and stripping rewrites
    // the tree. A `ginary.stage.json` still holding the pre-strip sizes would
    // be trusted by every later phase.
    //
    // The runtime here is the *shrinking* stub, because a stub that changed no
    // bytes would leave every size already correct and this whole test would
    // pass with `StagedRoot::refresh` deleted.
    let (_dir, shipment, otp, out) = stage_trees_with_shrinking_erl();

    stage_command(&shipment, &otp, &out).assert().success();

    let text = std::fs::read_to_string(out.join("ginary.stage.json")).expect("the listing");
    let listing: Value = serde_json::from_str(&text).expect("the listing parses");
    let mut shrunken = 0;
    for file in listing["files"].as_array().expect("files is an array") {
        let path = file["path"].as_str().expect("a path");
        let size = file["size"].as_u64().expect("a size");
        let actual = std::fs::metadata(out.join(path))
            .unwrap_or_else(|error| panic!("cannot stat {path}: {error}"))
            .len();
        assert_eq!(size, actual, "the listing's size for `{path}` is stale");
        if path.ends_with(".beam") && size < DUMMY_BEAM.len() as u64 {
            shrunken += 1;
        }
    }
    assert!(
        shrunken > 0,
        "the stub has to have changed the tree, or this test asserts nothing"
    );
}

// ------------------------------------------------------------ ginary cache --

/// A `ginary` whose cache resolution is pinned to `root` and nothing else.
///
/// Every other variable is cleared: a cache test that read the developer's own
/// `XDG_CACHE_HOME` would empty their cache and pass while doing it.
fn ginary_with_cache(root: &Path) -> Command {
    let mut command = ginary();
    command.env_clear().env("GINARY_CACHE_DIR", root);
    crate::common::coverage::preserve_coverage_env_assert(&mut command);
    command
}

/// Plants `<root>/<app>/<key>/ginary.json`, the shape one complete entry has.
fn plant_entry(root: &Path, app: &str, bytes: &[u8]) {
    let entry = root.join(app).join("0123456789abcdef");
    std::fs::create_dir_all(&entry).expect("create a cache entry");
    std::fs::write(entry.join("ginary.json"), bytes).expect("write the marker");
}

#[test]
fn cache_dir_prints_the_root_and_the_rule_that_produced_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("cache");
    let assert = ginary_with_cache(&root)
        .args(["cache", "dir"])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8");
    assert_eq!(
        stdout,
        format!("cache dir: {} (from GINARY_CACHE_DIR)\n", root.display()),
        "the provenance is the point: a path without it does not say why it is that path"
    );
}

#[test]
fn cache_dir_json_carries_the_provenance_and_the_fallback_flag() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("cache");
    let assert = ginary_with_cache(&root)
        .args(["cache", "dir", "--json"])
        .assert()
        .success();
    let value: Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("`cache dir --json` is JSON");
    assert_eq!(value["format_version"], Value::from(1));
    assert_eq!(value["path"], Value::from(root.display().to_string()));
    assert_eq!(value["origin"], Value::from("GINARY_CACHE_DIR"));
    assert_eq!(value["is_fallback"], Value::from(false));
}

#[test]
fn cache_dir_reports_the_temporary_fallback_when_nothing_is_set() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut command = ginary();
    command.env_clear().env("TMPDIR", dir.path());
    crate::common::coverage::preserve_coverage_env_assert(&mut command);
    let assert = command.args(["cache", "dir", "--json"]).assert().success();
    let value: Value = serde_json::from_slice(&assert.get_output().stdout).expect("JSON");
    assert_eq!(value["origin"], Value::from("TMPDIR fallback"));
    assert_eq!(value["is_fallback"], Value::from(true));
    assert!(
        value["path"]
            .as_str()
            .is_some_and(|path| path.starts_with(&dir.path().display().to_string())),
        "the fallback must live under TMPDIR, and it is {:?}",
        value["path"]
    );
}

#[test]
fn cache_clean_empties_one_application_and_leaves_the_others() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("cache");
    plant_entry(&root, "hello", b"{}");
    plant_entry(&root, "other", b"{}");

    let assert = ginary_with_cache(&root)
        .args(["cache", "clean", "--app", "hello"])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8");

    assert!(
        stdout.contains(&format!("removed {}", root.join("hello").display())),
        "the removal must name what went, and it said:\n{stdout}"
    );
    assert!(
        stdout.contains("total: 1 directory, 2 bytes"),
        "the summary must count what went, and it said:\n{stdout}"
    );
    assert!(!root.join("hello").exists());
    assert!(
        root.join("other").is_dir(),
        "`--app` must not empty the whole cache"
    );
}

#[test]
fn cache_clean_without_an_application_empties_the_root_and_keeps_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("cache");
    plant_entry(&root, "hello", b"{}");
    plant_entry(&root, "other", b"{}");

    let assert = ginary_with_cache(&root)
        .args(["cache", "clean", "--json"])
        .assert()
        .success();
    let value: Value = serde_json::from_slice(&assert.get_output().stdout).expect("JSON");

    assert_eq!(value["format_version"], Value::from(1));
    assert_eq!(value["app"], Value::Null);
    assert_eq!(value["bytes"], Value::from(4));
    assert_eq!(
        value["removed"],
        Value::from(vec![
            root.join("hello").display().to_string(),
            root.join("other").display().to_string(),
        ])
    );
    assert!(root.is_dir(), "the cache root itself stays");
    assert_eq!(std::fs::read_dir(&root).expect("list the root").count(), 0);
}

#[test]
fn cache_clean_of_a_cache_that_was_never_created_removes_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("absent");
    let assert = ginary_with_cache(&root)
        .args(["cache", "clean"])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8");
    assert_eq!(stdout, "total: 0 directories, 0 bytes\n");
    assert!(!root.exists(), "cleaning must not create the root");
}

// ------------------------------------------------------ ginary cache prune --

/// Plants `<root>/<app>/<key>/ginary.json` and back-dates it by `days`.
fn plant_aged(root: &Path, app: &str, key: &str, days: u64) -> PathBuf {
    let entry = root.join(app).join(key);
    std::fs::create_dir_all(&entry).expect("create a cache entry");
    let manifest = entry.join("ginary.json");
    std::fs::write(&manifest, b"{}\n").expect("write the marker");
    crate::common::cachefs::set_mtime(
        &manifest,
        std::time::SystemTime::now()
            .checked_sub(crate::common::cachefs::DAY * u32::try_from(days).expect("a day count"))
            .expect("a date the clock can hold"),
    );
    entry
}

#[test]
fn the_cache_help_lists_prune_beside_dir_and_clean() {
    let assert = ginary().args(["cache", "--help"]).assert().success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8");
    for name in ["dir", "clean", "prune"] {
        assert!(
            stdout.contains(name),
            "`ginary cache --help` must list `{name}`, and it said:\n{stdout}"
        );
    }
}

#[test]
fn cache_prune_removes_the_old_and_keeps_the_fresh_with_a_reason() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("cache");
    let old = plant_aged(&root, "hello", "1111111111111111", 30);
    let fresh = plant_aged(&root, "hello", "2222222222222222", 2);

    let assert = ginary_with_cache(&root)
        .args(["cache", "prune"])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8");

    assert!(
        stdout.contains(&format!("removed {}", old.display())),
        "the table must name what went, and it said:\n{stdout}"
    );
    assert!(
        stdout.contains(&format!("kept {} (fresh)", fresh.display())),
        "the table must say why something stayed, and it said:\n{stdout}"
    );
    assert!(
        stdout.contains("total: 1 removed, 1 kept"),
        "the summary must count both columns, and it said:\n{stdout}"
    );
    assert!(!old.exists());
    assert!(fresh.is_dir());
}

#[test]
fn cache_prune_days_moves_the_line_between_old_and_fresh() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("cache");
    let entry = plant_aged(&root, "hello", "1111111111111111", 5);

    ginary_with_cache(&root)
        .args(["cache", "prune", "--days", "30"])
        .assert()
        .success();
    assert!(
        entry.is_dir(),
        "five days is fresh against a thirty-day age"
    );

    ginary_with_cache(&root)
        .args(["cache", "prune", "--days", "1"])
        .assert()
        .success();
    assert!(!entry.exists(), "and stale against a one-day one");
}

#[test]
fn cache_prune_all_ignores_the_age() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("cache");
    let fresh = plant_aged(&root, "hello", "1111111111111111", 0);

    let assert = ginary_with_cache(&root)
        .args(["cache", "prune", "--all"])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8");

    assert!(!fresh.exists(), "`--all` prunes whatever the entry's age");
    assert!(
        stdout.contains("total: 1 removed, 0 kept"),
        "the summary said:\n{stdout}"
    );
}

#[test]
fn cache_prune_all_still_keeps_an_entry_a_process_is_holding() {
    let Some(tools) = require_tools(&["flock"]) else {
        return;
    };
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("cache");
    let busy = plant_aged(&root, "hello", "1111111111111111", 400);
    let lock = crate::common::cachefs::HeldLock::take(tools.path("flock"), &busy);

    let assert = ginary_with_cache(&root)
        .args(["cache", "prune", "--all"])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8");

    assert!(
        stdout.contains(&format!("kept {} (locked)", busy.display())),
        "`--all` is `whatever its age`, not `whatever is using it`; it said:\n{stdout}"
    );
    assert!(busy.join("ginary.json").is_file());
    lock.release(tools.path("flock"));
}

#[test]
fn cache_prune_app_reaches_one_application_only() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("cache");
    let hello = plant_aged(&root, "hello", "1111111111111111", 30);
    let other = plant_aged(&root, "other", "2222222222222222", 30);

    ginary_with_cache(&root)
        .args(["cache", "prune", "--app", "hello", "--all"])
        .assert()
        .success();

    assert!(!hello.exists());
    assert!(other.is_dir(), "`--app` must not reach another application");
}

#[test]
fn cache_prune_app_that_is_not_a_name_is_a_usage_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("cache");
    std::fs::create_dir_all(&root).expect("create the root");
    // Something old enough that a prune which got as far as running would take
    // it: the claim below is that the name is checked before anything is
    // removed, and a cache with nothing in it could not tell.
    let planted = plant_aged(&root, "hello", "1111111111111111", 400);

    let assert = ginary_with_cache(&root)
        .args(["cache", "prune", "--app", "../etc"])
        .assert()
        .failure();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf-8");

    assert!(
        stderr.contains("is not an application name"),
        "the refusal must be about the name rather than about the flag, and it said:\n{stderr}"
    );
    assert!(
        planted.join("ginary.json").is_file(),
        "nothing may be removed before the name is checked, and {} is four hundred days old",
        planted.display()
    );
}

#[test]
fn cache_prune_of_a_cache_that_was_never_created_removes_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("absent");

    let assert = ginary_with_cache(&root)
        .args(["cache", "prune"])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8");

    assert_eq!(stdout, "total: 0 removed, 0 kept\n");
    assert!(!root.exists(), "pruning must not create the root");
}

#[test]
fn build_help_lists_the_three_runtime_flags() {
    let assert = ginary().args(["build", "--help"]).assert().success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8");
    for flag in ["--distribution", "--vm-args", "--sys-config"] {
        assert!(
            stdout.contains(flag),
            "`ginary build --help` must list `{flag}`, and it said:\n{stdout}"
        );
    }
}

#[test]
fn build_help_names_the_flag_that_ships_native_code_anyway() {
    let assert = ginary().args(["build", "--help"]).assert().success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8");

    assert!(
        stdout.contains("--allow-native-mismatch"),
        "the refusal names this flag as its own remedy, so `build --help` has \
         to list it:\n{stdout}"
    );
}

#[test]
fn allowing_a_native_mismatch_is_a_flag_and_not_a_usage_error() {
    // Nothing is built here: the project is not one, so the run fails. What is
    // asserted is *how* — clap accepting the flag and the build then refusing
    // for its own reason, rather than clap refusing an argument it has never
    // heard of.
    let dir = tempfile::tempdir().expect("a temporary directory");
    let assert = ginary_in(dir.path())
        .args(["build", "--allow-native-mismatch"])
        .assert()
        .failure();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf-8");

    assert!(
        !stderr.contains("unexpected argument"),
        "`--allow-native-mismatch` is a flag `ginary build` has:\n{stderr}"
    );
}

// ------------------------------- `ginary build` and `ginary inspect` (A4) --

/// A `ginary` run from a directory the test owns, so no ambient `gleam.toml`
/// above the crate root can be found by an upward search.
fn ginary_in(dir: &Path) -> Command {
    let mut command = Command::cargo_bin("ginary").expect("the `ginary` binary is built for tests");
    command.current_dir(dir);
    command
}

#[test]
fn the_help_lists_the_build_and_inspect_commands_and_no_longer_calls_build_planned() {
    let assert = ginary().arg("--help").assert().success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8");

    assert!(stdout.contains("build"), "{stdout}");
    assert!(stdout.contains("inspect"), "{stdout}");

    // The long help carried a pre-alpha notice saying `build` was not
    // implemented. A command that works and a help text that says it does not
    // is a worse defect than either alone.
    let long = ginary().arg("help").assert().success();
    let long = String::from_utf8(long.get_output().stdout.clone()).expect("utf-8");
    assert!(
        !long.contains("pre-alpha"),
        "the pre-alpha notice must go when `build` lands:\n{long}"
    );
}

#[test]
fn build_outside_a_gleam_project_exits_one_and_says_where_to_run_it() {
    let dir = tempfile::tempdir().expect("a temporary directory");

    let assert = ginary_in(dir.path()).arg("build").assert().code(1);
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf-8");

    assert!(
        stderr.contains("gleam.toml") && stderr.contains("Gleam project"),
        "the message must say what was missing and what to do: {stderr}"
    );
}

#[test]
fn build_with_a_compression_level_outside_the_range_is_a_usage_error() {
    let dir = tempfile::tempdir().expect("a temporary directory");

    for level in ["0", "23", "-1"] {
        ginary_in(dir.path())
            .args(["build", "--compression-level", level])
            .assert()
            .code(2);
    }
}

#[test]
fn build_with_both_strip_only_flags_is_a_usage_error() {
    let dir = tempfile::tempdir().expect("a temporary directory");

    ginary_in(dir.path())
        .args(["build", "--strip-elf-only", "--strip-beams-only"])
        .assert()
        .code(2);
    ginary_in(dir.path())
        .args(["build", "--no-strip", "--strip-elf-only"])
        .assert()
        .code(2);
}

#[test]
fn inspect_without_a_path_is_a_usage_error() {
    ginary().arg("inspect").assert().code(2);
}

#[test]
fn inspect_of_the_command_line_tool_itself_exits_one_with_no_ginary_trailer() {
    let assert = ginary()
        .args(["inspect", env!("CARGO_BIN_EXE_ginary")])
        .assert()
        .code(1);
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf-8");

    assert!(
        stderr.contains("no ginary trailer"),
        "a plain ginary is not an artifact and must say so: {stderr}"
    );
}

#[test]
fn inspect_prints_the_application_its_versions_and_its_geometry() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let artifact = common::artifact::SyntheticArtifact::build(dir.path());

    let assert = ginary()
        .args(["inspect".as_ref(), artifact.path().as_os_str()])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8");

    for expected in [
        common::artifact::APP,
        common::artifact::OTP_VERSION,
        common::artifact::ERTS_VSN,
        &artifact.file_len().to_string(),
    ] {
        assert!(
            stdout.contains(expected),
            "the report must name `{expected}`:\n{stdout}"
        );
    }
}

#[test]
fn inspect_json_carries_the_documented_keys() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let artifact = common::artifact::SyntheticArtifact::build(dir.path());

    let assert = ginary()
        .args([
            "inspect".as_ref(),
            "--json".as_ref(),
            artifact.path().as_os_str(),
        ])
        .assert()
        .success();
    let value: Value = serde_json::from_slice(&assert.get_output().stdout).expect("JSON");

    assert_eq!(value["format_version"], Value::from(1));
    assert_eq!(
        value["path"],
        Value::from(artifact.path().display().to_string())
    );
    assert_eq!(value["payload_offset"], Value::from(artifact.stub_len()));
    assert_eq!(value["payload_len"], Value::from(artifact.packed().len));
    assert_eq!(value["total_len"], Value::from(artifact.file_len()));
    assert_eq!(
        value["payload_sha256"],
        Value::from(hex::encode(artifact.packed().sha256))
    );
    assert_eq!(value["manifest"]["app"], Value::from(common::artifact::APP));
    assert!(
        value["index"]["files"]
            .as_array()
            .is_some_and(|files| !files.is_empty()),
        "the index must list what was staged: {value}"
    );
    assert!(
        value.get("verify").is_none(),
        "a flag that was not given must be absent rather than null: {value}"
    );
    assert!(
        value.get("launch_plan").is_none(),
        "a flag that was not given must be absent rather than null: {value}"
    );
}

#[test]
fn inspect_verify_passes_on_an_intact_artifact() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let artifact = common::artifact::SyntheticArtifact::build(dir.path());

    let assert = ginary()
        .args([
            "inspect".as_ref(),
            "--verify".as_ref(),
            artifact.path().as_os_str(),
        ])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8");

    assert!(
        stdout.contains("verify: ok"),
        "an intact artifact must say so in as many words:\n{stdout}"
    );
}

#[test]
fn inspect_verify_exits_one_when_a_payload_byte_changed() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let artifact = common::artifact::SyntheticArtifact::build(dir.path());
    artifact.break_payload_tail();

    let assert = ginary()
        .args([
            "inspect".as_ref(),
            "--verify".as_ref(),
            artifact.path().as_os_str(),
        ])
        .assert()
        .code(1);
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8");
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf-8");

    assert!(
        stdout.contains("MISMATCH"),
        "the report must say which of the two digests disagreed:\n{stdout}"
    );
    assert!(
        stderr.contains("digest"),
        "the failure must be reported on standard error too: {stderr}"
    );
}

#[test]
fn inspect_without_verify_still_prints_the_manifest_of_a_damaged_artifact() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let artifact = common::artifact::SyntheticArtifact::build(dir.path());
    artifact.break_payload_tail();

    let assert = ginary()
        .args(["inspect".as_ref(), artifact.path().as_os_str()])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8");

    assert!(
        stdout.contains(common::artifact::APP),
        "a user has to be able to find out what the damaged file was supposed to be:\n{stdout}"
    );
}

#[test]
fn inspect_launch_plan_prints_the_argv_against_a_placeholder_root() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let artifact = common::artifact::SyntheticArtifact::build(dir.path());

    let assert = ginary()
        .args([
            "inspect".as_ref(),
            "--launch-plan".as_ref(),
            artifact.path().as_os_str(),
        ])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8");

    assert!(
        stdout.contains(ginary::inspect::PLACEHOLDER_ROOT),
        "the plan must be printed against the placeholder root, not this machine's cache:\n{stdout}"
    );
    assert!(
        !stdout.contains(&dir.path().display().to_string()),
        "no path from the machine that ran the inspection may reach the plan:\n{stdout}"
    );
    for expected in ["-boot", "-noshell", "-start_epmd", "-eval", "-extra"] {
        assert!(
            stdout.contains(expected),
            "the plan must show `{expected}`:\n{stdout}"
        );
    }
}

#[test]
fn inspect_launch_plan_json_carries_the_program_argv_and_env_edits() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let artifact = common::artifact::SyntheticArtifact::build(dir.path());

    let assert = ginary()
        .args([
            "inspect".as_ref(),
            "--launch-plan".as_ref(),
            "--json".as_ref(),
            artifact.path().as_os_str(),
        ])
        .assert()
        .success();
    let value: Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("inspect --json is JSON");

    let plan = value
        .get("launch_plan")
        .expect("the launch plan is present when --launch-plan is asked for");
    assert!(
        plan.get("program")
            .and_then(Value::as_str)
            .is_some_and(|program| program.contains(ginary::inspect::PLACEHOLDER_ROOT)),
        "the program is resolved against the placeholder root: {plan}"
    );
    let argv: Vec<&str> = plan
        .get("argv")
        .and_then(Value::as_array)
        .expect("argv is an array")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    for expected in ["-boot", "-noshell", "-eval", "-extra"] {
        assert!(
            argv.contains(&expected),
            "the JSON argv must carry `{expected}`: {argv:?}"
        );
    }
    assert!(
        plan.get("set").and_then(Value::as_array).is_some(),
        "the plan reports the environment it sets: {plan}"
    );
    assert!(
        plan.get("remove").and_then(Value::as_array).is_some(),
        "the plan reports the environment it removes: {plan}"
    );
}
