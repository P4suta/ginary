// SPDX-License-Identifier: MIT OR Apache-2.0
//! The release credentials: the names, the two guard expressions, who may be
//! handed the values, and the one rule about which steps have to carry the
//! guard that says they are there.
//!
//! Five test files ask the same questions of `release.yml` — the milestone
//! rules in [`tests/release_workflow.rs`](../release_workflow.rs), and the
//! regressions that pin E5's, E17's and E18's bugs — and until E18 each
//! carried its own copy of the two guard expressions, the two credential
//! names, the selector that finds the notice, the walk that finds every site a
//! credential is written at and the filter that decides which steps the guard
//! is about. Copies of a rule are separate rules, and the one this module
//! exists for was wrong in two of them at once. Every one of those files now
//! reads its vocabulary from here; nothing about the release credentials is
//! declared twice in `tests/`.
//!
//! The rules are functions of a *workflow path*, or of a list of them, rather
//! than of `release.yml` and `.github/workflows`, which is the other half of
//! why they live here: a rule that can only read the files it was written for
//! cannot be shown a workflow that breaks it, so the only evidence it works is
//! that it has never fired. `tests/fixtures/release/` holds the workflows they
//! fire on, and the two `e18_…` regressions listed on
//! [`steps_that_need_the_credentials`] and
//! [`jobs_declaring_the_environment`] are where the rules and the fixtures are
//! held against each other.

use crate::common::repo::{
    NameSite, WorkflowJob, WorkflowStep, name_sites, read, workflow_jobs, workflow_steps,
    yaml_files_under, yaml_text_occurrences,
};

/// The workflow the release credentials are read in.
pub const RELEASE_WORKFLOW: &str = ".github/workflows/release.yml";

/// The variable holding the App's client id.
pub const CLIENT_ID_VAR: &str = "RELEASE_PLEASE_APP_CLIENT_ID";

/// The secret holding the App's private key.
pub const PRIVATE_KEY_SECRET: &str = "RELEASE_PLEASE_APP_PRIVATE_KEY";

/// The GitHub Environment both credentials live in.
///
/// E17 moved them off repository scope. The environment's deployment-branch
/// policy admits exactly the `main` branch and the `v*` tags, and a job
/// receives an environment's values only when it declares that environment —
/// which is what the environment does and, since E18, all it is credited
/// with. It does not make the release job the only job that *could* declare
/// it; that bound is [`jobs_declaring_the_environment`]'s, and
/// `tests/regressions/e18_the_environment_was_credited_with_keeping_other_jobs_out.rs`
/// is where the difference is written down.
pub const ENVIRONMENT: &str = "release";

/// The id of the step that reads what the environment holds.
pub const CHECK_STEP_ID: &str = "credentials";

/// The expression every step that needs the credentials is guarded by.
pub const CONFIGURED: &str = "steps.credentials.outputs.state == 'configured'";

/// The expression the missing-credentials notice is guarded by.
pub const ABSENT: &str = "steps.credentials.outputs.state == 'absent'";

/// Every workflow this repository commits, sorted.
///
/// The rules below take the list rather than reading it themselves, so that a
/// regression can hand one of them a fixture alongside the committed files and
/// watch it fire.
pub fn committed_workflows() -> Vec<String> {
    yaml_files_under(".github/workflows")
}

/// The id of the job that computes the guard, if one workflow has such a job.
///
/// The job is found by the step that computes the guard rather than by the
/// step that runs release-please: the rules below are about the credentials,
/// and a workflow that has lost its release-please step still has to answer
/// for the credentials it reads.
pub fn credentials_job(workflow: &str) -> Option<String> {
    workflow_steps(workflow)
        .into_iter()
        .find(|step| step.id == CHECK_STEP_ID)
        .map(|step| step.job)
}

