// SPDX-License-Identifier: MIT OR Apache-2.0
//! `--stub` pointed at a fifo, and the error read "is not a regular file
//! rather than a file".
//!
//! **What went wrong.** C2 gave `StubError::NotAFile` a `found` field that is
//! documented as the tail of a sentence, and `Display` wrote
//! `"the stub {path} is {found} rather than a file"`. `describe_file_type`
//! answers with a noun phrase for a directory (`"a directory"`) but with a
//! negation for everything else (`"not a regular file"`), so the two halves
//! only compose for one of the two answers. A directory read
//! "is a directory rather than a file"; a fifo, a socket or a device node read
//! "is not a regular file rather than a file", which says the same thing twice
//! and contradicts itself doing so.
//!
//! **The input.** `--stub <a fifo>`, `--stub <a unix socket>` and
//! `--stub /dev/null`; `--stub <a directory>` is the case that already read
//! correctly and must keep reading that way.
//!
//! **The correct behaviour.** Every file type produces one grammatical
//! sentence. The `rather than a file` clause belongs to the answer that needs
//! it, so a directory still reads "is a directory rather than a file" and
//! everything else reads "is not a regular file", each followed by the same
//! remedy.

#![cfg(unix)]
#![cfg(feature = "cli")]

use std::path::{Path, PathBuf};

use ginary::stub::{self, StubError, StubOpts};
use ginary::target::Target;

/// A search whose only source is `explicit`, so the assertion is about that
/// candidate and nothing else.
fn opts_for(explicit: &Path, cache: &Path) -> StubOpts {
    StubOpts {
        explicit: Some(explicit.to_path_buf()),
        env_dir: None,
        cache_dir: cache.to_path_buf(),
    }
}

/// The message `--stub <path>` produces, with the failure named rather than
/// unwrapped so a passing branch is a test failure and not a panic.
fn message_for(path: &Path, cache: &Path) -> String {
    match stub::locate(&Target::host(), &opts_for(path, cache)) {
        Ok((found, _)) => panic!(
            "{} is not a stub, yet it was located at {found:?}",
            path.display()
        ),
        Err(error) => {
            assert!(
                matches!(&error, StubError::NotAFile { path: named, .. } if named == path),
                "expected StubError::NotAFile for {}, got {error:?}",
                path.display()
            );
            error.to_string()
        }
    }
}

/// The sentence has to be one sentence: it names what the path is, it does not
/// negate the same fact twice, and it still carries the remedy.
fn assert_one_sentence(message: &str, expected: &str) {
    assert!(
        !message.contains("not a regular file rather than a file"),
        "the two halves of the sentence contradict each other: {message}"
    );
    assert!(
        message.contains(expected),
        "expected {expected:?} in the message, got: {message}"
    );
    assert!(
        message.contains("`--stub` names the stub binary itself"),
        "the remedy survives the rewording: {message}"
    );
}

/// `mkfifo` through libc is a dependency this crate does not have, so the fifo
/// comes from the program every unix has. A machine without it is a skip that
/// says so rather than a silent pass.
fn make_fifo(path: &Path) -> bool {
    match std::process::Command::new("mkfifo").arg(path).status() {
        Ok(status) if status.success() => true,
        Ok(status) => panic!("mkfifo {} exited {status}", path.display()),
        Err(error) => {
            eprintln!("skipping: mkfifo is not usable here ({error})");
            false
        }
    }
}

#[test]
fn a_fifo_named_by_stub_reads_as_one_sentence() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let named = dir.path().join("fifo");
    if !make_fifo(&named) {
        return;
    }

    assert_one_sentence(&message_for(&named, dir.path()), "is not a regular file");
}

#[test]
fn a_socket_named_by_stub_reads_as_one_sentence() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let named = dir.path().join("socket");
    let listener = std::os::unix::net::UnixListener::bind(&named).expect("a unix socket");

    assert_one_sentence(&message_for(&named, dir.path()), "is not a regular file");

    drop(listener);
}

#[test]
fn a_device_node_named_by_stub_reads_as_one_sentence() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let named = PathBuf::from("/dev/null");
    if !named.exists() {
        eprintln!("skipping: /dev/null is not there");
        return;
    }

    assert_one_sentence(&message_for(&named, dir.path()), "is not a regular file");
}

#[test]
fn a_directory_named_by_stub_keeps_the_wording_it_already_had() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let named = dir.path().join("stubs");
    std::fs::create_dir_all(&named).expect("the directory the user pointed at");

    assert_one_sentence(
        &message_for(&named, dir.path()),
        "is a directory rather than a file",
    );
}
