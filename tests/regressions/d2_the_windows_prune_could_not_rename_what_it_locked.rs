// SPDX-License-Identifier: MIT OR Apache-2.0
//! Pruning on Windows opened `<entry>/.lock` sharing nothing and then renamed
//! the directory the handle is inside, which Windows refuses.
//!
//! `cache::prune_app` takes `cache_lock::try_exclusive` on the entry, renames
//! the entry aside, drops the lock and removes the tree. On unix that order is
//! deliberate and correct: `flock` is advisory and `rename(2)` does not care
//! what is open. On Windows a directory with an open handle beneath it cannot
//! be renamed *unless every one of those handles was opened with*
//! `FILE_SHARE_DELETE` — and the exclusive lock's share mode was zero. Every
//! prunable entry would therefore have been reported `Unremovable` and
//! `ginary cache prune` would never have removed anything.
//!
//! The fix was the share mode: `FILE_SHARE_DELETE` permits deleting and
//! renaming and says nothing about reading or writing, so what the two locks
//! mean to each other is unchanged — a runtime's shared handle asks for read
//! access and shares read access, and this open still asks for write access
//! that the shared handle does not permit. The bit is what lets the removal
//! that follows delete `<entry>/.lock` along with the tree it is in, and it is
//! what these three assertions are about.
//!
//! **E8 correction.** This file also claimed the bit was enough to rename the
//! entry *while the lock was still held*, and that dropping the lock first was
//! therefore avoidable. A real Windows kernel refuted that on 2026-09-03:
//! every complete entry the first Windows runner found was reported
//! `unremovable`. `FILE_SHARE_DELETE` speaks for the file it is on, not for an
//! ancestor directory of it, so `cache::prune_app` and `cache::uninstall` now
//! release the lock before the rename where the platform requires it —
//! [`ginary::platform::rename_refuses_open_children`], pinned by
//! `tests/regressions/e8_a_removal_renamed_a_directory_it_still_held_open.rs`.
//! Nothing below changed: the share mode is still the right one and is still
//! asserted here.
//!
//! `windows_share_mode` is a `const fn` on every platform precisely so that
//! this rule is a test on the machine ginary is developed on rather than a
//! claim nobody here can check.

use ginary::cache_lock::{FILE_SHARE_READ, LockKind, windows_share_mode};

/// `FILE_SHARE_DELETE`: another handle may delete or rename *this file*.
///
/// Not the directory it is in: that is the reading E8 had to correct.
///
/// Spelled here rather than taken from the crate, so that the test states the
/// value it is about rather than agreeing with whatever the code says.
const FILE_SHARE_DELETE: u32 = 0x0000_0004;

#[test]
fn the_exclusive_share_mode_permits_the_deletion_the_removal_performs() {
    let exclusive = windows_share_mode(LockKind::Exclusive);

    assert_eq!(
        exclusive & FILE_SHARE_DELETE,
        FILE_SHARE_DELETE,
        "the removal deletes `<entry>/.lock` along with the tree it is in, and \
         Windows refuses to delete a file whose open handles do not permit it"
    );
    assert_eq!(
        exclusive & FILE_SHARE_READ,
        0,
        "and it shares nothing else: an entry a prune can open is an entry no \
         runtime is running out of"
    );
    assert_eq!(
        FILE_SHARE_DELETE, 4,
        "the Win32 value, not a name for a guess"
    );
}
