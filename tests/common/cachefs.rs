// SPDX-License-Identifier: MIT OR Apache-2.0
//! Planting cache entries with a chosen age, and holding a lock on one from
//! outside the code under test.
//!
//! Pruning is a decision about two things ginary cannot fake for itself in a
//! test: how old an entry is, and whether anybody is using it. Age is a
//! modification time, which [`plant_entry`] sets outright rather than waiting
//! for. Use is an `flock`, and the lock has to be taken by a *different*
//! process — a test that took it through `ginary::cache_lock` would be asking
//! the code under test whether the code under test works.
//!
//! So [`HeldLock`] shells out to util-linux `flock(1)`, which is also the tool
//! the ADR's inheritance proof uses. A test that needs it gates on
//! [`crate::common::tools::require_tools`] first.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, SystemTime};

/// Seconds in a day, for turning a prune age into a modification time.
pub const DAY: Duration = Duration::from_secs(86_400);

/// How long a poll for a lock state waits before the test fails.
pub const LOCK_BUDGET: Duration = Duration::from_secs(10);

/// Writes `<app_dir>/<key>/ginary.json` and back-dates it by `age`.
///
/// The manifest is the completeness marker and its modification time is what
/// pruning reads, so this is the whole of what one prunable entry is. The
/// contents are a placeholder object: nothing in pruning parses it.
///
/// # Panics
///
/// If the directory or the file cannot be written, or if the clock cannot
/// represent `now - age`.
pub fn plant_entry(app_dir: &Path, key: &str, age: Duration) -> PathBuf {
    let entry = app_dir.join(key);
    std::fs::create_dir_all(&entry)
        .unwrap_or_else(|error| panic!("cannot create {}: {error}", entry.display()));
    let manifest = entry.join("ginary.json");
    std::fs::write(&manifest, b"{}\n")
        .unwrap_or_else(|error| panic!("cannot write {}: {error}", manifest.display()));
    let when = SystemTime::now()
        .checked_sub(age)
        .unwrap_or_else(|| panic!("the clock cannot represent {age:?} ago"));
    set_mtime(&manifest, when);
    entry
}

/// Sets one file's modification time.
///
/// # Panics
///
/// If the file cannot be opened or the time cannot be set.
pub fn set_mtime(path: &Path, when: SystemTime) {
    let file = std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap_or_else(|error| panic!("cannot open {}: {error}", path.display()));
    file.set_times(std::fs::FileTimes::new().set_modified(when))
        .unwrap_or_else(|error| panic!("cannot back-date {}: {error}", path.display()));
}

/// `<entry>/.lock`, the file both locks are taken on.
pub fn lock_path(entry: &Path) -> PathBuf {
    entry.join(ginary::cache_lock::LOCK_NAME)
}

/// Whether `flock -n -x <path>` succeeds right now.
///
/// `true` means nobody holds the file: this is the question pruning asks, put
/// to a program that is not ginary.
///
/// # Panics
///
/// If `flock` cannot be run at all.
pub fn is_unlocked(flock: &Path, path: &Path) -> bool {
    let status = Command::new(flock)
        .arg("-n")
        .arg("-x")
        .arg(path)
        .args(["true"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap_or_else(|error| panic!("cannot run {}: {error}", flock.display()));
    status.success()
}

/// Blocks until [`is_unlocked`] answers `wanted`, or gives up.
///
/// Answers whether it got there, so the caller asserts rather than hangs: a
/// lock that never appears is a failed claim about the launcher, not a reason
/// for a test binary to stall.
pub fn wait_until_unlocked(flock: &Path, path: &Path, wanted: bool) -> bool {
    let deadline = std::time::Instant::now() + LOCK_BUDGET;
    while std::time::Instant::now() < deadline {
        if path.exists() && is_unlocked(flock, path) == wanted {
            return true;
        }
        if !path.exists() && wanted {
            // A lock file that is not there is a lock nobody holds.
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

/// An exclusive `flock` on a cache entry, held by a child process.
///
/// The child is `flock -x <lock> sh -c 'read line'` with a pipe on its
/// standard input, so the lock lives in a process ginary knows nothing about
/// and lets go the moment this test closes the pipe. Killing the `flock`
/// process would not do: it forks, and the *grandchild* is the one holding the
/// inherited descriptor — which is the same inheritance the launcher relies on
/// and the reason this helper cannot take the shortcut.
#[derive(Debug)]
pub struct HeldLock {
    child: Child,
    stdin: Option<std::process::ChildStdin>,
    path: PathBuf,
}

impl HeldLock {
    /// Takes an exclusive lock on `<entry>/.lock` and waits until it is held.
    ///
    /// # Panics
    ///
    /// If the entry cannot be created, if `flock` cannot be spawned, or if the
    /// lock is not observably held within [`LOCK_BUDGET`] — all three are the
    /// test's own scaffolding failing rather than a claim about ginary.
    pub fn take(flock: &Path, entry: &Path) -> Self {
        std::fs::create_dir_all(entry)
            .unwrap_or_else(|error| panic!("cannot create {}: {error}", entry.display()));
        let path = lock_path(entry);
        let mut child = Command::new(flock)
            .arg("-x")
            .arg(&path)
            .args(["sh", "-c", "read line"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap_or_else(|error| panic!("cannot run {}: {error}", flock.display()));
        let stdin = child.stdin.take();
        let held = Self { child, stdin, path };
        assert!(
            wait_until_unlocked(flock, &held.path, false),
            "the test's own exclusive lock on {} was never taken",
            held.path.display()
        );
        held
    }

    /// The lock file the child holds.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Closes the pipe the holder is reading and waits until the lock is free.
    ///
    /// # Panics
    ///
    /// If the lock is still held after [`LOCK_BUDGET`].
    pub fn release(mut self, flock: &Path) {
        self.stop();
        assert!(
            wait_until_unlocked(flock, &self.path, true),
            "the test's own lock on {} outlived the process holding it",
            self.path.display()
        );
    }

    /// Closes the pipe and reaps the holder.
    fn stop(&mut self) {
        drop(self.stdin.take());
        let deadline = std::time::Instant::now() + LOCK_BUDGET;
        while std::time::Instant::now() < deadline {
            match self.child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) => std::thread::sleep(Duration::from_millis(10)),
                Err(_) => break,
            }
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for HeldLock {
    fn drop(&mut self) {
        self.stop();
    }
}
