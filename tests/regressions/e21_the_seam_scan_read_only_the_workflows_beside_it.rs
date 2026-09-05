// SPDX-License-Identifier: MIT OR Apache-2.0
//! The rule that no workflow may point the release gate at another tree was
//! stated over a file list that leaves out half of what CI executes.
//!
//! **What went wrong.** E20's
//! `e20_a_workflow_could_point_the_version_check_at_another_tree.rs` scanned
//! for `GINARY_VERSION_ROOT` with a `read_dir` of its own:
//!
//! ```text
//! for entry in std::fs::read_dir(root().join(".github/workflows")) { .. }
//! if path.extension() != Some("yml") { continue; }
//! ```
//!
//! Two holes, both in the direction that passes. GitHub accepts `.yaml` as
//! readily as `.yml`, so renaming a workflow switches the rule off for it; and
//! a *composite action* is not a document beside the workflows — its steps run
//! in the caller's job, on the caller's runner, with the caller's environment,
//! so `.github/actions/install-actionlint/action.yml` can export the variable
//! and every workflow that uses it inherits a release gate pointed at another
//! directory. `crate::common::repo::yaml_files_under` already walks a tree
//! recursively and accepts both extensions, and
//! `crate::common::repo::rust_toolchain_sites` already scans both directories
//! for exactly this reason.
//!
//! **The input.** A workflow named `*.yaml`, or a local composite action that
//! sets the variable. The one composite action this repository has is used by
//! the job that lints the workflows.
//!
//! **The correct behaviour.** The rule is stated over everything GitHub
//! executes out of `.github/`: every `.yml` and `.yaml` under
//! `.github/workflows` and `.github/actions`, found recursively, from one
//! helper the rule's other callers share.

use crate::common::repo::{executed_yaml_files, yaml_files_under};

/// The composite action this repository calls from a workflow.
///
/// Named rather than derived: a scan that missed it is the finding, and a test
/// that computed the expectation the same way the scan does would agree with
/// the scan about anything.
const COMPOSITE_ACTION: &str = ".github/actions/install-actionlint/action.yml";

#[test]
fn the_scan_covers_the_composite_actions_a_workflow_calls() {
    let scanned = executed_yaml_files();

    assert!(
        scanned.contains(&COMPOSITE_ACTION.to_owned()),
        "`{COMPOSITE_ACTION}` runs inside whichever job uses it, so a variable it exports reaches \
         that job's steps. A rule about what CI may not set, stated over `.github/workflows` \
         alone, does not cover the file: {scanned:#?}"
    );
}

/// Every file GitHub executes out of `.github/`, written out.
///
/// Named rather than derived, for the reason above: an expectation computed
/// the way the scan computes it agrees with the scan about anything, including
/// about a directory neither of them reads. Adding a workflow means adding a
/// line here, which is the review this list exists to force.
const EXECUTED: [&str; 8] = [
    ".github/actions/install-actionlint/action.yml",
    ".github/workflows/ci.yml",
    ".github/workflows/codeql.yml",
    ".github/workflows/dependency-review.yml",
    ".github/workflows/distribute.yml",
    ".github/workflows/nightly.yml",
    ".github/workflows/release.yml",
    ".github/workflows/scorecard.yml",
];

/// A committed `.yaml` file, so the half of the rule about the *other*
/// spelling is pinned against a real file rather than against a claim.
///
/// It is deliberately outside [`EXECUTED`] — it configures actionlint rather
/// than being run by GitHub — which makes it exactly the file that shows
/// `yaml_files_under` accepting the extension while `executed_yaml_files`
/// scans the two directories that are run and no others.
const A_COMMITTED_YAML: &str = ".github/actionlint.yaml";

#[test]
fn the_scan_is_every_yaml_file_under_the_two_directories_that_are_executed() {
    assert_eq!(
        executed_yaml_files(),
        EXECUTED,
        "both directories, recursively, and nothing outside them. A file GitHub executes that \
         this list does not name is a file every rule stated over the list skips"
    );
}

#[test]
fn the_scan_reads_the_other_spelling_of_the_extension() {
    // GitHub accepts `.yaml` as readily as `.yml`, so a collector that reads
    // one of them is a rule a rename switches off — and no file under the two
    // executed directories is spelled that way today, so nothing there can
    // show the collector doing it.
    let under_github = yaml_files_under(".github");

    assert!(
        under_github.contains(&A_COMMITTED_YAML.to_owned()),
        "`{A_COMMITTED_YAML}` is a committed `.yaml` file, and the reader every rule about what \
         CI executes is stated through did not find it: {under_github:#?}"
    );
}
