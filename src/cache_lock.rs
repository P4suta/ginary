// SPDX-License-Identifier: MIT OR Apache-2.0
//! The advisory lock that keeps a cache entry alive for as long as the
//! application that is running out of it.
//!
//! A cache entry is a directory another process may want to delete. Pruning
//! must not delete the tree a running BEAM is executing out of, and the
//! launcher cannot stay alive to say so: it `execve`s, and after that there is
//! no ginary process left on the machine.
//!
//! `flock(2)` is what closes that gap. A lock belongs to the *open file
//! description* rather than to the process, and an open file description
//! survives `execve` as long as its descriptor does not carry `FD_CLOEXEC`.
//! So the launcher opens `<entry>/.lock`, clears `FD_CLOEXEC`, takes a shared
//! lock, and then `execve`s: `erlexec` inherits the descriptor, `beam.smp`
//! inherits it from `erlexec`, and the lock is released by the kernel when the
//! last of them exits. Nothing has to remember to unlock, and a `SIGKILL`
//! releases it as reliably as a clean exit.
//!
//! Pruning takes the other side: `flock(LOCK_EX | LOCK_NB)` on the same file
//! succeeds only when no runtime holds the shared lock, so an entry that is in
//! use is skipped rather than raced.
//!
//! Neither side ever blocks. A launcher takes the shared lock with `LOCK_NB`
//! too, retrying for [`SHARED_LOCK_BUDGET`] and then starting the application
//! unlocked: a lock that cannot be taken is a pruning risk, and an application
//! that hung until somebody else let go of a housekeeping lock would be a
//! worse outcome than an entry that might be pruned.
//!
//! See `docs/adr/0010-cache-locking-and-pruning.md`; the claim about `execve`
//! is not an argument, it is a test — `tests/launcher.rs` starts an artifact
//! whose runtime sleeps and asserts from outside that `flock -n -x` on the
//! entry's lock fails while the child runs and succeeds after it exits.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The name of the lock file inside a cache entry.
///
/// A dotted name so that it sorts and lists with the other bookkeeping the
/// cache puts beside an entry rather than looking like part of the runtime.
pub const LOCK_NAME: &str = ".lock";

/// How long [`SharedLock::acquire`] keeps trying before it gives up.
///
/// The shared lock is never contended by another launcher — shared locks do
/// not exclude each other — so the only thing that can be in the way is a
/// prune holding the entry exclusively, and a prune holds it only across one
/// `rename`. Long enough to outlast that, short enough that an application
/// whose entry is held by something else starts anyway rather than hanging.
pub const SHARED_LOCK_BUDGET: Duration = Duration::from_millis(500);

/// How often [`SharedLock::acquire`] retries inside [`SHARED_LOCK_BUDGET`].
const SHARED_LOCK_POLL: Duration = Duration::from_millis(10);

/// `<entry>/.lock`, the file both locks are taken on.
pub fn lock_path(entry: &Path) -> PathBuf {
    entry.join(LOCK_NAME)
}

/// A shared `flock` on a cache entry, held across `execve`.
///
/// Dropping it closes the descriptor and releases the lock, so a launcher that
/// fails between taking the lock and `execve` leaves nothing behind. A launcher
/// that reaches `execve` never drops it: the descriptor is inherited and the
/// kernel releases the lock when the last process holding it exits.
#[derive(Debug)]
pub struct SharedLock {
    /// The open `<entry>/.lock`, with `FD_CLOEXEC` cleared.
    file: File,
    /// The file the lock is on, for diagnostics.
    path: PathBuf,
}

impl SharedLock {
    /// Opens `<entry>/.lock`, clears `FD_CLOEXEC` and takes `LOCK_SH`.
    ///
    /// Never blocks. The lock is taken with `LOCK_SH | LOCK_NB` and retried
    /// for at most [`SHARED_LOCK_BUDGET`], because a launcher that waited on
    /// this lock would be a packaged application that hangs for as long as
    /// some other process holds the entry — the one failure mode this design
    /// exists to rule out.
    ///
    /// # Errors
    ///
    /// Whatever the open, the `fcntl` or the last `flock` failed with. A
    /// caller on the launcher path treats every one of them as "no lock"
    /// rather than as a reason not to start: a cache entry that could not be
    /// locked is a pruning risk, and refusing to run would be worse.
    pub fn acquire(entry: &Path) -> std::io::Result<Self> {
        let path = lock_path(entry);
        let file = open_lock(&path)?;
        // The whole design in one call. The standard library opens every file
        // `O_CLOEXEC`, which is right for every other descriptor a launcher
        // holds and wrong for this one: the lock has to outlive `execve` and
        // belong to the runtime, so the flag comes off before the lock goes on.
        rustix::io::fcntl_setfd(&file, rustix::io::FdFlags::empty())?;

        let deadline = Instant::now() + SHARED_LOCK_BUDGET;
        loop {
            match rustix::fs::flock(&file, rustix::fs::FlockOperation::NonBlockingLockShared) {
                Ok(()) => return Ok(Self { file, path }),
                Err(error) if Instant::now() >= deadline => return Err(error.into()),
                Err(_) => std::thread::sleep(SHARED_LOCK_POLL),
            }
        }
    }

    /// The file the lock is held on.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The open descriptor, so a caller can prove it is not close-on-exec.
    pub fn file(&self) -> &File {
        &self.file
    }
}

/// An exclusive `flock` on a cache entry, held only while it is being removed.
///
/// Unlike [`SharedLock`] this one *is* close-on-exec: nothing execs while it
/// is held, and a descriptor that leaked into a child would keep an entry
/// locked long after the pruning that took it had finished.
#[derive(Debug)]
pub struct ExclusiveLock {
    /// The open `<entry>/.lock`.
    file: File,
    /// The file the lock is on.
    path: PathBuf,
}

impl ExclusiveLock {
    /// The file the lock is held on.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The open descriptor.
    pub fn file(&self) -> &File {
        &self.file
    }
}

/// Takes `LOCK_EX | LOCK_NB` on `<entry>/.lock`, or answers [`None`].
///
/// [`None`] means one of two things and pruning treats them the same: another
/// process holds the entry, or the lock file could not be opened at all. Both
/// are "leave this entry alone", because the cost of skipping an entry is a
/// directory that stays on disk and the cost of removing one wrongly is a
/// running application losing its runtime.
pub fn try_exclusive(entry: &Path) -> Option<ExclusiveLock> {
    let path = lock_path(entry);
    let file = open_lock(&path).ok()?;
    // `FD_CLOEXEC` is left alone, and deliberately: nothing execs while this
    // is held, and a descriptor that leaked into a child would keep an entry
    // locked long after the pruning that took it had finished.
    rustix::fs::flock(&file, rustix::fs::FlockOperation::NonBlockingLockExclusive).ok()?;
    Some(ExclusiveLock { file, path })
}

/// Opens `<entry>/.lock`, creating it if this is the first lock on the entry.
///
/// Read and write, because `flock` needs neither and a file opened for neither
/// cannot be created. The contents are never written: what carries the meaning
/// is the lock, not the bytes.
fn open_lock(path: &Path) -> std::io::Result<File> {
    File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
}
