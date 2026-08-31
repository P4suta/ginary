// SPDX-License-Identifier: MIT OR Apache-2.0
//! `GINARY_CMD=uninstall` deleted the crash dump beside the entries and
//! reported it as a cache entry it had removed.
//!
//! **What went wrong.** `cache::uninstall` walked the application directory
//! and treated *every* path without a `ginary.json` in it as residue: it
//! removed it and pushed it into the `removed` column. The launcher writes
//! `ERL_CRASH_DUMP=<app>/erl_crash.dump`, one level above the entry, so the
//! dump is a plain file in exactly that directory. An uninstall therefore
//! deleted the one artefact a user keeps an application directory around for,
//! and printed `removed <app>/erl_crash.dump` as though it were a runtime —
//! while the function's own comment justified keeping the directory because
//! "the crash dumps beside it are still worth reading".
//!
//! **The input.** An application directory holding one entry, one
//! `.<key>.tmp-<pid>` residue tree, and an `erl_crash.dump`.
//!
//! **The correct behaviour.** Uninstall removes what the cache owns — entries
//! and the temporary, corrupt and trashed trees beside them — and leaves
//! everything else alone. The dump survives, is in neither column, and the
//! application directory stays because it is not empty.

use std::path::Path;

use ginary::cache;
use ginary::launch::CRASH_DUMP_NAME;

use crate::common::cachefs::{DAY, plant_entry};

/// A cache key, which is what an entry directory is named.
const KEY: &str = "0123456789abcdef";

/// What the runtime wrote when it died, and what a user came back for.
const DUMP: &[u8] = b"=erl_crash_dump:0.5\nSlogan: init terminating in do_boot\n";

#[test]
fn an_uninstall_removes_what_the_cache_owns_and_leaves_the_crash_dump() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let app_dir = dir.path().join("hello");
    let entry = plant_entry(&app_dir, KEY, DAY);
    let residue = app_dir.join(format!(".{KEY}.tmp-4000000000"));
    std::fs::create_dir_all(residue.join("lib")).expect("plant the residue");
    let dump = app_dir.join(CRASH_DUMP_NAME);
    std::fs::write(&dump, DUMP).expect("plant the crash dump");

    let report = cache::uninstall(&app_dir);

    assert_eq!(
        std::fs::read(&dump).expect("the crash dump is still there"),
        DUMP,
        "the dump is not the cache's to delete: it is why the application directory is worth \
         keeping at all"
    );
    assert!(
        !report.removed.iter().any(|path| path == &dump),
        "and it may not be reported as a cache entry that was removed: {:?}",
        report.removed
    );
    assert!(
        !report.kept.iter().any(|(path, _)| path == &dump),
        "nor as one that was kept: {:?}",
        report.kept
    );

    // The two things uninstall does own still go, so the survival above is a
    // rule rather than an uninstall that did nothing.
    assert!(!entry.exists(), "the entry is the cache's own");
    assert!(!residue.exists(), "and so is the residue beside it");
    assert!(
        app_dir.is_dir(),
        "an application directory that still holds a dump is not empty"
    );
}

#[test]
fn a_stray_file_that_is_not_the_cache_s_is_left_alone_too() {
    // The rule is about ownership rather than about one file name: anything
    // the cache did not write stays, whatever it is called.
    let dir = tempfile::tempdir().expect("a temporary directory");
    let app_dir = dir.path().join("hello");
    plant_entry(&app_dir, KEY, DAY);
    let notes = app_dir.join("notes.txt");
    std::fs::write(&notes, b"why this application keeps dying\n").expect("plant the file");

    let report = cache::uninstall(&app_dir);

    assert!(Path::new(&notes).is_file(), "{notes:?} was not the cache's");
    assert_eq!(report.removed.len(), 1, "{:?}", report.removed);
}
