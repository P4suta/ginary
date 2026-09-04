// SPDX-License-Identifier: MIT OR Apache-2.0
//! The test that was supposed to prove the missing-credentials notice runs
//! *only* when the credentials are missing could not fail.
//!
//! **What went wrong.** E5 split `release.yml` in two: `release-please` runs
//! behind `if: vars.RELEASE_PLEASE_APP_CLIENT_ID != ''`, and a
//! `credentials-notice` job behind the complementary `== ''` prints what a
//! maintainer has to add and exits 0. That pair is the whole
//! degrade-gracefully requirement — a repository with no release credentials
//! gets a green Release workflow that says why it did nothing.
//!
//! Only the first half was held by a test. `tests/release_workflow.rs` found
//! the notice step *by its content* — the step whose `run:` names both
//! credential names — and then asserted
//!
//! ```text
//! guard.contains("vars.RELEASE_PLEASE_APP_CLIENT_ID") || step.run.contains(CLIENT_ID_VAR)
//! ```
//!
//! The right-hand side is true by construction of the search, so the whole
//! assertion was a tautology: it could not fail, whatever the notice job's
//! `if:` said. Flipping `credentials-notice` to `!= ''` — which makes the
//! notice print exactly when release automation *is* configured and print
//! nothing when it is not, the precise inverse of the requirement — left the
//! suite green. So did deleting the guard altogether.
//!
//! **The input.** Any edit to `release.yml` that changes or drops the notice
//! job's `if:`.
//!
//! **The correct behaviour.** The two guards are read out of the parsed YAML
//! and held against each other: one job runs release-please when the client-id
//! variable is non-empty, a different job prints the notice when it is empty,
//! and the notice's guard is not the release job's. The step is still *found*
//! by the credential names it prints — there is no other way to find it — but
//! no assertion here reads that text, so no assertion here can be satisfied by
//! the search that produced the step.

use saphyr::YamlOwned;

use crate::common::repo::{WorkflowStep, workflow_steps, yaml};

/// The workflow the two guards live in.
const RELEASE: &str = ".github/workflows/release.yml";

/// The repository variable both guards are written against.
const CLIENT_ID_VAR: &str = "RELEASE_PLEASE_APP_CLIENT_ID";

/// The repository secret the notice also names.
const PRIVATE_KEY_SECRET: &str = "RELEASE_PLEASE_APP_PRIVATE_KEY";

/// One job's `if:` as written, or `<no if:>` when it has none.
///
/// A job with no condition always runs, which is a *different* wrong answer
/// from a job with the wrong condition, so it gets its own text rather than
/// an `Option` the caller would flatten.
fn job_guard(id: &str) -> Option<String> {
    let parsed = yaml(RELEASE);
    let jobs = parsed.as_mapping_get("jobs")?.as_mapping()?;
    for (key, job) in jobs {
        if key.as_str() == Some(id) {
            return Some(
                job.as_mapping_get("if")
                    .and_then(YamlOwned::as_str)
                    .unwrap_or("<no if:>")
                    .to_owned(),
            );
        }
    }
    None
}

/// The id of the job that runs `release-please`.
fn release_please_job() -> String {
    let parsed = yaml(RELEASE);
    let jobs = parsed
        .as_mapping_get("jobs")
        .and_then(YamlOwned::as_mapping)
        .unwrap_or_else(|| panic!("{RELEASE} declares no jobs"));
    for (key, job) in jobs {
        let Some(steps) = job.as_mapping_get("steps").and_then(YamlOwned::as_vec) else {
            continue;
        };
        if steps.iter().any(|step| {
            step.as_mapping_get("uses")
                .and_then(YamlOwned::as_str)
                .is_some_and(|uses| uses.contains("release-please-action"))
        }) {
            return key.as_str().unwrap_or_default().to_owned();
        }
    }
    panic!("no job of {RELEASE} runs googleapis/release-please-action");
}

/// The step that tells a maintainer which credentials are missing.
///
/// The release-please job has a step naming both credentials as well — the
/// check that reports the half-configured state, a variable with no secret
/// behind it — so the notice is the one *outside* that job. Selecting it by
/// content is safe here in a way it was not before: every assertion below
/// reads a job's `if:` and none reads a step's text, so no assertion can be
/// satisfied by the search that found the step.
fn notice_step() -> WorkflowStep {
    let release_job = release_please_job();
    workflow_steps(RELEASE)
        .into_iter()
        .find(|step| {
            step.job != release_job
                && step.run.contains(CLIENT_ID_VAR)
                && step.run.contains(PRIVATE_KEY_SECRET)
        })
        .unwrap_or_else(|| {
            panic!(
                "no step outside job `{release_job}` names both `{CLIENT_ID_VAR}` and \
                 `{PRIVATE_KEY_SECRET}`: there is nothing that could tell a maintainer what to \
                 add when the credentials are absent, because a step of the release job runs only \
                 when they are present"
            )
        })
}

#[test]
fn the_notice_runs_exactly_when_the_release_job_does_not() {
    let release_job = release_please_job();
    let notice_job = notice_step().job;

    let release_guard = job_guard(&release_job)
        .unwrap_or_else(|| panic!("job `{release_job}` is not in {RELEASE}"));
    let notice_guard =
        job_guard(&notice_job).unwrap_or_else(|| panic!("job `{notice_job}` is not in {RELEASE}"));

    assert!(
        release_guard.contains(&format!("vars.{CLIENT_ID_VAR}")) && release_guard.contains("!= ''"),
        "job `{release_job}` runs release-please when the client-id variable is *not* the empty \
         string, or it runs with credentials that do not exist. Its `if:` is `{release_guard}`"
    );
    assert!(
        notice_guard.contains(&format!("vars.{CLIENT_ID_VAR}")) && notice_guard.contains("== ''"),
        "job `{notice_job}` prints the missing-credentials notice, so it runs when the client-id \
         variable *is* the empty string and at no other time. Its `if:` is `{notice_guard}`"
    );
    assert!(
        !notice_guard.contains("!= ''"),
        "job `{notice_job}` is guarded by `{notice_guard}`, which is not the complement of \
         `{release_guard}`: a repository with the credentials would be told to add them, and one \
         without them would be told nothing"
    );
}
