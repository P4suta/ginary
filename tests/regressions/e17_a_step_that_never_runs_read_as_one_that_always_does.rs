// SPDX-License-Identifier: MIT OR Apache-2.0
//! A workflow step disabled with `if: false` was read as a step with no
//! condition at all — the opposite of what it says.
//!
//! **What went wrong.** E17 gave [`WorkflowStep`] and `WorkflowJob` a `cond`
//! field so that a guard which moved off a job's `if:` and onto its steps
//! could still be asserted on. Both read it as
//!
//! ```text
//! node.as_mapping_get("if").and_then(YamlOwned::as_str).unwrap_or_default()
//! ```
//!
//! and `if: false` is not a string. YAML parses it as a boolean, `as_str`
//! answers `None`, and the condition became the empty string — which every
//! rule in the suite reads as "this always runs", the doc comment included.
//!
//! The rule that pays for it is
//! `the_credentials_check_is_the_first_step_and_publishes_what_it_found`,
//! which asserts `first.cond == ""` under the message "the credentials check
//! itself is unguarded". Disabling that step with the ordinary one-line edge
//! — `if: false`, the way anybody temporarily switches a step off — left the
//! assertion green while the whole release job produced no `state` output at
//! all, so every later step's guard read an empty string and nothing ran.
//! `tests/release_workflow.rs` was checked against a `release.yml` carrying
//! exactly that edit and reported no failure.
//!
//! **The input.** Any `if:` GitHub accepts that YAML does not parse as a
//! string. `if: false` is the one that occurs in practice; `if: true` is the
//! same hole in the other direction.
//!
//! **The correct behaviour.** A non-string condition is rendered rather than
//! dropped, so `if: false` reads as `"false"` and `cond.is_empty()` keeps
//! meaning "there is no condition here".

use crate::common::repo::{parse_yaml, workflow_jobs_of, workflow_steps_of};

/// A workflow carrying the condition in both places it can appear.
///
/// Written here rather than committed under `.github/`: the bug is in the
/// *reader*, and a workflow whose only purpose is to be misread is not one
/// GitHub should be handed.
const DISABLED: &str = "\
name: Disabled
on:
  push:
jobs:
  never:
    if: false
    runs-on: ubuntu-latest
    steps:
      - name: A step that never runs
        if: false
        run: echo off
      - name: A step that always runs
        run: echo on
      - name: A step with a real guard
        if: ${{ github.ref == 'refs/heads/main' }}
        run: echo guarded
";

/// The synthetic workflow's steps, under the label a failure names it by.
fn steps() -> Vec<crate::common::repo::WorkflowStep> {
    let parsed = parse_yaml(DISABLED).expect("the synthetic workflow is valid YAML");
    workflow_steps_of("<the disabled workflow>", &parsed)
}

#[test]
fn a_step_disabled_with_a_boolean_does_not_read_as_unconditional() {
    let steps = steps();
    let disabled = steps.first().expect("the synthetic workflow has steps");
    assert_eq!(
        disabled.cond, "false",
        "step 1 is `if: false` — it never runs — and its condition reads as `{}`. The empty \
         string is what a step with no `if:` at all reads as, so every rule asserting \
         `cond.is_empty()` to mean `this is unguarded` is satisfied by the one step that is \
         guarded hardest",
        disabled.cond
    );
    assert!(
        !disabled.cond.is_empty(),
        "`cond.is_empty()` is the question `does this step always run`, and step 1 never runs"
    );
}

#[test]
fn a_step_with_no_condition_still_reads_as_unconditional() {
    let steps = steps();
    let always = steps
        .get(1)
        .expect("the synthetic workflow has a second step");
    assert_eq!(
        always.cond, "",
        "step 2 carries no `if:`, so its condition is the empty string. Rendering a non-string \
         `if:` must not put text on a step that has none"
    );
    let guarded = steps
        .get(2)
        .expect("the synthetic workflow has a third step");
    assert_eq!(
        guarded.cond, "${{ github.ref == 'refs/heads/main' }}",
        "an ordinary string condition is still returned as written"
    );
}

#[test]
fn a_job_disabled_with_a_boolean_does_not_read_as_unconditional() {
    let parsed = parse_yaml(DISABLED).expect("the synthetic workflow is valid YAML");
    let jobs = workflow_jobs_of("<the disabled workflow>", &parsed);
    let job = jobs.first().expect("the synthetic workflow has a job");
    assert_eq!(
        job.cond, "false",
        "job `{}` is `if: false` and its condition reads as `{}`. A job that never runs must not \
         be reported as one that always does — `tests/release_workflow.rs` asserts over job \
         conditions to decide which guard a workflow carries",
        job.id, job.cond
    );
}
