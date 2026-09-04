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

/// How long [`wait_exclusive`] keeps trying before it gives up.
///
/// Long, because what holds the lock is another process filling the same cache
/// entry, and filling one means fetching forty megabytes and unpacking it. A
/// caller that gave up after a few seconds would start a second download of
/// the file it is waiting for, which is the outcome the lock exists to
/// prevent. Bounded all the same: a holder that died without releasing — which
/// `flock` makes impossible for a process that exits, and possible for one
/// that hangs — must not hang every build on the machine for ever.
pub const FILL_LOCK_BUDGET: Duration = Duration::from_secs(900);

/// How often [`wait_exclusive`] retries inside [`FILL_LOCK_BUDGET`].
const FILL_LOCK_POLL: Duration = Duration::from_millis(25);

/// Which of the two locks a caller wants.
///
/// The unix implementation takes an advisory `flock` and the kind is the
/// operation; the Windows implementation has no advisory lock at all and takes
/// a *share mode* on the open handle instead, so the kind is what decides
/// which sharing the handle refuses. See [`windows_share_mode`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LockKind {
    /// The lock a running application holds on its own entry.
    Shared,
    /// The lock pruning needs before it may remove another.
    Exclusive,
}

/// `FILE_SHARE_READ`: another handle may open the file for reading.
pub const FILE_SHARE_READ: u32 = 0x0000_0001;

/// `FILE_SHARE_DELETE`: another handle may delete or rename *this file*.
///
/// It is here because the removal that follows a prune deletes `<entry>/.lock`
/// along with the tree it is in, and Windows refuses to delete a file whose
/// open handles do not permit it.
///
/// It does **not** extend to the directory the file is in, which is what this
/// crate believed until a real Windows runner said otherwise: renaming
/// `<entry>` while `<entry>/.lock` is open is refused whatever this handle
/// shares. [`crate::cache::prune_app`] therefore releases the lock before the
/// rename where the platform requires that —
/// [`crate::platform::rename_refuses_open_children`] — and
/// `docs/dev/log/E8.md` records the run.
pub const FILE_SHARE_DELETE: u32 = 0x0000_0004;

/// The `dwShareMode` `<entry>/.lock` is opened with on Windows.
///
/// Windows has no `flock(2)`, and the advisory lock this module is built on
/// does not exist there. What does exist is mandatory sharing on the handle
/// itself: a file opened for writing with `FILE_SHARE_READ` can be opened
/// again for reading and cannot be opened again for writing, and a file opened
/// sharing no read or write access cannot be opened again for either. So the
/// two locks become two share modes, and the correspondence is exact where it
/// matters — several runtimes may hold one entry at once, and a prune that
/// wants it exclusively is refused while any of them does.
///
/// Two differences a reader has to know about, both recorded in
/// `docs/adr/0015-windows-launcher-stays-resident.md`:
///
/// - the lock is *mandatory* rather than advisory, so a program that knows
///   nothing about ginary is held to it too;
/// - it belongs to the handle rather than to an open file description, and
///   there is no `execve` for it to survive, which is why the Windows launcher
///   stays alive as the runtime's parent and holds the lock itself.
pub const fn windows_share_mode(kind: LockKind) -> u32 {
    match kind {
        // The runtime opens the file for reading only and lets other readers
        // in, so every launcher of the same entry gets its handle and the
        // prune below gets none of them.
        LockKind::Shared => FILE_SHARE_READ,
        // No reading and no writing: an entry a prune can open is an entry no
        // runtime is holding. Deletion is the one thing it does share, and it
        // shares it with the removal that follows, which deletes this very
        // file along with the tree it is in. Sharing deletion says nothing
        // about read or write access, so what the two locks mean to each other
        // is unchanged.
        LockKind::Exclusive => FILE_SHARE_DELETE,
    }
}

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
    #[cfg(unix)]
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

    /// Opens `<entry>/.lock` for reading with [`windows_share_mode`]'s shared
    /// mode, and holds it.
    ///
    /// The Windows half of the same contract, reached by a different mechanism.
    /// There is no advisory lock and nothing to inherit across an `execve` that
    /// does not exist, so what makes the entry unremovable is the open handle
    /// itself: it asks for read access and shares read access, so another
    /// launcher's identical open succeeds and [`try_exclusive`]'s open — which
    /// shares no read access — is refused for as long as any of them is held. The
    /// launcher keeps this value alive for the runtime's whole lifetime, which
    /// is why the Windows launcher does not exit at the spawn.
    ///
    /// Never blocks, and retries for [`SHARED_LOCK_BUDGET`], for the reason the
    /// unix half does.
    ///
    /// # Errors
    ///
    /// Whatever the last open failed with, which a caller on the launcher path
    /// treats as "no lock" rather than as a reason not to start.
    #[cfg(windows)]
    pub fn acquire(entry: &Path) -> std::io::Result<Self> {
        let path = lock_path(entry);
        let deadline = Instant::now() + SHARED_LOCK_BUDGET;
        loop {
            match open_windows_shared(&path) {
                Ok(file) => return Ok(Self { file, path }),
                Err(error) if Instant::now() >= deadline => return Err(error),
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
#[cfg(unix)]
pub fn try_exclusive(entry: &Path) -> Option<ExclusiveLock> {
    let path = lock_path(entry);
    let file = open_lock(&path).ok()?;
    // `FD_CLOEXEC` is left alone, and deliberately: nothing execs while this
    // is held, and a descriptor that leaked into a child would keep an entry
    // locked long after the pruning that took it had finished.
    rustix::fs::flock(&file, rustix::fs::FlockOperation::NonBlockingLockExclusive).ok()?;
    Some(ExclusiveLock { file, path })
}

/// Opens `<entry>/.lock` sharing only deletion, or answers [`None`].
///
/// The Windows half of [`try_exclusive`], and it answers [`None`] in exactly
/// the same two cases: a runtime holds the entry — its handle shares read
/// access, and this open asks for write access as well as read — or the lock
/// file could not be opened at all. Both mean "leave this entry alone". What
/// this handle shares is [`FILE_SHARE_DELETE`] and nothing else, so the answer
/// above is unchanged and the removal that follows may delete this file along
/// with the tree it is in. It does *not* let the caller rename the entry
/// directory while this handle is open — `FILE_SHARE_DELETE` speaks for the
/// file it is on and not for an ancestor of it, which is why
/// [`crate::cache::prune_app`] releases the lock first on a platform that
/// refuses that; see [`crate::platform::rename_refuses_open_children`].
#[cfg(windows)]
pub fn try_exclusive(entry: &Path) -> Option<ExclusiveLock> {
    let path = lock_path(entry);
    let file = open_windows_exclusive(&path).ok()?;
    Some(ExclusiveLock { file, path })
}

/// Takes `LOCK_EX` on `<entry>/.lock`, waiting up to `budget` for it.
///
/// [`try_exclusive`] is the pruning side, where a lock that cannot be taken
/// means "leave this entry alone". This is the filling side, where it means
/// "somebody else is doing the work you were about to do": the right answer is
/// to wait for them and then look again, not to skip and not to race.
///
/// Never blocks in the kernel. `flock` is taken with `LOCK_NB` and retried, so
/// the budget is honoured on a machine whose holder never lets go.
///
/// # Errors
///
/// Whatever the open or the last `flock` failed with, and
/// [`std::io::ErrorKind::WouldBlock`] when `budget` ran out with the lock
/// still held elsewhere.
#[cfg(unix)]
pub fn wait_exclusive(entry: &Path, budget: Duration) -> std::io::Result<ExclusiveLock> {
    let path = lock_path(entry);
    let file = open_lock(&path)?;
    let deadline = Instant::now() + budget;
    loop {
        match rustix::fs::flock(&file, rustix::fs::FlockOperation::NonBlockingLockExclusive) {
            Ok(()) => return Ok(ExclusiveLock { file, path }),
            Err(error) if Instant::now() >= deadline => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WouldBlock,
                    format!("{} is held by another process: {error}", path.display()),
                ));
            }
            Err(_) => std::thread::sleep(FILL_LOCK_POLL),
        }
    }
}

