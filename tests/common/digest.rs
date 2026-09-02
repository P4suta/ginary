// SPDX-License-Identifier: MIT OR Apache-2.0
//! The published SHA-256 vectors, and the staged tree that feeds them through
//! the crate's own hashing path.
//!
//! Every SHA-256 ginary computes is on-disk format. The trailer carries the
//! digest of the payload's bytes, `ginary.index.json` carries one per staged
//! file, and a catalog entry carries one per tarball; a launcher that
//! recomputes a different number refuses to start, and an artifact built by a
//! newer ginary would no longer verify against one built by an older. Nothing
//! in the crate states that as a fact a test can check — the digests are
//! produced deep inside `manifest::hash_file`, `payload::HashingWriter` and
//! `catalog::digest_file`, all private — so a hashing library swapped
//! underneath them would be caught by nothing but luck.
//!
//! These vectors are that statement. Three inputs whose digests are published
//! constants, staged as a tree so [`ginary::manifest::Index::from_staged`]
//! hashes them the same way it hashes a real payload: not `sha2` called from
//! the test, which would only prove `sha2` is `sha2`, but the crate's own call
//! site reached through its own public API.

use std::path::Path;

use ginary::assemble::{Category, StagedFile};

/// One published vector: an input this repository hashes the same way
/// forever, and the digest it has to produce.
pub struct Vector {
    /// The path it is staged at, and the name a failure reports.
    pub path: &'static str,
    /// What the input is, in words.
    pub what: &'static str,
    /// The bytes.
    pub bytes: Vec<u8>,
    /// The lower-case hexadecimal SHA-256 of `bytes`.
    pub sha256: &'static str,
}

/// The SHA-256 of the empty input, from FIPS 180-4.
pub const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

/// The SHA-256 of `abc`, the first published SHA-256 example.
pub const ABC_SHA256: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

/// The SHA-256 of [`mib_pattern`].
pub const MIB_PATTERN_SHA256: &str =
    "631b84027d6b9e52b539c4e8373622d23032dfadc64d60af87339c9037e4f769";

/// How long [`mib_pattern`] is: one mebibyte.
pub const MIB: usize = 1024 * 1024;

/// The period of [`mib_pattern`], the largest prime below 256.
///
/// A prime, and coprime with both the 64 KiB buffer `manifest::hash_file`
/// reads through and SHA-256's own 64-byte block, so a hasher fed the wrong
/// slice of a buffer — one byte short, one block twice — lands on a different
/// digest instead of the same one by symmetry.
pub const PATTERN_PERIOD: usize = 251;

/// One mebibyte of `index % 251`.
///
/// A megabyte rather than a handful of bytes because every digest in the
/// format is computed incrementally over a buffer: `hash_file` reads 64 KiB at
/// a time, `payload::pack` hashes each `write` as it goes, and `download`
/// hashes each chunk off the socket. An input smaller than one buffer never
/// reaches the second `update` call.
pub fn mib_pattern() -> Vec<u8> {
    (0..MIB)
        .map(|index| (index % PATTERN_PERIOD) as u8)
        .collect()
}

/// The three published vectors, in the order they are staged.
pub fn vectors() -> Vec<Vector> {
    vec![
        Vector {
            path: "vectors/abc",
            what: "the three bytes `abc`",
            bytes: b"abc".to_vec(),
            sha256: ABC_SHA256,
        },
        Vector {
            path: "vectors/empty",
            what: "the empty input",
            bytes: Vec::new(),
            sha256: EMPTY_SHA256,
        },
        Vector {
            path: "vectors/mib_pattern",
            what: "one mebibyte of `index % 251`",
            bytes: mib_pattern(),
            sha256: MIB_PATTERN_SHA256,
        },
    ]
}

/// Writes [`vectors`] into `root` and returns the staging listing over them.
///
/// The listing is sorted by path, which is the order
/// [`ginary::manifest::Index::from_staged`] returns its files in, so a caller
/// can zip the two together. Every file is [`Category::Other`]: the category
/// travels into `ginary.index.json` beside the digest and has no effect on it,
/// and calling these bytes anything more specific would be a claim about a
/// tree that is not a real staged runtime.
pub fn vector_listing(root: &Path) -> Vec<StagedFile> {
    let mut files = Vec::new();
    for vector in vectors() {
        let full = root.join(vector.path);
        std::fs::create_dir_all(full.parent().expect("a staged path has a parent"))
            .expect("create the vector directory");
        std::fs::write(&full, &vector.bytes).expect("write the vector");
        files.push(StagedFile {
            path: vector.path.to_owned(),
            size: vector.bytes.len() as u64,
            mode: 0o644,
            category: Category::Other,
        });
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    files
}
