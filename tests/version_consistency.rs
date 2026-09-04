// SPDX-License-Identifier: MIT OR Apache-2.0
//! One version, everywhere it is written down.
//!
//! ginary is version-locked to its stubs: every artifact of one release — the
//! command line tool, the seven stubs, the catalog tarballs — shares a version,
//! so a launcher never reads a payload a different build wrote. The single
//! source of that number is `Cargo.toml`; `.release-please-manifest.json`
//! mirrors it, and a release tag has to match it or the artifacts a workflow
//! uploads under `v0.1.0` would carry `0.2.0` internals. `scripts/ci/
//! version-consistency.sh` is the check a release job runs, and this file holds
//! it to its contract against the committed `Cargo.toml` — a matching tag
//! accepted, a mismatched one refused and named — and pins that a workflow
//! actually runs it.
//!
//! The script does not exist yet: this is the milestone that writes it. Until
//! it does, every test here fails at the assertion that looks for it.

// A unix file, for the reason `tests/coverage_gate.rs` is: it spawns
// `scripts/ci/version-consistency.sh` and asserts its execute bit.
#![cfg(unix)]

mod common;

use std::path::PathBuf;
use std::process::Command;

use crate::common::repo::{read, root};

/// The version-consistency script, asserted present so a missing one is a named
/// failure rather than a spawn error.
fn script() -> PathBuf {
    let path = root().join("scripts/ci/version-consistency.sh");
    assert!(
        path.is_file(),
        "scripts/ci/version-consistency.sh is what a release job runs to prove the tag matches \
         Cargo.toml; it is not committed"
    );
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(&path)
        .expect("script metadata")
        .permissions()
        .mode();
    assert_eq!(
        mode & 0o111,
        0o111,
        "a script a workflow runs directly has to be executable"
    );
    path
}

/// Runs the script with `tag` as its argument, returning (code, stdout, stderr).
fn run(tag: &str) -> (i32, String, String) {
    let output = Command::new(script())
        .arg(tag)
        .current_dir(root())
        .env_remove("GITHUB_REF_NAME")
        .output()
        .expect("spawn version-consistency.sh");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// The version `Cargo.toml` actually carries, so the fixtures track the tree.
fn cargo_version() -> String {
    read("Cargo.toml")
        .lines()
        .find_map(|l| l.strip_prefix("version = \""))
        .and_then(|rest| rest.strip_suffix('"'))
        .expect("Cargo.toml has a version")
        .to_owned()
}

#[test]
fn a_tag_that_matches_cargo_toml_is_accepted() {
    let version = cargo_version();
    let (code, stdout, _stderr) = run(&format!("v{version}"));
    assert_eq!(code, 0, "a tag of v{version} matches Cargo.toml {version}");
    assert!(
        stdout.contains(&version),
        "the OK line names the version it agreed on: {stdout}"
    );
}

#[test]
fn a_bare_version_tag_without_the_v_prefix_is_accepted_too() {
    let version = cargo_version();
    let (code, _stdout, stderr) = run(&version);
    assert_eq!(
        code, 0,
        "the leading `v` is optional; {version} matches {version}: {stderr}"
    );
}

#[test]
fn a_tag_that_disagrees_with_cargo_toml_is_refused_and_names_both() {
    let version = cargo_version();
    let (code, _stdout, stderr) = run("v9.9.9");
    assert_ne!(
        code, 0,
        "a tag of v9.9.9 does not match Cargo.toml {version}, so the check fails"
    );
    assert!(
        stderr.contains("9.9.9") && stderr.contains(&version),
        "the failure names the tag and the Cargo.toml version, so a maintainer sees the drift: \
         {stderr}"
    );
}

#[test]
fn cargo_toml_and_the_release_please_manifest_already_agree() {
    // The half of the check that holds with no tag at all: the two committed
    // records of the version must never drift, because release-please bumps
    // one and a human edits the other.
    let version = cargo_version();
    let manifest = read(".release-please-manifest.json");
    assert!(
        manifest.contains(&format!("\"{version}\"")),
        ".release-please-manifest.json must carry the same version as Cargo.toml ({version}): \
         {manifest}"
    );
}

#[test]
fn a_workflow_runs_the_version_consistency_check() {
    // A check nothing runs is a check nobody passes. The distribute workflow is
    // where the tag exists, so that is where the script belongs.
    let distribute = root().join(".github/workflows/distribute.yml");
    assert!(
        distribute.is_file(),
        "the distribute workflow, which the version check runs inside, is not committed"
    );
    let text = std::fs::read_to_string(&distribute).expect("read distribute.yml");
    assert!(
        text.contains("version-consistency.sh"),
        "distribute.yml has to run scripts/ci/version-consistency.sh before it uploads anything:\n\
         {text}"
    );
}
