// SPDX-License-Identifier: MIT OR Apache-2.0
//! `GINARY_REQUIRE_TOOLCHAIN=1` demanded `actionlint` of every runner, and no
//! runner installs it.
//!
//! **What went wrong.** `actionlint_accepts_every_workflow` is gated on
//! `require_tools(&["actionlint"])`, and `require_tools` escalates a missing
//! program to a panic when `GINARY_REQUIRE_TOOLCHAIN` is `1`. Three CI jobs
//! set that variable and run the `regressions` target — `test`, `coverage`
//! and, since E6 added a stub-gated step to it, `smoke-matrix` — and none of
//! the three installs `actionlint`. All three failed identically:
//!
//! ```text
//! thread 'e1_the_sha256sums_step_read_and_wrote_one_file::actionlint_accepts_every_workflow'
//! panicked at tests/common/tools.rs:63:17:
//! `actionlint` is not on PATH and GINARY_REQUIRE_TOOLCHAIN=1 forbids skipping
//! ```
//!
//! (`Coverage`
//! <https://github.com/P4suta/ginary/actions/runs/33702776627/job/100485724897>,
//! `Test (both flavors, stable)`
//! <https://github.com/P4suta/ginary/actions/runs/33702776627/job/100485421869>
//! and `Cross-Linux smoke matrix`
//! <https://github.com/P4suta/ginary/actions/runs/33702776627/job/100485421511>.)
//! It cannot be seen on a developer machine that has `actionlint` installed,
//! which is every machine that has ever run `mise run check`.
//!
//! **The input.** Any machine without `actionlint` on `PATH` and with
//! `GINARY_REQUIRE_TOOLCHAIN=1`. Every hosted runner is one.
//!
//! **The correct behaviour.** `GINARY_REQUIRE_TOOLCHAIN` is a claim about the
//! *toolchain* an artifact is built with — `gleam`, `erl`, `strip` — and
//! `actionlint` is not part of it: it is a lint, it belongs to the job that
//! lints, and it has nothing to do with whether a runtime can be packaged.
//! The split is the one E6 already made for the cross-built stubs, where
//! `GINARY_REQUIRE_STUBS` became the claim about stubs and
//! `GINARY_REQUIRE_TOOLCHAIN` stopped speaking for them. So there is a third
//! variable, [`REQUIRE_ACTIONLINT`], exactly one job sets it, that job
//! installs `actionlint`, and that job *runs the test*, because a check moved
//! out of three jobs and into none is a check that was deleted.
//!
//! The rules below are therefore four, and the fourth is the one that matters:
//! the gate exists, only the job that installs the tool claims it, the gated
//! test no longer reads the toolchain variable, and the lint job runs it.

use std::collections::BTreeSet;

use crate::common::repo::{read, workflow_jobs, workflow_steps, yaml_files_under};

/// The variable that says `actionlint` is supposed to be on this machine.
const REQUIRE_ACTIONLINT: &str = "GINARY_REQUIRE_ACTIONLINT";

/// The workflow and job that lints, installs `actionlint` and claims it.
const LINT: (&str, &str) = (".github/workflows/ci.yml", "lint");

/// The file holding the gated test, whose gate is the subject here.
const GATED_TEST_FILE: &str = "tests/regressions/e1_the_sha256sums_step_read_and_wrote_one_file.rs";

/// The name of the gated test, which the lint job has to select.
const GATED_TEST: &str = "actionlint_accepts_every_workflow";

/// The target the lint job names, which has to compile the gated test in.
const REGRESSIONS_TARGET: &str = "tests/regressions.rs";

#[test]
fn the_lint_job_installs_actionlint_and_says_so() {
    let steps = workflow_steps(LINT.0);
    let installed: Vec<String> = steps
        .iter()
        .filter(|step| step.job == LINT.1)
        .filter_map(|step| step.with.get("tool").cloned())
        .filter(|tool| tool.starts_with("actionlint"))
        .collect();
    assert_eq!(
        installed.len(),
        1,
        "the `{}` job of {} must install actionlint exactly once, and pin it, because it is the \
         one job that requires the tool. Installed tools: {:?}",
        LINT.1,
        LINT.0,
        steps
            .iter()
            .filter(|step| step.job == LINT.1)
            .filter_map(|step| step.with.get("tool"))
            .collect::<Vec<_>>()
    );
    assert!(
        installed[0].contains('@'),
        "the actionlint install is pinned to a version, like every other tool this repository \
         installs: {installed:?}"
    );

    let job = workflow_jobs(LINT.0)
        .into_iter()
        .find(|job| job.id == LINT.1)
        .unwrap_or_else(|| panic!("{} has a `{}` job", LINT.0, LINT.1));
    assert_eq!(
        job.env.get(REQUIRE_ACTIONLINT).map(String::as_str),
        Some("1"),
        "the job that installs actionlint is the job that forbids skipping it; without \
         {REQUIRE_ACTIONLINT} a broken install is a green run. Its env is {:?}",
        job.env
    );
}

