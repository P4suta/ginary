// SPDX-License-Identifier: MIT OR Apache-2.0
//! The release gate guards both of its records with `-f`, which asks whether a
//! file is there and not whether it can be opened.
//!
//! **What went wrong.** E20 gave `scripts/ci/version-consistency.sh` the
//! symmetric guard the missing-`Cargo.toml` regression asked for, and wrote it
//! as an existence test:
//!
//! ```sh
//! if [ ! -f "$root/Cargo.toml" ]; then ... exit 2; fi
//! cargo_version=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$root/Cargo.toml" | head -1)
//! ```
//!
//! A file whose mode denies the reader passes `-f` and fails the read, so the
//! guard's whole point — that exit 2 is the script's own sentence naming the
//! record it could not read — is lost in exactly the case the guard was added
//! for. The manifest half is worse than a leaked message: its read is wrapped
//! in `|| true`, so an unreadable manifest becomes an empty
//! `manifest_version` and is reported as a manifest with *no `"."` entry* —
//! release-please's spelling of "a package I have never seen". That is a false
//! statement about a file the script never managed to look at.
//!
//! **The input.** A record whose permissions deny the process running the
//! check: a release job whose checkout was restored from a cache with other
//! ownership, a hand-run check inside a container whose uid does not match the
//! working tree, a tree on a mount that answers `EACCES`.
//!
//! **The correct behaviour.** Both guards test `-r` as well as `-f`, so a
//! record that cannot be read is exit 2 with the script's own sentence naming
//! that record — the same promise its header makes for one that is not there.

// A unix file: it spawns `scripts/ci/version-consistency.sh`, and the state
// under test is a unix file mode.
#![cfg(unix)]

use std::process::Command;

use crate::common::repo::root;
use crate::common::version::{MANIFEST_FILE, ROOT_VAR, VersionRoot, files_can_be_made_unreadable};

/// The version the fixture trees are built around.
const FIXTURE_VERSION: &str = "1.2.3";

/// Runs the check over `tree`, and answers `(code, stderr)`.
fn run_over(tree: &std::path::Path) -> (Option<i32>, String) {
    let output = Command::new(root().join("scripts/ci/version-consistency.sh"))
        .arg(format!("v{FIXTURE_VERSION}"))
        .current_dir(root())
        .env_remove("GITHUB_REF_NAME")
        .env(ROOT_VAR, tree)
        .output()
        .expect("spawn version-consistency.sh");
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// Whether this machine can hold a file shut against its own suite.
///
/// Root can read a file whatever its mode says, so the fixture cannot be built
/// there. Reported rather than passed silently: a skip that says nothing is a
/// test that has stopped running without anybody noticing.
fn can_build_the_fixture() -> bool {
    if files_can_be_made_unreadable() {
        return true;
    }
    eprintln!(
        "skipping: this suite is running as a user that can read a file with no permission bits \
         set, so an unreadable record cannot be built here"
    );
    false
}

#[test]
fn an_unreadable_cargo_toml_is_named_by_the_script_and_not_by_sed() {
    if !can_build_the_fixture() {
        return;
    }
    let tree = VersionRoot::released(FIXTURE_VERSION).unreadable("Cargo.toml");

    let (code, stderr) = run_over(tree.path());

    assert_eq!(
        code,
        Some(2),
        "a record that cannot be read is exit 2, which the script's header promises: {stderr}"
    );
    assert!(
        stderr.starts_with("version-consistency:"),
        "the failure has to be the script's own sentence. `sed`'s message is written in the \
         runner's locale and names neither which of the three records could not be read nor what \
         it is for — which is the whole of what the `-f` guard beside it was added to fix: \
         {stderr}"
    );
    assert!(
        stderr.contains("Cargo.toml"),
        "and it names the record it could not read: {stderr}"
    );
}

#[test]
fn an_unreadable_manifest_is_not_reported_as_one_with_no_entry() {
    if !can_build_the_fixture() {
        return;
    }
    let tree = VersionRoot::released(FIXTURE_VERSION).unreadable(MANIFEST_FILE);

    let (code, stderr) = run_over(tree.path());

    assert_eq!(
        code,
        Some(2),
        "a record that cannot be read is exit 2: {stderr}"
    );
    assert!(
        !stderr.contains("carries no"),
        "the manifest's read is wrapped in `|| true`, so a read that failed and a file with no \
         `\".\"` entry end in the same empty variable. Reporting the first as the second tells a \
         maintainer release-please has never seen this package, about a file the script never \
         opened: {stderr}"
    );
    assert!(
        stderr.contains(MANIFEST_FILE) && stderr.contains("read"),
        "it says which record could not be read, in the script's own voice: {stderr}"
    );
}
