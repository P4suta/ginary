// SPDX-License-Identifier: MIT OR Apache-2.0
//! A payload could repeat `ginary.json` after the front matter and plant the
//! completeness marker in a destination it was then refused from.
//!
//! **What went wrong.** `payload::unpack` fixed the entry names only at
//! positions 0 and 1; every later entry went straight to `unpack_in`. Holding
//! entry 0 back until the digest matched — the fix for the previous bug — took
//! `ginary.json` out of the loop's way, so `set_overwrite(false)` no longer
//! stood between a second entry of that name and the destination. A payload
//! whose entry 2 was called `ginary.json` therefore wrote attacker-chosen
//! bytes to `<dest>/ginary.json` during the loop, and the final `create_new`
//! failed on the file the archive had just planted: the caller saw a bare
//! `Io(AlreadyExists)`, and the rejected payload left behind exactly the file
//! `docs/format.md` says a cache entry's completeness is judged by. The same
//! gap existed on the writing side: `pack` exempted only `ginary.stage.json`,
//! so a staged tree holding a file called `ginary.json` produced an artifact
//! that ginary's own reader could never extract.
//!
//! **The input.** Four hand-built payloads whose entry 2 is `ginary.json`,
//! `./ginary.json`, `ginary.index.json` and a directory named `ginary.json`;
//! and a staging root whose listing names a file called `ginary.json`.
//!
//! **The correct behaviour.** `PayloadError::DuplicateEntry` naming the
//! position and the reserved name, with nothing written to
//! `<dest>/ginary.json`; and `PayloadError::ReservedName` from `pack`, so the
//! tool cannot emit an artifact it would itself reject.

use crate::common::payload::{
    RawEntry, RawTar, TYPE_DIRECTORY, sample_manifest, sample_manifest_json, sha256, staging_tree,
    tree_listing,
};
use ginary::assemble::{Category, StagedFile};
use ginary::manifest::{INDEX_NAME, Index, MANIFEST_NAME};
use ginary::payload::{PayloadError, pack, unpack};

/// The compression level these tests pack at; none of them measures the
/// compressor.
const LEVEL: i32 = 3;

/// A destination inside a directory that also holds a sibling nothing may
/// touch.
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

/// A well-formed front matter followed by `extra`.
fn payload_with(extra: RawEntry) -> (Vec<u8>, [u8; 32]) {
    let index = serde_json::to_vec(&Index { files: Vec::new() }).expect("serialise an index");
    let bytes = RawTar::new()
        .push(RawEntry::file(MANIFEST_NAME, &sample_manifest_json()))
        .push(RawEntry::file(INDEX_NAME, &index))
        .push(extra)
        .build_zstd(LEVEL);
    let sha = sha256(&bytes);
    (bytes, sha)
}

/// Unpacks a payload whose entry 2 is `extra` and asserts it is refused as a
/// repeat of `reserved`, without the marker being planted.
fn refuses(extra: RawEntry, reserved: &str, fixed: usize) {
    let (bytes, sha) = payload_with(extra);
    let dir = tempfile::tempdir().expect("tempdir");
    let dest = destination(dir.path());

    let error = unpack(bytes.as_slice(), bytes.len() as u64, &sha, &dest)
        .expect_err("an entry may not repeat a name the format fixes at the front");

    match error {
        PayloadError::DuplicateEntry {
            position,
            name,
            fixed: at,
        } => {
            assert_eq!(position, 2);
            assert_eq!(name, reserved);
            assert_eq!(at, fixed);
        }
        other => panic!("expected DuplicateEntry, got {other:?}"),
    }
    assert!(
        !dest.join(MANIFEST_NAME).exists(),
        "the refused payload planted the file that says the tree is complete: {:?}",
        tree_listing(&dest)
    );
    assert_eq!(outside(dir.path()), ["sentinel.txt"]);
}

#[test]
fn a_second_manifest_entry_is_refused_and_plants_nothing() {
    refuses(
        RawEntry::file(MANIFEST_NAME, b"ATTACKER MARKER"),
        MANIFEST_NAME,
        0,
    );
}

#[test]
fn a_second_manifest_entry_behind_a_current_directory_component_is_refused() {
    refuses(
        RawEntry::file("./ginary.json", b"ATTACKER MARKER"),
        MANIFEST_NAME,
        0,
    );
}

#[test]
fn a_directory_entry_named_like_the_manifest_is_refused() {
    refuses(
        RawEntry::special(MANIFEST_NAME, TYPE_DIRECTORY, ""),
        MANIFEST_NAME,
        0,
    );
}

#[test]
fn a_second_index_entry_is_refused() {
    refuses(RawEntry::file(INDEX_NAME, b"[not an index]"), INDEX_NAME, 1);
}

#[test]
fn packing_a_staged_file_with_a_reserved_name_is_refused() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut tree = staging_tree(dir.path());
    let data = b"a staged file that pretends to be the manifest";
    std::fs::write(tree.root.join(MANIFEST_NAME), data).expect("write the reserved file");
    tree.listing.files.push(StagedFile {
        path: MANIFEST_NAME.to_owned(),
        size: data.len() as u64,
        mode: 0o644,
        category: Category::Priv,
    });
    tree.listing
        .files
        .sort_by(|left, right| left.path.cmp(&right.path));
    std::fs::write(
        tree.root.join("ginary.stage.json"),
        serde_json::to_vec(&tree.listing).expect("serialise the listing"),
    )
    .expect("rewrite the listing");

    let mut bytes = Vec::new();
    let error = pack(&tree.root, &sample_manifest(), LEVEL, &mut bytes)
        .expect_err("ginary may not write an artifact its own reader refuses");

    match error {
        PayloadError::ReservedName { path, fixed } => {
            assert_eq!(path, MANIFEST_NAME);
            assert_eq!(fixed, 0);
        }
        other => panic!("expected ReservedName, got {other:?}"),
    }
}