/// Opens `<entry>/.lock` sharing only deletion, retrying up to `budget`.
///
/// The Windows half of [`wait_exclusive`]. The open is retried rather than the
/// lock, because on Windows the open *is* the lock.
///
/// # Errors
///
/// [`std::io::ErrorKind::WouldBlock`] when `budget` ran out with the entry
/// still held elsewhere, carrying the last open's failure in its message.
#[cfg(windows)]
pub fn wait_exclusive(entry: &Path, budget: Duration) -> std::io::Result<ExclusiveLock> {
    let path = lock_path(entry);
    let deadline = Instant::now() + budget;
    loop {
        match open_windows_exclusive(&path) {
            Ok(file) => return Ok(ExclusiveLock { file, path }),
            Err(error) if Instant::now() >= deadline => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WouldBlock,
                    format!("{} is held by another process: {error}", path.display()),
                ));
            }
            Err(_) => std::thread::sleep(FILL_LOCK_POLL),
        }
    }
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

/// Opens `<entry>/.lock` the way a running application holds it on Windows.
///
/// Read access only, sharing read access. Both halves matter. Asking for read
/// access alone is what lets a second launcher of the same entry open it too —
/// two handles that each share read access are compatible with each other, and
/// two that asked for write access would not be. Sharing read access alone is
/// what refuses [`open_windows_exclusive`], which asks for write access.
///
/// The share-mode open is tried **first**, and the file is created only when
/// there is not one. The order is the whole correctness of the function. A
/// create needs write access, and a launcher that already holds this lock
/// permits read access and nothing else, so a create attempted first is
/// refused with a sharing violation and the second launcher of an entry ends
/// up with no lock at all — the opposite of what the shared lock is for. A
/// write handle is needed on an entry's very first lock and never again: when
/// a shared holder exists, so does the file. A prune that took the entry in
/// between is what the retry loop in [`SharedLock::acquire`] is for.
#[cfg(windows)]
fn open_windows_shared(path: &Path) -> std::io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt as _;

    let shared = || {
        File::options()
            .read(true)
            .share_mode(windows_share_mode(LockKind::Shared))
            .open(path)
    };
    match shared() {
        Ok(file) => Ok(file),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            drop(open_lock(path)?);
            shared()
        }
        Err(error) => Err(error),
    }
}

/// Opens `<entry>/.lock` the way pruning holds it on Windows.
///
/// Read and write access, sharing only deletion: an open that succeeds proves
/// no runtime holds the entry, and holds it against every one that arrives
/// while the prune runs. Deletion is shared because the removal that follows
/// deletes this file along with the tree it is in; see [`FILE_SHARE_DELETE`].
#[cfg(windows)]
fn open_windows_exclusive(path: &Path) -> std::io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt as _;

    File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .share_mode(windows_share_mode(LockKind::Exclusive))
        .open(path)
}
