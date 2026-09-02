// SPDX-License-Identifier: MIT OR Apache-2.0
//! The CI matrix, held against the repository.
//!
//! None of this is code the suite can run: a GitHub Actions job needs a remote,
//! runners this machine is not, and a docker daemon the harness may not have.
//! What every workflow shares with `tests/formal.rs` and `tests/smoke_matrix.rs`
//! is that it rots silently — a job that stops running, a SHA that drifts off
//! its tag, a matrix row quietly dropped — and a workflow nobody checks reads as
//! evidence the project is tested when it is not. This file pins the jobs the
//! milestone promised, the discipline every `uses:` is held to, and the two
//! local scripts CI runs, so "CI covers seven targets" is a claim the tree can
//! be held to even though only a fraction of it runs on Linux today.
//!
//! Ungated: a workflow is neither half of the crate.

mod common;

use std::path::PathBuf;

use crate::common::repo::{read, read_opt, root};

/// Every workflow and composite-action file under `.github/`, as (path, text).
fn action_yaml() -> Vec<(String, String)> {
    let mut out = Vec::new();
    let workflows = root().join(".github/workflows");
    if let Ok(entries) = std::fs::read_dir(&workflows) {
        for entry in entries.flatten() {
            let path = entry.path();
            if matches!(
                path.extension().and_then(|e| e.to_str()),
                Some("yml") | Some("yaml")
            ) {
                let rel = format!(
                    ".github/workflows/{}",
                    path.file_name().unwrap().to_string_lossy()
                );
                out.push((rel, std::fs::read_to_string(&path).expect("read workflow")));
            }
        }
    }
    // Composite actions live at .github/actions/<name>/action.yml.
    let actions = root().join(".github/actions");
    if let Ok(entries) = std::fs::read_dir(&actions) {
        for entry in entries.flatten() {
            for leaf in ["action.yml", "action.yaml"] {
                let path: PathBuf = entry.path().join(leaf);
                if path.is_file() {
                    let rel = format!(
                        ".github/actions/{}/{leaf}",
                        entry.file_name().to_string_lossy()
                    );
                    out.push((rel, std::fs::read_to_string(&path).expect("read action")));
                }
            }
        }
    }
    out.sort();
    out
}

// ------------------------------------------------------- the CI jobs --

#[test]
fn ci_defines_every_job_the_milestone_named() {
    let ci = read(".github/workflows/ci.yml");
    for job in [
        "lint:",
        "test:",
        "smoke:",
        "cross-build:",
        "smoke-matrix:",
        "macos:",
        "windows:",
        "coverage:",
        "required:",
    ] {
        assert!(
            ci.contains(job),
            "the CI matrix is missing the `{job}` job:\n{ci}"
        );
    }
}

#[test]
fn the_required_fan_in_waits_on_every_runnable_job() {
    let ci = read(".github/workflows/ci.yml");
    let needs = ci
        .split("required:")
        .nth(1)
        .and_then(|tail| tail.split("needs:").nth(1))
        .and_then(|tail| tail.lines().next())
        .expect("the required job declares a needs: list")
        .to_owned();
    // Every CI job feeds the fan-in, including macos and windows: they are not
    // locally runnable, but they are real jobs, and unhooking either from
    // `required` would let a red Mac or Windows run stop blocking the merge.
    for job in [
        "lint",
        "test",
        "smoke",
        "cross-build",
        "smoke-matrix",
        "macos",
        "windows",
        "coverage",
    ] {
        assert!(
            needs.contains(job),
            "`required` has to gate on `{job}`, or a red {job} would not block the merge: {needs}"
        );
    }
}

#[test]
fn the_lint_job_runs_all_three_clippy_flavors_and_the_deny_check() {
    let ci = read(".github/workflows/ci.yml");
    let lint = ci.split("lint:").nth(1).expect("a lint job").to_owned();
    // Stop at the next job so a needle from another job cannot satisfy it.
    let lint = lint.split("\n  test:").next().unwrap_or(&lint);
    for needle in [
        "cargo fmt",
        "--all-features",
        "--no-default-features",
        "cargo doc",
        "deny check",
        "1.88",
        "--locked",
    ] {
        assert!(
            lint.contains(needle),
            "the lint job is missing `{needle}`:\n{lint}"
        );
    }
}

#[test]
fn the_test_job_runs_both_flavors_under_a_pinned_toolchain() {
    let ci = read(".github/workflows/ci.yml");
    for needle in [
        "erlef/setup-beam",
        "29.0.5",
        "1.18.1",
        "GINARY_REQUIRE_TOOLCHAIN",
        "--features fault-injection",
        "--no-default-features",
    ] {
        assert!(
            ci.contains(needle),
            "the test job is missing `{needle}`: the stub flavor or the toolchain gate is not \
             exercised"
        );
    }
}

#[test]
fn the_cross_build_job_covers_all_seven_stubs_and_uploads_them() {
    let ci = read(".github/workflows/ci.yml");
    let job = ci.split("cross-build:").nth(1).expect("a cross-build job");
    for target in [
        "linux-x86_64-gnu",
        "linux-x86_64-musl",
        "linux-aarch64-gnu",
        "linux-aarch64-musl",
        "windows-x86_64",
    ] {
        assert!(
            job.contains(target),
            "the cross-build matrix does not build `{target}`"
        );
    }
    assert!(
        job.contains("--no-default-features"),
        "the cross-build job builds the launcher-only stub"
    );
    assert!(
        job.contains("upload-artifact"),
        "the stubs feed the release workflow, so the job uploads them"
    );
}

