// SPDX-License-Identifier: MIT OR Apache-2.0
//! The release gate read `.release-please-manifest.json` with a line-anchored
//! `sed`, so a manifest written on one line was a manifest with no `"."` entry
//! at all — and the gate said so, in a sentence that was the opposite of the
//! truth.
//!
//! **What went wrong.** `scripts/ci/version-consistency.sh` read the manifest
//! with
//!
//! ```sh
//! manifest_version=$(sed -n 's/^[[:space:]]*"\.":[[:space:]]*"\([^"]*\)".*$/\1/p' ...)
//! ```
//!
//! anchored on `^` plus optional whitespace, so only the pretty-printed shape
//! matched. Against `{".": "1.2.3"}` the gate exited 2 with
//!
//! ```text
//! version-consistency: .release-please-manifest.json carries no "." entry;
//! release-please would read this package as one it has never seen
//! ```
//!
//! while `tests/common/version.rs`, which parses the same bytes with
//! `serde_json`, read `1.2.3`. Two readers of one record, disagreeing about
//! what a valid record is.
//!
//! **The input.** Any manifest that is not multi-line: `jq -c`, a `prettier`
//! pass, `JSON.stringify` without an indent argument, or a hand edit. JSON is
//! not a line-oriented format and nothing in the repository says the manifest
//! has to be indented.
//!
//! **The correct behaviour.** The shell reader answers what the JSON says, so
//! the script and the Rust helper hold one definition of a valid manifest. A
//! compact manifest recording the version being released passes; a compact
//! manifest recording `0.0.0` fails for the reason it records nothing — not
//! for being compact.

// A unix file: it spawns `scripts/ci/version-consistency.sh`.
#![cfg(unix)]

use std::path::Path;
use std::process::Command;

use crate::common::repo::root;
use crate::common::version::{
    MANIFEST_FILE, NOTHING_RELEASED, ROOT_VAR, VersionRoot, manifest_version_in,
};

/// The version the fixture trees are built around, deliberately not this
/// repository's own.
const FIXTURE_VERSION: &str = "1.2.3";

/// Runs the version check over `tree`, returning (code, stderr).
fn run_over(tree: &Path, tag: &str) -> (i32, String) {
    let output = Command::new(root().join("scripts/ci/version-consistency.sh"))
        .arg(tag)
        .current_dir(root())
        .env_remove("GITHUB_REF_NAME")
        .env(ROOT_VAR, tree)
        .output()
        .expect("spawn version-consistency.sh");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn a_one_line_manifest_recording_the_release_is_accepted() {
    let tree = VersionRoot::compact(FIXTURE_VERSION, FIXTURE_VERSION);
    assert_eq!(
        manifest_version_in(tree.path()),
        FIXTURE_VERSION,
        "the fixture is the state under test: a compact manifest that records {FIXTURE_VERSION}"
    );
    let (code, stderr) = run_over(tree.path(), &format!("v{FIXTURE_VERSION}"));
    assert_eq!(
        code, 0,
        "the tag, Cargo.toml and the manifest all name {FIXTURE_VERSION}; the manifest being \
         written on one line is not a disagreement about the version, and a release gate that \
         reads it as one turns a correct release into a red gate: {stderr}"
    );
}

#[test]
fn a_one_line_manifest_that_records_nothing_fails_for_that_reason() {
    let tree = VersionRoot::compact(FIXTURE_VERSION, NOTHING_RELEASED);
    let (code, stderr) = run_over(tree.path(), &format!("v{FIXTURE_VERSION}"));
    assert_eq!(
        code, 1,
        "a manifest recording {NOTHING_RELEASED} is a record that reads fine and says no release \
         has been made: that is exit 1, the three records disagreeing, not exit 2, a record that \
         cannot be read: {stderr}"
    );
    assert!(
        stderr.contains(NOTHING_RELEASED) && !stderr.contains("carries no"),
        "the refusal names the {NOTHING_RELEASED} the manifest records. Reporting `carries no \
         \".\" entry` against a manifest that plainly has one sends a maintainer to fix a file \
         that is not broken: {stderr}"
    );
    assert!(
        stderr.contains(MANIFEST_FILE),
        "the refusal names the file it read: {stderr}"
    );
}
