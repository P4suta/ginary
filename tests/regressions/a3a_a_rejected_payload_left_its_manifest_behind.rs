// SPDX-License-Identifier: MIT OR Apache-2.0
//! A payload that failed its SHA-256 check left `ginary.json` behind, and a
//! second unpack rewrote one that was already there.
//!
//! **What went wrong.** `payload::unpack` wrote entry 0 to
//! `<dest>/ginary.json` as soon as it had parsed it, before the remaining
//! entries and long before the digest comparison at the end. The presence of
//! that file is exactly what `docs/format.md` says a cache entry's
//! completeness is judged by, so a payload whose bytes did not hash to the
//! trailer — a truncated download, a tampered artifact — left behind a
//! directory that announced itself as finished. It was written with
//! `std::fs::write`, which overwrites, while every other entry was unpacked
//! with `set_overwrite(false)`: unpacking twice into one destination replaced
//! the manifest and only then failed on entry 1.
//!
//! **The input.** A three-entry payload handed to `unpack` with a digest that
//! is one byte away from the right one; and a valid payload unpacked twice
//! into the same destination, with the manifest replaced by a sentinel in
//! between so that a rewrite is visible.
//!
//! **The correct behaviour.** `ginary.json` is written last, after the digest
//! has matched, with `create_new`, so a rejected payload never leaves the
//! completeness marker and a destination that already holds one is refused
//! rather than rewritten.

use crate::common::payload::{
    RawEntry, RawTar, sample_manifest, sample_manifest_json, sha256, staging_tree,
};
use ginary::manifest::{INDEX_NAME, Index, MANIFEST_NAME};
use ginary::payload::{PayloadError, pack, unpack};

/// The compression level these tests pack at; none of them measures the
/// compressor.
const LEVEL: i32 = 3;

/// A payload holding the manifest, an empty index and one ordinary file.
fn payload() -> (Vec<u8>, u64, [u8; 32]) {
    let index = serde_json::to_vec(&Index { files: Vec::new() }).expect("serialise an index");
    let bytes = RawTar::new()
        .push(RawEntry::file(MANIFEST_NAME, &sample_manifest_json()))
        .push(RawEntry::file(INDEX_NAME, &index))
        .push(RawEntry::file("lib/hello/ebin/hello.beam", b"FOR1"))
        .build_zstd(LEVEL);
    let len = bytes.len() as u64;
    let sha = sha256(&bytes);
    (bytes, len, sha)
}

#[test]
fn a_payload_that_fails_its_digest_leaves_no_manifest_behind() {
    let dir = tempfile::tempdir().expect("tempdir");
    let dest = dir.path().join("dest");
    let (bytes, len, sha) = payload();
    let mut wrong = sha;
    wrong[0] ^= 0xff;

    let error = unpack(bytes.as_slice(), len, &wrong, &dest)
        .expect_err("a digest that does not match is refused");

    assert!(
        matches!(error, PayloadError::ChecksumMismatch { .. }),
        "expected ChecksumMismatch, got {error:?}"
    );
    assert!(
        !dest.join(MANIFEST_NAME).exists(),
        "a payload that was refused left the file that says the tree is complete: {:?}",
        std::fs::read_dir(&dest).map(|entries| entries
            .flatten()
            .map(|entry| entry.path())
            .collect::<Vec<_>>())
    );
}

#[test]
fn a_payload_that_passes_its_digest_still_writes_the_manifest() {
    let source = tempfile::tempdir().expect("tempdir");
    let tree = staging_tree(source.path());
    let mut bytes = Vec::new();
    let packed = pack(&tree.root, &sample_manifest(), LEVEL, &mut bytes).expect("pack");
    let dir = tempfile::tempdir().expect("tempdir");
    let dest = dir.path().join("dest");

    unpack(bytes.as_slice(), packed.len, &packed.sha256, &dest)
        .expect("a payload this ginary wrote");

    let written = std::fs::read(dest.join(MANIFEST_NAME)).expect("the manifest is on disk");
    let parsed: serde_json::Value = serde_json::from_slice(&written).expect("it is the JSON");
    assert_eq!(
        parsed["app"], "hello",
        "and it is the manifest that was packed"
    );
}

#[test]
fn unpacking_twice_into_one_destination_does_not_rewrite_the_manifest() {
    let dir = tempfile::tempdir().expect("tempdir");
    let dest = dir.path().join("dest");
    let (bytes, len, sha) = payload();
    unpack(bytes.as_slice(), len, &sha, &dest).expect("the first unpack");
    std::fs::write(dest.join(MANIFEST_NAME), b"a sentinel nobody may overwrite")
        .expect("replace the manifest");

    let error = unpack(bytes.as_slice(), len, &sha, &dest)
        .expect_err("a destination that already holds the tree is refused");

    assert!(
        matches!(error, PayloadError::Io(_)),
        "expected an I/O error for a file that is already there, got {error:?}"
    );
    assert_eq!(
        std::fs::read(dest.join(MANIFEST_NAME)).expect("read the manifest"),
        b"a sentinel nobody may overwrite",
        "the second unpack rewrote a manifest it was not allowed to overwrite"
    );
}
