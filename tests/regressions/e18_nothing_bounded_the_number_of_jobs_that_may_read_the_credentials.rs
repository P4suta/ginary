// SPDX-License-Identifier: MIT OR Apache-2.0
//! `docs/RELEASE.md` named a test as what keeps this repository to one reader
//! of the release credentials, and that test bounds something else.
//!
//! **What went wrong.** E18's first repair of thread 2 took the exclusivity
//! away from GitHub and gave it to
//! `no_job_of_any_workflow_reads_the_release_credentials_without_declaring_the_environment`:
//!
//! ```text
//! What keeps this repository to a single one is this repository's own test …
//! walks every scalar of every workflow for either credential name and
//! requires each site it finds to sit in a job declaring the same environment
//! ```
//!
//! That rule does exactly one thing per site: the job naming a credential has
//! to declare the environment. It bounds **where** a credential may be read
//! and says nothing about **how many** jobs read it. A second job that writes
//! `environment: release` and `secrets.RELEASE_PLEASE_APP_PRIVATE_KEY`
//! satisfies it — the site sits in a job declaring the environment, so the
//! assertion holds, the occurrence counts still match, and the suite is green.
//! The only second reader it turns red is one that does *not* declare the
//! environment, which is the one that cannot read the values anyway, because
//! it is handed two empty strings.
//!
//! So the sentence was the thread's own defect with a new subject: an
//! access-control guarantee attributed to something that does not provide it,
//! moved from the platform onto one of our own tests — and worse in one
//! respect, because a citation looks checkable and this one was checked.
//!
//! **The input.** A second job, in any workflow of this repository, that
//! declares the release environment.
//! `tests/fixtures/release/a_second_job_reads_the_credentials.yml` is that
//! workflow: `release.yml`'s job unchanged, plus an `announce` job that
//! declares the same environment and reads the private key.
//!
//! **The correct behaviour.** The bound the document claims is a rule of its
//! own — [`jobs_declaring_the_environment`] collects every job of every
//! workflow whose `environment:` is the release one, and
//! `exactly_one_job_of_this_repository_declares_the_release_environment` in
//! [`tests/release_workflow.rs`](../release_workflow.rs) requires there to be
//! one — and the document cites both halves, which
//! `e18_the_environment_was_credited_with_keeping_other_jobs_out.rs` holds it
//! to.
//!
//! [`jobs_declaring_the_environment`]: crate::common::release::jobs_declaring_the_environment

use crate::common::release::{
    ENVIRONMENT, RELEASE_WORKFLOW, committed_workflows, jobs_declaring_the_environment,
};

/// The workflow with a second declaring job.
const FIXTURE: &str = "tests/fixtures/release/a_second_job_reads_the_credentials.yml";

/// The job in it that is one line of YAML away from the credentials.
const SECOND_JOB: &str = "announce";

/// Every declaring job as `<workflow>: <job id>`, for an assertion that names
/// which job it means.
fn declared(workflows: &[String]) -> Vec<String> {
    jobs_declaring_the_environment(workflows)
        .into_iter()
        .map(|job| format!("{}: {}", job.workflow, job.id))
        .collect()
}

#[test]
fn a_second_job_that_declares_the_release_environment_is_visible_to_the_rule() {
    let found = declared(&[RELEASE_WORKFLOW.to_owned(), FIXTURE.to_owned()]);
    assert_eq!(
        found,
        vec![
            format!("{RELEASE_WORKFLOW}: release-please"),
            format!("{FIXTURE}: release-please"),
            format!("{FIXTURE}: {SECOND_JOB}"),
        ],
        "job `{SECOND_JOB}` of {FIXTURE} writes `environment: {ENVIRONMENT}` and is therefore \
         handed the same client id and the same private key as the release job, on every ref the \
         deployment-branch policy admits. A rule that is shown this pair and reports one job per \
         workflow, or reports only the job that computes the guard, is not the bound \
         `docs/RELEASE.md` cites: the bound is on the number of jobs, across every workflow, that \
         declare the environment at all"
    );
}

#[test]
fn the_committed_workflows_declare_the_environment_from_exactly_one_job() {
    assert_eq!(
        declared(&committed_workflows()),
        vec![format!("{RELEASE_WORKFLOW}: release-please")],
        "the `{ENVIRONMENT}` environment holds both release credentials, and every job that \
         declares it is handed both. `docs/RELEASE.md` says one job does; this is the tree it \
         says it about, and the milestone rule \
         `exactly_one_job_of_this_repository_declares_the_release_environment` is the same \
         assertion with the message a maintainer adding a job would want to read"
    );
}
