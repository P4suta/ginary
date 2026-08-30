// SPDX-License-Identifier: MIT OR Apache-2.0
//! Step 1's "a `<key>` without a manifest is moved aside" branch deleted a
//! *complete* entry.
//!
//! **What went wrong.** The branch was written as `if target.exists()`, which
//! is true of a finished cache entry as well as of a half-written one. On the
//! ordinary path it never fired, because step 1 returns early on a hit — but
//! every path that reaches step 2 with an entry in place is one where the hit
//! was deliberately skipped, and there the launcher renamed the *winner's*
//! tree to `.<key>.corrupt-<pid>` and removed it. A process that had lost the
//! rename race destroyed the entry it was about to reuse, and with it every
//! other process's cache hit.
//!
//! **The input.** A complete `<key>` — a directory holding `ginary.json` — in
//! an application directory, with the step-1 decision made against it.
//!
//! **The correct behaviour.** Completeness is decided by `ginary.json` and by
//! nothing else, at both ends: a `<key>` that holds one is left exactly as it
//! is, and a `<key>` that does not is moved aside and removed.

use std::path::Path;

use ginary::cache;
use ginary::manifest::MANIFEST_NAME;

/// The key these entries live under; nothing here hashes anything.
const KEY: &str = "0123456789abcdef";

/// The pid the move-aside names its temporary tree after.
const PID: u32 = 4_000_000_001;

/// Builds `<app_dir>/<key>` holding `payload.txt`, and `ginary.json` too when
/// `complete`.
fn plant_entry(app_dir: &Path, complete: bool) -> std::path::PathBuf {
    let entry = app_dir.join(KEY);
    std::fs::create_dir_all(entry.join("erts-17.0.5/bin")).expect("create the entry");
    std::fs::write(
        entry.join("erts-17.0.5/bin/erlexec"),
        b"the winner's runtime",
    )
    .expect("write a file the entry is made of");
    if complete {
        std::fs::write(entry.join(MANIFEST_NAME), b"{}").expect("write the manifest");
    }
    entry
}

#[test]
fn a_complete_entry_is_never_moved_aside() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app_dir = dir.path().join("hello");
    let entry = plant_entry(&app_dir, true);

    let discarded = cache::discard_incomplete(&app_dir, KEY, PID);

    assert!(
        !discarded,
        "an entry holding `{MANIFEST_NAME}` is complete and must be left alone"
    );
    assert!(entry.join(MANIFEST_NAME).is_file(), "the marker survives");
    assert_eq!(
        std::fs::read(entry.join("erts-17.0.5/bin/erlexec")).expect("read the runtime back"),
        b"the winner's runtime",
        "the winner's tree was destroyed by the process that lost the race"
    );
}

#[test]
fn an_entry_without_a_manifest_is_moved_aside_and_removed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app_dir = dir.path().join("hello");
    let entry = plant_entry(&app_dir, false);

    let discarded = cache::discard_incomplete(&app_dir, KEY, PID);

    assert!(discarded, "a `<key>` with no manifest is not an entry");
    assert!(
        !entry.exists(),
        "and it is gone rather than emptied in place"
    );
    let left: Vec<String> = std::fs::read_dir(&app_dir)
        .expect("list the application directory")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        left.is_empty(),
        "the tree it was moved aside to must go too, and {left:?} is left"
    );
}

#[test]
fn a_key_that_is_not_there_is_nothing_to_discard() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app_dir = dir.path().join("hello");
    std::fs::create_dir_all(&app_dir).expect("create the application directory");

    assert!(!cache::discard_incomplete(&app_dir, KEY, PID));
}
