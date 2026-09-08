// SPDX-License-Identifier: MIT OR Apache-2.0
//! A manual distribution run checked out an arbitrary input tag while its
//! workflow and cache scope still belonged to the default branch. A tag input
//! must name the actual triggering tag, and every checkout must use that
//! event's revision. Reusable workflows inherit the caller's event context.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crate::common::bounded::run_bounded;
use crate::common::repo::{WorkflowStep, workflow_steps, yaml};

const WORKFLOW: &str = ".github/workflows/distribute.yml";

fn execution_guard() -> WorkflowStep {
    workflow_steps(WORKFLOW)
        .into_iter()
        .find(|step| step.job == "version-check" && step.id == "execution-context")
        .expect("distribution must refuse a mismatched event context before checking out code")
}

fn bash() -> Option<PathBuf> {
    std::env::var_os("ProgramFiles")
        .map(PathBuf::from)
        .map(|path| path.join("Git/bin/bash.exe"))
        .filter(|path| path.is_file())
        .or_else(|| {
            crate::common::tools::require_tools(&["bash"])
                .map(|tools| tools.path("bash").to_owned())
        })
}

#[test]
fn the_event_context_guard_runs_before_any_checkout_and_cannot_be_skipped() {
    let guard = execution_guard();
    assert_eq!(
        guard.position, 1,
        "no checkout or repository code may precede the guard"
    );
    assert!(
        guard.uses.is_empty(),
        "the guard must not load a repository action"
    );
    assert_eq!(guard.shell, "bash");
    assert!(
        guard.cond.is_empty(),
        "a skipped guard must not permit distribution"
    );
    assert_eq!(guard.env["RELEASE_TAG"], "${{ inputs.tag }}");
    assert!(
        !guard.run.contains("${{"),
        "untrusted tag input belongs in the environment, never in shell source"
    );

    let workflow = yaml(WORKFLOW);
    let version = &workflow["jobs"]["version-check"];
    let first = &version["steps"].as_sequence().expect("version-check steps")[0];
    for node in [version, first] {
        assert!(
            node.as_mapping_get("continue-on-error").is_none(),
            "context refusal must fail the version-check job"
        );
    }
}

#[test]
fn the_real_guard_accepts_only_the_requested_tag_in_the_callers_tag_context() {
    let guard = execution_guard();
    let Some(shell) = bash() else {
        return;
    };
    let work = tempfile::tempdir().expect("isolated guard rehearsal");
    let script = work.path().join("guard.sh");
    std::fs::write(
        &script,
        format!("{}\nprintf 'context-accepted\\n'\n", guard.run),
    )
    .expect("write the exact workflow guard");

    // A workflow_call retains its caller's event name, ref and SHA. A caller
    // on main does not gain the tag's context by naming a tagged reusable file.
    for (event, reference, kind, requested, accepted) in [
        (
            "workflow_dispatch",
            "refs/tags/v0.1.0",
            "tag",
            "v0.1.0",
            true,
        ),
        ("push", "refs/tags/v0.1.0", "tag", "v0.1.0", true),
        ("push", "refs/tags/v0.1.0-RC1", "tag", "v0.1.0-RC1", true),
        ("push", "refs/tags/v0.1.0-RC1", "tag", "v0.1.0-rc1", false),
        ("workflow_dispatch", "refs/tags/0.1.0", "tag", "0.1.0", true),
        (
            "workflow_dispatch",
            "refs/heads/main",
            "branch",
            "v0.1.0",
            false,
        ),
        ("push", "refs/heads/main", "branch", "v0.1.0", false),
        (
            "workflow_dispatch",
            "refs/tags/v0.2.0",
            "tag",
            "v0.1.0",
            false,
        ),
        (
            "workflow_dispatch",
            "refs/heads/v0.1.0",
            "branch",
            "v0.1.0",
            false,
        ),
        (
            "workflow_dispatch",
            "refs/tags/v0.1.0",
            "branch",
            "v0.1.0",
            false,
        ),
        (
            "workflow_dispatch",
            "refs/tags/v0.1.0",
            "tag",
            "refs/tags/v0.1.0",
            false,
        ),
        (
            "workflow_dispatch",
            "refs/tags/v0.1.0",
            "tag",
            "HEAD~1",
            false,
        ),
        (
            "workflow_dispatch",
            "refs/tags/v0.1.0",
            "tag",
            "0123456789abcdef0123456789abcdef01234567",
            false,
        ),
        ("workflow_dispatch", "refs/tags/v0.1.0", "tag", "", false),
        (
            "workflow_dispatch",
            "refs/tags/v0.1.0",
            "tag",
            "v0.1.0\nother",
            false,
        ),
    ] {
        let output = run_bounded(
            Command::new(&shell)
                .args(["--noprofile", "--norc", "-e", "-o", "pipefail"])
                .arg(&script)
                .current_dir(work.path())
                .env("GITHUB_EVENT_NAME", event)
                .env("GITHUB_REF", reference)
                .env("GITHUB_REF_TYPE", kind)
                .env("GITHUB_REF_NAME", reference.rsplit('/').next().unwrap())
                .env("GITHUB_SHA", "0123456789abcdef0123456789abcdef01234567")
                .env("RELEASE_TAG", requested),
            Duration::from_secs(15),
            "the actual distribution event-context guard",
        );
        assert_eq!(
            output.status.success(),
            accepted,
            "event={event}, ref={reference}, kind={kind}, tag={requested:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).contains("context-accepted"),
            accepted,
            "a refused context must stop before subsequent distribution steps"
        );
    }
}

