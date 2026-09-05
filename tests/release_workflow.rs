// SPDX-License-Identifier: MIT OR Apache-2.0
//! The release and distribute workflows, held against the repository.
//!
//! These two workflows are authored, never run: an Action needs a remote, and
//! the house rule is that nothing is tagged or published without an explicit
//! request. So the deliverable is a workflow that is correct by inspection —
//! `release-please` drives the version bump and the draft, and `distribute.yml`
//! builds every artifact, checks it, and only then flips the release out of
//! draft. This file pins the discipline that makes "verify then publish" a
//! property of the YAML rather than of the person who runs it: the seven
//! targets, the checksums, the attestations, the re-download-and-check, and the
//! order the draft is flipped in.
//!
//! E5 adds the half the first live run found: `release-please` cannot open a
//! pull request with the default token on a repository that forbids Actions
//! from creating them, so the job authenticates as a GitHub App — and, until a
//! maintainer installs it, says so and stays green.
//!
//! Ungated: a workflow is neither half of the crate.

mod common;

use std::collections::BTreeMap;

use saphyr::YamlOwned;

use crate::common::release::{
    ABSENT, CHECK_STEP_ID, CLIENT_ID_VAR, CONFIGURED, ENVIRONMENT, PRIVATE_KEY_SECRET,
    RELEASE_WORKFLOW as RELEASE, committed_workflows, credential_occurrences, credential_sites,
    credentials_job, job_environment, jobs_declaring_the_environment, notice_step,
    steps_that_need_the_credentials,
};
use crate::common::repo::{WorkflowStep, read, workflow_jobs, workflow_steps, yaml};

// ------------------------------------------------------- release.yml --

#[test]
fn the_release_workflow_is_driven_by_release_please() {
    let release = read(".github/workflows/release.yml");
    assert!(
        release.contains("release-please"),
        "release.yml drives version bumps and the draft release through release-please:\n{release}"
    );
    assert!(
        release.contains("permissions: {}"),
        "release.yml sets the default permissions to none and widens per job"
    );
}

#[test]
fn a_published_release_triggers_the_distribute_workflow() {
    let release = read(".github/workflows/release.yml");
    let distribute = read(".github/workflows/distribute.yml");
    // distribute runs on `release: published` or is called by release.yml.
    assert!(
        distribute.contains("workflow_call") || distribute.contains("release:"),
        "distribute.yml runs on a published release or as a reusable workflow:\n{distribute}"
    );
    assert!(
        release.contains("distribute") || distribute.contains("workflow_call"),
        "the two workflows are wired together"
    );
}

// ---------------------------------------------------- distribute.yml --

#[test]
fn distribute_builds_every_target_of_the_release() {
    let distribute = read(".github/workflows/distribute.yml");
    for target in [
        "linux-x86_64-gnu",
        "linux-x86_64-musl",
        "linux-aarch64-gnu",
        "linux-aarch64-musl",
        "macos-x86_64",
        "macos-aarch64",
        "windows-x86_64",
    ] {
        assert!(
            distribute.contains(target),
            "distribute.yml does not produce artifacts for `{target}`; a release must carry all \
             seven"
        );
    }
}

#[test]
fn distribute_produces_both_the_binary_and_the_stub_and_the_otp_tarballs() {
    let distribute = read(".github/workflows/distribute.yml");
    assert!(
        distribute.contains("ginary-stub") || distribute.contains("--no-default-features"),
        "each target ships the launcher-only stub as well as the full binary"
    );
    assert!(
        distribute.contains("otp repack"),
        "the OTP catalog tarballs are produced by `ginary otp repack` on the right runner"
    );
}

#[test]
fn distribute_verifies_before_it_publishes() {
    let distribute = read(".github/workflows/distribute.yml");
    // The checksums, the attestation, the re-download check, and the flip.
    for needle in [
        "SHA256SUMS",
        "attest-build-provenance",
        "sha256sum",
        "--check",
    ] {
        assert!(
            distribute.contains(needle),
            "distribute.yml is missing `{needle}`: the verify-then-publish discipline is \
             incomplete"
        );
    }
    assert!(
        distribute.contains("attestation verify") || distribute.contains("gh attestation"),
        "the attestation is verified after re-download, not only produced"
    );
}

