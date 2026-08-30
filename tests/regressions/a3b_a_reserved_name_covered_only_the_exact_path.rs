// SPDX-License-Identifier: MIT OR Apache-2.0
//! A *directory* named `ginary.json` walked past the reserved-name check at
//! both ends of the payload format.
//!
//! **What went wrong.** Both reserved-name checks compared the whole path for
//! equality with `ginary.json` or `ginary.index.json`. A staging root holding
//! `ginary.json/nested.txt` therefore packed cleanly, and `unpack` let the
//! entry through to `unpack_in`, which created `<dest>/ginary.json` as a
//! *directory* before the final `create_new` of the manifest failed on it with
//! an unattributed `Io(AlreadyExists)` — the same unowned failure, and the same
//! occupied completeness marker, that reserving the names was meant to end.
//! `pack` deferred the whole problem to the machine that runs the binary,
//! because the artifact it wrote was one this reader always refuses.
//!
//! **The input.** Payloads whose entry 2 is `ginary.json/nested.txt`,
//! `./ginary.json/nested.txt` and `ginary.index.json/nested.txt`; and staging
//! roots holding a directory named `ginary.json` or `ginary.index.json`.
//!
//! **The correct behaviour.** A path whose *first component* is a reserved
//! name is refused at both ends — `DuplicateEntry` from `unpack`, naming the
//! path it would have landed on and leaving the destination untouched, and
//! `ReservedName` from `pack`.

use crate::common::payload::{
    RawEntry, RawTar, sample_manifest, sample_manifest_json, sha256, staging_tree, tree_listing,
};
use ginary::assemble::{Category, StagedFile};
use ginary::manifest::{INDEX_NAME, Index, MANIFEST_NAME};
use ginary::payload::{PayloadError, pack, unpack};

/// The compression level these tests pack at; none of them measures the
/// compressor.
const LEVEL: i32 = 3;

/// Unpacks a payload whose entry 2 is a file called `entry` and asserts it is
/// refused as a path under the reserved name fixed at `fixed`, with nothing
/// written to the destination.
fn unpack_refuses(entry: &str, destined: &str, fixed: usize) {
    let index = serde_json::to_vec(&Index { files: Vec::new() }).expect("serialise an index");
    let bytes = RawTar::new()
        .push(RawEntry::file(MANIFEST_NAME, &sample_manifest_json()))
        .push(RawEntry::file(INDEX_NAME, &index))
        .push(RawEntry::file(entry, b"ATTACKER MARKER"))
        .build_zstd(LEVEL);
    let sha = sha256(&bytes);
    let dir = tempfile::tempdir().expect("tempdir");
    let dest = dir.path().join("dest");
    std::fs::create_dir(&dest).expect("create dest");

    let error = unpack(bytes.as_slice(), bytes.len() as u64, &sha, &dest)
        .expect_err("an entry may not land under a name the format fixes at the front");

    match error {
        PayloadError::DuplicateEntry {
            position,
            name,
            fixed: at,
        } => {
            assert_eq!(position, 2);
            assert_eq!(name, destined);
            assert_eq!(at, fixed);
        }
        other => panic!("expected DuplicateEntry, got {other:?}"),
    }
    // Entry 1 is the one front-matter entry that is unpacked, so it is all a
    // refused payload may leave behind: no `ginary.json`, and nothing under
    // either reserved name.
    assert_eq!(
        tree_listing(&dest),
        [INDEX_NAME],
        "the refused payload occupied a path the format reserves"
    );
}

#[test]
fn an_entry_under_a_directory_named_like_the_manifest_is_refused() {
    unpack_refuses("ginary.json/nested.txt", "ginary.json/nested.txt", 0);
}

#[test]
fn an_entry_under_the_manifest_behind_a_current_directory_component_is_refused() {
    unpack_refuses("./ginary.json/nested.txt", "ginary.json/nested.txt", 0);
}

#[test]
fn an_entry_under_a_directory_named_like_the_index_is_refused() {
    unpack_refuses(
        "ginary.index.json/nested.txt",
        "ginary.index.json/nested.txt",
        1,
    );
}

/// Packs a staging root that also holds `<reserved>/nested.txt` and asserts
/// `pack` refuses it rather than writing an artifact it could not read back.
fn pack_refuses(reserved: &str, fixed: usize) {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut tree = staging_tree(dir.path());
    let data = b"a staged file inside a directory that pretends to be front matter";
    let nested = format!("{reserved}/nested.txt");
    std::fs::create_dir(tree.root.join(reserved)).expect("create the reserved directory");
    std::fs::write(tree.root.join(&nested), data).expect("write the nested file");
    tree.listing.files.push(StagedFile {
        path: nested.clone(),
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
        PayloadError::ReservedName { path, fixed: at } => {
            assert_eq!(path, nested);
            assert_eq!(at, fixed);
        }
        other => panic!("expected ReservedName, got {other:?}"),
    }
}

#[test]
fn packing_a_directory_named_like_the_manifest_is_refused() {
    pack_refuses(MANIFEST_NAME, 0);
}

#[test]
fn packing_a_directory_named_like_the_index_is_refused() {
    pack_refuses(INDEX_NAME, 1);
}
