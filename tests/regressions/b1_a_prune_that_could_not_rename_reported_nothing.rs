// SPDX-License-Identifier: MIT OR Apache-2.0
//! An entry a prune could not move aside disappeared from the report
//! altogether.
//!
//! **What went wrong.** `cache::prune_app` renames a stale entry to
//! `.<key>.trash-<pid>` before removing it, and when that rename failed it
//! simply went on to the next entry: the path was pushed into neither
//! `removed` nor `kept`. `PruneReport` is documented as "what one prune
//! removed and what it left, with the reason", and `ginary cache prune` prints
//! `total: N removed, M kept` from it — so an application directory that is
//! read-only produced a prune that reported nothing at all, having done
//! nothing at all. `cache::uninstall` reported the same failure as `kept` on
//! the line below, which is how the two drifted apart.
//!
//! **The input.** An application directory holding one stale entry, made
//! read-only so the rename fails while the lock still succeeds.
//!
//! **The correct behaviour.** The entry is reported as kept, with a reason
//! that is neither `locked` nor `fresh`, because it is neither.

use std::os::unix::fs::PermissionsExt as _;

use ginary::cache::{self, KeptReason, PruneOptions};
use ginary::diag::Diag;

use crate::common::cachefs::{DAY, plant_entry};

/// The stale entry, old enough that only the file system saves it.
const KEY: &str = "0000000000000000";

#[test]
fn an_entry_whose_rename_fails_is_reported_as_kept() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let app_dir = dir.path().join("hello");
    let old = plant_entry(&app_dir, KEY, DAY * 30);
    // Read-only for the *directory*, so the rename inside it fails while the
    // entry itself stays writable and its `.lock` can still be taken.
    std::fs::set_permissions(&app_dir, std::fs::Permissions::from_mode(0o500))
        .expect("make the application directory read-only");

    let report = cache::prune_app(
        &app_dir,
        None,
        PruneOptions::default(),
        std::time::SystemTime::now(),
        &Diag::disabled(),
    );

    std::fs::set_permissions(&app_dir, std::fs::Permissions::from_mode(0o700))
        .expect("restore the mode so the temporary directory can be cleaned up");

    assert!(
        report.removed.is_empty(),
        "nothing was removed: {:?}",
        report.removed
    );
    assert_eq!(
        report.kept,
        vec![(old.clone(), KeptReason::Unremovable)],
        "an entry a prune could not move is an entry it kept, and the reason is not that \
         somebody holds it"
    );
    assert!(old.join("ginary.json").is_file(), "and it is still there");
}