#[test]
fn distribute_creates_a_draft_first_and_flips_it_only_after_the_checks() {
    let distribute = read(".github/workflows/distribute.yml");
    let draft = distribute
        .find("draft: true")
        .or_else(|| distribute.find("--draft"))
        .expect("the release is created as a draft first");
    let flip = distribute
        .find("draft=false")
        .or_else(|| distribute.find("draft: false"))
        .or_else(|| distribute.find("--draft=false"))
        .expect("and the draft is flipped to a published release at the end");
    assert!(
        draft < flip,
        "the draft is created before it is flipped: an artifact that fails its checks never \
         becomes a published release"
    );
    let check = distribute
        .find("--check")
        .expect("sha256sum --check runs on the re-downloaded assets");
    assert!(
        check < flip,
        "the checksum re-check comes before the flip, or a bad asset would already be public"
    );
}

#[test]
fn distribute_runs_the_version_consistency_check() {
    let distribute = read(".github/workflows/distribute.yml");
    assert!(
        distribute.contains("version-consistency.sh"),
        "the tag and Cargo.toml are proved equal before anything is uploaded:\n{distribute}"
    );
}

// ------------------------------------------------ the release credentials --

/// The action that mints a repository-scoped installation token.
const APP_TOKEN_ACTION: &str = "actions/create-github-app-token";

/// The commit `actions/create-github-app-token` v3.2.0 points at.
///
/// Resolved with `git ls-remote --tags` against the action's own repository
/// rather than copied from a sibling project, and it is also what `v3` points
/// at today. A pin nobody verified is a pin to whatever the last force-push
/// left behind.
const APP_TOKEN_SHA: &str = "bcd2ba49218906704ab6c1aa796996da409d3eb1";

/// The repository the App has to be installed on.
const REPOSITORY: &str = "P4suta/ginary";

