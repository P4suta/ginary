// SPDX-License-Identifier: MIT OR Apache-2.0
//! Nothing could ever be pruned or uninstalled on Windows, because the
//! removal renamed the entry directory while still holding the lock file
//! inside it open.
//!
//! **What went wrong.** Every complete cache entry was reported `unremovable`
//! on the first Windows runner, whatever its age and whether or not anybody
//! held it:
//!
//! ```text
//! ---- cache_prune_removes_the_old_and_keeps_the_fresh_with_a_reason stdout ----
//! the table must name what went, and it said:
//! kept C:\Users\RUNNER~1\...\cache\hello\1111111111111111 (unremovable)
//! kept C:\Users\RUNNER~1\...\cache\hello\2222222222222222 (fresh)
//! total: 0 removed, 2 kept
//! ```
//!
//! The entry was found, its age was read, the exclusive lock was taken — and
//! the `std::fs::rename` that moves it aside was refused.
//!
//! (`Windows build and exit-code propagation`
//! <https://github.com/P4suta/ginary/actions/runs/33712111530/job/100513644404>.)
//!
//! **The input.** Any `ginary cache prune`, `ginary cache clean` or
//! `ginary uninstall` on Windows. [`ginary::cache_lock::try_exclusive`] opens
//! `<entry>/.lock` and holds it across the rename;
//! `docs/adr/0015-windows-launcher-stays-resident.md` argued that
//! `FILE_SHARE_DELETE` on that handle would let the rename through. A real
//! Windows kernel says otherwise: sharing deletion permits *that file* to be
//! deleted or renamed, not an ancestor directory of it.
//!
//! **The correct behaviour.** The lock proves nobody is using the entry and
//! the rename is the claim, and on a platform that refuses the second while
//! the first is held they happen in that order rather than at once.
//! [`ginary::platform::rename_refuses_open_children`] is where that fact is
//! written down. Releasing the lock first costs nothing on unix — the rename
//! is still the claim there — so there is one order rather than two, and a
//! rename that is still refused is still reported as `unremovable` rather
//! than forced.

use ginary::platform::rename_refuses_open_children;
use ginary::target::Os;

#[test]
fn the_platform_that_refuses_to_rename_a_directory_it_holds_open_is_named() {
    assert!(
        rename_refuses_open_children(Os::Windows),
        "a directory with an open handle anywhere inside it cannot be renamed there, whatever \
         share mode that handle asked for"
    );
}

#[test]
fn a_unix_rename_does_not_care_what_is_open_underneath() {
    assert_eq!(
        [
            rename_refuses_open_children(Os::Linux),
            rename_refuses_open_children(Os::Macos),
        ],
        [false, false],
        "a rename there moves a directory entry and says nothing about the descriptors anybody \
         holds"
    );
}
