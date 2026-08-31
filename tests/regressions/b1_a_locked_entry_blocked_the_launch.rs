// SPDX-License-Identifier: MIT OR Apache-2.0
//! A foreign exclusive lock on a cache entry stopped the application from
//! starting at all.
//!
//! **What went wrong.** `cache_lock::SharedLock::acquire` took a *blocking*
//! `LOCK_SH`. Its own documentation said a caller on the launcher path "treats
//! every one of them as `no lock` rather than as a reason not to start", but a
//! block is not an error: any process holding `<entry>/.lock` exclusively —
//! another `ginary cache prune`, a stray `flock -x`, an operator's shell — made
//! the packaged application hang for as long as that process lived, with no
//! output, no exit code and no trace record. The one failure mode the design
//! forbids was the one it had.
//!
//! **The input.** A warm cache entry with an exclusive `flock` held on its
//! `.lock` by a process that is not going away.
//!
//! **The correct behaviour.** The lock is attempted without blocking, briefly
//! retried, and then given up on: the run is recorded as unlocked and the
//! application starts. A lock that cannot be taken is a pruning risk, not a
//! reason to refuse to run.

use std::time::{Duration, Instant};

use crate::common::artifact::{STUB_EXIT, SyntheticArtifact};
use crate::common::cachefs::{HeldLock, lock_path};
use crate::common::tools::require_tools;

/// How long a launch may take while somebody else holds the entry.
///
/// Generous next to the few hundred milliseconds the launcher spends trying,
/// and far short of the forever it used to take.
const LAUNCH_BUDGET: Duration = Duration::from_secs(20);

#[test]
fn a_foreign_exclusive_lock_does_not_stop_the_application_starting() {
    let Some(tools) = require_tools(&["flock"]) else {
        return;
    };
    let dir = tempfile::tempdir().expect("a temporary directory");
    let artifact = SyntheticArtifact::build(dir.path());

    // Warm the cache first, so the entry the lock is taken on is the complete
    // one the second run will hit rather than a directory extraction would
    // move aside.
    let first = artifact.run().output();
    assert_eq!(first.code(), STUB_EXIT, "{}", first.stderr_text());

    let held = HeldLock::take(tools.path("flock"), &artifact.key_dir());
    assert_eq!(
        held.path(),
        lock_path(&artifact.key_dir()),
        "the test holds the same file the launcher locks"
    );

    let mut child = artifact.run().spawn();
    let deadline = Instant::now() + LAUNCH_BUDGET;
    let status = loop {
        match child.try_wait().expect("wait for the artifact") {
            Some(status) => break Some(status),
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    };

    let status = status.unwrap_or_else(|| {
        panic!(
            "the application did not start within {LAUNCH_BUDGET:?} while another process held \
             {}: a lock that cannot be taken must not be a reason to refuse to run",
            held.path().display()
        )
    });
    assert_eq!(
        status.code(),
        Some(STUB_EXIT),
        "the run must reach the runtime and mirror its exit code"
    );

    held.release(tools.path("flock"));
}
