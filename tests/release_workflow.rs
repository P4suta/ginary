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
//! Required OIDC retrieval replaces the old optional stored credentials.
//! Historical notice regressions use a fixture of that former workflow.
//!
//! Ungated: a workflow is neither half of the crate.

mod common;

use std::collections::BTreeMap;

use crate::common::release::{
    CURRENT_CLIENT_ID, CURRENT_PRIVATE_KEY, ENVIRONMENT, LEGACY_RELEASE_WORKFLOW,
    RELEASE_WORKFLOW as RELEASE, VERIFY_WORKFLOW, committed_workflows, credential_occurrences,
    credential_sites, job_environment, jobs_declaring_the_environment, normalized, notice_step,
    required_outputs_guard, steps_that_need_the_credentials_and_are_not_gated,
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
fn distribution_is_an_explicit_tagged_rehearsal_by_default() {
    let workflow = yaml(".github/workflows/distribute.yml");
    let events = &workflow["on"];
    assert!(events.as_mapping_get("release").is_none());
    for trigger in ["workflow_dispatch", "workflow_call"] {
        assert_eq!(
            events[trigger]["inputs"]["tag"]["required"].as_bool(),
            Some(true)
        );
        assert_eq!(
            events[trigger]["inputs"]["publish"]["default"].as_bool(),
            Some(false)
        );
    }
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
    let script = read("scripts/ci/publish-distribution.sh");
    assert!(
        script.contains("gh attestation verify") && script.contains("gh release download"),
        "the attestation is verified after re-download, not only produced"
    );
}

#[test]
fn distribution_reuses_an_existing_draft_and_publishes_only_after_verification() {
    let script = read("scripts/ci/publish-distribution.sh");
    assert!(!script.contains("gh release create"));
    let draft = script.find("--json isDraft").unwrap();
    let upload = script.find("gh release upload").unwrap();
    let verify = script.find("gh attestation verify").unwrap();
    let publish = script.find("--draft=false").unwrap();
    assert!(draft < upload && upload < verify && verify < publish);
    for step in workflow_steps(".github/workflows/distribute.yml") {
        if step.run.contains("publish-distribution.sh")
            || step.uses.contains("attest-build-provenance")
        {
            assert!(
                step.cond.contains("inputs.publish"),
                "hosted mutation requires an explicit publish option: {}",
                step.name
            );
        }
    }
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

const FETCH_ACTION: &str =
    "dopplerhq/secrets-fetch-action@451892f16195f9ac360e1a5bcbf0b5fd0e957534";
const APP_ACTION: &str = "actions/create-github-app-token@64ab729a6294000f5aa4a4bc0b73d1f32bc45510";

fn steps_of(workflow: &str, job: &str) -> Vec<WorkflowStep> {
    workflow_steps(workflow)
        .into_iter()
        .filter(|step| step.job == job)
        .collect()
}

fn assert_required_prefix(workflow: &str, job: &str) {
    let steps = steps_of(workflow, job);
    let fetch = &steps[0];
    let guard = &steps[1];
    assert_eq!(fetch.id, "credentials");
    assert_eq!(fetch.uses, FETCH_ACTION);
    assert!(fetch.cond.is_empty());
    for (key, expected) in [
        ("auth-method", "oidc"),
        ("doppler-project", "ginary"),
        ("doppler-config", "automation"),
        (
            "doppler-identity-id",
            "${{ vars.DOPPLER_AUTOMATION_IDENTITY_ID }}",
        ),
    ] {
        assert_eq!(
            fetch.with.get(key).map(String::as_str),
            Some(expected),
            "{workflow}: {key}"
        );
    }
    let parsed = yaml(workflow);
    let raw = &parsed["jobs"][job]["steps"];
    assert_eq!(raw[0]["with"]["inject-env-vars"].as_bool(), Some(false));
    assert!(
        parsed["jobs"][job]
            .as_mapping_get("continue-on-error")
            .is_none()
    );
    for index in 0..2 {
        assert!(raw[index].as_mapping_get("continue-on-error").is_none());
    }
    assert_eq!(guard.run.trim(), "exit 1");
    assert_eq!(guard.shell, "bash");
    assert_eq!(
        normalized(&guard.cond),
        normalized(&required_outputs_guard("credentials"))
    );
    assert!(guard.uses.is_empty());
    assert_eq!(
        parsed["jobs"][job]["permissions"]["id-token"].as_str(),
        Some("write")
    );
}

#[test]
fn release_authentication_fetches_once_and_fails_before_consuming_missing_fields() {
    assert_required_prefix(RELEASE, "release-please");
    let steps = steps_of(RELEASE, "release-please");
    assert_eq!(steps.len(), 4);
    let app = &steps[2];
    assert_eq!(app.uses, APP_ACTION);
    assert_eq!(app.id, "app-token");
    for field in [CURRENT_CLIENT_ID, CURRENT_PRIVATE_KEY] {
        let input = if field == CURRENT_CLIENT_ID {
            "client-id"
        } else {
            "private-key"
        };
        assert_eq!(
            app.with.get(input),
            Some(&format!("${{{{ steps.credentials.outputs.{field} }}}}"))
        );
    }
    assert_eq!(app.with["owner"], "${{ github.repository_owner }}");
    assert_eq!(
        app.with["repositories"],
        "${{ github.event.repository.name }}"
    );
    let granted: BTreeMap<_, _> = app
        .with
        .iter()
        .filter_map(|(key, value)| {
            key.strip_prefix("permission-")
                .map(|key| (key, value.as_str()))
        })
        .collect();
    assert_eq!(
        granted,
        BTreeMap::from([
            ("contents", "write"),
            ("pull-requests", "write"),
            ("issues", "write")
        ])
    );
    assert!(
        steps[3]
            .uses
            .starts_with("googleapis/release-please-action@")
    );
    assert_eq!(
        steps[3].with["token"],
        "${{ steps.app-token.outputs.token }}"
    );
    let parsed = yaml(RELEASE);
    for (index, step) in steps.iter().enumerate().skip(2) {
        assert!(step.cond.is_empty());
        assert!(
            parsed["jobs"]["release-please"]["steps"][index]
                .as_mapping_get("continue-on-error")
                .is_none()
        );
    }
    assert!(steps_that_need_the_credentials_and_are_not_gated(RELEASE).is_empty());
}

#[test]
fn only_release_and_manual_verification_jobs_declare_the_release_environment() {
    let declaring: Vec<_> = jobs_declaring_the_environment(&committed_workflows())
        .into_iter()
        .map(|job| (job.workflow, job.id))
        .collect();
    assert_eq!(
        declaring,
        vec![
            (RELEASE.to_owned(), "release-please".to_owned()),
            (VERIFY_WORKFLOW.to_owned(), "automation_0".to_owned()),
        ]
    );
    let verify = yaml(VERIFY_WORKFLOW);
    let triggers = verify["on"].as_mapping().unwrap();
    assert_eq!(triggers.len(), 2);
    assert!(verify["on"].as_mapping_get("workflow_dispatch").is_some());
    assert_eq!(verify["on"]["push"]["branches"][0].as_str(), Some("main"));
    assert_eq!(
        verify["on"]["push"]["paths"][0].as_str(),
        Some(VERIFY_WORKFLOW)
    );
    assert_eq!(
        normalized(verify["jobs"]["automation_0"]["if"].as_str().unwrap()),
        "github.event_name=='workflow_dispatch'&&github.ref=='refs/heads/main'"
    );
}

#[test]
fn no_job_of_any_workflow_reads_the_release_credentials_without_declaring_the_environment() {
    let sites = credential_sites(&committed_workflows());
    assert!(sites.iter().any(|site| site.workflow == RELEASE));
    assert!(sites.iter().any(|site| site.workflow == VERIFY_WORKFLOW));
    for site in sites {
        assert!(!site.job.is_empty(), "{}: {}", site.workflow, site.path);
        assert_eq!(job_environment(&site.workflow, &site.job), ENVIRONMENT);
    }
    for workflow in committed_workflows() {
        for count in credential_occurrences(&workflow) {
            assert_eq!(
                count.found, count.written,
                "{workflow}: {}",
                count.credential
            );
        }
    }
}

#[test]
fn authentication_verification_has_only_read_only_and_fixed_identity_access_checks() {
    assert_required_prefix(VERIFY_WORKFLOW, "automation_0");
    let steps = steps_of(VERIFY_WORKFLOW, "automation_0");
    assert_eq!(steps.len(), 5);
    let app = &steps[2];
    assert_eq!(app.uses, APP_ACTION);
    let permissions: BTreeMap<_, _> = app
        .with
        .iter()
        .filter_map(|(key, value)| {
            key.strip_prefix("permission-")
                .map(|key| (key, value.as_str()))
        })
        .collect();
    assert_eq!(permissions, BTreeMap::from([("contents", "read")]));
    assert_eq!(app.with["owner"], "${{ github.repository_owner }}");
    assert_eq!(
        app.with["repositories"],
        "${{ github.event.repository.name }}"
    );
    let negative = &steps[3];
    assert_eq!(negative.id, "forbidden");
    assert_eq!(negative.uses, FETCH_ACTION);
    assert_eq!(
        negative.with["doppler-identity-id"],
        steps[0].with["doppler-identity-id"]
    );
    assert!(
        negative.with["doppler-project"] != steps[0].with["doppler-project"]
            || negative.with["doppler-config"] != steps[0].with["doppler-config"]
    );
    let parsed = yaml(VERIFY_WORKFLOW);
    assert_eq!(
        parsed["jobs"]["automation_0"]["steps"][3]["continue-on-error"].as_bool(),
        Some(true)
    );
    assert_eq!(steps[4].run.trim(), "exit 1");
    assert_eq!(
        normalized(&steps[4].cond),
        "${{always()&&steps.forbidden.outcome!='failure'}}"
    );
    for step in [app, negative] {
        assert!(step.cond.is_empty());
        assert!(step.run.is_empty());
    }
}

#[test]
fn the_release_workflow_only_runs_on_refs_the_environment_admits() {
    let parsed = yaml(RELEASE);
    assert_eq!(parsed["on"].as_mapping().unwrap().len(), 1);
    assert_eq!(parsed["on"]["push"]["branches"][0].as_str(), Some("main"));
    assert_eq!(parsed["on"]["push"]["branches"].as_vec().unwrap().len(), 1);
    for job in workflow_jobs(RELEASE) {
        assert!(job.cond.is_empty());
    }
    for forbidden in [
        "RELEASE_PLEASE_APP_CLIENT_ID",
        "RELEASE_PLEASE_APP_PRIVATE_KEY",
        "secrets.GITHUB_TOKEN",
        "github.token",
    ] {
        assert!(!read(RELEASE).contains(forbidden), "{forbidden}");
    }
}

#[test]
fn legacy_missing_credentials_notice_keeps_its_fixture_record() {
    let rendered = notice_step(LEGACY_RELEASE_WORKFLOW)
        .unwrap()
        .run
        .trim_end()
        .to_owned();
    insta::assert_snapshot!("missing_credentials_notice", rendered);
}

#[test]
fn altered_credential_guards_and_unconditional_consumers_are_rejected() {
    let source = read(RELEASE);
    assert!(steps_that_need_the_credentials_and_are_not_gated(RELEASE).is_empty());
    for changed in [
        source.replacen("run: exit 1", "run: exit 0", 1),
        source.replacen("id: app-token", "id: app-token\n        if: always()", 1),
    ] {
        assert_ne!(source, changed);
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("workflow.yml");
        std::fs::write(&path, changed).unwrap();
        assert!(
            !steps_that_need_the_credentials_and_are_not_gated(path.to_str().unwrap()).is_empty()
        );
    }
}

#[test]
fn the_release_document_records_oidc_and_required_credential_failure() {
    let document = read("docs/RELEASE.md");
    let setup = document
        .split("## One-time setup")
        .nth(1)
        .unwrap()
        .split("\n## ")
        .next()
        .unwrap();
    for required in [
        "DOPPLER_AUTOMATION_IDENTITY_ID",
        "GITHUB_APP_CLIENT_ID",
        "GITHUB_APP_PRIVATE_KEY",
        "ginary.automation",
        "can_admins_bypass: false",
        "Nothing wider",
        "fail",
        "any job of any workflow",
    ] {
        assert!(setup.contains(required), "{required}");
    }
    assert!(!setup.contains("Environment secrets"));
}