#[test]
fn the_macos_job_builds_the_darwin_stub_natively_and_verifies_the_signature() {
    let ci = read(".github/workflows/ci.yml");
    let job = ci.split("macos:").nth(1).expect("a macos job");
    for needle in [
        "macos-13",
        "macos-14",
        "--no-default-features",
        "codesign",
        "--verify",
    ] {
        assert!(
            job.contains(needle),
            "the macos job is missing `{needle}`: this is the job that closes the D3 Mac-runner \
             gap"
        );
    }
}

#[test]
fn the_windows_job_asserts_exit_code_propagation() {
    let ci = read(".github/workflows/ci.yml");
    let job = ci.split("windows:").nth(1).expect("a windows job");
    assert!(
        job.contains("windows-2022"),
        "the windows job runs on windows-2022"
    );
    assert!(
        job.contains("halt(3)") || job.contains("ERRORLEVEL") || job.contains("exit-code"),
        "the windows job proves an exit code crosses the launcher, the wine gap from D2:\n{job}"
    );
}

#[test]
fn the_smoke_matrix_job_bootstraps_binfmt_and_runs_the_committed_script() {
    let ci = read(".github/workflows/ci.yml");
    let job = ci
        .split("smoke-matrix:")
        .nth(1)
        .expect("a smoke-matrix job");
    for needle in ["binfmt", "arm64", "smoke-matrix.sh"] {
        assert!(
            job.contains(needle),
            "the smoke-matrix job is missing `{needle}`"
        );
    }
}

// ------------------------------------------------------- the nightly --

#[test]
fn the_nightly_workflow_runs_mutants_fuzz_and_the_full_smoke_matrix() {
    let nightly = read_opt(".github/workflows/nightly.yml")
        .expect("the heavy passes live in .github/workflows/nightly.yml, off the PR path");
    assert!(
        nightly.contains("schedule:") && nightly.contains("cron:"),
        "nightly runs on a schedule so PR CI stays fast:\n{nightly}"
    );
    for needle in ["cargo mutants", "cargo fuzz", "smoke-matrix.sh"] {
        assert!(
            nightly.contains(needle),
            "the nightly workflow is missing `{needle}`"
        );
    }
    for module in [
        "trailer", "payload", "cache", "closure", "appfile", "launch", "verify",
    ] {
        assert!(
            nightly.contains(module),
            "the mutants shard list is missing the high-value module `{module}`"
        );
    }
}

// --------------------------------------------------- the local scripts --

#[test]
fn the_ci_scripts_directory_holds_the_two_gates_ci_runs() {
    for script in [
        "scripts/ci/coverage-gate.sh",
        "scripts/ci/version-consistency.sh",
    ] {
        let path = root().join(script);
        assert!(
            path.is_file(),
            "{script} is a gate CI runs; it is not committed"
        );
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path)
            .expect("metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o111, 0o111, "{script} has to be executable");
    }
}

// ---------------------------------------------- top-level hardening --

#[test]
fn every_workflow_sets_the_default_permissions_to_none_and_never_persists_credentials() {
    let files = action_yaml();
    assert!(!files.is_empty(), "there are no workflow files to check");
    for (path, text) in &files {
        if !path.contains("/workflows/") {
            continue;
        }
        assert!(
            text.contains("permissions: {}"),
            "{path} must set `permissions: {{}}` at the top level and widen per job"
        );
        assert!(
            text.contains("persist-credentials: false"),
            "{path} must check out with `persist-credentials: false`"
        );
    }
}

// ------------------------------------------------ the SHA-pin table --

#[test]
fn every_uses_reference_is_pinned_to_a_full_sha_or_marked_todo() {
    let files = action_yaml();
    assert!(!files.is_empty(), "there are no workflow files to check");
    let mut offenders: Vec<String> = Vec::new();
    for (path, text) in &files {
        for (n, line) in text.lines().enumerate() {
            let trimmed = line.trim_start();
            let Some(rest) = trimmed
                .strip_prefix("- uses:")
                .or_else(|| trimmed.strip_prefix("uses:"))
            else {
                continue;
            };
            let reference = rest.trim();
            // A local composite action (`./.github/actions/x`) is not pinned.
            if reference.starts_with("./") {
                continue;
            }
            let Some((action, after)) = reference.split_once('@') else {
                offenders.push(format!(
                    "{path}:{}: `{reference}` names no version at all",
                    n + 1
                ));
                continue;
            };
            // Only third-party actions (owner/repo) are pinned this way.
            if !action.contains('/') {
                continue;
            }
            let after = after.trim();
            let (git_ref, comment) = match after.split_once('#') {
                Some((r, c)) => (r.trim(), c.trim()),
                None => (after, ""),
            };
            let is_sha = git_ref.len() == 40 && git_ref.chars().all(|c| c.is_ascii_hexdigit());
            let has_version_comment = comment.starts_with('v')
                && comment[1..]
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_digit());
            let todo_pin = comment.contains("TODO(pin)");
            if is_sha && has_version_comment {
                continue;
            }
            if todo_pin {
                // A host that could not be reached is an honest, listed exception.
                continue;
            }
            offenders.push(format!(
                "{path}:{}: `{reference}` is not `<40-hex> # vX.Y.Z` and is not marked TODO(pin)",
                n + 1
            ));
        }
    }
    assert!(
        offenders.is_empty(),
        "every third-party `uses:` is pinned to a full commit SHA with a `# vX.Y.Z` comment; \
         these are not:\n{}",
        offenders.join("\n")
    );
}