#[test]
fn the_lint_job_runs_the_test_that_needs_actionlint() {
    let job = workflow_jobs(LINT.0)
        .into_iter()
        .find(|job| job.id == LINT.1)
        .unwrap_or_else(|| panic!("{} has a `{}` job", LINT.0, LINT.1));
    let selecting: Vec<&String> = job
        .commands
        .iter()
        .filter(|command| command.contains("cargo test") && command.contains(GATED_TEST))
        .collect();
    assert_eq!(
        selecting.len(),
        1,
        "moving `{GATED_TEST}` out of three jobs and into none would delete the check rather \
         than fix it: the `{}` job has to select it by name. Its commands are {:?}",
        LINT.1,
        job.commands
    );

    // The name in the workflow and the name in the tree are two literals, and
    // a filter that selects nothing is not an error: `cargo test --test
    // regressions <a name nobody has>` prints `0 passed` and exits 0. So the
    // step would stay green over a renamed, moved or deleted test while
    // actionlint ran over nothing, which is the loss this whole file exists to
    // prevent. Tie the two literals together.
    let source = read(GATED_TEST_FILE);
    assert!(
        source.contains(&format!("fn {GATED_TEST}(")),
        "{GATED_TEST_FILE} has to define `{GATED_TEST}`: the `{}` job selects that name with a \
         filter, and a filter that matches nothing exits 0, so a rename would leave the step \
         green with actionlint running over no workflow at all",
        LINT.1
    );

    // And it has to be in the target the step names. A module dropped from
    // `tests/regressions.rs`, or hidden behind a `cfg` the lint job's feature
    // set does not enable, is the same silent loss by another route.
    let target = read(REGRESSIONS_TARGET);
    let declaration = format!(
        "#[path = \"{}\"]",
        GATED_TEST_FILE.trim_start_matches("tests/")
    );
    assert!(
        target.contains(&declaration),
        "{REGRESSIONS_TARGET} has to compile {GATED_TEST_FILE} into the `regressions` target \
         unconditionally, since that is the target and feature set the `{}` job runs. Expected \
         to find `{declaration}`",
        LINT.1
    );
    let gated = target
        .split(&declaration)
        .next()
        .unwrap_or_default()
        .lines()
        .rev()
        .take_while(|line| line.trim_start().starts_with("#["))
        .any(|line| line.contains("cfg("));
    assert!(
        !gated,
        "{GATED_TEST_FILE} is compiled conditionally, so the feature set the `{}` job builds \
         decides whether the check runs at all. The one test that needs actionlint is not \
         behind a `cfg`",
        LINT.1
    );
}

#[test]
fn only_the_job_that_installs_actionlint_requires_it() {
    let mut claimed: BTreeSet<String> = BTreeSet::new();
    let mut unequipped: Vec<String> = Vec::new();
    for workflow in yaml_files_under(".github/workflows") {
        let steps = workflow_steps(&workflow);
        for job in workflow_jobs(&workflow) {
            if job.env.get(REQUIRE_ACTIONLINT).map(String::as_str) != Some("1") {
                continue;
            }
            claimed.insert(format!("{workflow}:{}", job.id));
            let installs = steps.iter().filter(|step| step.job == job.id).any(|step| {
                step.with
                    .get("tool")
                    .is_some_and(|tool| tool.starts_with("actionlint"))
            });
            if !installs {
                unequipped.push(format!("{workflow}:{}", job.id));
            }
        }
    }
    assert!(
        unequipped.is_empty(),
        "a job may claim {REQUIRE_ACTIONLINT} exactly when it installs actionlint; these claim \
         it and do not install it:\n{}",
        unequipped.join("\n")
    );
    assert_eq!(
        claimed,
        BTreeSet::from([format!("{}:{}", LINT.0, LINT.1)]),
        "actionlint belongs to the lint job and to no other; a second job requiring it is a \
         second job that has to install it"
    );
}

#[test]
fn the_gated_test_no_longer_reads_the_toolchain_variable() {
    let source = read(GATED_TEST_FILE);
    assert!(
        !source.contains("require_tools(&[\"actionlint\"])"),
        "{GATED_TEST_FILE} still gates `{GATED_TEST}` on the toolchain flag, which is what makes \
         every runner without actionlint fail. The tool has its own gate now"
    );
    assert!(
        source.contains("require_actionlint("),
        "{GATED_TEST_FILE} has to reach for the actionlint gate, so that a machine without the \
         tool skips loudly and the lint job — which has it — cannot"
    );
}

#[test]
fn the_third_variable_is_documented_where_the_other_two_are() {
    let testing = read("docs/dev/testing.md");
    assert!(
        testing.contains(REQUIRE_ACTIONLINT),
        "docs/dev/testing.md documents GINARY_REQUIRE_TOOLCHAIN and GINARY_REQUIRE_STUBS; a \
         third gate nobody can find is a third gate nobody sets"
    );
}
