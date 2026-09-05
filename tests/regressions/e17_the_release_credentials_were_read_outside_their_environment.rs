// SPDX-License-Identifier: MIT OR Apache-2.0
//! `release.yml` read two credentials from a scope that did not hold them, and
//! reported the repository as unconfigured while it was configured.
//!
//! **What went wrong.** E5 wrote the release job against repository scope:
//! `vars.RELEASE_PLEASE_APP_CLIENT_ID` and
//! `secrets.RELEASE_PLEASE_APP_PRIVATE_KEY`, read from a job that declared no
//! `environment:`. Both values were then added to the `release` GitHub
//! Environment of `P4suta/ginary` instead — the right home for them, because
//! that environment's deployment-branch policy admits only the `main` branch
//! and the `v*` tags, so neither is reachable from a pull request, from a
//! fork, or from any other branch.
//!
//! A job's `vars` and `secrets` contexts carry an environment's values **only
//! when the job declares that environment**. The job declared none, so both
//! expressions expanded to the empty string, the guard concluded that no
//! credentials existed, `release-please` stayed skipped, and the
//! missing-credentials notice printed instructions to add the values in the
//! one place they must not go. The live `Release` run for `6ffb2be` did
//! exactly that, in seven seconds, and reported `success` — the failure had no
//! symptom a green run could show.
//!
//! **The input.** Any workflow of this repository that names either
//! credential from a job with no `environment: release`, and any guard on
//! those names written as a job-level `if:` — which is evaluated before the
//! job's environment is bound, and which cannot see a `secrets` context at
//! all.
//!
//! **The correct behaviour.** Every job that names either credential declares
//! `environment: release`, and nothing decides whether to run by reading those
//! names in a job condition. The decision is a step that reads what the
//! environment holds and publishes the answer, and every step that needs the
//! credentials is guarded on it.
//!
//! **The scan is the rule, so the scan is what has to be wide.** This file's
//! first version read `release.yml` alone, through four node kinds: a job's
//! `if:` and `env:`, and a step's `run:`, `if:`, `env:` and `with:`. Both
//! narrowings let the same bug back in with no symptom a green run could show
//! — a workflow-level `env:`, a job's `container.env` or a reusable call's
//! `with:`/`secrets:` was invisible to it, and so was any other workflow,
//! including the pull-request-triggered ones where a repository-scope read
//! would quietly get the empty string. It now walks every scalar of every
//! workflow, and holds that walk against the file text so that a node kind it
//! cannot reach fails loudly instead of passing.

use crate::common::repo::{
    NameSite, name_sites, read, workflow_jobs, yaml_files_under, yaml_text_occurrences,
};

/// The workflow the credentials are read in.
const RELEASE: &str = ".github/workflows/release.yml";

/// The variable holding the App's client id.
const CLIENT_ID_VAR: &str = "RELEASE_PLEASE_APP_CLIENT_ID";

/// The secret holding the App's private key.
const PRIVATE_KEY_SECRET: &str = "RELEASE_PLEASE_APP_PRIVATE_KEY";

/// The environment that holds both.
const ENVIRONMENT: &str = "release";

/// Every place any workflow of this repository writes either credential.
fn credential_sites() -> Vec<NameSite> {
    let mut out = Vec::new();
    for workflow in yaml_files_under(".github/workflows") {
        for credential in [CLIENT_ID_VAR, PRIVATE_KEY_SECRET] {
            out.extend(name_sites(&workflow, credential));
        }
    }
    out
}

/// The environment one job of one workflow declares, or `<none>`.
fn job_environment(workflow: &str, id: &str) -> String {
    workflow_jobs(workflow)
        .into_iter()
        .find(|job| job.id == id)
        .map(|job| job.environment)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "<none>".to_owned())
}

#[test]
fn every_job_that_names_a_release_credential_declares_the_environment_holding_it() {
    let sites = credential_sites();
    assert!(
        !sites.is_empty(),
        "no workflow of this repository names `{CLIENT_ID_VAR}` or `{PRIVATE_KEY_SECRET}`, so \
         nothing authenticates and nothing says that it cannot"
    );
    assert!(
        sites.iter().any(|site| site.workflow == RELEASE),
        "{RELEASE} is where the release credentials are read; every site found is somewhere else: \
         {sites:#?}"
    );
    for site in &sites {
        assert!(
            !site.job.is_empty(),
            "{} writes a release credential at `{}`, which belongs to no job and therefore to no \
             environment. A workflow-level `env:` cannot see a `secrets` context at all, and the \
             value every job inherits from it is the empty string",
            site.workflow,
            site.path
        );
        assert_eq!(
            job_environment(&site.workflow, &site.job),
            ENVIRONMENT,
            "{}: job `{}` reads a credential that lives in the `{ENVIRONMENT}` environment, at \
             `{}`, but declares `environment: {}`. Outside that declaration both \
             `vars.{CLIENT_ID_VAR}` and `secrets.{PRIVATE_KEY_SECRET}` expand to the empty string \
             however the environment is filled, which is a green run that says the repository is \
             unconfigured when it is not. Every site: {sites:#?}",
            site.workflow,
            site.job,
            site.path,
            job_environment(&site.workflow, &site.job)
        );
    }
}

#[test]
fn the_scan_behind_that_rule_reaches_every_occurrence_there_is() {
    for workflow in yaml_files_under(".github/workflows") {
        let text = read(&workflow);
        for credential in [CLIENT_ID_VAR, PRIVATE_KEY_SECRET] {
            let written = yaml_text_occurrences(&text, credential);
            let found: usize = name_sites(&workflow, credential)
                .iter()
                .map(|site| site.count)
                .sum();
            assert_eq!(
                found, written,
                "{workflow} writes `{credential}` {written} times outside a whole-line comment \
                 and the walk reaches {found} of them. The rule above is exactly as wide as this \
                 walk, so an occurrence the walk cannot see is a credential read from a scope \
                 nothing checks — the bug this file exists for, in the shape it took the first \
                 time. If the extra occurrence is an inline comment, move it onto a line of its \
                 own; the walk cannot see comments and should not have to"
            );
        }
    }
}

#[test]
fn no_job_condition_decides_on_a_value_only_the_environment_holds() {
    for job in workflow_jobs(RELEASE) {
        for credential in [CLIENT_ID_VAR, PRIVATE_KEY_SECRET] {
            assert!(
                !job.cond.contains(credential),
                "job `{}` reads `{credential}` in its own `if:`. A job condition is evaluated \
                 before the job's environment is bound — and a job `if:` cannot see a `secrets` \
                 context at all — so a guard written there answers about repository scope, which \
                 by design holds neither value. Its `if:` is `{}`",
                job.id,
                job.cond
            );
        }
    }
}
