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

use crate::common::repo::{WorkflowStep, read, workflow_steps, yaml};

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

/// The workflow this section is about.
const RELEASE: &str = ".github/workflows/release.yml";

/// The action that mints a repository-scoped installation token.
const APP_TOKEN_ACTION: &str = "actions/create-github-app-token";

/// The commit `actions/create-github-app-token` v3.2.0 points at.
///
/// Resolved with `git ls-remote --tags` against the action's own repository
/// rather than copied from a sibling project, and it is also what `v3` points
/// at today. A pin nobody verified is a pin to whatever the last force-push
/// left behind.
const APP_TOKEN_SHA: &str = "bcd2ba49218906704ab6c1aa796996da409d3eb1";

/// The repository variable holding the App's client id.
const CLIENT_ID_VAR: &str = "RELEASE_PLEASE_APP_CLIENT_ID";

/// The repository secret holding the App's private key.
const PRIVATE_KEY_SECRET: &str = "RELEASE_PLEASE_APP_PRIVATE_KEY";

/// The repository the App has to be installed on.
const REPOSITORY: &str = "P4suta/ginary";

/// One job of `release.yml`, with its `if:` guard as written.
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

/// The step that tells a maintainer which credentials are missing.
///
/// The release-please job carries a step that names both credentials too — the
/// half-configured check, which reports the *other* state — so the search is
/// for a step outside that job. What the notice says is not read here; only
/// which job it is in, because the assertions about it are about that job's
/// `if:`.
fn notice_step() -> Option<WorkflowStep> {
    let release_job = release_please_job();
    workflow_steps(RELEASE).into_iter().find(|step| {
        step.job != release_job
            && step.run.contains(CLIENT_ID_VAR)
            && step.run.contains(PRIVATE_KEY_SECRET)
    })
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
        "the client id comes from the repository variable `{CLIENT_ID_VAR}`:\n{step}"
    );
    assert!(
        step.contains(&format!(
            "private-key: ${{{{ secrets.{PRIVATE_KEY_SECRET} }}}}"
        )),
        "the private key comes from the repository secret `{PRIVATE_KEY_SECRET}`:\n{step}"
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

#[test]
fn the_release_job_is_gated_on_the_client_id_variable_being_present() {
    let job = release_please_job();
    let guard = job_guard(&job).unwrap_or_else(|| panic!("job `{job}` is not in {RELEASE}"));
    assert!(
        guard.contains(&format!("vars.{CLIENT_ID_VAR}")) && guard.contains("!= ''"),
        "job `{job}` runs release-please unconditionally. The credentials do not exist on this \
         repository yet and only a human can add them, so without a guard every push to `main` \
         paints the Release workflow red for a reason nobody in this tree can fix. Its `if:` is \
         `{guard}`"
    );
}

#[test]
fn a_repository_without_the_credentials_is_told_what_to_add_and_the_job_stays_green() {
    let step = notice_step().unwrap_or_else(|| {
        panic!(
            "no step of {RELEASE} names both `{CLIENT_ID_VAR}` and `{PRIVATE_KEY_SECRET}`. A \
             repository with no release credentials must have a *green* Release workflow that \
             says why it did nothing, not a red one and not a silent one."
        )
    });
    // The guard, and nothing about the step's own text. `notice_step` finds
    // the step *by* the credential names it prints, so any assertion that
    // reads `step.run` here is satisfied by the search that produced it — that
    // is the tautology this test used to be. The complementarity of the two
    // guards has a regression file of its own,
    // `tests/regressions/e5_the_credentials_notice_was_not_tied_to_the_missing_credentials.rs`.
    let guard = job_guard(&step.job).unwrap_or_default();
    assert!(
        guard.contains(&format!("vars.{CLIENT_ID_VAR}")) && guard.contains("== ''"),
        "the notice has to be reachable exactly when the credentials are absent; job `{}` is \
         guarded by `{guard}`",
        step.job
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
fn the_missing_credentials_notice_is_the_committed_record() {
    let rendered = notice_step().map_or_else(
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
                "no step of job `{}` checks `{PRIVATE_KEY_SECRET}`. A job `if:` reads `vars.` and \
                 never `secrets.`, so the guard on `{CLIENT_ID_VAR}` proves nothing about the \
                 private key: set the variable, forget the secret, and the job fails inside \
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
        "a repository that set `{CLIENT_ID_VAR}` asked for release automation, so a missing \
         `{PRIVATE_KEY_SECRET}` is a failure and not a notice — the one state this workflow does \
         report red:\n{}",
        step.run
    );
}

#[test]
fn the_notice_script_exits_zero_under_the_shell_that_runs_it() {
    let Some(step) = notice_step() else {
        panic!("no step of {RELEASE} names both credentials");
    };
    // GitHub runs a `run:` block on Linux as `bash -e -o pipefail {0}`, so the
    // last command's status is the step's status and any command failing ends
    // it. Reading the script and asserting it has no `exit 1` in it says
    // nothing about that; running it does. `bash` not being here is a reported
    // skip rather than a failure — this file is ungated, and the assertion is
    // about the script rather than about the machine.
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
