// SPDX-License-Identifier: MIT OR Apache-2.0
//! `--stub` pointed at a directory, and the build said the file was not there.
//!
//! **What went wrong.** `stub::locate` asked `Path::is_file`, which answers
//! `false` for everything that is not a regular file *and* for every `stat`
//! that fails — a directory, a dangling symlink, a path whose parent the user
//! cannot search. All of them became `StubError::Missing`, whose sentence is
//! "the stub <path> is not there". For a path that is very much there that is
//! a false statement, and it sends the reader to check a spelling that is
//! correct instead of at the thing that is wrong.
//!
//! **The input.** `--stub <a directory>`, and `--stub <a file inside a
//! directory with mode 000>`.
//!
//! **The correct behaviour.** A path that exists and is not a regular file
//! says so; a path that cannot be looked at at all reports what the operating
//! system said. Only a genuine `ENOENT` keeps the "is not there" sentence.
//! `--stub` is still an instruction rather than a hint in every case: nothing
//! falls back to another source.

#![cfg(unix)]
#![cfg(feature = "cli")]

use std::path::Path;

use assert_cmd::Command;
use ginary::stub::{self, StubError, StubOpts};
use ginary::target::Target;

use crate::common::project::TempProject;

/// A search whose only source is `explicit`, so the assertion is about that
/// candidate and nothing else.
fn opts_for(explicit: &Path, cache: &Path) -> StubOpts {
    StubOpts {
        explicit: Some(explicit.to_path_buf()),
        env_dir: None,
        cache_dir: cache.to_path_buf(),
    }
}

#[test]
fn a_directory_named_by_stub_is_not_reported_as_a_missing_file() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let named = dir.path().join("stubs");
    std::fs::create_dir_all(&named).expect("the directory the user pointed at");

    let error = stub::locate(&Target::host(), &opts_for(&named, dir.path()))
        .expect_err("a directory is not a stub");

    assert!(
        matches!(&error, StubError::NotAFile { path, .. } if *path == named),
        "expected StubError::NotAFile, got {error:?}"
    );
    let message = error.to_string();
    assert!(
        message.contains("a directory") && !message.contains("is not there"),
        "a path that is there is not reported as absent: {message}"
    );
}

#[test]
fn a_stub_that_cannot_be_looked_at_reports_what_the_system_said() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let locked = dir.path().join("locked");
    std::fs::create_dir_all(&locked).expect("the directory that will be closed");
    let named = locked.join("stub");
    std::fs::write(&named, b"anything").expect("the file inside it");
    // Mode 000 on the parent: the file is there and `stat` on it cannot say so.
    // Restored below, because a `TempDir` cannot delete what it cannot search.
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000))
        .expect("the parent is closed");

    let error = stub::locate(&Target::host(), &opts_for(&named, dir.path()))
        .expect_err("an unreadable path is not a stub");

    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755))
        .expect("the parent is reopened");

    assert!(
        matches!(&error, StubError::Io { .. }),
        "expected StubError::Io, got {error:?}"
    );
    let message = error.to_string();
    assert!(
        message.contains(&named.display().to_string()) && !message.contains("is not there"),
        "a permission problem names itself rather than claiming the file is absent: {message}"
    );
}

#[test]
fn a_missing_path_still_says_it_is_not_there() {
    // The other half of the same branch: only a genuine `ENOENT` keeps the
    // sentence, and a regression that turned every failure into an I/O error
    // would be the same defect facing the other way.
    let dir = tempfile::tempdir().expect("a temporary directory");
    let named = dir.path().join("no-such-stub");

    let error = stub::locate(&Target::host(), &opts_for(&named, dir.path()))
        .expect_err("an absent path is not a stub");

    assert!(
        matches!(&error, StubError::Missing { path } if *path == named),
        "expected StubError::Missing, got {error:?}"
    );
}

#[test]
fn the_build_command_says_which_of_the_two_it_is() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let named = dir.path().join("stubs");
    std::fs::create_dir_all(&named).expect("the directory the user pointed at");
    let project = TempProject::named("hello");

    let assert = Command::cargo_bin("ginary")
        .expect("the `ginary` binary is built for tests")
        .current_dir(project.root())
        .env(stub::STUB_DIR_VAR, dir.path().join("empty"))
        .env("GINARY_CACHE_DIR", dir.path().join("cache"))
        .args([
            "build",
            "--skip-export",
            "--stub",
            &named.display().to_string(),
        ])
        .assert()
        .failure();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf-8");

    assert!(
        stderr.contains("a directory") && !stderr.contains("is not there"),
        "the message a user reads names what is wrong with the path they typed: {stderr}"
    );
}
