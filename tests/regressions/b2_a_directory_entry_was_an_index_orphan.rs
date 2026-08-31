// SPDX-License-Identifier: MIT OR Apache-2.0
//! `ginary verify` reported a legal directory entry as an orphan.
//!
//! `verify` hashed and matched every tar entry by name, with no test of the
//! entry's type. `docs/format.md` permits a directory entry — "a directory
//! entry appears only for a directory that would otherwise be lost" — and
//! `payload::unpack` accepts one, but `ginary.index.json` lists files only. A
//! directory entry therefore hashed to the digest of no bytes at all, found no
//! index row, and came back as `IndexOrphan`. ginary's own packer emits none
//! today, which is why nothing caught it; `verify` is aimed at artifacts other
//! people built.
//!
//! An entry that is neither a regular file nor a directory had the opposite
//! problem: `unpack` refuses a symlink with `UnsupportedEntry` and `verify`
//! checked its (empty) bytes against the index and said nothing about what it
//! was.
//!
//! The correct behaviour is that a directory entry is passed over — counted
//! against neither `files_checked` nor the index — and everything else is
//! named as the kind of entry it is.

use ginary::verify;

use crate::common::repack::{AppendedEntry, RepackOptions, build};

/// A temporary directory the test owns.
fn tempdir() -> tempfile::TempDir {
    tempfile::tempdir().expect("a temporary directory")
}

#[test]
fn a_directory_entry_the_index_cannot_name_is_not_an_orphan() {
    let dir = tempdir();
    let artifact = build(
        dir.path(),
        &RepackOptions {
            appended: vec![AppendedEntry::directory("lib/hello/priv/empty")],
            ..RepackOptions::default()
        },
    );

    let report = verify::verify(artifact.path()).expect("the artifact opens");

    assert!(
        report.ok(),
        "the format permits a directory entry: {:#?}",
        report.issues
    );
}

#[test]
fn an_entry_that_is_neither_a_file_nor_a_directory_is_named_for_what_it_is() {
    let dir = tempdir();
    let artifact = build(
        dir.path(),
        &RepackOptions {
            appended: vec![AppendedEntry::symlink("lib/hello/priv/link", "/etc/passwd")],
            ..RepackOptions::default()
        },
    );

    let report = verify::verify(artifact.path()).expect("the artifact opens");

    let sentences: Vec<String> = report.issues.iter().map(ToString::to_string).collect();
    assert!(
        sentences
            .iter()
            .any(|sentence| sentence.contains("symlink")),
        "a symlink is what `unpack` refuses the payload for, and `verify` said: {sentences:#?}"
    );
}
