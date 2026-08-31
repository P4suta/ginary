// SPDX-License-Identifier: MIT OR Apache-2.0
//! `SharedLock::acquire` on Windows created `<entry>/.lock` for writing before
//! taking its read-only handle, so the second launcher of the same entry was
//! refused and ran with no lock at all.
//!
//! The Windows shared lock is an open handle: read access, sharing read
//! access. Two of those are compatible, which is what makes several runtimes
//! able to hold one cache entry. The implementation reached that handle
//! through `open_lock`, which asks for `GENERIC_WRITE` so that it can create
//! the file — and a handle the first launcher holds shares read access only,
//! so the second launcher's *create* was refused with a sharing violation
//! before its share-mode open was ever attempted. `SharedLock::acquire` then
//! spun for its whole budget over a holder that never lets go, and
//! `launcher::take_lock` started the application unlocked, which is exactly
//! the state a concurrent prune may delete an entry out from under.
//!
//! The fix: try the share-mode open first and create only when the file is not
//! there. A write handle is needed on a cache entry's very first lock and
//! never again — when a shared holder exists, the file does too.
//!
//! **This test does not run on Linux.** It is the Windows half of a Windows
//! rule and there is no `CreateFile` here to hold it to; it compiles for
//! `x86_64-pc-windows-gnu` and `docs/dev/log/D2.md` lists it among the claims
//! the GitHub Actions milestone has to check.
#![cfg(windows)]

use ginary::cache_lock::SharedLock;

#[test]
fn two_launchers_of_one_entry_both_hold_the_shared_lock() {
    let dir = tempfile::tempdir().expect("tempdir");
    let entry = dir.path().join("0123456789abcdef");
    std::fs::create_dir(&entry).expect("the entry");

    let first = SharedLock::acquire(&entry).expect("the first launcher takes the lock");
    let second = SharedLock::acquire(&entry)
        .expect("a second launcher of the same entry must take it too: both ask for read access");

    assert_eq!(first.path(), second.path());
    assert!(
        ginary::cache_lock::try_exclusive(&entry).is_none(),
        "and a prune is refused while either of them holds it"
    );
    drop(second);
    drop(first);
    assert!(
        ginary::cache_lock::try_exclusive(&entry).is_some(),
        "an entry nobody is running out of is one a prune may take"
    );
}
