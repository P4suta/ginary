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

use saphyr::YamlOwned;

use crate::common::repo::{parse_yaml, read, read_opt, read_or_missing, root};

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

// --------------------------------------------- the security workflows --

/// The `language: build-mode` pairs a CodeQL matrix declares, sorted.
///
/// The matrix has to be written as `include:` rows — one `language:` with the
/// `build-mode:` that language is analysed under — because the build mode is
/// per language here: `actions` needs no build and Rust may need one. The rows
/// come out of the parsed document rather than out of the file's text, so a
/// row is a row and prose in a comment is prose.
fn codeql_matrix(text: &str) -> Vec<(String, String)> {
    let parsed = workflow(".github/workflows/codeql.yml", text);
    let mut out: Vec<(String, String)> = jobs(&parsed)
        .into_iter()
        .filter_map(|(_, job)| {
            job.as_mapping_get("strategy")?
                .as_mapping_get("matrix")?
                .as_mapping_get("include")?
                .as_vec()
        })
        .flatten()
        .map(|row| {
            let field = |key: &str| {
                row.as_mapping_get(key)
                    .and_then(YamlOwned::as_str)
                    .unwrap_or_default()
                    .to_owned()
            };
            (field("language"), field("build-mode"))
        })
        .collect();
    out.sort();
    out
}

/// Every `cron:` expression a workflow schedules itself on.
///
/// A schedule is a YAML sequence, so the key almost always arrives as the
/// first entry of a list item — `- cron: "41 20 * * 3"` — and only rarely on a
/// line of its own under a bare `-`. Both spellings are the same schedule, so
/// the leading dash is stripped before the key is looked for; reading only the
/// second spelling would make every one of these assertions vacuous against a
/// workflow written the ordinary way.
fn crons(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            let entry = trimmed.strip_prefix("- ").unwrap_or(trimmed);
            entry.trim_start().strip_prefix("cron:")
        })
        .map(|value| value.trim().trim_matches('"').trim_matches('\'').to_owned())
        .collect()
}

#[test]
fn the_security_workflows_the_milestone_named_are_committed() {
    for workflow in [
        ".github/workflows/codeql.yml",
        ".github/workflows/scorecard.yml",
        ".github/workflows/dependency-review.yml",
    ] {
        assert!(
            read_opt(workflow).is_some(),
            "{workflow} is a public-repository gate E3 promised; it is not committed"
        );
    }
}

#[test]
fn codeql_analyzes_the_languages_this_repository_actually_has() {
    let codeql = read(".github/workflows/codeql.yml");
    let matrix = codeql_matrix(&codeql);
    let languages: Vec<&str> = matrix.iter().map(|(l, _)| l.as_str()).collect();
    assert_eq!(
        languages,
        vec!["actions", "rust"],
        "CodeQL analyses exactly the two languages in this tree — the Rust crate and the \
         workflows — as `include:` rows, sorted here; it declares: {matrix:?}"
    );
    for needle in [
        "github/codeql-action/init",
        "github/codeql-action/analyze",
        "queries: security-extended",
    ] {
        assert!(
            codeql.contains(needle),
            "the CodeQL workflow is missing `{needle}`"
        );
    }
    // A language this repository does not hold is an analysis that can only
    // report nothing, and a matrix row that will not be noticed when it breaks.
    // The check is against the parsed rows, not against the file's text: a
    // `cargo build` step spells `go`, and a rule that reads the whole file
    // would report the build command the spec names as the `manual` fallback
    // as if it were a Go analysis.
    for absent in [
        "javascript-typescript",
        "python",
        "go",
        "java",
        "ruby",
        "c-cpp",
        "csharp",
        "swift",
    ] {
        assert!(
            !languages.contains(&absent),
            "the CodeQL matrix analyses `{absent}`, which this repository does not contain"
        );
    }
}

#[test]
fn codeql_gives_every_language_an_explicit_build_mode_and_never_autobuilds() {
    let codeql = read(".github/workflows/codeql.yml");
    let matrix = codeql_matrix(&codeql);
    assert!(
        !matrix.is_empty(),
        "the CodeQL matrix has to be `include:` rows carrying a build-mode per language"
    );
    for (language, build_mode) in &matrix {
        assert!(
            matches!(build_mode.as_str(), "none" | "manual"),
            "`{language}` is analysed with build-mode `{build_mode}`: every row states `none` or \
             `manual`, so the mode is a decision the diff records rather than a default"
        );
    }
    let by_language: std::collections::BTreeMap<&str, &str> = matrix
        .iter()
        .map(|(l, m)| (l.as_str(), m.as_str()))
        .collect();
    assert_eq!(
        by_language.get("actions").copied(),
        Some("none"),
        "workflow analysis builds nothing: `actions` is `build-mode: none`"
    );
    // `autobuild` on a Rust crate guesses at the build; this repository has one
    // build command and a committed lock file, so the workflow says so itself.
    assert!(
        !codeql.contains("autobuild"),
        "CodeQL never autobuilds here: the Rust row is `none`, or `manual` with the crate's own \
         `cargo build`"
    );
    if by_language.get("rust").copied() == Some("manual") {
        for needle in ["cargo build", "--all-features", "--locked"] {
            assert!(
                codeql.contains(needle),
                "a manual build mode owns the build: the Rust row needs an explicit \
                 `cargo build --all-features --locked` step, and `{needle}` is missing"
            );
        }
    }
}