/// Every job of the given workflows that declares the release environment.
///
/// A job is handed what the environment holds exactly when it declares the
/// environment, so this is the list of jobs that can read either credential at
/// all — and its *length* is the bound `docs/RELEASE.md` claims and
/// `exactly_one_job_of_this_repository_declares_the_release_environment` in
/// [`tests/release_workflow.rs`](../release_workflow.rs) enforces. Declaring
/// the environment is not a privilege GitHub hands to one job: any job of any
/// workflow may write it and be handed the same two values on a ref the
/// deployment-branch policy admits, so nothing outside this repository bounds
/// the number. See
/// `tests/regressions/e18_nothing_bounded_the_number_of_jobs_that_may_read_the_credentials.rs`,
/// which hands this a fixture holding a second declaring job.
pub fn jobs_declaring_the_environment(workflows: &[String]) -> Vec<WorkflowJob> {
    workflows
        .iter()
        .flat_map(|workflow| workflow_jobs(workflow))
        .filter(|job| job.environment == ENVIRONMENT)
        .collect()
}

/// The environment one job of one workflow declares, or `<no environment:>`.
///
/// The placeholder is not a name any environment can have, so an assertion
/// that compares against [`ENVIRONMENT`] reports the missing declaration
/// rather than an empty string nobody can read.
pub fn job_environment(workflow: &str, id: &str) -> String {
    workflow_jobs(workflow)
        .into_iter()
        .find(|job| job.id == id)
        .map(|job| job.environment)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "<no environment:>".to_owned())
}

/// Every place the given workflows write either credential name.
///
/// Not a fixed list of node kinds. A credential reaches a job through more
/// than its `run:` blocks — the `client-id:` input is a `with:` value, the
/// check reads its secret through `env:`, a guard is an `if:` — and it reaches
/// a *different* workflow just as easily, where it would expand to the empty
/// string and leave that run green. Both halves of the narrowing are how E17's
/// first scan let three node kinds and six workflows through;
/// [`name_sites`] walks the whole parsed document, and
/// [`credential_occurrences`] is the cross-check that holds the walk against
/// the file text so a node kind it cannot reach fails loudly.
pub fn credential_sites(workflows: &[String]) -> Vec<NameSite> {
    let mut out = Vec::new();
    for workflow in workflows {
        for credential in [CLIENT_ID_VAR, PRIVATE_KEY_SECRET] {
            out.extend(name_sites(workflow, credential));
        }
    }
    out
}

/// How many times one workflow writes a credential name, and how many of those
/// the site walk reaches.
///
/// A rule about where a credential may be read is only as wide as the scan
/// behind it, so the two counts are asserted equal and an occurrence the walk
/// cannot see fails loudly instead of passing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CredentialOccurrences {
    /// The credential name counted.
    pub credential: &'static str,
    /// How many times the file text writes it outside a whole-line comment.
    pub written: usize,
    /// How many of those [`credential_sites`] reaches.
    pub found: usize,
}

/// Both credentials' occurrence counts for one workflow.
pub fn credential_occurrences(workflow: &str) -> Vec<CredentialOccurrences> {
    let text = read(workflow);
    [CLIENT_ID_VAR, PRIVATE_KEY_SECRET]
        .into_iter()
        .map(|credential| CredentialOccurrences {
            credential,
            written: yaml_text_occurrences(&text, credential),
            found: name_sites(workflow, credential)
                .iter()
                .map(|site| site.count)
                .sum(),
        })
        .collect()
}

/// The step that tells a maintainer which credentials are missing.
///
/// Two steps of `release.yml` may name both credentials, because there are
/// two states to report: the environment holds neither, which is a notice and
/// a green run, and it holds the variable without the secret, which is a
/// failure. The notice is the one that does not exit non-zero. Selecting it by
/// content is safe as long as no assertion *about* it reads that content —
/// the tautology E5 paid for once already.
pub fn notice_step(workflow: &str) -> Option<WorkflowStep> {
    workflow_steps(workflow).into_iter().find(|step| {
        step.run.contains(CLIENT_ID_VAR)
            && step.run.contains(PRIVATE_KEY_SECRET)
            && !step
                .commands()
                .iter()
                .any(|command| command.starts_with("exit ") && command != "exit 0")
    })
}

/// Whether one step writes either credential name anywhere of its own.
fn names_a_credential(step: &WorkflowStep) -> bool {
    [&step.run, &step.cond]
        .into_iter()
        .chain(step.env.values())
        .chain(step.with.values())
        .any(|text| text.contains(CLIENT_ID_VAR) || text.contains(PRIVATE_KEY_SECRET))
}

