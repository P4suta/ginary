// SPDX-License-Identifier: MIT OR Apache-2.0
//! A cache probe that ran out of time was reported as a `noexec` mount.
//!
//! **What went wrong.** `doctor`'s cache probe writes a tiny program into the
//! cache directory and runs it, because a packaged application execs its
//! runtime out of there and a `noexec` mount is a failure a user has to be
//! told about in words they can act on. The probe gives the child
//! [`ginary::doctor::PROBE_TIMEOUT`] — ten seconds — and a child that does not
//! finish in that time was folded into the same answer as one the kernel
//! refused to start:
//!
//! ```text
//! cache writable: yes
//! cache executable: no (mounted noexec?)
//! cache detail: `child 84206` did not exit within 10000ms
//! hint: set GINARY_CACHE_DIR to a directory this user can write to on a
//!       filesystem that is not mounted `noexec`
//! ```
//!
//! Every line of that is about a mount option, and the mount is not the
//! problem. Under `noexec` the exec fails *immediately*, with `EACCES`; a
//! program that started and did not return is the opposite observation. The
//! `detail` line was honest and the two lines around it sent the reader to
//! `mount(8)` for a machine that was merely busy.
//!
//! This is not hypothetical and it is not rare on one platform. macOS assesses
//! a newly written executable the first time it is exec'd, and the probe writes
//! a new one every time it runs; on a host whose `syspolicyd` is saturated that
//! first exec was measured at thirty to sixty seconds, so `ginary doctor`
//! reported a perfectly good `~/.cache/ginary` as `noexec` and told the user to
//! move it. `docs/dev/log/F1-macos-native.md` records the measurement.
//!
//! **The input.** A cache directory that is writable, and a probe child that
//! outlives its budget.
//!
//! **The correct behaviour.** A timeout is its own answer. The probe records
//! that it ran out of time, `cache executable:` says so rather than naming a
//! mount option, and the hint sends the reader at the machine's load rather
//! than at the mount table. Nothing else about the probe changes: a child the
//! kernel refuses to start is still `noexec`, and a directory nothing could be
//! written to still says that instead.
#![cfg(feature = "cli")]

use ginary::doctor::{CACHE_DIR_HINT, CacheProbe};

/// The probe as it comes back from a child that outlived its budget.
fn timed_out() -> CacheProbe {
    CacheProbe {
        writable: true,
        executable: false,
        timed_out: true,
        detail: Some("`child 84206` did not exit within 10000ms".to_owned()),
    }
}

/// The probe as it comes back from a `noexec` mount.
fn refused() -> CacheProbe {
    CacheProbe {
        writable: true,
        executable: false,
        timed_out: false,
        detail: Some("Permission denied (os error 13)".to_owned()),
    }
}

#[test]
fn a_probe_that_ran_out_of_time_does_not_blame_the_mount() {
    let rendered = timed_out().render();
    assert!(
        !rendered.contains("noexec"),
        "under `noexec` the exec fails at once; a child that started and did not return is the \
         opposite observation, and naming the mount sends the reader to `mount(8)` for a machine \
         that is busy:\n{rendered}"
    );
    assert!(
        !rendered.contains(CACHE_DIR_HINT),
        "and the hint that follows it is the same claim in a sentence:\n{rendered}"
    );
    assert!(
        rendered.contains("cache detail: `child 84206` did not exit within 10000ms"),
        "the observation itself is still reported:\n{rendered}"
    );
}

#[test]
fn a_probe_the_kernel_refused_still_names_the_mount() {
    let rendered = refused().render();
    assert!(
        rendered.contains("noexec"),
        "this is the failure the `noexec` wording exists for, and it keeps it:\n{rendered}"
    );
    assert!(
        rendered.contains(CACHE_DIR_HINT),
        "with the hint that tells the user what to do about it:\n{rendered}"
    );
}

#[test]
fn a_directory_nothing_could_be_written_to_is_neither() {
    let rendered = CacheProbe {
        writable: false,
        executable: false,
        timed_out: false,
        detail: Some("Read-only file system (os error 30)".to_owned()),
    }
    .render();
    assert!(
        !rendered.contains("noexec"),
        "nothing was written, so nothing was run, and the mount is not the subject:\n{rendered}"
    );
    assert!(
        rendered.contains("nothing could be written to run"),
        "the existing wording for that case is unchanged:\n{rendered}"
    );
}
