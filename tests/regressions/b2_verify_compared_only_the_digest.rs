// SPDX-License-Identifier: MIT OR Apache-2.0
//! `ginary verify` checked one of the index's three columns.
//!
//! `ginary.index.json` describes every payload entry with a `path`, a `size`,
//! a `mode` and a `sha256`, and the stream loop matched a row by path and then
//! compared the digest and nothing else. An artifact whose index row promised
//! `0700` over a payload entry the launcher extracts `0644`, or whose row
//! claimed a length the entry does not have, therefore verified clean: exit 0,
//! no findings, over an artifact whose own two descriptions of a file
//! disagree. The digest hid it, because a row can carry the right digest and
//! the wrong metadata — nothing in the packer recomputes one from the other.
//!
//! `docs/format.md` fixes the relation the two columns stand in: the header's
//! `size` is the file's own length, which is the index's `size`, and the
//! header's `mode` is the *normalisation* of the staged mode the index
//! records — `0755` when the row carries the user execute bit and `0644`
//! otherwise. The correct behaviour is to check both relations and to name
//! the column that broke: `Issue::IndexSizeMismatch` and
//! `Issue::IndexModeMismatch`, beside the digest's `Issue::IndexMismatch`.

use assert_cmd::Command;
use ginary::verify::{self, Issue};

use crate::common::repack::{RepackOptions, build};

/// A staged file with no execute bit and thirteen bytes of content.
const FILE: &str = "lib/hello/priv/greeting.txt";

/// How long `FILE` really is.
const LENGTH: u64 = 13;

/// A temporary directory the test owns.
fn tempdir() -> tempfile::TempDir {
    tempfile::tempdir().expect("a temporary directory")
}

/// The issues of one report, as their rendered sentences.
fn sentences(report: &verify::VerifyReport) -> Vec<String> {
    report.issues.iter().map(ToString::to_string).collect()
}

/// Verifies an artifact built from `options`.
fn report(dir: &std::path::Path, options: &RepackOptions) -> verify::VerifyReport {
    let artifact = build(dir, options);
    let report = verify::verify(artifact.path()).expect("the artifact opens");
    assert!(
        report.payload.ok(),
        "the payload digest is the trailer's, which is what makes this the hard case"
    );
    report
}

#[test]
fn an_index_row_promising_an_execute_bit_the_payload_does_not_carry_is_reported() {
    let dir = tempdir();

    let found = report(
        dir.path(),
        &RepackOptions {
            index_mode_lies: vec![(FILE.to_owned(), 0o700)],
            ..RepackOptions::default()
        },
    );

    assert!(
        !found.ok(),
        "an index that promises an executable verified clean: {:#?}",
        sentences(&found)
    );
    assert!(
        found.issues.iter().any(|issue| matches!(
            issue,
            Issue::IndexModeMismatch { path, expected, actual, .. }
                if path == FILE && expected == "0755" && actual == "0644"
        )),
        "nothing names the mode column: {:#?}",
        sentences(&found)
    );
    assert!(
        !found
            .issues
            .iter()
            .any(|issue| matches!(issue, Issue::IndexMismatch { .. })),
        "the bytes are the ones the row describes, so the digest is not the finding: {:#?}",
        sentences(&found)
    );
}

#[test]
fn an_index_row_claiming_a_length_the_entry_does_not_have_is_reported() {
    let dir = tempdir();

    let found = report(
        dir.path(),
        &RepackOptions {
            index_size_lies: vec![(FILE.to_owned(), 1)],
            ..RepackOptions::default()
        },
    );

    assert!(
        !found.ok(),
        "an index that lies about a length verified clean: {:#?}",
        sentences(&found)
    );
    assert!(
        found.issues.iter().any(|issue| matches!(
            issue,
            Issue::IndexSizeMismatch { path, expected, actual }
                if path == FILE && *expected == 1 && *actual == LENGTH
        )),
        "nothing names the size column: {:#?}",
        sentences(&found)
    );
}

#[test]
fn a_row_that_agrees_with_its_entry_raises_neither_finding() {
    let dir = tempdir();

    let found = report(dir.path(), &RepackOptions::default());

    assert!(
        found.ok(),
        "a whole artifact raised a finding about its own metadata: {:#?}",
        sentences(&found)
    );
}

#[test]
fn verify_exits_one_and_names_the_column() {
    let dir = tempdir();
    let artifact = build(
        dir.path(),
        &RepackOptions {
            index_mode_lies: vec![(FILE.to_owned(), 0o700)],
            index_size_lies: vec![(FILE.to_owned(), 1)],
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

    assert!(stdout.contains("mode"), "{stdout}");
    assert!(stdout.contains("bytes"), "{stdout}");
}
