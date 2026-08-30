// SPDX-License-Identifier: MIT OR Apache-2.0
//! A tar entry with the contiguous-file type flag was extracted as a regular
//! file.
//!
//! **What went wrong.** `payload::unpack` allowlists the entry types the
//! format permits, and the arm read
//! `EntryType::Regular | EntryType::Continuous | EntryType::Directory`.
//! `docs/format.md` says only `Regular` and `Directory` are legal, and `pack`
//! has never written a `'7'` entry, so the extra type widened only what a
//! hostile archive was allowed to contain. An allowlist that is quietly wider
//! than the document is the document being wrong about the code.
//!
//! **The input.** A hand-built archive whose third entry carries typeflag
//! `'7'` and a body, behind a valid manifest and index.
//!
//! **The correct behaviour.** The entry is refused as
//! `PayloadError::UnsupportedEntry` naming it a contiguous file, and nothing
//! is written outside the destination.

use crate::common::payload::{
    RawEntry, RawTar, TYPE_CONTIGUOUS, sample_manifest_json, sha256, tree_listing,
};
use ginary::manifest::{INDEX_NAME, Index, MANIFEST_NAME};
use ginary::payload::{PayloadError, unpack};

/// The compression level this test packs at.
const LEVEL: i32 = 3;

#[test]
fn a_contiguous_file_entry_is_refused_like_every_other_type_that_is_not_a_file() {
    let index = serde_json::to_vec(&Index { files: Vec::new() }).expect("serialise an index");
    let archive = RawTar::new()
        .push(RawEntry::file(MANIFEST_NAME, &sample_manifest_json()))
        .push(RawEntry::file(INDEX_NAME, &index))
        .push(RawEntry::special("contiguous.bin", TYPE_CONTIGUOUS, "").with_data(b"owned"));
    let bytes = archive.build_zstd(LEVEL);
    let len = bytes.len() as u64;
    let sha = sha256(&bytes);
    let dir = tempfile::tempdir().expect("tempdir");
    let dest = dir.path().join("dest");
    std::fs::create_dir(&dest).expect("create dest");
    std::fs::write(dir.path().join("sentinel.txt"), b"untouched").expect("sentinel");

    let error = unpack(bytes.as_slice(), len, &sha, &dest)
        .expect_err("a contiguous file is not one of the two legal entry types");

    match error {
        PayloadError::UnsupportedEntry { path, kind } => {
            assert_eq!(path, "contiguous.bin");
            assert_eq!(kind, "contiguous file");
        }
        other => panic!("expected UnsupportedEntry, got {other:?}"),
    }
    assert!(
        !dest.join("contiguous.bin").exists(),
        "the refused entry was written anyway"
    );
    let outside: Vec<String> = tree_listing(dir.path())
        .into_iter()
        .filter(|path| !path.starts_with("dest"))
        .collect();
    assert_eq!(outside, ["sentinel.txt"]);
}
