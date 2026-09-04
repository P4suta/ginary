// SPDX-License-Identifier: MIT OR Apache-2.0
//! The cache sweep decided a process was dead by looking for `/proc/<pid>`,
//! on two platforms that have no `/proc`.
//!
//! **What went wrong.** `cache::sweep` removes a `.<key>.tmp-<pid>` tree when
//! the process that owns it has gone, and leaves it alone when that process is
//! still extracting into it. The whole decision is one line:
//!
//! ```rust,ignore
//! fn is_alive(pid: u32) -> bool {
//!     Path::new("/proc").join(pid.to_string()).exists()
//! }
//! ```
//!
//! `/proc` is a Linux filesystem. Windows has no such directory and macOS has
//! not carried one since 10.5, so on both of them the answer is `false` for
//! every process that has ever run, and a launcher sweeping the cache deletes
//! the tree another launcher is at that moment extracting into. The Windows
//! runner is where it surfaced:
//!
//! ```text
//! ---- a_live_process_s_temporary_tree_is_kept ----
//! assertion `left == right` failed: another launcher's extraction in progress
//! must be left alone
//!   left: []
//!  right: ["C:\\Users\\RUNNER~1\\AppData\\Local\\Temp\\.tmpDIwQIo\\hello\\.abc.tmp-2300"]
//! ```
//!
//! (`Windows build and exit-code propagation`
//! <https://github.com/P4suta/ginary/actions/runs/33823103540/job/100869848230>,
//! `tests/cache.rs:434`.) This is not a spelling: it is two concurrent
//! launchers on the same machine, one of which loses the tree it is unpacking
//! into, and it is on the launcher path where there is nothing to catch it.
//!
//! **The input.** Any host that is not Linux. The failure needs no unusual
//! timing — every sweep on such a host answers "dead" for every owner.
//!
//! **The correct behaviour.** Liveness is a question for the operating
//! system's process table, not for its filesystem namespace. This test states
//! the half a Linux machine can hold: no code on the launcher path decides it
//! by naming `/proc`, because the file `cache::sweep` is asked to protect is
//! on all three platforms and the rule that protects it may not be true on one
//! of them. The behavioural half is `tests/cache.rs`'s own
//! `a_live_process_s_temporary_tree_is_kept`, which runs on each of them.

#![cfg(feature = "cli")]

use crate::common::repo::read;
use crate::common::srcscan::literal_sites;

/// The directory that is not a process table.
const PROC: &str = "/proc";

#[test]
fn the_scanner_reads_code_and_not_the_prose_that_describes_it() {
    // The instrument, calibrated the way
    // `e7_the_xdg_rule_used_the_hosts_idea_of_an_absolute_path` calibrates its
    // own: a rule has to be describable in the file it governs.
    let planted = "\
// A tree is removed when `/proc/<pid>` does not exist.\n\
    Path::new(\"/proc\").join(pid.to_string()).exists()\n\
    process_is_alive(pid)\n";

    assert_eq!(
        literal_sites(planted, PROC),
        vec![2],
        "a comment may name the Linux filesystem; the decision may not use it"
    );
}

#[test]
fn the_sweeps_liveness_rule_does_not_name_a_linux_filesystem() {
    assert_eq!(
        literal_sites(&read("src/cache.rs"), PROC),
        Vec::<usize>::new(),
        "`cache::sweep` protects an extraction in progress on all three platforms, and `/proc` \
         exists on one of them: a rule written this way answers `dead` for every live launcher \
         on Windows and on macOS, and the tree that launcher is unpacking into is deleted \
         underneath it"
    );
}
