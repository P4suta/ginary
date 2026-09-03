// SPDX-License-Identifier: MIT OR Apache-2.0
//! `ginary build --report json` failed *after* writing the artifact when the
//! output path was not UTF-8.
//!
//! **What went wrong.** `BuildReport::out` is a `PathBuf` and serde's
//! implementation for `PathBuf` refuses a path that is not valid UTF-8. The
//! JSON report is serialised after `bundle::build` returns, so the artifact
//! was already on disk, complete and executable, when the command printed
//! "cannot serialise the report" and exited 1. A wrapper reading the exit code
//! concluded that a build which had succeeded had failed. `--report text` was
//! unaffected, because `Path::display` is lossy.
//!
//! **The input.** A report whose `out` holds a byte that is not UTF-8, which
//! on Linux is an ordinary file name.
//!
//! **The correct behaviour.** The report serialises, with the path rendered
//! the same lossy way the text report and `InspectReport::path` render it. A
//! path the build handled is not a path the report may fail on.

// A unix file: a path holding a byte that is not UTF-8 is the whole subject,
// and a Windows path is UTF-16 with no such byte to write. See
// tests/regressions/e6_the_test_helpers_did_not_compile_on_windows.rs.
#![cfg(unix)]

use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt as _;
use std::path::PathBuf;

use ginary::bundle::BuildReport;
use ginary::report::SizeReport;
use ginary::strip::StripReport;

use crate::common::payload::sample_manifest;

#[test]
fn a_report_whose_output_path_is_not_utf8_still_serialises() {
    let out = PathBuf::from(OsString::from_vec(b"/w/app/out\xff/hello_ffi".to_vec()));
    let report = BuildReport {
        app: "hello_ffi".to_owned(),
        out: out.clone(),
        stub_len: 5_242_880,
        payload_len: 9_437_184,
        total_len: 5_242_880 + 9_437_184 + 64,
        sha256: "a".repeat(64),
        strip: StripReport::disabled(),
        size_report: SizeReport::default(),
        manifest: sample_manifest(),
        targets: Vec::new(),
        staging: None,
        explain: None,
        warnings: Vec::new(),
    };

    let json = serde_json::to_value(&report)
        .expect("a path the build wrote to is a path the report can name");

    assert_eq!(
        json.get("out").and_then(serde_json::Value::as_str),
        Some(out.display().to_string().as_str()),
        "the path is rendered the way every other report renders one"
    );
    // The text form was never in doubt; it is asserted here so that a fix that
    // made the JSON lossy by making the text form lossy too would be visible.
    assert!(
        report.artifact_line().contains("hello_ffi"),
        "the text form still names the artifact: {}",
        report.artifact_line()
    );
}

#[test]
fn a_kept_staging_directory_that_is_not_utf8_serialises_too() {
    let staging = PathBuf::from(OsString::from_vec(
        b"/w/app/build/ginary/.work\xff-1".to_vec(),
    ));
    let report = BuildReport {
        app: "hello_ffi".to_owned(),
        out: PathBuf::from("build/ginary/hello_ffi"),
        stub_len: 1,
        payload_len: 2,
        total_len: 67,
        sha256: "b".repeat(64),
        strip: StripReport::disabled(),
        size_report: SizeReport::default(),
        manifest: sample_manifest(),
        targets: Vec::new(),
        staging: Some(staging.clone()),
        explain: None,
        warnings: Vec::new(),
    };

    let json = serde_json::to_value(&report).expect("--keep-staging is not a serialisation error");

    assert_eq!(
        json.get("staging").and_then(serde_json::Value::as_str),
        Some(staging.display().to_string().as_str())
    );
}