/// The job id of the step that runs `release-please`.
fn release_please_job() -> String {
    let parsed = yaml(RELEASE);
    let jobs = parsed
        .as_mapping_get("jobs")
        .and_then(YamlOwned::as_mapping)
        .expect("release.yml declares jobs");
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

/// The steps of one job of `release.yml`, in file order.
fn steps_of(job: &str) -> Vec<WorkflowStep> {
    workflow_steps(RELEASE)
        .into_iter()
        .filter(|step| step.job == job)
        .collect()
}

/// Whether one job of `release.yml` is excused from failing.
///
/// A job that is allowed to fail is green whatever its steps do, which would
/// make "the notice exits 0" true and meaningless. Anything but a literal
/// `false` counts as excused: `continue-on-error` also takes an expression,
/// and a helper that read only booleans would answer "not excused" for
/// `${{ true }}`.
fn job_continues_on_error(id: &str) -> bool {
    let parsed = yaml(RELEASE);
    let Some(jobs) = parsed
        .as_mapping_get("jobs")
        .and_then(YamlOwned::as_mapping)
    else {
        return false;
    };
    for (key, job) in jobs {
        if key.as_str() == Some(id) {
            return match job.as_mapping_get("continue-on-error") {
                None => false,
                Some(value) => value.as_bool() != Some(false),
            };
        }
    }
    false
}

/// Every `with:` input of the step that mints the App token.
///
/// Read out of the parsed workflow and anchored to that one step, because the
/// question this answers is "what is the token scoped to" and a substring
/// search of the whole file answers "does the file say that anywhere" — which
/// a comment, or a second action, would also satisfy.
///
/// # Panics
///
/// If no step uses [`APP_TOKEN_ACTION`]: the workflow that mints no token has
/// a bigger problem than its scopes, and
/// `the_release_job_authenticates_with_a_github_app_token` is where it is
/// reported.
fn app_token_inputs() -> BTreeMap<String, String> {
    let parsed = yaml(RELEASE);
    let jobs = parsed
        .as_mapping_get("jobs")
        .and_then(YamlOwned::as_mapping)
        .expect("release.yml declares jobs");
    for (_, job) in jobs {
        let Some(steps) = job.as_mapping_get("steps").and_then(YamlOwned::as_vec) else {
            continue;
        };
        for step in steps {
            let uses = step
                .as_mapping_get("uses")
                .and_then(YamlOwned::as_str)
                .unwrap_or_default();
            if !uses.contains(APP_TOKEN_ACTION) {
                continue;
            }
            let mut inputs = BTreeMap::new();
            if let Some(with) = step.as_mapping_get("with").and_then(YamlOwned::as_mapping) {
                for (key, value) in with {
                    let key = key.as_str().unwrap_or_default().to_owned();
                    let value = value
                        .as_str()
                        .map(str::to_owned)
                        .or_else(|| value.as_bool().map(|flag| flag.to_string()))
                        .unwrap_or_default();
                    inputs.insert(key, value);
                }
            }
            return inputs;
        }
    }
    panic!("no step of {RELEASE} uses {APP_TOKEN_ACTION}");
}

#[test]
fn the_release_job_authenticates_with_a_github_app_token() {
    let release = read(RELEASE);
    let step = release
        .split("- uses:")
        .find(|chunk| chunk.contains(APP_TOKEN_ACTION))
        .unwrap_or_else(|| {
            panic!(
                "{RELEASE} mints no App token. The default GITHUB_TOKEN cannot open a pull \
                 request on this repository — `can_approve_pull_request_reviews` is false, which \
                 is the hardening E3 applied on purpose — so release-please failed with \
                 `GitHub Actions is not permitted to create or approve pull requests`.\n{release}"
            )
        });
    assert!(
        step.contains(&format!("{APP_TOKEN_ACTION}@{APP_TOKEN_SHA}")),
        "the App-token step is pinned to {APP_TOKEN_SHA} (v3.2.0), like every other `uses:` in \
         this repository:\n{step}"
    );
    assert!(
        step.contains("id: app-token"),
        "the App-token step carries `id: app-token`, or nothing can read its output:\n{step}"
    );
    assert!(
        step.contains(&format!("client-id: ${{{{ vars.{CLIENT_ID_VAR} }}}}")),
        "the client id comes from the variable `{CLIENT_ID_VAR}`:\n{step}"
    );
    assert!(
        step.contains(&format!(
            "private-key: ${{{{ secrets.{PRIVATE_KEY_SECRET} }}}}"
        )),
        "the private key comes from the secret `{PRIVATE_KEY_SECRET}`:\n{step}"
    );
}

#[test]
fn release_please_runs_on_the_app_token_and_never_on_the_default_one() {
    let release = read(RELEASE);
    assert!(
        release.contains("token: ${{ steps.app-token.outputs.token }}"),
        "release-please is handed the App token explicitly; without a `token:` input it falls \
         back to the workflow's own GITHUB_TOKEN, which is the token that just failed:\n{release}"
    );
    for forbidden in ["secrets.GITHUB_TOKEN", "github.token"] {
        assert!(
            !release.contains(forbidden),
            "{RELEASE} still names `{forbidden}`: the default token is the one this repository \
             forbids from creating pull requests"
        );
    }
}

// ------------------------------------- the credentials live in an environment --

#[test]
fn the_credential_using_job_declares_the_release_environment() {
    let job = release_please_job();
    assert_eq!(
        job_environment(RELEASE, &job),
        ENVIRONMENT,
        "job `{job}` reads `{CLIENT_ID_VAR}` and `{PRIVATE_KEY_SECRET}`, and both live in the \
         `{ENVIRONMENT}` environment of {REPOSITORY} rather than at repository scope. A job's \
         `vars` and `secrets` contexts carry an environment's values only when the job declares \
         that environment, so without `environment: {ENVIRONMENT}` the job reads two empty \
         strings and release-please never runs — which is exactly what the live run of the merged \
         tree did"
    );
}

#[test]
fn exactly_one_job_of_this_repository_declares_the_release_environment() {
    let declaring: Vec<String> = jobs_declaring_the_environment(&committed_workflows())
        .into_iter()
        .map(|job| format!("{}: {}", job.workflow, job.id))
        .collect();
    assert_eq!(
        declaring,
        vec![format!("{RELEASE}: release-please")],
        "the `{ENVIRONMENT}` environment holds `{CLIENT_ID_VAR}` and `{PRIVATE_KEY_SECRET}`, and \
         every job that declares it is handed both on every ref the deployment-branch policy \
         admits. Declaring it is not a privilege GitHub hands to one job — any job of any \
         workflow may write `environment: {ENVIRONMENT}` and be handed the same two values — so \
         the bound on how many do is this repository's own, and this is it. \
         `no_job_of_any_workflow_reads_the_release_credentials_without_declaring_the_environment` \
         is the other half and does not cover this one: it requires a job that *names* a \
         credential to declare the environment, which a second declaring job does. If a later \
         milestone needs a second environment-bound job, give it an environment of its own rather \
         than widening this list; `docs/RELEASE.md` says so under `## One-time setup` and cites \
         this test by name"
    );
}

#[test]
fn no_job_of_any_workflow_reads_the_release_credentials_without_declaring_the_environment() {
    let sites = credential_sites(&committed_workflows());
    assert!(
        !sites.is_empty(),
        "no workflow of this repository names either credential, so nothing authenticates and \
         nothing reports that it cannot"
    );
    for site in &sites {
        assert!(
            !site.job.is_empty(),
            "{} writes a release credential at `{}`, outside every job. A workflow-level `env:` \
             belongs to no job and therefore to no environment — and the `secrets` context is not \
             visible there at all — so the value expands to nothing and every job that inherits it \
             gets an empty string",
            site.workflow,
            site.path
        );
        assert_eq!(
            job_environment(&site.workflow, &site.job),
            ENVIRONMENT,
            "{}: job `{}` writes a release credential at `{}` but declares no `environment: \
             {ENVIRONMENT}`. Both values live in that environment, and a job's `vars` and \
             `secrets` contexts carry an environment's values only when the job declares it, so \
             this one reads the empty string however the environment is filled — a green run that \
             says the repository is unconfigured when it is not. Every site: {sites:#?}",
            site.workflow,
            site.job,
            site.path
        );
    }
}

#[test]
fn every_occurrence_of_a_release_credential_is_accounted_for() {
    for workflow in committed_workflows() {
        for count in credential_occurrences(&workflow) {
            let (credential, written, found) = (count.credential, count.written, count.found);
            assert_eq!(
                found, written,
                "{workflow} writes `{credential}` {written} times outside a whole-line comment, \
                 and the scan behind \
                 `no_job_of_any_workflow_reads_the_release_credentials_without_declaring_the_environment` \
                 accounts for {found} of them. That rule is only as wide as this walk, so an \
                 occurrence it cannot reach is a credential read from a scope nothing checks — \
                 which is E17's own bug, and it has no symptom a green run could show. If the \
                 extra occurrence is an inline comment, the two counts are allowed to disagree \
                 for no reason worth reporting: move it onto a line of its own"
            );
        }
    }
}

#[test]
fn the_release_credentials_are_not_read_from_a_job_level_if() {
    for job in workflow_jobs(RELEASE) {
        for credential in [CLIENT_ID_VAR, PRIVATE_KEY_SECRET] {
            assert!(
                !job.cond.contains(credential),
                "job `{}` decides whether to run by reading `{credential}` in its own `if:`, \
                 which is E5's guard and no longer works: a job condition is evaluated before the \
                 job's environment is bound, so an environment-scoped value cannot be relied on \
                 there — and a `secrets` context is not visible to a job `if:` at all. The guard \
                 belongs on the steps, behind a step that reads the environment and says what it \
                 found. Its `if:` is `{}`",
                job.id,
                job.cond
            );
        }
    }
}

#[test]
fn the_credentials_check_is_the_first_step_and_publishes_what_it_found() {
    let job = release_please_job();
    let steps = steps_of(&job);
    let first = steps
        .first()
        .unwrap_or_else(|| panic!("job `{job}` of {RELEASE} has no steps"));
    assert_eq!(
        first.id, CHECK_STEP_ID,
        "the first step of `{job}` is the one that reads what the `{ENVIRONMENT}` environment \
         holds, and it carries `id: {CHECK_STEP_ID}` so that every later step can be guarded on \
         its answer. It is `{}`",
        first.name
    );
    assert_eq!(
        first.cond, "",
        "the credentials check itself is unguarded — it is what computes the guard. Its `if:` is \
         `{}`",
        first.cond
    );
    for credential in [CLIENT_ID_VAR, PRIVATE_KEY_SECRET] {
        assert!(
            first.env.values().any(|value| value.contains(credential)),
            "the check reads `{credential}` through the step's `env:`; `secrets` is not a context \
             a shell can reach on its own, and reading a value straight into a `run:` block would \
             interpolate a private key into a script. Its `env:` is {:?}",
            first.env
        );
    }
    assert!(
        first.run.contains("$GITHUB_OUTPUT") || first.run.contains("${GITHUB_OUTPUT}"),
        "the check writes what it found to `$GITHUB_OUTPUT`, or no later step can be guarded on \
         it:\n{}",
        first.run
    );
    assert!(
        first.run.contains("state=configured") && first.run.contains("state=absent"),
        "the check publishes both states under the name every guard reads, `state`: \
         `configured` when the environment holds both credentials and `absent` when it holds \
         neither. The third state, a variable with no secret behind it, is a failure rather than \
         an output:\n{}",
        first.run
    );
}

#[test]
fn every_step_that_needs_the_credentials_is_gated_on_the_check() {
    // The job the rule actually scanned, which is the job holding the
    // credentials check rather than the job running release-please. Naming the
    // second here would report a job the scan need not have looked at, and
    // would panic before the credential rule was ever evaluated if the
    // release-please step moved — the exact coupling the helper's own doc
    // gives as the reason it finds the job by the check.
    let job = credentials_job(RELEASE)
        .unwrap_or_else(|| panic!("no job of {RELEASE} reads what the release environment holds"));
    // Every step of every job that can be handed the credentials, but the two
    // that are the guard itself: the check, which computes the answer and so
    // cannot be guarded on it, and the notice, which runs on the complement.
    // Everything else either uses an action, and needs the credentials to be
    // there, or names a credential in a `run:`, an `env:` or a `with:` of its
    // own — a `run:` step that reads the private key out of its `env:` and
    // calls the API needs the guard exactly as much as the App-token step
    // does, and the `!uses.is_empty()` filter this rule used to carry excused
    // it.
    //
    // The filter itself is `crate::common::release`'s, and not this file's,
    // because it was written here and copied into the E5 regression, and the
    // copy that was wrong was wrong in both. See
    // `tests/regressions/e18_a_step_that_was_not_the_notice_wore_the_notice_guard.rs`
    // and
    // `tests/regressions/e18_a_credential_reading_step_outside_the_check_job_was_not_scanned.rs`.
    let guarded = steps_that_need_the_credentials(RELEASE);
    assert!(
        !guarded.is_empty(),
        "job `{job}` uses no action at all, so there is nothing for the credentials to be for"
    );
    for step in guarded {
        assert!(
            step.cond.contains(CONFIGURED),
            "step {} of `{job}` (`{}`) runs whatever the `{ENVIRONMENT}` environment holds. Since \
             the guard left the job's `if:` it has to be on every step that needs the \
             credentials, or a repository with none reaches {APP_TOKEN_ACTION} with an empty \
             `client-id:` and fails on a message about signing — and a `run:` step reading an \
             empty private key does something worse than fail. Its `if:` is `{}`, and the guard \
             is `{CONFIGURED}`",
            step.position,
            step.name,
            step.cond
        );
    }
}

#[test]
fn the_release_workflow_only_runs_on_refs_the_environment_admits() {
    let parsed = yaml(RELEASE);
    let triggers = parsed
        .as_mapping_get("on")
        .or_else(|| parsed.as_mapping_get("true"))
        .and_then(YamlOwned::as_mapping)
        .unwrap_or_else(|| panic!("{RELEASE} declares no `on:` triggers"));
    let mut names: Vec<String> = triggers
        .iter()
        .filter_map(|(key, _)| key.as_str().map(str::to_owned))
        .collect();
    names.sort();
    assert_eq!(
        names,
        vec!["push".to_owned()],
        "`{RELEASE}` binds a job to the `{ENVIRONMENT}` environment, whose deployment-branch \
         policy admits the `main` branch and the `v*` tags and nothing else. A job that declares \
         an environment the current ref may not deploy to does not skip: the run fails with \
         `Branch is not allowed to deploy to {ENVIRONMENT} due to environment protection rules`. \
         So every trigger this workflow carries has to produce an admitted ref, and a \
         `pull_request` or a `workflow_dispatch` trigger cannot promise that"
    );
    let branches = triggers
        .iter()
        .find(|(key, _)| key.as_str() == Some("push"))
        .and_then(|(_, value)| value.as_mapping_get("branches"))
        .and_then(YamlOwned::as_vec)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_owned))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    assert_eq!(
        branches,
        vec!["main".to_owned()],
        "the push trigger names exactly `main`, the one branch the `{ENVIRONMENT}` environment's \
         policy admits"
    );
}