#[test]
fn the_commit_binding_is_checked_before_repository_version_code_runs() {
    let steps: Vec<_> = workflow_steps(WORKFLOW)
        .into_iter()
        .filter(|step| step.job == "version-check")
        .collect();
    let checkout = steps
        .iter()
        .find(|step| step.uses.starts_with("actions/checkout@"))
        .expect("event revision checkout");
    let revision = steps
        .iter()
        .find(|step| step.id == "revision")
        .expect("inline revision binding check");
    let version = steps
        .iter()
        .find(|step| step.run.contains("scripts/ci/version-consistency.sh"))
        .expect("repository version consistency script");
    assert_eq!(revision.position, checkout.position + 1);
    assert!(revision.position < version.position);
    assert_eq!(revision.shell, "bash");
    assert!(revision.uses.is_empty() && revision.cond.is_empty());
    assert!(!revision.run.contains("${{"));
    assert_eq!(checkout.with["ref"], "${{ github.sha }}");

    let workflow = yaml(WORKFLOW);
    let raw = workflow["jobs"]["version-check"]["steps"]
        .as_sequence()
        .expect("version-check steps");
    assert_eq!(
        raw[checkout.position - 1]["with"]["fetch-depth"].as_integer(),
        Some(0)
    );
    assert!(
        raw[revision.position - 1]
            .as_mapping_get("continue-on-error")
            .is_none()
    );
}

/// Runs only against a test-owned repository. Global signing and hooks never
/// participate, and Git's repository-redirection variables are removed.
fn fixture_git(git: &Path, directory: &Path, args: &[&str]) -> String {
    let mut command = crate::common::tools::git_command(git);
    command
        .current_dir(directory)
        .args([
            "-c",
            "user.name=Distribution fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "tag.gpgsign=false",
            "-c",
            "core.hooksPath=disabled-hooks",
            "-c",
            "init.templateDir=",
        ])
        .args(args);
    let output = run_bounded(
        &mut command,
        Duration::from_secs(15),
        "isolated distribution Git fixture",
    );
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("Git object IDs are UTF-8")
        .trim()
        .to_owned()
}