#[test]
fn codeql_runs_weekly_on_a_slot_of_its_own_plus_push_pull_request_and_dispatch() {
    let codeql = read(".github/workflows/codeql.yml");
    assert_eq!(
        crons(&codeql),
        vec!["41 20 * * 3".to_owned()],
        "CodeQL runs weekly on one slot of its own. The sibling repositories hold `31 20 * * 3` \
         and `11 20 * * 3`; a third repository joining either would queue three scans at once"
    );
    for trigger in ["pull_request:", "workflow_dispatch:", "push:"] {
        assert!(
            codeql.contains(trigger),
            "the CodeQL workflow does not run on `{trigger}`"
        );
    }
    assert!(
        codeql.contains("concurrency:") && codeql.contains("cancel-in-progress: true"),
        "a superseded scan is cancelled rather than queued, as in the sibling repositories"
    );
    assert!(
        codeql.contains("timeout-minutes:"),
        "every job carries a timeout, so a hung analysis fails rather than burning the budget"
    );
}

#[test]
fn scorecard_publishes_its_results_and_uploads_them_to_code_scanning() {
    let scorecard = read(".github/workflows/scorecard.yml");
    for trigger in [
        "branch_protection_rule:",
        "push:",
        "schedule:",
        "workflow_dispatch:",
    ] {
        assert!(
            scorecard.contains(trigger),
            "the Scorecard workflow does not run on `{trigger}`"
        );
    }
    for needle in [
        "ossf/scorecard-action",
        "publish_results: true",
        "id-token: write",
        "security-events: write",
        "github/codeql-action/upload-sarif",
        "results.sarif",
    ] {
        assert!(
            scorecard.contains(needle),
            "the Scorecard workflow is missing `{needle}`: publishing the result is what makes \
             the badge and the public record real, and it needs the OIDC token to do it"
        );
    }
    let crons = crons(&scorecard);
    assert_eq!(
        crons.len(),
        1,
        "Scorecard runs on exactly one weekly slot: {crons:?}"
    );
    let slot = &crons[0];
    for taken in ["31 20 * * 3", "11 20 * * 3", "41 20 * * 3"] {
        assert_ne!(
            slot, taken,
            "`{taken}` is already taken by a sibling repository or by our own CodeQL scan; \
             Scorecard needs a slot of its own"
        );
    }
}

#[test]
fn dependency_review_gates_pull_requests_and_defers_to_cargo_deny() {
    let review = read(".github/workflows/dependency-review.yml");
    assert!(
        review.contains("pull_request:"),
        "dependency review is a pull-request gate; it has no other trigger"
    );
    for needle in ["actions/dependency-review-action", "fail-on-severity: high"] {
        assert!(
            review.contains(needle),
            "the dependency-review workflow is missing `{needle}`"
        );
    }
    // deny.toml is the licence and advisory authority: it runs in `lint`, on
    // every target the crate names, and locally through `mise run deny`. This
    // workflow is the pull-request convenience, and says so, so nobody grows a
    // second allow-list here for the two to drift apart.
    assert!(
        review.contains("deny.toml") && review.to_lowercase().contains("cargo deny"),
        "the workflow has to record that `cargo deny` is the authoritative gate and this is the \
         pull-request-time convenience"
    );
    assert!(
        !review.contains("allow-licenses:") && !review.contains("deny-licenses:"),
        "a licence list here would be a second copy of `deny.toml`'s allow-list, free to drift"
    );
}

// ----------------------------------------------------------- dependabot --

/// One `updates:` entry of `.github/dependabot.yml`, as the fields E3 pins.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Update {
    ecosystem: String,
    directory: String,
    interval: String,
    day: String,
    timezone: String,
    cooldown_days: String,
    limit: String,
    labels: Vec<String>,
    groups: Vec<String>,
}

/// A YAML scalar as the text a snapshot can hold.
///
/// `open-pull-requests-limit: 5` is an integer and `directory: "/"` is a
/// string, and both are one line of the rendered table, so the three scalar
/// shapes dependabot uses come back as their own spelling.
fn scalar(node: Option<&YamlOwned>) -> String {
    let Some(node) = node else {
        return String::new();
    };
    node.as_str()
        .map(str::to_owned)
        .or_else(|| node.as_integer().map(|value| value.to_string()))
        .or_else(|| node.as_bool().map(|value| value.to_string()))
        .unwrap_or_default()
}