// ---------------------------------------- what an unconfigured repository sees --

#[test]
fn a_repository_without_the_credentials_is_told_what_to_add_and_the_job_stays_green() {
    let step = notice_step(RELEASE).unwrap_or_else(|| {
        panic!(
            "no step of {RELEASE} names both `{CLIENT_ID_VAR}` and `{PRIVATE_KEY_SECRET}` \
             without failing. A repository whose `{ENVIRONMENT}` environment holds no \
             credentials must have a *green* Release workflow that says why it did nothing, not \
             a red one and not a silent one."
        )
    });
    // The guard, and nothing about the step's own text. `notice_step` finds
    // the step *by* the credential names it prints, so any assertion that
    // reads `step.run` here is satisfied by the search that produced it — that
    // is the tautology this test used to be. The complementarity of the two
    // guards has a regression file of its own,
    // `tests/regressions/e5_the_credentials_notice_was_not_tied_to_the_missing_credentials.rs`.
    assert!(
        step.cond.contains(ABSENT),
        "the notice has to be reachable exactly when the environment holds no credentials; step \
         {} of `{}` is guarded by `{}` and the guard is `{ABSENT}`",
        step.position,
        step.job,
        step.cond
    );
    assert!(
        step.run.contains(REPOSITORY),
        "the notice names the repository the App is installed on ({REPOSITORY}), so a maintainer \
         reading it does not have to guess:\n{}",
        step.run
    );
    assert!(
        !job_continues_on_error(&step.job),
        "job `{}` sets `continue-on-error`, which makes it green whatever it does. The notice is \
         green because it succeeds, not because failure was excused",
        step.job
    );
    // Scoped to the notice's own script: the release-please job deliberately
    // *does* exit non-zero when the client id is set and the private key is
    // not, and a whole-file search cannot tell the two apart.
    for command in step.commands() {
        let non_zero_exit = command.starts_with("exit ") && command != "exit 0";
        assert!(
            !non_zero_exit && command != "false",
            "the notice exits 0. A missing credential is a state to report, not a failure; this \
             line is `{command}`:\n{}",
            step.run
        );
    }
}

