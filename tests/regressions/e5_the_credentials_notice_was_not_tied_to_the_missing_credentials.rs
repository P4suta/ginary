// SPDX-License-Identifier: MIT OR Apache-2.0
//! The test that was supposed to prove the missing-credentials notice runs
//! *only* when the credentials are missing could not fail.
//!
//! **What went wrong.** E5 split `release.yml` in two: `release-please` ran
//! behind `if: vars.RELEASE_PLEASE_APP_CLIENT_ID != ''`, and a
//! `credentials-notice` job behind the complementary `== ''` printed what a
//! maintainer has to add and exited 0. That pair is the whole
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
//! **The input.** Any edit to `release.yml` that changes or drops the guard
//! the notice runs behind.
//!
//! **The correct behaviour.** The two guards are read out of the parsed YAML
//! and held against each other: the credentials are read once and the answer
//! published, the steps that need them run when it says `configured`, the
//! notice runs when it says `absent`, and the notice's guard is not the
//! release steps'. The step is still *found* by the credential names it prints
//! — there is no other way to find it — but no assertion here reads that text,
//! so no assertion here can be satisfied by the search that produced the step.
//!
//! **E17 moved the guards without weakening this.** Both credentials now live
//! in the `release` GitHub Environment, and a job's `vars` and `secrets`
//! contexts carry an environment's values only when the job declares that
//! environment — while a job's own `if:` is evaluated before that binding, so
//! E5's job-level guard could no longer read the variable it was written
//! against. The guard therefore moved from two job conditions to two step
//! conditions inside the one job that declares the environment. The bug this
//! file pins is the same one: a notice whose reachability nothing checks.

use crate::common::repo::{WorkflowStep, workflow_steps, yaml};

use saphyr::YamlOwned;

/// The workflow the two guards live in.
const RELEASE: &str = ".github/workflows/release.yml";

/// The variable both guards are ultimately written against.
const CLIENT_ID_VAR: &str = "RELEASE_PLEASE_APP_CLIENT_ID";

/// The secret the notice also names.
const PRIVATE_KEY_SECRET: &str = "RELEASE_PLEASE_APP_PRIVATE_KEY";

/// The guard the steps that need the credentials carry.
const CONFIGURED: &str = "steps.credentials.outputs.state == 'configured'";

/// The guard the notice carries.
const ABSENT: &str = "steps.credentials.outputs.state == 'absent'";

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
/// Two steps name both credentials, because there are two states to report:
/// the environment holds neither, which is a notice and a green run, and it
/// holds the variable without the secret, which is a failure. The notice is
/// the one that does not exit non-zero. Selecting it by content is safe here
/// in a way it was not before: every assertion below reads a guard and none
/// reads a step's text, so no assertion can be satisfied by the search that
/// found the step.
fn notice_step() -> WorkflowStep {
    workflow_steps(RELEASE)
        .into_iter()
        .find(|step| {
            step.run.contains(CLIENT_ID_VAR)
                && step.run.contains(PRIVATE_KEY_SECRET)
                && !step
                    .commands()
                    .iter()
                    .any(|command| command.starts_with("exit ") && command != "exit 0")
        })
        .unwrap_or_else(|| {
            panic!(
                "no step of {RELEASE} names both `{CLIENT_ID_VAR}` and `{PRIVATE_KEY_SECRET}` \
                 without failing: there is nothing that could tell a maintainer what to add when \
                 the credentials are absent"
            )
        })
}

#[test]
fn the_notice_runs_exactly_when_the_release_steps_do_not() {
    let release_job = release_please_job();
    let notice = notice_step();

    // Every step of the job but the two that are the guard itself: the check
    // at position one, which computes the answer, and the notice, which runs
    // on its complement and is asserted on below. A `!uses.is_empty()` filter
    // here — which is what this rule used to carry — excuses a `run:` step
    // that reads the private key out of its own `env:` and calls the API, and
    // that step needs the guard exactly as much as the App-token step does.
    let guards: Vec<(usize, String)> = workflow_steps(RELEASE)
        .into_iter()
        .filter(|step| step.job == release_job)
        .filter(|step| step.position != 1 && !step.cond.contains(ABSENT))
        .filter(|step| {
            !step.uses.is_empty()
                || [&step.run, &step.cond]
                    .into_iter()
                    .chain(step.env.values())
                    .chain(step.with.values())
                    .any(|text| text.contains(CLIENT_ID_VAR) || text.contains(PRIVATE_KEY_SECRET))
        })
        .map(|step| (step.position, step.cond))
        .collect();
    assert!(
        !guards.is_empty(),
        "job `{release_job}` uses no action, so there is nothing the credentials are for"
    );
    for (position, guard) in &guards {
        assert!(
            guard.contains(CONFIGURED),
            "step {position} of `{release_job}` runs whether or not the credentials exist. Its \
             `if:` is `{guard}`, and the guard is `{CONFIGURED}`"
        );
    }

    assert!(
        notice.cond.contains(ABSENT),
        "step {} of `{}` prints the missing-credentials notice, so it runs when the credentials \
         are absent and at no other time. Its `if:` is `{}`",
        notice.position,
        notice.job,
        notice.cond
    );
    assert!(
        !notice.cond.contains("'configured'"),
        "the notice is guarded by `{}`, which is not the complement of `{CONFIGURED}`: a \
         repository with the credentials would be told to add them, and one without them would \
         be told nothing",
        notice.cond
    );
}
