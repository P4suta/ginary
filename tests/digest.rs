// SPDX-License-Identifier: MIT OR Apache-2.0
//! SHA-256 is on-disk format.
//!
//! Three digests leave this crate and are read back by a different build of
//! it: the trailer's `sha256` over the payload's bytes, the per-file `sha256`
//! of `ginary.index.json`, and the `sha256` of a catalog entry. A launcher
//! recomputes the first before it will extract anything, `ginary verify`
//! recomputes the second, and `otp fetch` recomputes the third. If any of them
//! ever produced a different number for the same bytes, every artifact built
//! before the change would stop verifying against every artifact built after —
//! silently, because both halves would agree with themselves.
//!
//! Nothing in the crate stated that as a checkable fact. The hashing lives in
//! `manifest::hash_file`, `payload::HashingWriter`, `payload::HashingReader`,
//! `verify::read_entry`, `download::stream_to_file` and `catalog::digest_file`
//! — six private call sites — so the digest library underneath them could be
//! swapped, or one call site rewired to a new API, with the suite green.
//!
//! This file is the missing statement, and it is written to survive the
//! migration it was added for: the vectors come from `tests/common/digest.rs`
//! and are reached through the crate's own public API, not through `sha2`
//! called from the test, which would only prove `sha2` is `sha2`.
//!
//! Ungated: every digest here is on the launcher path as well as the build
//! side.

mod common;

use std::path::Path;

use ginary::assemble::{Category, StagedFile};
use ginary::manifest::Index;
use ginary::payload::{pack, read_index, unpack};

use crate::common::deps::{Version, dependency_requirement};
use crate::common::digest::{MIB, PATTERN_PERIOD, mib_pattern, vector_listing, vectors};
use crate::common::payload::{sample_manifest, sha256, staging_tree};

/// The compression level the payload tests pack at. Low: nothing here is
/// measuring the compressor.
const LEVEL: i32 = 3;

/// The committed files of the `hello_ffi` fixture, in the order they are
/// staged.
///
/// Named one by one rather than walked, because `tests/fixtures/*/build/` is
/// git-ignored: a walk over the fixture directory would pick up whatever the
/// last `gleam export erlang-shipment` left behind and snapshot a different
/// tree on every machine. These five are what the repository actually carries.
const HELLO_FFI_FILES: [&str; 5] = [
    "gleam.toml",
    "manifest.toml",
    "priv/greeting.txt",
    "src/hello_ffi.gleam",
    "src/hello_ffi_ffi.erl",
];

#[test]
fn the_index_hashes_the_published_vectors_to_their_published_digests() {
    let dir = tempfile::tempdir().expect("tempdir");
    let listing = vector_listing(dir.path());
    let index = Index::from_staged(dir.path(), &listing).expect("hash the staged vectors");

    let expected = vectors();
    assert_eq!(
        index.files.len(),
        expected.len(),
        "the index has to hold one entry per staged vector"
    );
    for (file, vector) in index.files.iter().zip(&expected) {
        assert_eq!(file.path, vector.path, "the index is sorted by path");
        assert_eq!(
            file.size,
            vector.bytes.len() as u64,
            "{}: the index records the byte count of {}",
            vector.path,
            vector.what
        );
        assert_eq!(
            file.sha256, vector.sha256,
            "{}: the SHA-256 of {} is published and cannot move — an artifact built before this \
             change would stop verifying against one built after it",
            vector.path, vector.what
        );
    }
}

#[test]
fn the_one_mebibyte_vector_is_the_pattern_it_claims_to_be() {
    // The digest of `mib_pattern` is a constant in the helper, so the bytes it
    // is the digest *of* have to be pinned too: a helper that quietly changed
    // its pattern would turn the vector above into a test of nothing.
    let bytes = mib_pattern();
    assert_eq!(bytes.len(), MIB, "the vector is exactly one mebibyte");
    assert_eq!(bytes[0], 0, "the pattern starts at zero");
    assert_eq!(
        bytes[PATTERN_PERIOD - 1],
        (PATTERN_PERIOD - 1) as u8,
        "the pattern counts up to its period"
    );
    assert_eq!(bytes[PATTERN_PERIOD], 0, "and wraps at it");
    assert_eq!(
        bytes[MIB - 1],
        ((MIB - 1) % PATTERN_PERIOD) as u8,
        "the last byte is the pattern's, so no tail was padded on"
    );
}

#[test]
fn the_committed_hello_ffi_fixture_hashes_to_its_recorded_digests() {
    let dir = tempfile::tempdir().expect("tempdir");
    let listing = stage_hello_ffi(dir.path());
    let index = Index::from_staged(dir.path(), &listing).expect("hash the hello_ffi fixture");

    // Path, size and digest only: the mode a `git checkout` produces depends
    // on the umask, and it is not what this snapshot is about.
    let mut rendered = String::new();
    for file in &index.files {
        rendered.push_str(&format!("{} {} {}\n", file.path, file.size, file.sha256));
    }
    insta::assert_snapshot!("hello_ffi_fixture_digests", rendered);
}

