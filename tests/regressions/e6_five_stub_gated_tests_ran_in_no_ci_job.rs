// SPDX-License-Identifier: MIT OR Apache-2.0
//! The step that was supposed to run the stub-gated tests in the one job that
//! has the stubs ran four of the nine, on the strength of a comment that was
//! not true.
//!
//! **What went wrong.** E6 gave `smoke-matrix` a step of its own:
//!
//! ```yaml
//! # Two targets only, because these are the only two files
//! # `stubfile::cross_stub` gates.
//! - name: Run the tests that need a cross-built stub
//!   run: cargo test --locked --test e2e_cross --test stub
//! ```
//!
//! There are four such files, not two:
//!
//! ```text
//! $ grep -rln 'cross_stub' tests/ --include=*.rs
//! tests/e2e_cross.rs   (3 tests)   tests/e2e_native.rs (4 tests)
//! tests/stub.rs        (1 test)    tests/regressions/c2_… (1 test)
//! ```
//!
//! So five stub-gated tests ran in no CI job at all — `test` and `coverage`
//! skip them for want of a stub, and the job that has the stubs was not told
//! to run them. That is the opposite of the design the step was added for,
//! and it was invisible because the count lived in a comment. Every target
//! those five ask for (`linux-aarch64-musl`, `linux-x86_64-gnu`,
//! `Target::host()`) is one the job already cross-builds, so nothing but the
//! miscount kept them out.
//!
//! **The input.** Any file that starts calling `cross_stub` without the
//! workflow learning about it. A fifth caller today would repeat this exactly.
//!
//! **The correct behaviour.** The set of test targets a job with the stubs
//! runs is derived from the tree, not transcribed into a comment: every
//! tracked test file that calls `cross_stub` names a `cargo test` target, and
//! each of those targets is run by a job that obtains the stubs.

use std::collections::{BTreeMap, BTreeSet};

use crate::common::portability::tracked_test_sources;
use crate::common::repo::{WorkflowJob, workflow_jobs};

/// The workflow that runs the stub-gated tests.
const CI: &str = ".github/workflows/ci.yml";

/// The `cargo test` target name a tracked test source belongs to.
///
/// `tests/e2e_cross.rs` is its own target; every file under
/// `tests/regressions/` is compiled into `tests/regressions.rs`, so they all
/// share the target `regressions`. Anything under `tests/common/` is a module
/// of whoever includes it and is no target of its own.
fn test_target_of(file: &str) -> Option<String> {
    let rest = file.strip_prefix("tests/")?;
    if let Some((directory, _)) = rest.split_once('/') {
        if directory == "common" || directory == "fixtures" || directory == "snapshots" {
            return None;
        }
        return Some(directory.to_owned());
    }
    Some(rest.strip_suffix(".rs")?.to_owned())
}

/// Whether a line of source calls `cross_stub`, as opposed to naming it in
/// prose or calling the pure `choose_cross_stub` the rule lives in.
fn calls_cross_stub(line: &str) -> bool {
    let mut rest = line;
    while let Some(at) = rest.find("cross_stub(") {
        let before = &rest[..at];
        if !before.ends_with("choose_") {
            return true;
        }
        rest = &rest[at + "cross_stub(".len()..];
    }
    false
}

/// Every `--test <name>` a `cargo test` command line names, or `None` when it
/// names none — which is `cargo test` running every target there is.
fn test_targets_of(command: &str) -> Option<BTreeSet<String>> {
    let tokens: Vec<&str> = command.split_whitespace().collect();
    let mut named = BTreeSet::new();
    for (index, token) in tokens.iter().enumerate() {
        if *token == "--test" {
            if let Some(name) = tokens.get(index + 1) {
                named.insert((*name).to_owned());
            }
        } else if let Some(name) = token.strip_prefix("--test=") {
            named.insert(name.to_owned());
        }
    }
    if named.is_empty() { None } else { Some(named) }
}

/// Whether a job obtains the cross-built stubs: it builds them with `cross`,
/// or it downloads what `cross-build` uploaded.
fn obtains_stubs(job: &WorkflowJob) -> bool {
    (job.runs("cross build") && job.runs("target/stubs"))
        || (job.uses_action("download-artifact") && job.needs.iter().any(|n| n == "cross-build"))
}

#[test]
fn a_command_line_that_names_no_target_runs_every_target() {
    assert_eq!(
        test_targets_of("cargo test --locked --test e2e_cross --test stub"),
        Some(
            ["e2e_cross", "stub"]
                .into_iter()
                .map(str::to_owned)
                .collect()
        )
    );
    assert_eq!(
        test_targets_of("cargo test --locked --test=regressions"),
        Some(["regressions"].into_iter().map(str::to_owned).collect())
    );
    assert_eq!(
        test_targets_of("cargo test --features fault-injection"),
        None
    );
    assert!(calls_cross_stub(
        "    let Some(stub) = cross_stub(&target) else {"
    ));
    assert!(calls_cross_stub("    stubfile::cross_stub(&host)"));
    assert!(!calls_cross_stub(
        "    let choice = choose_cross_stub(NAME, &dirs, true, false, &|_| false);"
    ));
}

#[test]
fn every_test_target_that_needs_a_cross_stub_is_run_by_a_job_that_has_one() {
    let Some(sources) = tracked_test_sources() else {
        eprintln!("skipping: git could not list the tracked test sources");
        return;
    };
    assert!(
        sources.unreadable.is_empty(),
        "the scan could not read {:?}, so it cannot say which targets need a stub",
        sources.unreadable
    );

    // target -> the files that put it there, so a failure names the caller.
    let mut needed: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (file, text) in &sources.files {
        if !text.lines().any(calls_cross_stub) {
            continue;
        }
        let Some(target) = test_target_of(file) else {
            continue;
        };
        needed.entry(target).or_default().push(file.clone());
    }
    assert!(
        needed.len() >= 3,
        "only {needed:?} call `cross_stub`; this test has lost its subject"
    );

    let mut covered: BTreeSet<String> = BTreeSet::new();
    let mut runs_everything = false;
    for job in workflow_jobs(CI) {
        if !obtains_stubs(&job) {
            continue;
        }
        for command in &job.commands {
            if !command.contains("cargo test") {
                continue;
            }
            match test_targets_of(command) {
                Some(named) => covered.extend(named),
                None => runs_everything = true,
            }
        }
    }

    let missing: Vec<String> = needed
        .iter()
        .filter(|(target, _)| !runs_everything && !covered.contains(*target))
        .map(|(target, files)| format!("`{target}` (from {})", files.join(", ")))
        .collect();
    assert!(
        missing.is_empty(),
        "these `cargo test` targets hold tests that need a cross-built stub, and no job that \
         obtains the stubs runs them — so they skip in `test` and `coverage` for want of a stub \
         and are never run anywhere: {}. The job with the stubs runs {covered:?}",
        missing.join(", ")
    );
}
