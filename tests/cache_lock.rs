// SPDX-License-Identifier: MIT OR Apache-2.0
//! The advisory lock a running application holds on its cache entry.
//!
//! Two claims, and only one of them can be made from inside this process.
//!
//! The first is that the two locks exclude each other: while a [`SharedLock`]
//! is held, `flock -n -x` on the same file fails, and once it is dropped the
//! same command succeeds. That is asserted here, against util-linux
//! `flock(1)` rather than against ginary's own [`ginary::cache_lock`], because
//! a lock proved with the code that takes it proves nothing about the kernel.
//!
//! The second is that the shared lock survives `execve`. It cannot be made
//! here: there is no `execve` in a test binary that wants to keep running.
//! `tests/launcher.rs` makes it against a real launcher and a real runtime.

mod common;

use std::path::Path;

use common::cachefs::{HeldLock, is_unlocked, lock_path, wait_until_unlocked};
use common::tools::require_tools;

use ginary::cache_lock::{self, LOCK_NAME, SharedLock};

/// A cache entry directory that exists and holds nothing else.
fn entry(dir: &Path) -> std::path::PathBuf {
    let entry = dir.join("hello").join("0123456789abcdef");
    std::fs::create_dir_all(&entry).expect("create the entry");
    entry
}

#[test]
fn taking_the_shared_lock_creates_the_dotted_file_inside_the_entry() {
    assert_eq!(LOCK_NAME, ".lock");
    assert_eq!(
        cache_lock::lock_path(Path::new("/c/hello/abc")),
        Path::new("/c/hello/abc/.lock"),
        "the lock lives in the entry it locks, so removing the entry removes it"
    );

    let dir = tempfile::tempdir().expect("tempdir");
    let entry = entry(dir.path());

    let lock = SharedLock::acquire(&entry).expect("a fresh entry must be lockable");

    assert_eq!(lock.path(), lock_path(&entry));
    assert!(
        lock_path(&entry).is_file(),
        "the lock file is created rather than required to be there"
    );
}

#[test]
fn a_shared_lock_blocks_an_exclusive_one_until_it_is_dropped() {
    let Some(tools) = require_tools(&["flock"]) else {
        return;
    };
    let dir = tempfile::tempdir().expect("tempdir");
    let entry = entry(dir.path());
    let path = lock_path(&entry);

    let lock = SharedLock::acquire(&entry).expect("a fresh entry must be lockable");
    assert!(
        !is_unlocked(tools.path("flock"), &path),
        "while the runtime holds the entry, pruning must not be able to take it"
    );

    drop(lock);
    // Waited for rather than demanded at once, and the reason is this file's
    // own subject: the descriptor is deliberately *not* close-on-exec, so a
    // `fork` another test in this binary makes while the lock is held carries
    // a copy of it, and the lock outlives the drop until that child exits. A
    // launcher execs immediately after acquiring, so nothing on the real path
    // can hold a second copy; a multi-threaded test binary can.
    assert!(
        wait_until_unlocked(tools.path("flock"), &path, true),
        "a released lock must leave the entry prunable"
    );
}

#[test]
fn two_shared_locks_on_one_entry_coexist() {
    // Two runs of one artifact share a cache entry, and neither may wait for
    // the other: a shared lock that behaved like an exclusive one would
    // serialise every start of an application.
    let dir = tempfile::tempdir().expect("tempdir");
    let entry = entry(dir.path());

    let first = SharedLock::acquire(&entry).expect("the first run locks the entry");
    let second = SharedLock::acquire(&entry).expect("the second run must not be blocked");

    assert_eq!(first.path(), second.path());
}

#[test]
fn try_exclusive_succeeds_on_a_free_entry_and_fails_on_a_held_one() {
    let dir = tempfile::tempdir().expect("tempdir");
    let entry = entry(dir.path());

    let taken = cache_lock::try_exclusive(&entry).expect("nobody holds a fresh entry");
    assert_eq!(taken.path(), lock_path(&entry));
    drop(taken);

    let shared = SharedLock::acquire(&entry).expect("the runtime takes the entry");
    assert!(
        cache_lock::try_exclusive(&entry).is_none(),
        "pruning must not be able to remove an entry a runtime is running out of"
    );
    drop(shared);

    // Polled for the reason above: a concurrent test's `fork` may still hold
    // an inherited copy of the descriptor this file deliberately leaves
    // inheritable.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut freed = false;
    while !freed && std::time::Instant::now() < deadline {
        freed = cache_lock::try_exclusive(&entry).is_some();
        if !freed {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
    assert!(
        freed,
        "the entry becomes prunable again when the runtime lets go"
    );
}

#[test]
fn two_exclusive_locks_on_one_entry_do_not_coexist() {
    let dir = tempfile::tempdir().expect("tempdir");
    let entry = entry(dir.path());

    let first = cache_lock::try_exclusive(&entry).expect("the first pruner takes the entry");
    assert!(
        cache_lock::try_exclusive(&entry).is_none(),
        "two pruners must not both believe they own the entry"
    );
    drop(first);
}

#[test]
fn a_lock_another_process_holds_is_refused_to_pruning() {
    let Some(tools) = require_tools(&["flock"]) else {
        return;
    };
    let dir = tempfile::tempdir().expect("tempdir");
    let entry = entry(dir.path());
    let held = HeldLock::take(tools.path("flock"), &entry);

    assert!(
        cache_lock::try_exclusive(&entry).is_none(),
        "the holder is another process entirely, which is the case pruning exists for"
    );

    held.release(tools.path("flock"));
    assert!(
        cache_lock::try_exclusive(&entry).is_some(),
        "and the entry is prunable once that process is gone"
    );
}

#[test]
fn the_shared_lock_gives_up_rather_than_waiting_for_an_exclusive_one() {
    // The failure mode the launcher's design forbids: a packaged application
    // that hangs for as long as somebody else holds a housekeeping lock. The
    // shared lock is non-blocking with a budget, so this returns.
    let Some(tools) = require_tools(&["flock"]) else {
        return;
    };
    let dir = tempfile::tempdir().expect("tempdir");
    let entry = entry(dir.path());
    let held = HeldLock::take(tools.path("flock"), &entry);

    let started = std::time::Instant::now();
    let result = SharedLock::acquire(&entry);
    let waited = started.elapsed();

    assert!(
        result.is_err(),
        "an entry another process holds exclusively cannot be shared-locked"
    );
    assert!(
        waited < cache_lock::SHARED_LOCK_BUDGET * 4,
        "acquire must give up inside its own budget, and it waited {waited:?}"
    );
    held.release(tools.path("flock"));

    SharedLock::acquire(&entry).expect("and the entry is lockable again once nobody holds it");
}

#[test]
fn locking_an_entry_that_is_not_there_is_an_error_rather_than_a_panic() {
    let dir = tempfile::tempdir().expect("tempdir");
    let missing = dir.path().join("hello").join("0123456789abcdef");

    let error = SharedLock::acquire(&missing)
        .expect_err("a lock file cannot be created in a directory that is not there");
    assert_eq!(
        error.kind(),
        std::io::ErrorKind::NotFound,
        "the launcher decides what to do about a missing entry; the lock only reports"
    );
    assert!(
        cache_lock::try_exclusive(&missing).is_none(),
        "and pruning treats it as an entry to leave alone"
    );
}
