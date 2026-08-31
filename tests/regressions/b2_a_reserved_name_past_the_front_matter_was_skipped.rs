// SPDX-License-Identifier: MIT OR Apache-2.0
//! `ginary verify` dropped every entry named after the front matter.
//!
//! The stream loop skipped an entry whenever its name was `ginary.json` or
//! `ginary.index.json`, wherever in the archive it appeared, on the reasoning
//! that entries 0 and 1 are the artifact's own description of itself. But
//! `docs/format.md` reserves those names: an entry at position 2 or later that
//! lands on either — as the name itself, or as a directory holding a file — is
//! `DuplicateEntry`, and `payload::unpack` refuses the whole payload. `verify`
//! skipped it silently instead, so a payload no launcher would extract came
//! back clean, and the entry appeared in neither `files_checked` nor any issue.
//! CLAUDE.md: skipping is a reported decision or an error, never a default.
//!
//! The correct behaviour is that only *positions* 0 and 1 are the front matter.
//! A reserved first component anywhere after them is an issue, `verify` exits
//! 1, and the entry is named.

use assert_cmd::Command;
use ginary::manifest::{INDEX_NAME, MANIFEST_NAME};
use ginary::verify;

use crate::common::repack::{AppendedEntry, RepackOptions, build};

/// A temporary directory the test owns.
fn tempdir() -> tempfile::TempDir {
    tempfile::tempdir().expect("a temporary directory")
}

/// The issues of one report, as their rendered sentences.
fn sentences(report: &verify::VerifyReport) -> Vec<String> {
    report.issues.iter().map(ToString::to_string).collect()
}

#[test]
fn a_second_manifest_entry_is_a_finding_rather_than_a_skip() {
    let dir = tempdir();
    let artifact = build(
        dir.path(),
        &RepackOptions {
            appended: vec![AppendedEntry::file(MANIFEST_NAME, b"{}\n")],
            ..RepackOptions::default()
        },
    );

    let report = verify::verify(artifact.path()).expect("the artifact opens");

    assert!(
        !report.ok(),
        "a payload `unpack` refuses cannot verify clean: {:#?}",
        report.issues
    );
    assert!(
        sentences(&report)
            .iter()
            .any(|sentence| sentence.starts_with(MANIFEST_NAME) && sentence.contains("reserve")),
        "nothing names the repeated entry for what it is: {:#?}",
        sentences(&report)
    );
}

#[test]
fn a_file_under_a_reserved_directory_name_is_the_same_finding() {
    let dir = tempdir();
    let name = format!("{INDEX_NAME}/nested.txt");
    let artifact = build(
        dir.path(),
        &RepackOptions {
            appended: vec![AppendedEntry::file(&name, b"hidden\n")],
            ..RepackOptions::default()
        },
    );

    let report = verify::verify(artifact.path()).expect("the artifact opens");

    assert!(
        sentences(&report)
            .iter()
            .any(|sentence| sentence.starts_with(&name) && sentence.contains("reserve")),
        "a directory carrying a reserved name occupies it just as surely: {:#?}",
        sentences(&report)
    );
}

#[test]
fn verify_exits_one_and_names_the_entry() {
    let dir = tempdir();
    let artifact = build(
        dir.path(),
        &RepackOptions {
            appended: vec![AppendedEntry::file(MANIFEST_NAME, b"{}\n")],
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

    assert!(stdout.contains(MANIFEST_NAME), "{stdout}");
}