#[test]
fn the_notice_sends_a_maintainer_to_the_environment_and_not_to_repository_scope() {
    let step = notice_step(RELEASE)
        .unwrap_or_else(|| panic!("no step of {RELEASE} prints the missing-credentials notice"));
    // The needle is the whole click-path down to the environment's own name,
    // and not the name on its own: `notice_step` finds this step by the text
    // it prints, that text has to contain `release-please` whatever else it
    // says, and a `contains("release")` is therefore true by construction of
    // the search — the tautology class E5 already paid for once. Only a
    // maintainer being sent to *this* environment produces `Environments ->
    // release`. See
    // `tests/regressions/e17_the_notice_named_the_environment_by_accident.rs`.
    let path = format!("Environments -> {ENVIRONMENT}");
    assert!(
        step.run.contains(&path),
        "the notice sends a maintainer to `{path}`, naming the environment both values go in. \
         `Settings -> Environments` alone is a page with every environment on it, and the name on \
         its own is a word this notice cannot avoid printing:\n{}",
        step.run
    );
    assert!(
        step.run.contains(&format!("Settings -> {path}")),
        "the notice gives the whole path a maintainer clicks, `Settings -> {path}`, and not the \
         Actions secrets page the E5 wording sent them to:\n{}",
        step.run
    );
    for wrong in ["repository variable", "repository secret"] {
        assert!(
            !step.run.contains(wrong),
            "the notice still says `{wrong}`. Repository scope is the one place these two must \
             not go: it is readable from a pull-request run and from a fork, which is the \
             property the environment exists to deny:\n{}",
            step.run
        );
    }
}