#[test]
fn the_trailer_digest_is_the_digest_of_the_bytes_pack_wrote() {
    let dir = tempfile::tempdir().expect("tempdir");
    let tree = staging_tree(dir.path());
    let mut payload = Vec::new();
    let packed = pack(&tree.root, &sample_manifest(), LEVEL, &mut payload).expect("pack the tree");

    assert_eq!(
        packed.len,
        payload.len() as u64,
        "the trailer's length is the number of bytes the packer wrote"
    );
    assert_eq!(
        hex::encode(packed.sha256),
        hex::encode(sha256(&payload)),
        "the trailer's digest is the SHA-256 of the payload's bytes and nothing else: a hasher \
         fed the compressor's input, or finalised over the wrong slice, still produces 32 bytes"
    );
}

#[test]
fn unpack_recomputes_exactly_the_digests_the_index_recorded() {
    let dir = tempfile::tempdir().expect("tempdir");
    let tree = staging_tree(dir.path());
    let mut payload = Vec::new();
    let packed = pack(&tree.root, &sample_manifest(), LEVEL, &mut payload).expect("pack the tree");

    // `unpack` refuses a payload whose bytes do not hash to `expected_sha`, so
    // reaching the destination at all is the reader half of the round trip.
    let dest = tempfile::tempdir().expect("dest tempdir");
    unpack(payload.as_slice(), packed.len, &packed.sha256, dest.path())
        .expect("unpack the payload the packer just wrote");

    let (_, index) = read_index(payload.as_slice()).expect("read the index out of the payload");
    let staged: Vec<StagedFile> = index
        .files
        .iter()
        .map(|file| StagedFile {
            path: file.path.clone(),
            size: file.size,
            mode: file.mode,
            category: file.category,
        })
        .collect();
    let recomputed =
        Index::from_staged(dest.path(), &staged).expect("hash the extracted tree again");

    let before: Vec<(&str, u64, &str)> = index
        .files
        .iter()
        .map(|file| (file.path.as_str(), file.size, file.sha256.as_str()))
        .collect();
    let after: Vec<(&str, u64, &str)> = recomputed
        .files
        .iter()
        .map(|file| (file.path.as_str(), file.size, file.sha256.as_str()))
        .collect();
    assert_eq!(
        before, after,
        "the digest the index recorded at pack time and the digest the same code computes over \
         the extracted file are the one number `ginary verify` compares"
    );
}

#[test]
fn the_vectors_were_recorded_before_the_hashing_library_moved() {
    // Every constant in this file and in `tests/common/digest.rs` was recorded
    // against sha2 0.10.9, the release this tree carried when E4 opened, and
    // checked against `sha256sum` by hand. That order is the whole proof: a
    // regression file written *after* a digest library is swapped records
    // whatever the new library produces and demonstrates nothing at all.
    //
    // So this test asserts the recording is now being read on the far side of
    // the move. Until sha2 0.11 is in the manifest it fails, and the digests
    // above are a baseline; once it passes, every other test in this file is
    // the statement that not one byte of any digest changed on the way.
    let requirement =
        dependency_requirement("sha2").expect("`Cargo.toml` states a version requirement for sha2");
    let parsed = Version::parse(&requirement)
        .unwrap_or_else(|| panic!("`sha2 = \"{requirement}\"` is not a version requirement"));
    assert!(
        (parsed.major, parsed.minor) >= (0, 11),
        "the vectors in this file were recorded under sha2 0.10.9 and are still being read under \
         sha2 {requirement}, so they have not yet proved anything about the 0.11 migration. \
         Bump the dependency and run this file again: every digest it asserts has to come out \
         identical."
    );
}

/// Copies the committed `hello_ffi` fixture into `root` and returns the
/// staging listing over it.
///
/// Every file is [`Category::Other`]: the category travels into
/// `ginary.index.json` beside the digest and has no effect on it, and calling
/// a `gleam.toml` anything more specific would be a claim about a staged
/// runtime this tree is not.
fn stage_hello_ffi(root: &Path) -> Vec<StagedFile> {
    let fixture = common::fixture::fixtures_dir().join("hello_ffi");
    let mut files = Vec::new();
    for relative in HELLO_FFI_FILES {
        let source = fixture.join(relative);
        let bytes = std::fs::read(&source).unwrap_or_else(|error| {
            panic!(
                "{} is a committed fixture file; it cannot be read: {error}",
                source.display()
            )
        });
        // The digests below are over these bytes exactly. A checkout with
        // `core.autocrlf=true` would hand back CRLF and the snapshot would
        // fail as an opaque size-and-digest diff, so say what happened
        // instead. `.gitattributes` pins this — see
        // `tests/regressions/e4_a_crlf_checkout_rewrote_the_hashed_fixtures.rs`.
        assert!(
            !bytes.windows(2).any(|pair| pair == b"\r\n"),
            "{} holds CRLF in this working tree. Its SHA-256 is published in \
             tests/snapshots/digest__hello_ffi_fixture_digests.snap over the committed LF bytes; \
             a checkout rewrote it, which `core.autocrlf` does by default on Windows. Set it to \
             `false` or `input` and check the tree out again.",
            source.display()
        );
        let destination = root.join(relative);
        std::fs::create_dir_all(destination.parent().expect("a parent"))
            .expect("create the fixture directory");
        std::fs::write(&destination, &bytes).expect("copy the fixture file");
        files.push(StagedFile {
            path: relative.to_owned(),
            size: bytes.len() as u64,
            mode: 0o644,
            category: Category::Other,
        });
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    files
}