/// A YAML sequence of strings, or an empty list for anything else.
fn string_sequence(node: Option<&YamlOwned>) -> Vec<String> {
    node.and_then(YamlOwned::as_vec)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// The names of the `groups:` one entry declares, in the order it declares them.
fn group_names(entry: &YamlOwned) -> Vec<String> {
    entry
        .as_mapping_get("groups")
        .and_then(YamlOwned::as_mapping)
        .map(|groups| {
            groups
                .keys()
                .filter_map(|name| name.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// Every `updates:` entry of a `dependabot.yml`, sorted.
///
/// Parsed rather than scanned line by line: dependabot's own reader is a YAML
/// reader, so a file it cannot load is a policy that never runs, and a
/// hand-rolled scan is happy with one.
fn dependabot_updates(text: &str) -> Vec<Update> {
    let parsed = parse_yaml(text)
        .unwrap_or_else(|error| panic!(".github/dependabot.yml is not valid YAML: {error}"));
    let Some(entries) = parsed.as_mapping_get("updates").and_then(YamlOwned::as_vec) else {
        return Vec::new();
    };
    let mut out: Vec<Update> = entries
        .iter()
        .map(|entry| {
            let schedule = entry.as_mapping_get("schedule");
            let at = |node: Option<&YamlOwned>, key: &str| {
                scalar(node.and_then(|node| node.as_mapping_get(key)))
            };
            Update {
                ecosystem: at(Some(entry), "package-ecosystem"),
                directory: at(Some(entry), "directory"),
                interval: at(schedule, "interval"),
                day: at(schedule, "day"),
                timezone: at(schedule, "timezone"),
                cooldown_days: at(entry.as_mapping_get("cooldown"), "default-days"),
                limit: at(Some(entry), "open-pull-requests-limit"),
                labels: string_sequence(entry.as_mapping_get("labels")),
                groups: group_names(entry),
            }
        })
        .collect();
    out.sort();
    out
}

/// The dependabot entries as the table the snapshot pins.
fn render_updates(updates: &[Update]) -> String {
    updates
        .iter()
        .map(|update| {
            format!(
                "{} {}\n  schedule: {} {} {}\n  cooldown-days: {}\n  \
                 open-pull-requests-limit: {}\n  labels: {}\n  groups: {}",
                update.ecosystem,
                update.directory,
                update.interval,
                update.day,
                update.timezone,
                update.cooldown_days,
                update.limit,
                update.labels.join(", "),
                update.groups.join(", "),
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[test]
fn dependabot_watches_every_manifest_this_repository_actually_has() {
    let text = read(".github/dependabot.yml");
    let updates = dependabot_updates(&text);
    let watched: Vec<(&str, &str)> = updates
        .iter()
        .map(|u| (u.ecosystem.as_str(), u.directory.as_str()))
        .collect();
    // `fuzz/` is a workspace of its own — deliberately not a member of the root
    // one, see its Cargo.toml — so the root `cargo` entry does not reach its
    // manifest and it needs an entry of its own.
    assert_eq!(
        watched,
        vec![("cargo", "/"), ("cargo", "/fuzz"), ("github-actions", "/")],
        "dependabot covers the crate, the fuzz workspace and the actions, and nothing this \
         repository does not have; it covers: {watched:?}"
    );
    for update in &updates {
        assert_eq!(
            (
                update.interval.as_str(),
                update.day.as_str(),
                update.timezone.as_str()
            ),
            ("weekly", "monday", "Asia/Tokyo"),
            "{} {} does not update weekly on a Monday morning in the author's timezone",
            update.ecosystem,
            update.directory
        );
        assert_eq!(
            update.limit, "5",
            "{} {} does not cap its open pull requests at 5",
            update.ecosystem, update.directory
        );
        assert!(
            !update.cooldown_days.is_empty(),
            "{} {} has no `cooldown`, so a release yanked hours later still opens a pull request",
            update.ecosystem,
            update.directory
        );
        assert!(
            update.labels.contains(&"type: dependencies".to_owned()),
            "{} {} is not labelled `type: dependencies`: {:?}",
            update.ecosystem,
            update.directory,
            update.labels
        );
        // Per entry rather than once for the file: a single `groups:` anywhere
        // in the document says nothing about the entry beside it, and an
        // ungrouped `github-actions` entry opens one pull request per pinned
        // SHA every Monday.
        assert!(
            !update.groups.is_empty(),
            "{} {} declares no `groups:`, so one Monday is one pull request per dependency \
             rather than one for the ecosystem",
            update.ecosystem,
            update.directory
        );
    }
}

#[test]
fn the_dependabot_schedule_is_the_committed_record() {
    let text = read_or_missing(".github/dependabot.yml");
    let rendered = if text.starts_with("(missing") {
        text
    } else {
        render_updates(&dependabot_updates(&text))
    };
    insta::assert_snapshot!("dependabot_updates", rendered);
}

// ---------------------------------------------- top-level hardening --

/// A workflow parsed as the document GitHub itself loads.
///
/// The two guards below used to read `permissions:` out of the file as text,
/// which accepts the word wherever it appears: in a comment, in a `with:`
/// input spelled the same way, in a step's script. Parsing first means a key
/// of the mapping is the only thing that can satisfy them.
fn workflow(path: &str, text: &str) -> YamlOwned {
    parse_yaml(text).unwrap_or_else(|error| panic!("{path} is not valid YAML: {error}"))
}

/// The `jobs:` mapping of a parsed workflow, as (job id, job node).
fn jobs(workflow: &YamlOwned) -> Vec<(String, &YamlOwned)> {
    let Some(jobs) = workflow
        .as_mapping_get("jobs")
        .and_then(YamlOwned::as_mapping)
    else {
        return Vec::new();
    };
    jobs.iter()
        .map(|(id, job)| {
            (
                id.as_str()
                    .unwrap_or("<a job id that is not a string>")
                    .to_owned(),
                job,
            )
        })
        .collect()
}

/// Whether a `permissions:` node grants nothing but read scopes.
///
/// An empty mapping — `permissions: {}`, no token at all — counts, and so
/// does a mapping whose every value is the string `read`. Anything else does
/// not: `write` is the one this refuses, and the `read-all` / `write-all`
/// shorthands are scalars rather than mappings and are refused with it, since
/// a scope this repository cannot name per job is a scope it does not want.
fn grants_only_reads(permissions: &YamlOwned) -> bool {
    permissions
        .as_mapping()
        .is_some_and(|scopes| scopes.values().all(|value| value.as_str() == Some("read")))
}

/// A `permissions:` node as the one-line `scope: level` list a message shows.
fn render_permissions(permissions: &YamlOwned) -> String {
    match permissions.as_mapping() {
        Some(scopes) if scopes.is_empty() => "{}".to_owned(),
        Some(scopes) => scopes
            .iter()
            .map(|(scope, level)| format!("{}: {}", scalar(Some(scope)), scalar(Some(level))))
            .collect::<Vec<_>>()
            .join(", "),
        None => scalar(Some(permissions)),
    }
}

#[test]
fn every_workflow_defaults_to_a_read_only_token_and_never_persists_credentials() {
    let files = action_yaml();
    assert!(!files.is_empty(), "there are no workflow files to check");
    for (path, text) in &files {
        if !path.contains("/workflows/") {
            continue;
        }
        // `permissions: {}` — no token at all — is what the CI, release,
        // distribute and nightly workflows start from. The security workflows
        // follow the sibling repositories' `contents: read`, which the CodeQL
        // and Scorecard actions read the repository with before a job widens
        // to `security-events: write`. Either is a default that can write
        // nothing; a top-level `write` is what this refuses, because it hands
        // every job in the file a token it never asked for.
        let parsed = workflow(path, text);
        let permissions = parsed
            .as_mapping_get("permissions")
            .unwrap_or_else(|| panic!("{path} declares no top-level `permissions:`"));
        assert!(
            grants_only_reads(permissions),
            "{path} defaults to `permissions: {}`, which grants more than read; widen per job \
             instead",
            render_permissions(permissions)
        );
        assert!(
            text.contains("persist-credentials: false"),
            "{path} must check out with `persist-credentials: false`"
        );
    }
}

#[test]
fn every_job_of_every_workflow_declares_its_own_permissions() {
    let files = action_yaml();
    let mut offenders: Vec<String> = Vec::new();
    for (path, text) in &files {
        if !path.contains("/workflows/") {
            continue;
        }
        let parsed = workflow(path, text);
        let jobs = jobs(&parsed);
        assert!(!jobs.is_empty(), "{path} declares no jobs");
        for (id, job) in jobs {
            // A key of the job's own mapping, not the word anywhere in its
            // text: a comment that mentions permissions is not a declaration.
            let declared = job
                .as_mapping_get("permissions")
                .is_some_and(|node| node.is_mapping());
            if !declared {
                offenders.push(format!("{path}: job `{id}`"));
            }
        }
    }
    // A read-only default is only half the rule: the other half is that no job
    // inherits it silently. A job that names its own scopes is a job whose
    // token is visible in the diff that widens it.
    assert!(
        offenders.is_empty(),
        "every job states the scopes it needs as a mapping of its own, so a widening is a line \
         in the diff; these do not:\n{}",
        offenders.join("\n")
    );
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
