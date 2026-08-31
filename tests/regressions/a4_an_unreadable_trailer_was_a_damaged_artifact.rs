// SPDX-License-Identifier: MIT OR Apache-2.0
//! Reading the last 64 bytes could fail for an ordinary IO reason, and both
//! callers reported that as a statement about the artifact's contents.
//!
//! **What went wrong.** `Trailer::read_from` returns `Err(TrailerError)` both
//! for bytes that are a malformed trailer and for a file it could not read at
//! all: `TrailerError::Io` wraps whatever `metadata` or `read_exact_at` said.
//! `bundle::check_stub` folded every `Err` into `BundleError::BundledStub`,
//! whose remedy is "install plain ginary", and `inspect::open` folded every
//! `Err` into `InspectError::Trailer`, whose headline is "the ginary trailer
//! is damaged". Neither is true of a file the process could not read.
//!
//! **The input.** A directory. `File::open` on a directory succeeds on Linux
//! and `read_exact_at` then fails with `EISDIR`, which is the cheapest
//! reproduction of the class; an `EIO` on a real stub takes the same branch.
//!
//! **The correct behaviour.** An IO failure is an IO failure. `check_stub`
//! reports `BundleError::Io` naming the stub, `inspect::open` reports
//! `InspectError::Io` naming the file, and both keep their own variants for a
//! trailer whose bytes are actually wrong.

use std::path::{Path, PathBuf};

use ginary::bundle::{self, BundleError};
use ginary::inspect::{self, InspectError};

/// A directory whose `st_size` is at least [`ginary::trailer::TRAILER_LEN`].
///
/// A directory shorter than the trailer cannot hold one, and both callers
/// answer "no trailer" for it without reading anything — which is a different
/// branch from the one this file is about. `tmpfs` reports a small size for an
/// empty directory, so the directory is given entries until it is long enough.
fn readable_directory(dir: &Path) -> PathBuf {
    let path = dir.join("a-directory");
    std::fs::create_dir_all(&path).expect("the directory");
    for index in 0..32 {
        std::fs::write(path.join(format!("entry-{index:030}")), b"x").expect("an entry");
    }
    let len = std::fs::metadata(&path).expect("stat the directory").len();
    assert!(
        len >= ginary::trailer::TRAILER_LEN,
        "this test needs a directory at least {} bytes long, and this one is {len}",
        ginary::trailer::TRAILER_LEN
    );
    path
}

#[test]
fn a_stub_that_cannot_be_read_is_an_io_error_and_not_the_bundled_refusal() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let unreadable = readable_directory(dir.path());

    let error = bundle::check_stub(&unreadable).expect_err("a directory is not a stub");

    match &error {
        BundleError::Io { what, .. } => assert!(
            what.contains(&unreadable.display().to_string()),
            "the message must name the file: {what}"
        ),
        other => panic!("expected BundleError::Io, got {other:?}"),
    }
    assert!(
        !error.to_string().contains("install plain ginary"),
        "a file that could not be read is not a packaged application: {error}"
    );
}

#[test]
fn an_artifact_that_cannot_be_read_is_an_io_error_and_not_a_damaged_trailer() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let unreadable = readable_directory(dir.path());

    let error = inspect::open(&unreadable).expect_err("a directory is not an artifact");

    match &error {
        InspectError::Io { path, .. } => assert_eq!(path, &unreadable),
        other => panic!("expected InspectError::Io, got {other:?}"),
    }
    assert!(
        !error.to_string().contains("damaged"),
        "a file that could not be read is not a damaged artifact: {error}"
    );
}