/// Whether one step needs the credentials to be there.
///
/// Either it uses an action — in a job that can be handed the credentials
/// every one of those is part of the release, and reaching
/// `create-github-app-token` with an empty `client-id:` fails somewhere inside
/// JWT signing — or it names a credential in a `run:`, an `if:`, an `env:` or
/// a `with:` of its own. A `!uses.is_empty()` test alone, which this rule used
/// to be, excuses a `run:` step that reads the private key out of its own
/// `env:` and calls the API, and that step needs the guard exactly as much as
/// the App-token step does.
fn needs_the_credentials(step: &WorkflowStep) -> bool {
    !step.uses.is_empty() || names_a_credential(step)
}

/// Every job of one workflow that can be handed what the environment holds.
///
/// Three ways in, and the rule takes all of them because taking one is how the
/// scan gets narrow enough to miss the bug: a job that **declares** the
/// environment is handed the values whether or not it names them; the job that
/// computes the guard is the one the rule was written about; and a job that
/// names a credential is asking for one however it declared itself. Scoping
/// the step rule to the credentials job alone — which is what it did until
/// E18 — drops every step of every other job before the question is asked. See
/// `tests/regressions/e18_a_credential_reading_step_outside_the_check_job_was_not_scanned.rs`.
fn jobs_that_can_read_the_credentials(workflow: &str) -> Vec<String> {
    let mut jobs: Vec<String> = workflow_jobs(workflow)
        .into_iter()
        .filter(|job| job.environment == ENVIRONMENT)
        .map(|job| job.id)
        .collect();
    for step in workflow_steps(workflow) {
        if (step.id == CHECK_STEP_ID || names_a_credential(&step)) && !jobs.contains(&step.job) {
            jobs.push(step.job);
        }
    }
    jobs
}

/// The two steps that are the guard itself, as `(job, position)`.
///
/// The check computes the answer and therefore cannot be guarded on it, and
/// the notice runs on the answer's complement and is asserted on separately.
///
/// Both name the *step* they excuse rather than a property several steps
/// share, and both had to. An exception written as "whose condition contains
/// [`ABSENT`]" excuses the notice and every later step that carries the same
/// condition with it, and an exception written as "position one" excuses the
/// first step of every job the scan reaches. In both shapes a step that reads
/// the private key is dropped from the scan rather than reported by it, and
/// there is no symptom a green run could show, because the run that reaches
/// such a step is the run of a repository with no credentials. See
/// `tests/regressions/e18_a_step_that_was_not_the_notice_wore_the_notice_guard.rs`
/// and
/// `tests/regressions/e18_a_credential_reading_step_outside_the_check_job_was_not_scanned.rs`.
fn steps_that_are_the_guard(workflow: &str) -> Vec<(String, usize)> {
    let check = workflow_steps(workflow)
        .into_iter()
        .find(|step| step.id == CHECK_STEP_ID);
    check
        .map(|step| (step.job, step.position))
        .into_iter()
        .chain(notice_step(workflow).map(|step| (step.job, step.position)))
        .collect()
}

/// Every step that needs the credentials, in every job that can be handed
/// them, the two steps that are the guard itself aside.
pub fn steps_that_need_the_credentials(workflow: &str) -> Vec<WorkflowStep> {
    let jobs = jobs_that_can_read_the_credentials(workflow);
    let guard = steps_that_are_the_guard(workflow);
    workflow_steps(workflow)
        .into_iter()
        .filter(|step| jobs.contains(&step.job))
        .filter(|step| !guard.contains(&(step.job.clone(), step.position)))
        .filter(needs_the_credentials)
        .collect()
}

/// Those of them whose `if:` is not the guard.
pub fn steps_that_need_the_credentials_and_are_not_gated(workflow: &str) -> Vec<WorkflowStep> {
    steps_that_need_the_credentials(workflow)
        .into_iter()
        .filter(|step| !step.cond.contains(CONFIGURED))
        .collect()
}
