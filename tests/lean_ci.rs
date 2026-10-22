// SPDX-License-Identifier: MIT OR Apache-2.0

mod common;

use common::repo::yaml;

const CAMPAIGNS: &[&str] = &[
    "test",
    "smoke",
    "cross-build",
    "smoke-matrix",
    "macos",
    "windows",
    "coverage",
    "formal",
    "mutants",
    "fuzz",
];

#[test]
fn routine_changes_never_start_release_campaigns() {
    let ci = yaml(".github/workflows/ci.yml");
    let triggers = &ci["on"];
    for event in [
        "pull_request",
        "merge_group",
        "workflow_dispatch",
        "workflow_call",
    ] {
        assert!(triggers.as_mapping_get(event).is_some(), "missing {event}");
    }
    for event in ["push", "schedule"] {
        assert!(
            triggers.as_mapping_get(event).is_none(),
            "unexpected {event}"
        );
    }
    assert_eq!(
        ci["concurrency"]["cancel-in-progress"].as_str(),
        Some("${{ inputs.assurance != true }}")
    );
    for campaign in CAMPAIGNS {
        assert_eq!(
            ci["jobs"][*campaign]["if"].as_str(),
            Some("${{ inputs.assurance == true }}"),
            "{campaign} must require an explicit assurance run"
        );
    }
}

#[test]
fn distribution_requires_same_source_assurance() {
    let distribution = yaml(".github/workflows/distribute.yml");
    let jobs = &distribution["jobs"];
    assert_eq!(jobs["assurance"]["needs"].as_str(), Some("version-check"));
    assert_eq!(
        jobs["assurance"]["uses"].as_str(),
        Some("./.github/workflows/ci.yml")
    );
    assert_eq!(jobs["assurance"]["with"]["assurance"].as_bool(), Some(true));
    assert!(
        jobs["publish"]["needs"]
            .as_vec()
            .expect("distribution prerequisites")
            .iter()
            .any(|job| job.as_str() == Some("assurance"))
    );
    assert!(
        jobs["publish"]
            .as_mapping_get("if")
            .and_then(|node| node.as_str())
            .is_none_or(|condition| !condition.contains("always()"))
    );
}

#[test]
fn parser_and_security_workflows_have_no_schedules() {
    for path in [
        ".github/workflows/nightly.yml",
        ".github/workflows/codeql.yml",
        ".github/workflows/scorecard.yml",
    ] {
        let workflow = yaml(path);
        assert!(
            workflow["on"].as_mapping_get("schedule").is_none(),
            "{path}"
        );
    }
    let fuzz = yaml(".github/workflows/nightly.yml");
    let jobs = fuzz["jobs"].as_mapping().expect("parser jobs");
    assert_eq!(
        jobs.len(),
        1,
        "the parser workflow must not repeat other campaigns"
    );
}
