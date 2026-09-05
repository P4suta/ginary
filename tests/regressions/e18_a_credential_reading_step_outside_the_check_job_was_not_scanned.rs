// SPDX-License-Identifier: MIT OR Apache-2.0
//! The rule about which steps have to carry the credentials guard read one job
//! of the workflow, so a step in any other job was excused by not being looked
//! at.
//!
//! **What went wrong.** E18's first repair fixed how the *notice* is excused —
//! by which step it is rather than by what its condition says — and left the
//! line above it alone:
//!
//! ```text
//! let Some(job) = credentials_job(workflow) else { return Vec::new() };
//! workflow_steps(workflow).into_iter().filter(|step| step.job == job)
//! ```
//!
//! `credentials_job` is the job holding the step with `id: credentials`. Every
//! step of every other job is dropped before the question "does this step need
//! the credentials" is ever asked, so the rule that exists to say "a `run:`
//! step reading an empty private key does something worse than fail" says
//! nothing about a second job that declares the same environment and reads the
//! same private key. That is the shape of the bug the same milestone had just
//! fixed one level down: an exclusion nobody wrote as an exclusion, because it
//! looks like scoping.
//!
//! The second exception had the same defect for the same reason. The check
//! that computes the guard was excused as *position one*, which is a property
//! every job has one of; a second job's first step is excused by a rule that
//! was written about a single step of a single job.
//!
//! **The input.** Any step, in any job of the workflow that can be handed what
//! the environment holds, that needs the credentials and does not carry the
//! configured guard.
//! `tests/fixtures/release/a_second_job_reads_the_credentials.yml` is that
//! workflow: `release.yml`'s job, correct on every count, plus an `announce`
//! job that declares the same environment and whose first step writes
//! `secrets.RELEASE_PLEASE_APP_PRIVATE_KEY` to a file and calls the API.
//!
//! **The correct behaviour.** The rule reads every job of the workflow that
//! can read what the environment holds — the one that computes the guard, the
//! ones that declare the environment, and the ones that name a credential —
//! and excuses exactly two steps, each by which step it is: the check, found
//! by its `id:`, and the notice, found by [`notice_step`]. A second job is
//! reported rather than skipped, which is the answer the tree wants: only one
//! job may declare the environment at all, and
//! `e18_nothing_bounded_the_number_of_jobs_that_may_read_the_credentials.rs` is
//! the rule that says so.
//!
//! [`notice_step`]: crate::common::release::notice_step

use crate::common::release::{
    CONFIGURED, RELEASE_WORKFLOW, steps_that_need_the_credentials_and_are_not_gated,
};
use crate::common::repo::{WorkflowStep, workflow_steps};

/// The workflow with one job too many.
const FIXTURE: &str = "tests/fixtures/release/a_second_job_reads_the_credentials.yml";

/// The job in it that the credentials check does not live in.
const SECOND_JOB: &str = "announce";

/// The name of the step in that job the rule has to report.
const INJECTED: &str = "Post the release note as the App";

/// The injected step, read straight off the fixture.
///
/// Read here rather than taken from the rule's own output, because the three
/// assertions below are about the fixture still posing the question: a
/// fixture that has been edited into a harmless workflow makes this file pass
/// while pinning nothing, which is the failure mode `tests/fixtures/release/README.md`
/// opens by naming.
fn injected_step() -> WorkflowStep {
    workflow_steps(FIXTURE)
        .into_iter()
        .find(|step| step.name == INJECTED)
        .unwrap_or_else(|| panic!("{FIXTURE} no longer holds a step named `{INJECTED}`"))
}

#[test]
fn a_step_of_a_second_job_that_reads_the_private_key_is_reported() {
    let injected = injected_step();
    assert_ne!(
        injected.job, "release-please",
        "the step this file is about is the one in the *other* job of {FIXTURE}; this one is in \
         the job that holds the credentials check, so the fixture no longer poses the question"
    );
    assert_eq!(
        injected.position, 1,
        "the injected step is the first step of `{}`, which is what makes it adversarial: a rule \
         that excuses position one rather than the step that computes the guard excuses it. It is \
         now at position {}",
        injected.job, injected.position
    );
    assert_eq!(
        injected.cond, "",
        "the injected step carries no guard at all — there is none it could carry, because \
         `steps.credentials` belongs to the other job. Its `if:` is now `{}`, so the fixture is \
         asking a different question",
        injected.cond
    );

    let reported: Vec<String> = steps_that_need_the_credentials_and_are_not_gated(FIXTURE)
        .into_iter()
        .map(|step| format!("{}: {}", step.job, step.name))
        .collect();
    assert_eq!(
        reported,
        vec![format!("{SECOND_JOB}: {INJECTED}")],
        "step 1 of job `{SECOND_JOB}` of {FIXTURE} declares the same environment as the release \
         job, is handed the same private key, writes it to a file and calls the API, and carries \
         no guard. A rule that reads only the job holding the credentials check never sees it: \
         the steps it is about are the steps of every job that can be handed what the environment \
         holds, and the two it excuses — the check and the notice — are excused by which step \
         they are"
    );

    assert!(
        steps_that_need_the_credentials_and_are_not_gated(RELEASE_WORKFLOW).is_empty(),
        "the same rule reports a step of {RELEASE_WORKFLOW}, where every step that needs the \
         credentials carries `{CONFIGURED}`. A rule wide enough to see a second job must not have \
         become one that reports the check or the notice as well: {:#?}",
        steps_that_need_the_credentials_and_are_not_gated(RELEASE_WORKFLOW)
    );
}
