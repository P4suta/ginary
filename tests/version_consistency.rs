// SPDX-License-Identifier: MIT OR Apache-2.0
//! One version, everywhere it is written down — and one honest record of what
//! has been released.
//!
//! ginary is version-locked to its stubs: every artifact of one release — the
//! command line tool, the seven stubs, the catalog tarballs — shares a version,
//! so a launcher never reads a payload a different build wrote. The single
//! source of that number is `Cargo.toml`, and a release tag has to match it or
//! the artifacts a workflow uploads under `v0.1.0` would carry `0.2.0`
//! internals. `scripts/ci/version-consistency.sh` is the check a release job
//! runs, and this file holds it to its contract.
//!
//! `.release-please-manifest.json` is **not** a second copy of `Cargo.toml`,
//! and E20 exists because this suite asserted that it was. release-please reads
//! it as *the last released version* and derives the next proposal from it, so
//! before the first release the two records legitimately differ: the manifest
//! records [`NOTHING_RELEASED`] and `Cargo.toml` carries the version being
//! prepared. The rule that is true in both states is the one asserted here —
//! the manifest is `0.0.0` while nothing has been released and equal to
//! `Cargo.toml` once something has — and the script is held to the same rule,
//! over fixture trees in every state rather than only over whichever state this
//! checkout is in.
//!
//! The script therefore proves three records agree rather than two, and the
//! fixture trees below drive it through the states this checkout is not in: a
//! manifest still at `0.0.0` while a tag is being cut, a manifest that has
//! moved on from `Cargo.toml`, a tag that agrees with neither. Each is a tree
//! built under [`ROOT_VAR`], which is the only seam by which the script can be
//! pointed at anything but its own repository.

// A unix file, for the reason `tests/coverage_gate.rs` is: it spawns
// `scripts/ci/version-consistency.sh` and asserts its execute bit.
#![cfg(unix)]

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::common::repo::{read, root};
use crate::common::version::{
    CONFIG_FILE, MANIFEST_FILE, NO_RELEASE_YET, NOTHING_RELEASED, RELEASE_DOC, ROOT_VAR,
    VersionRoot, cargo_version, last_released_version, manifest_version, nothing_has_been_released,
    package_setting,
};

/// The version the fixture trees are built around.
///
/// Deliberately not this repository's own version: a fixture sharing it would
/// pass whether or not the script honoured [`ROOT_VAR`], which is the seam
/// every test below depends on.
const FIXTURE_VERSION: &str = "1.2.3";

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

