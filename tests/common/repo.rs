// SPDX-License-Identifier: MIT OR Apache-2.0
//! Reading committed repository files, for the tests that hold the CI, the
//! release workflows and the v1 documentation against the tree.
//!
//! None of the E1 product is code the suite can execute: a workflow runs only
//! on GitHub, and a document is prose. What every one of those artifacts shares
//! is that it can rot silently, and a claim nobody checks reads as evidence.
//! These helpers are the same shape [`tests/formal.rs`](../formal.rs) and
//! [`tests/smoke_matrix.rs`](../smoke_matrix.rs) grew their own copies of; this
//! module is the one place the E1 targets share them from.

use std::path::PathBuf;

/// The repository root, the directory holding `Cargo.toml`.
pub fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Reads a repository file as text.
///
/// # Panics
///
/// If the file is not there. For the E1 targets that *is* the assertion: a
/// workflow or a document the milestone promised and did not write is a failed
/// test, named by the path it was looked for at.
pub fn read(relative: &str) -> String {
    let path = root().join(relative);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()))
}

/// Reads a repository file as text, or `None` when it is not there.
///
/// Unlike [`read`], a missing file is not a panic here, so a test can assert
/// on its absence or make its own message.
pub fn read_opt(relative: &str) -> Option<String> {
    std::fs::read_to_string(root().join(relative)).ok()
}

/// Whether a repository path exists at all, file or directory.
pub fn exists(relative: &str) -> bool {
    root().join(relative).exists()
}