#[test]
fn the_missing_credentials_notice_is_the_committed_record() {
    let rendered = notice_step(RELEASE).map_or_else(
        || format!("(no step of {RELEASE} names both credentials)"),
        |step| step.run.trim_end().to_owned(),
    );
    insta::assert_snapshot!("missing_credentials_notice", rendered);
}

#[test]
fn the_app_token_is_scoped_to_what_release_please_needs() {
    let inputs = app_token_inputs();
    let granted: BTreeMap<&str, &str> = inputs
        .iter()
        .filter_map(|(key, value)| {
            key.strip_prefix("permission-")
                .map(|scope| (scope, value.as_str()))
        })
        .collect();
    let expected = BTreeMap::from([
        ("contents", "write"),
        ("pull-requests", "write"),
        ("issues", "write"),
    ]);
    assert_eq!(
        granted, expected,
        "the installation token is narrowed to exactly the three scopes release-please uses — it \
         writes the version bump and the changelog, opens and updates the pull request, and \
         labels it. Both halves of that are the assertion: a missing scope breaks the release, \
         and an extra one is an App credential with more rights over this repository than the \
         job it exists for. `docs/RELEASE.md` promises `Nothing wider` in those words"
    );
}

#[test]
fn a_half_configured_repository_is_told_which_credential_is_missing() {
    let step = workflow_steps(RELEASE)
        .into_iter()
        .find(|step| step.job == release_please_job() && step.run.contains(PRIVATE_KEY_SECRET))
        .unwrap_or_else(|| {
            panic!(
                "no step of job `{}` checks `{PRIVATE_KEY_SECRET}`. A guard that reads only the \
                 client id proves nothing about the private key: add the variable to the \
                 `{ENVIRONMENT}` environment, forget the secret, and the job fails inside \
                 `{APP_TOKEN_ACTION}` on a message about signing rather than on the name of the \
                 credential nobody added",
                release_please_job()
            )
        });
    assert_eq!(
        step.position, 1,
        "the private-key check is the first step of `{}`: a missing credential is worth finding \
         before a checkout, not after",
        step.job
    );
    assert!(
        step.env
            .values()
            .any(|value| value.contains(&format!("secrets.{PRIVATE_KEY_SECRET}"))),
        "the check reads the secret through the step's `env:`, because `secrets` is not a context \
         a step `if:` can see:\n{:?}",
        step.env
    );
    assert!(
        step.run.contains("exit 1"),
        "a repository that put `{CLIENT_ID_VAR}` in the `{ENVIRONMENT}` environment asked for \
         release automation, so a missing `{PRIVATE_KEY_SECRET}` is a failure and not a notice — \
         the one state this workflow does report red:\n{}",
        step.run
    );
}

