// SPDX-License-Identifier: MIT OR Apache-2.0
//! A staging directory that could not be removed was removed from the report
//! instead.
//!
//! **What went wrong.** `bundle::build_with_stub` ended with
//! `let _ = std::fs::remove_dir_all(&work);` under a comment promising that
//! "a successful [build] leaves only the artifact". On `EACCES`, `EBUSY` or a
//! partially removed tree that promise is false and nothing said so: the build
//! printed its `artifact:` line, exited 0, and left tens of megabytes of
//! staging in the project. `CLAUDE.md` is explicit that skipping is a reported
//! decision or an error, never a default.
//!
//! **The input.** A work directory that `remove_dir_all` cannot remove — one
//! whose parent denies write permission, and one whose path is not a directory
//! at all.
//!
//! **The correct behaviour.** The removal stays non-fatal and becomes visible:
//! `bundle::remove_work_dir` returns a warning naming the directory and what
//! the operating system said, and `build` carries it in
//! `BuildReport::warnings`, which `BuildReport::render_text` prints. The
//! rendering half is asserted in `tests/bundle.rs`, over a report whose
//! numbers a test can choose.

// A unix file: the input is a directory whose parent denies write permission,
// which is a mode bit and therefore a unix idea.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;

/// Whether this process is subject to directory permissions at all.
///
/// `root` is not, and a machine that runs the suite as `root` would see the
/// permission half of this file pass for the wrong reason. It is reported
/// rather than silently skipped.
fn permissions_are_enforced(dir: &Path) -> bool {
    let probe = dir.join("probe");
    std::fs::create_dir_all(&probe).expect("the probe directory");
    std::fs::set_permissions(&probe, std::fs::Permissions::from_mode(0o555))
        .expect("make the probe read-only");
    let enforced = std::fs::write(probe.join("x"), b"x").is_err();
    std::fs::set_permissions(&probe, std::fs::Permissions::from_mode(0o755))
        .expect("restore the probe");
    enforced
}

#[test]
fn a_work_directory_that_was_removed_is_reported_as_nothing_at_all() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let work = dir.path().join("build/ginary/.work-4242");
    std::fs::create_dir_all(work.join("root/lib")).expect("a staging tree");
    std::fs::write(work.join("root/lib/x.beam"), b"beam").expect("a staged file");

    let warning = ginary::bundle::remove_work_dir(&work);

    assert!(
        warning.is_none(),
        "a removal that worked has nothing to report: {warning:?}"
    );
    assert!(!work.exists(), "the work directory is gone");
}

#[test]
fn a_work_directory_that_cannot_be_removed_is_reported_rather_than_dropped() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    if !permissions_are_enforced(dir.path()) {
        println!(
            "skipping: this process ignores directory permissions, so an EACCES cannot be staged"
        );
        return;
    }

    let parent = dir.path().join("build/ginary");
    let work = parent.join(".work-4242");
    std::fs::create_dir_all(work.join("root")).expect("a staging tree");
    std::fs::write(work.join("root/x.beam"), b"beam").expect("a staged file");
    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o555))
        .expect("make the parent read-only");

    let warning = ginary::bundle::remove_work_dir(&work);

    // Restored first, so the temporary directory can be cleaned up whatever
    // the assertions below do.
    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o755))
        .expect("restore the parent");

    let warning = warning.expect("a removal that failed has to be reported");
    assert!(
        warning.contains(&work.display().to_string()),
        "the warning must name the directory that is still there: {warning}"
    );
    assert!(
        warning.to_lowercase().contains("permission"),
        "the warning must carry what the operating system said: {warning}"
    );
    assert!(
        work.exists(),
        "the directory is still there, which is the point"
    );
}

#[test]
fn a_work_directory_that_is_not_a_directory_is_reported_too() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let work = dir.path().join("build/ginary/.work-4242");
    std::fs::create_dir_all(work.parent().expect("a parent")).expect("build/ginary");
    std::fs::write(&work, b"not a staging tree").expect("a file where the directory goes");

    let warning =
        ginary::bundle::remove_work_dir(&work).expect("a removal that failed has to be reported");

    assert!(
        warning.contains(&work.display().to_string()),
        "the warning must name the path: {warning}"
    );
}