/// Runs the script over `tree` with `tag` as its argument.
///
/// Returns (code, stdout, stderr).
fn run_over(tree: &Path, tag: &str) -> (i32, String, String) {
    let output = Command::new(script())
        .arg(tag)
        .current_dir(root())
        .env_remove("GITHUB_REF_NAME")
        .env(ROOT_VAR, tree)
        .output()
        .expect("spawn version-consistency.sh");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

// ------------------------------------------------- the tag half, unchanged --

#[test]
fn a_tag_that_matches_cargo_toml_and_a_manifest_that_records_it_is_accepted() {
    let tree = VersionRoot::released(FIXTURE_VERSION);
    let (code, stdout, stderr) = run_over(tree.path(), &format!("v{FIXTURE_VERSION}"));
    assert_eq!(
        code, 0,
        "a tag of v{FIXTURE_VERSION} against a Cargo.toml of {FIXTURE_VERSION} and a manifest \
         recording {FIXTURE_VERSION} is the state a release is cut in: {stderr}"
    );
    assert!(
        stdout.contains(FIXTURE_VERSION),
        "the OK line names the version it agreed on: {stdout}"
    );
}

#[test]
fn a_bare_version_tag_without_the_v_prefix_is_accepted_too() {
    let tree = VersionRoot::released(FIXTURE_VERSION);
    let (code, _stdout, stderr) = run_over(tree.path(), FIXTURE_VERSION);
    assert_eq!(
        code, 0,
        "the leading `v` is optional; {FIXTURE_VERSION} matches {FIXTURE_VERSION}: {stderr}"
    );
}

#[test]
fn a_tag_that_disagrees_with_cargo_toml_is_refused_and_names_both() {
    let tree = VersionRoot::released(FIXTURE_VERSION);
    let (code, _stdout, stderr) = run_over(tree.path(), "v9.9.9");
    assert_ne!(
        code, 0,
        "a tag of v9.9.9 does not match Cargo.toml {FIXTURE_VERSION}, so the check fails"
    );
    assert!(
        stderr.contains("9.9.9") && stderr.contains(FIXTURE_VERSION),
        "the failure names the tag and the Cargo.toml version, so a maintainer sees the drift: \
         {stderr}"
    );
}

// ------------------------------------------------------- the manifest half --

#[test]
fn a_tag_cut_while_the_manifest_says_nothing_has_been_released_is_refused() {
    // The state this repository is in today. A tag here would mean the release
    // pull request never ran: release-please writes the version into the
    // manifest and creates the tag in that order, so a tag against a `0.0.0`
    // manifest is a hand-cut one, which `docs/RELEASE.md` forbids.
    let tree = VersionRoot::never_released(FIXTURE_VERSION);
    let (code, _stdout, stderr) = run_over(tree.path(), &format!("v{FIXTURE_VERSION}"));
    assert_eq!(
        code, 1,
        "the tag matches Cargo.toml, but the manifest records no release at all; that is a \
         refusal, not an agreement: {stderr}"
    );
    assert!(
        stderr.contains(NOTHING_RELEASED) && stderr.contains(MANIFEST_FILE),
        "the refusal names the manifest and the {NOTHING_RELEASED} it records, so a maintainer \
         reads which of the three records is behind: {stderr}"
    );
}

#[test]
fn a_manifest_that_disagrees_with_cargo_toml_is_refused_and_names_both() {
    // The drift the original check existed to catch, stated where it is real:
    // release-please bumps both files in one commit, so two different non-zero
    // versions mean somebody edited one of them by hand.
    let tree = VersionRoot::new(FIXTURE_VERSION, "1.2.2");
    let (code, _stdout, stderr) = run_over(tree.path(), &format!("v{FIXTURE_VERSION}"));
    assert_eq!(
        code, 1,
        "Cargo.toml is {FIXTURE_VERSION} and the manifest records 1.2.2; the tag agreeing with \
         one of them does not make the pair consistent: {stderr}"
    );
    assert!(
        stderr.contains("1.2.2") && stderr.contains(FIXTURE_VERSION),
        "the failure names both versions: {stderr}"
    );
}

#[test]
fn the_three_refusals_read_as_one_account() {
    // One snapshot over the three failures, because the value of this check is
    // the sentence a maintainer reads at three in the morning with a half-built
    // release behind them.
    let mut rendered = String::new();
    for (label, tree, tag) in [
        (
            "a tag that disagrees with Cargo.toml",
            VersionRoot::released(FIXTURE_VERSION),
            format!("v{FIXTURE_VERSION}-wrong"),
        ),
        (
            "a tag cut before anything was released",
            VersionRoot::never_released(FIXTURE_VERSION),
            format!("v{FIXTURE_VERSION}"),
        ),
        (
            "a manifest that disagrees with Cargo.toml",
            VersionRoot::new(FIXTURE_VERSION, "1.2.2"),
            format!("v{FIXTURE_VERSION}"),
        ),
    ] {
        let (code, _stdout, stderr) = run_over(tree.path(), &tag);
        rendered.push_str(&format!("{label}\nexit {code}\n{}\n", stderr.trim_end()));
    }
    insta::assert_snapshot!("version_refusals", rendered);
}

// ---------------------------------------------- the records, as committed --

#[test]
fn the_manifest_records_what_has_been_released_and_nothing_has() {
    // The invariant that is true in both states, and the one E20 replaces the
    // old `manifest == Cargo.toml` assertion with. `docs/RELEASE.md` carries
    // the tree's answer to "has anything been released", because no committed
    // file can derive it: a tag and a GitHub release live on the server, and a
    // shallow CI checkout fetches no tags.
    let manifest = manifest_version();
    let cargo = cargo_version();
    // `last_released_version` is the same question asked the way the rest of
    // the suite should ask it: `None` while nothing has been released.
    let released = last_released_version();
    if nothing_has_been_released() {
        assert_eq!(
            released, None,
            "{RELEASE_DOC} says `{NO_RELEASE_YET}`, so there is no last released version; \
             {MANIFEST_FILE} records {manifest}"
        );
        assert_eq!(
            manifest, NOTHING_RELEASED,
            "{RELEASE_DOC} says `{NO_RELEASE_YET}`, and `git tag` and `gh release list` are both \
             empty (docs/dev/log/E20.md, section 3). release-please reads {MANIFEST_FILE} as the \
             last released version and derives the next proposal from it, so a manifest of \
             {manifest} for a repository that has released nothing is what made it propose a \
             *second* release. While nothing has been released the manifest records \
             {NOTHING_RELEASED}; Cargo.toml keeps {cargo}, the version being prepared"
        );
    } else {
        assert_eq!(
            released.as_deref(),
            Some(cargo.as_str()),
            "{RELEASE_DOC} no longer says `{NO_RELEASE_YET}`, so a release has been cut. \
             release-please writes {MANIFEST_FILE} and Cargo.toml in one commit, so from the \
             first release onward the two records hold one version between them; the manifest \
             records {manifest}"
        );
    }
}

#[test]
fn the_release_document_says_what_the_manifest_holds() {
    // The prose the old assertion was written from. `mirrors it` is the claim
    // that produced a manifest asserting a release nobody made; what the file
    // holds is the *last released* version, which before the first release is
    // no version at all.
    let release = read(RELEASE_DOC);
    assert!(
        release.contains("last released version"),
        "{RELEASE_DOC} has to say that {MANIFEST_FILE} records the last released version, not \
         that it mirrors Cargo.toml: mirroring is what asserted a release that never happened"
    );
    assert!(
        release.contains(NOTHING_RELEASED),
        "{RELEASE_DOC} has to name {NOTHING_RELEASED} as the value that records `nothing has been \
         released`, so the maintainer who reads the manifest knows what it is saying"
    );
    // And it has to say so everywhere it describes the check, not only where
    // the manifest is introduced. The three steps carried the old, two-record
    // sentence for a while after the section above was rewritten, so the
    // document contradicted itself about what a release gate proves.
    assert!(
        !release.contains("proves the tag equals"),
        "{RELEASE_DOC} still describes the version check as a comparison of two records. It \
         proves three: the tag, Cargo.toml and {MANIFEST_FILE}"
    );
    assert!(
        release
            .lines()
            .any(|line| line.contains("version-consistency.sh") && line.contains(MANIFEST_FILE)),
        "where {RELEASE_DOC} lists what the release steps prove, the line naming \
         version-consistency.sh has to name {MANIFEST_FILE} among the records it reads"
    );
}

#[test]
fn the_release_please_configuration_names_the_first_version_this_repository_will_release() {
    // Two independent reasons the first proposal is 0.1.0, and both are pinned
    // because either one moving would change the answer in silence.
    //
    // `release-type: rust` is the first: `Rust.initialReleaseVersion()` in
    // release-please returns `Version.parse('0.1.0')`, and it is what runs when
    // there is no latest release to bump. `initial-version` is the second: the
    // config schema carries it (`"Releases the initial library with a specified
    // version"`), and the base strategy honours it. The rust strategy currently
    // overrides the method without consulting it, so the key does not decide
    // today's answer — it states the intent in the file release-please reads,
    // and it agrees with what the strategy does, so neither can drift into
    // proposing a different first version alone. See docs/dev/log/E20.md for
    // the quoted source of both.
    assert_eq!(
        package_setting("release-type").and_then(|value| value.as_str().map(str::to_owned)),
        Some("rust".to_owned()),
        "{CONFIG_FILE} has to keep `release-type: rust`: it is what makes the first proposal \
         0.1.0 rather than the base strategy's 1.0.0"
    );
    assert_eq!(
        package_setting("initial-version").and_then(|value| value.as_str().map(str::to_owned)),
        Some("0.1.0".to_owned()),
        "{CONFIG_FILE} has to state the first version explicitly. Without it the number lives \
         only inside release-please's rust strategy, and a repository whose manifest records \
         {NOTHING_RELEASED} would take whatever that strategy defaults to next"
    );
}

// ------------------------------------------------------------- the workflow --

#[test]
fn a_workflow_runs_the_version_consistency_check() {
    // A check nothing runs is a check nobody passes. The distribute workflow is
    // where the tag exists, so that is where the script belongs. That no
    // workflow sets `ROOT_VAR`, which would point the gate at another tree, is
    // pinned by
    // `tests/regressions/e20_a_workflow_could_point_the_version_check_at_another_tree.rs`.
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