#[test]
fn the_notice_script_exits_zero_under_the_shell_that_runs_it() {
    let Some(step) = notice_step(RELEASE) else {
        panic!("no step of {RELEASE} names both credentials");
    };
    // GitHub runs a `run:` block on Linux as `bash -e -o pipefail {0}`, so the
    // last command's status is the step's status and any command failing ends
    // it. Reading the script and asserting it has no `exit 1` in it says
    // nothing about that; running it does.
    //
    // The gate is `require_posix_shell` and not a `Command::new("bash")` whose
    // failure is caught: `bash` is a `PATH` lookup, and on a Windows runner
    // the first one on `PATH` is `C:\Windows\System32\bash.exe`, the WSL
    // launcher, which starts, exits `1` in silence when no distribution is
    // installed, and made this test report the Release workflow as red. A
    // machine with no POSIX shell cannot answer the question at all, so it
    // says so.
    let Some(_shell) = crate::common::tools::require_posix_shell() else {
        return;
    };
    let run = std::process::Command::new("bash")
        .args(["-e", "-o", "pipefail", "-c", &step.run])
        .output();
    let Ok(output) = run else {
        eprintln!("skipping: no `bash` to run the notice script under");
        return;
    };
    assert!(
        output.status.success(),
        "the notice script exits {:?} under `bash -e -o pipefail`, so the Release workflow of a \
         repository with no credentials is red. stderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn the_release_document_records_the_one_time_credential_setup() {
    let document = read("docs/RELEASE.md");
    assert!(
        document.contains("## One-time setup"),
        "docs/RELEASE.md gains a `## One-time setup` section: the App and its two credentials \
         are the one part of cutting a release that no workflow can do for itself"
    );
    for needle in [
        CLIENT_ID_VAR,
        PRIVATE_KEY_SECRET,
        REPOSITORY,
        "create-github-app-token",
    ] {
        assert!(
            document.contains(needle),
            "docs/RELEASE.md does not mention `{needle}`, so the setup it describes is not one a \
             maintainer could carry out"
        );
    }
}

#[test]
fn the_release_document_sends_the_credentials_to_the_environment() {
    let document = read("docs/RELEASE.md");
    for needle in [
        "Settings -> Environments",
        "`release` environment",
        "Environment variables",
        "Environment secrets",
    ] {
        assert!(
            document.contains(needle),
            "docs/RELEASE.md does not mention `{needle}`. Its `## One-time setup` still describes \
             the repository-scope route E17 replaced, so a maintainer following it would add both \
             values where the workflow cannot read them"
        );
    }
    for wrong in ["repository variable", "repository secret"] {
        assert!(
            !document.contains(wrong),
            "docs/RELEASE.md still tells a maintainer to add a `{wrong}`. A value at repository \
             scope is handed to a job that declared nothing, with no branch policy in front of \
             it; the `{ENVIRONMENT}` environment's values reach only a job that declares that \
             environment, and only on `main` or a `v*` tag"
        );
    }
    // The section, and not the rest of the document behind it. `## The three
    // steps` says "review that pull request" and "the release pull request",
    // so a search over everything after the heading answers yes to `pull
    // request` however the setup section is written — deleting the paragraph
    // that gives the reason would leave half of this assertion satisfied by
    // prose about something else.
    let setup = document
        .split("## One-time setup")
        .nth(1)
        .unwrap_or_default()
        .split("\n## ")
        .next()
        .unwrap_or_default();
    for reason in ["fork", "pull request"] {
        assert!(
            setup.contains(reason),
            "`## One-time setup` says which route to take and not why. It has to name the \
             property that decides it — that neither value is reachable from a `{reason}` — or \
             the next maintainer moves them back to repository scope for convenience"
        );
    }
}
