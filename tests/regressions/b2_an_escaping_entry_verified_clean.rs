// SPDX-License-Identifier: MIT OR Apache-2.0
//! `ginary verify` never looked at where an entry would land.
//!
//! The stream loop grew a check on an entry's *position* (`ReservedEntry`) and
//! one on its *kind* (`UnsupportedEntry`), and never one on its *path*.
//! `payload::unpack` refuses an entry whose name is absolute, holds `..`, or
//! normalises to nothing — `PayloadError::UnsafePath`, exit 123 at run time —
//! and `verify` had no equivalent. An artifact carrying `/etc/cron.d/pwned`
//! therefore verified clean, exit 0, as long as the index named it: the
//! payload digest matches, the row's digest matches the bytes, and every check
//! `verify` made passed. Without the colluding row the entry was reported, but
//! as `IndexOrphan` — "in the payload and not in the index" — which is a
//! diagnosis about the index rather than about the escape.
//!
//! The correct behaviour is `payload`'s own rule, applied here: an entry that
//! does not stay under the extracted root is `Issue::UnsafePath`, raised
//! before the path is matched against the index and therefore never counted in
//! `files_checked`.

use assert_cmd::Command;
use ginary::verify;

use crate::common::repack::{AppendedEntry, RepackOptions, build};

/// A path that leaves the extracted root entirely.
const ESCAPING: &str = "/etc/cron.d/pwned";

/// A temporary directory the test owns.
fn tempdir() -> tempfile::TempDir {
    tempfile::tempdir().expect("a temporary directory")
}

/// The issues of one report, as their rendered sentences.
fn sentences(report: &verify::VerifyReport) -> Vec<String> {
    report.issues.iter().map(ToString::to_string).collect()
}

/// An artifact whose payload carries `ESCAPING`, with or without an index row.
fn escaping(dir: &std::path::Path, indexed: bool) -> ginary::verify::VerifyReport {
    let mut entry = AppendedEntry::file(ESCAPING, b"* * * * * root /tmp/x\n").absolute();
    if indexed {
        entry = entry.indexed();
    }
    let artifact = build(
        dir,
        &RepackOptions {
            appended: vec![entry],
            ..RepackOptions::default()
        },
    );
    verify::verify(artifact.path()).expect("the artifact opens")
}

#[test]
fn an_escaping_entry_the_index_names_is_a_finding_rather_than_a_clean_bill() {
    let dir = tempdir();

    let report = escaping(dir.path(), true);

    assert!(
        report.payload.ok(),
        "the payload digest is the trailer's, which is what makes this the hard case"
    );
    assert!(
        !report.ok(),
        "an artifact `unpack` refuses at run time verified clean: {:#?}",
        report.issues
    );
    assert!(
        sentences(&report)
            .iter()
            .any(|sentence| sentence.starts_with(ESCAPING) && sentence.contains("extracted root")),
        "nothing names the escaping entry for what it is: {:#?}",
        sentences(&report)
    );
}

#[test]
fn an_escaping_entry_is_not_diagnosed_as_an_index_orphan() {
    let dir = tempdir();

    let report = escaping(dir.path(), false);

    assert!(
        !report.issues.iter().any(|issue| matches!(
            issue,
            verify::Issue::IndexOrphan { path } if path == ESCAPING
        )),
        "an entry that leaves the root is not a bookkeeping mismatch: {:#?}",
        sentences(&report)
    );
    assert!(
        report.issues.iter().any(|issue| matches!(
            issue,
            verify::Issue::UnsafePath { path } if path == ESCAPING
        )),
        "{:#?}",
        sentences(&report)
    );
}

#[test]
fn an_escaping_entry_is_never_counted_as_a_file_that_was_checked() {
    let dir = tempdir();

    let clean = build(dir.path(), &RepackOptions::default());
    let baseline = verify::verify(clean.path()).expect("the artifact opens");
    let report = escaping(&dir.path().join("escaping"), true);

    assert_eq!(
        report.files_checked, baseline.files_checked,
        "the escaping entry was counted against the index it never reached"
    );
}

#[test]
fn verify_exits_one_and_names_the_entry() {
    let dir = tempdir();
    let mut entry = AppendedEntry::file(ESCAPING, b"* * * * * root /tmp/x\n").absolute();
    entry = entry.indexed();
    let artifact = build(
        dir.path(),
        &RepackOptions {
            appended: vec![entry],
            ..RepackOptions::default()
        },
    );

    let assert = Command::cargo_bin("ginary")
        .expect("the `ginary` binary is built for tests")
        .arg("verify")
        .arg(artifact.path())
        .assert()
        .failure();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8");

    assert!(stdout.contains(ESCAPING), "{stdout}");
}