#[test]
fn the_real_revision_check_peels_tags_and_rejects_stale_or_missing_commits() {
    let revision = workflow_steps(WORKFLOW)
        .into_iter()
        .find(|step| step.job == "version-check" && step.id == "revision")
        .expect("inline revision binding check");
    let Some(shell) = bash() else {
        return;
    };
    let Some(git) = crate::common::tools::require_working_git() else {
        return;
    };
    let work = tempfile::tempdir().expect("private Git repository for tag checks");
    let directory = work.path();
    std::fs::create_dir(directory.join("disabled-hooks")).expect("empty local hooks directory");
    fixture_git(&git, directory, &["init", "--quiet"]);
    fixture_git(
        &git,
        directory,
        &["commit", "--quiet", "--allow-empty", "-m", "first"],
    );
    let first = fixture_git(&git, directory, &["rev-parse", "HEAD"]);
    let epoch = fixture_git(&git, directory, &["show", "-s", "--format=%ct", "HEAD"]);
    fixture_git(&git, directory, &["tag", "v0.1.0"]);
    fixture_git(
        &git,
        directory,
        &["tag", "--annotate", "v0.2.0", "-m", "annotated fixture"],
    );
    let annotated_object = fixture_git(&git, directory, &["rev-parse", "refs/tags/v0.2.0"]);
    assert_ne!(
        annotated_object, first,
        "the annotated tag must require peeling to a commit"
    );
    fixture_git(&git, directory, &["tag", "v0.3.0"]);
    fixture_git(
        &git,
        directory,
        &["commit", "--quiet", "--allow-empty", "-m", "second"],
    );
    let second = fixture_git(&git, directory, &["rev-parse", "HEAD"]);
    fixture_git(&git, directory, &["tag", "--force", "v0.3.0", &second]);

    let script = directory.join("revision.sh");
    std::fs::write(&script, &revision.run).expect("write the exact inline revision check");
    for (index, (reference, head, event_sha, accepted)) in [
        ("refs/tags/v0.1.0", first.as_str(), first.as_str(), true),
        ("refs/tags/v0.2.0", first.as_str(), first.as_str(), true),
        ("refs/tags/v0.3.0", first.as_str(), first.as_str(), false),
        ("refs/tags/missing", first.as_str(), first.as_str(), false),
        ("refs/tags/v0.1.0", first.as_str(), second.as_str(), false),
        ("refs/tags/v0.1.0", second.as_str(), first.as_str(), false),
    ]
    .into_iter()
    .enumerate()
    {
        fixture_git(&git, directory, &["checkout", "--quiet", "--detach", head]);
        let output_name = format!("outputs-{index}");
        let mut command = Command::new(&shell);
        command
            .args(["--noprofile", "--norc", "-e", "-o", "pipefail"])
            .arg(&script)
            .current_dir(directory)
            .env("GITHUB_REF", reference)
            .env("GITHUB_SHA", event_sha)
            .env("GITHUB_OUTPUT", &output_name);
        for variable in crate::common::tools::GIT_REDIRECTING_VARS {
            command.env_remove(variable);
        }
        let output = run_bounded(
            &mut command,
            Duration::from_secs(15),
            "actual distribution commit binding check",
        );
        assert_eq!(
            output.status.success(),
            accepted,
            "ref={reference}, head={head}, event={event_sha}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let outputs = std::fs::read_to_string(directory.join(output_name)).unwrap_or_default();
        if accepted {
            let values: BTreeMap<_, _> = outputs
                .lines()
                .map(|line| line.split_once('=').expect("key=value output"))
                .collect();
            assert_eq!(
                values,
                BTreeMap::from([("commit", first.as_str()), ("epoch", epoch.as_str())])
            );
        } else {
            assert!(
                outputs.is_empty(),
                "a refused revision must expose no accepted commit/epoch outputs: {outputs}"
            );
        }
    }
}

#[test]
fn every_distribution_checkout_uses_the_event_revision_after_the_guard() {
    let steps = workflow_steps(WORKFLOW);
    let workflow = yaml(WORKFLOW);
    let mut checkout_jobs = BTreeSet::new();
    for step in steps
        .iter()
        .filter(|step| step.uses.starts_with("actions/checkout@"))
    {
        checkout_jobs.insert(step.job.as_str());
        if let Some(reference) = step.with.get("ref") {
            assert_eq!(
                reference.split_whitespace().collect::<String>(),
                "${{github.sha}}",
                "{} must checkout the event commit; tag inputs and job outputs can select other code",
                step.job
            );
        }
        if step.job == "version-check" {
            assert!(step.position > execution_guard().position);
        } else {
            let job = &workflow["jobs"][step.job.as_str()];
            let needs = &job["needs"];
            assert!(
                needs.as_str() == Some("version-check")
                    || needs.as_sequence().is_some_and(|values| values
                        .iter()
                        .any(|value| value.as_str() == Some("version-check"))),
                "{} must depend on the event-context gate",
                step.job
            );
            assert!(
                job.as_mapping_get("if").is_none(),
                "{} must not override failure propagation from version-check",
                step.job
            );
        }
    }
    assert_eq!(
        checkout_jobs,
        BTreeSet::from([
            "version-check",
            "build-linux",
            "build-macos",
            "build-windows",
            "publish"
        ]),
        "inspect every job that executes distribution source"
    );
}
