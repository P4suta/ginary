// SPDX-License-Identifier: MIT OR Apache-2.0
//! `unpack` accepted a payload whose second entry was not the index.
//!
//! **What went wrong.** `docs/format.md` fixes the front of the payload: entry
//! 0 is `ginary.json` and entry 1 is `ginary.index.json`. `read_index`
//! enforced both, and `unpack` — the reader that actually writes a cache
//! directory — enforced only entry 0. A payload whose index was misnamed, or
//! missing altogether, extracted happily, and the cache directory it produced
//! had no index for `ginary verify` to read. A format rule that only the
//! streaming reader applies is a rule the artifact on disk does not have to
//! obey.
//!
//! **The input.** Two hand-built payloads: one whose entry 1 is an ordinary
//! file, and one that stops after the manifest.
//!
//! **The correct behaviour.** `PayloadError::UnexpectedEntry` naming position
//! 1 for the first, `PayloadError::MissingEntry` naming position 1 for the
//! second, and nothing written outside the destination in either case.

use crate::common::payload::{RawEntry, RawTar, sample_manifest_json, sha256, tree_listing};
use ginary::manifest::{INDEX_NAME, MANIFEST_NAME};
use ginary::payload::{PayloadError, unpack};

/// The compression level these tests pack at.
const LEVEL: i32 = 3;

/// A destination with a sibling nothing may touch.
fn destination(dir: &std::path::Path) -> std::path::PathBuf {
    let dest = dir.join("dest");
    std::fs::create_dir(&dest).expect("create dest");
    std::fs::write(dir.join("sentinel.txt"), b"untouched").expect("sentinel");
    dest
}

/// Everything under `dir` that is not under `dest`.
fn outside(dir: &std::path::Path) -> Vec<String> {
    tree_listing(dir)
        .into_iter()
        .filter(|path| !path.starts_with("dest"))
        .collect()
}

#[test]
fn a_second_entry_that_is_not_the_index_is_refused() {
    let archive = RawTar::new()
        .push(RawEntry::file(MANIFEST_NAME, &sample_manifest_json()))
        .push(RawEntry::file("lib/hello/ebin/hello.beam", b"FOR1"));
    let bytes = archive.build_zstd(LEVEL);
    let sha = sha256(&bytes);
    let dir = tempfile::tempdir().expect("tempdir");
    let dest = destination(dir.path());

    let error = unpack(bytes.as_slice(), bytes.len() as u64, &sha, &dest)
        .expect_err("entry 1 is fixed by the format for every reader, not only the streaming one");

    match error {
        PayloadError::UnexpectedEntry {
            position,
            expected,
            found,
        } => {
            assert_eq!(position, 1);
            assert_eq!(expected, INDEX_NAME);
            assert_eq!(found, "lib/hello/ebin/hello.beam");
        }
        other => panic!("expected UnexpectedEntry, got {other:?}"),
    }
    assert_eq!(outside(dir.path()), ["sentinel.txt"]);
}

#[test]
fn a_payload_that_stops_after_the_manifest_is_refused() {
    let archive = RawTar::new().push(RawEntry::file(MANIFEST_NAME, &sample_manifest_json()));
    let bytes = archive.build_zstd(LEVEL);
    let sha = sha256(&bytes);
    let dir = tempfile::tempdir().expect("tempdir");
    let dest = destination(dir.path());

    let error = unpack(bytes.as_slice(), bytes.len() as u64, &sha, &dest)
        .expect_err("an artifact with no index is not an artifact this ginary wrote");

    match error {
        PayloadError::MissingEntry { position, expected } => {
            assert_eq!(position, 1);
            assert_eq!(expected, INDEX_NAME);
        }
        other => panic!("expected MissingEntry, got {other:?}"),
    }
    assert!(
        !dest.join(MANIFEST_NAME).exists(),
        "a payload that was refused left the file that says the tree is complete"
    );
    assert_eq!(outside(dir.path()), ["sentinel.txt"]);
}
