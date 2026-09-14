// SPDX-License-Identifier: MIT OR Apache-2.0
//! The evidence retained by the assurance workflows is part of their contract.
mod common;

#[test]
fn every_native_and_cross_test_job_preserves_its_own_failure_evidence() {
    let workflow = common::repo::yaml(".github/workflows/ci.yml");
    for job in ["test", "smoke-matrix", "macos", "windows", "coverage"] {
        let steps = workflow["jobs"][job]["steps"].as_sequence().unwrap();
        assert!(
            steps.iter().any(|step| step
                .as_mapping_get("uses")
                .and_then(saphyr::YamlOwned::as_str)
                .is_some_and(|value| value.starts_with("actions/upload-artifact@"))
                && step
                    .as_mapping_get("if")
                    .and_then(saphyr::YamlOwned::as_str)
                    .is_some_and(|value| value.contains("always()"))
                && step["with"]["retention-days"].as_integer() == Some(30)
                && step["with"]["path"]
                    .as_str()
                    .is_some_and(|value| value.starts_with("target/assurance"))),
            "{job} must retain its own evidence for thirty days even after failure"
        );
        if job == "coverage" {
            assert!(steps.iter().any(|step| {
                step.as_mapping_get("run")
                    .and_then(saphyr::YamlOwned::as_str)
                    .is_some_and(|run| run.contains("bash scripts/ci/coverage.sh"))
            }));
            let helper = common::repo::read("scripts/ci/coverage.sh");
            assert!(
                helper.contains("scripts/ci/test-evidence.py")
                    && helper.contains("GINARY_TEST_EVIDENCE_DIR")
            );
        } else if job != "macos" {
            assert!(
                steps.iter().any(|step| step
                    .as_mapping_get("run")
                    .and_then(saphyr::YamlOwned::as_str)
                    .is_some_and(|run| run.contains("scripts/ci/test-evidence.py")
                        && run.contains("cargo test"))
                    && step["env"]["GINARY_TEST_EVIDENCE_DIR"]
                        .as_str()
                        .is_some_and(|value| value.contains("target/assurance"))),
                "{job} must distinguish skipped/interrupted tests and preserve subprocess failures"
            );
        }
    }
    let smoke = common::repo::workflow_steps(".github/workflows/ci.yml")
        .into_iter()
        .find(|step| step.job == "smoke-matrix" && step.run.contains("scripts/smoke-matrix.sh"))
        .unwrap();
    assert!(
        smoke.run.contains("set -euo pipefail"),
        "tee must not hide a failed smoke command under the default GitHub shell"
    );
}

#[test]
fn macos_exit_probe_runs_the_workflow_assertion_and_keeps_failed_evidence() {
    let git_bash = std::env::var_os("ProgramFiles")
        .map(std::path::PathBuf::from)
        .map(|path| path.join("Git/bin/bash.exe"))
        .filter(|path| path.is_file());
    let Some(shell) = git_bash.or_else(|| {
        common::tools::require_tools(&["bash"]).map(|tools| tools.path("bash").to_owned())
    }) else {
        return;
    };
    let step = common::repo::workflow_steps(".github/workflows/ci.yml")
        .into_iter()
        .find(|step| step.job == "macos" && step.run == "bash scripts/ci/macos-smoke.sh")
        .expect("the macOS smoke helper is actually run by CI");
    assert_eq!(step.env["GINARY_SMOKE_TARGET"], "${{ matrix.target }}");
    let smoke_script = common::repo::read("scripts/ci/macos-smoke.sh");
    // Execute the real workflow's evidence wrapper and assertion. Only the
    // unavailable native build/runtime/signing tools are replaced by commands
    // whose exit statuses this test controls.
    let prefix = smoke_script.split("otp_root=").next().unwrap();
    let (_, assertion) = smoke_script.split_once("# Assert exit 3;").unwrap();
    let assertion = assertion.split_once('\n').unwrap().1;
    for (actual, sign_status, log_status, verdict) in [
        (3, 0, 0, 0),
        (0, 0, 0, 1),
        (7, 0, 0, 1),
        (3, 9, 0, 9),
        (3, 0, 12, 12),
        (7, 0, 12, 1),
    ] {
        let work = tempfile::tempdir().unwrap();
        let script = format!(
            "export PATH=\"/usr/bin:/bin:$PATH\"\n\
             if command -v cygpath >/dev/null 2>&1; then GITHUB_WORKSPACE=$(cygpath -u \"$GITHUB_WORKSPACE\"); fi\n\
             tee() {{ command tee \"$@\"; return {log_status}; }}\n{prefix}\n\
             artifact=mock_artifact\n\
             mock_artifact() {{ printf 'runtime stdout\\n'; printf 'runtime stderr\\n' >&2; return {actual}; }}\n\
             codesign() {{ printf 'signature checked\\n'; return {sign_status}; }}\n\
             printf 'trace evidence\\n' > \"$GINARY_TRACE\"\n{assertion}"
        );
        let script_path = work.path().join("step.sh");
        std::fs::write(&script_path, script).unwrap();
        let output = common::bounded::run_bounded(
            std::process::Command::new(&shell)
                .arg("-e")
                .arg("-o")
                .arg("pipefail")
                .arg(&script_path)
                .env("GITHUB_WORKSPACE", work.path())
                .env("GINARY_SMOKE_TARGET", "macos-test"),
            std::time::Duration::from_secs(20),
            "the exact macOS workflow exit probe and evidence wrapper",
        );
        assert_eq!(
            output.status.code(),
            Some(verdict),
            "actual={actual}, sign={sign_status}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let evidence = work.path().join("target/assurance/macos-test");
        let log = std::fs::read_to_string(evidence.join("smoke.log")).unwrap();
        assert!(log.contains("runtime stdout") && log.contains("runtime stderr"));
        assert_eq!(log.contains("signature checked"), actual == 3);
        assert!(evidence.join("trace.ndjson").is_file());
        let run: serde_json::Value =
            serde_json::from_slice(&std::fs::read(evidence.join("run.json")).unwrap()).unwrap();
        assert_eq!(run["exit_code"], verdict);
        assert_eq!(run["complete"], true);
        assert_eq!(run["log_exit_code"], log_status);
        assert_eq!(
            run["status"],
            if verdict == 0 { "successful" } else { "failed" }
        );
        let probe: serde_json::Value =
            serde_json::from_slice(&std::fs::read(evidence.join("exit-code.json")).unwrap())
                .unwrap();
        assert_eq!(probe["observed"], actual);
    }
}

#[test]
fn every_expensive_nightly_pass_preserves_thirty_days_of_evidence() {
    // The mutation pass is on the pull-request path now, not here: it mutates
    // the lines a change touched rather than the whole crate, and retains its
    // own evidence under `mutants-diff`. `ci.yml`'s own retention is asserted
    // beside it there.
    let workflow = common::repo::yaml(".github/workflows/nightly.yml");
    for name in ["fuzz", "formal", "smoke-matrix"] {
        let steps = workflow["jobs"][name]["steps"]
            .as_sequence()
            .expect("assurance job steps");
        assert!(
            steps.iter().any(|step| step
                .as_mapping_get("uses")
                .and_then(saphyr::YamlOwned::as_str)
                .is_some_and(|action| action.starts_with("actions/upload-artifact@"))
                && step
                    .as_mapping_get("if")
                    .and_then(saphyr::YamlOwned::as_str)
                    .is_some_and(|condition| condition.contains("always()"))
                && step["with"]["retention-days"].as_integer() == Some(30)),
            "{name} must preserve evidence even when its check fails"
        );
    }
    let helper = common::repo::read("scripts/ci/smoke-matrix-evidence.sh");
    assert!(helper.contains("bash scripts/smoke-matrix.sh"));
    assert!(helper.contains("GINARY_SMOKE_EVIDENCE_DIR") && helper.contains("GINARY_TRACE"));
}

/// The mutation pass is a pull-request check over the diff, and keeps evidence.
///
/// The whole crate was 920 candidates over 106 native shards and hours of
/// runners every night. A change is not the whole crate, and a pull request is
/// about a change: `cargo mutants --in-diff` keeps only the mutants in lines
/// the branch touched. The full pass is `mise run mutants` on a developer's
/// machine, where it can take as long as it takes.
#[test]
fn the_mutation_pass_runs_over_the_diff_and_retains_what_it_found() {
    let workflow = common::repo::yaml(".github/workflows/ci.yml");
    let steps = workflow["jobs"]["mutants"]["steps"]
        .as_sequence()
        .expect("the mutants job has steps");
    assert!(
        steps.iter().any(|step| step
            .as_mapping_get("run")
            .and_then(saphyr::YamlOwned::as_str)
            .is_some_and(|run| run.contains("scripts/ci/mutation-diff.sh"))),
        "the job runs the committed script rather than an inline copy of it"
    );
    assert!(
        steps.iter().any(|step| step
            .as_mapping_get("uses")
            .and_then(saphyr::YamlOwned::as_str)
            .is_some_and(|action| action.starts_with("actions/upload-artifact@"))
            && step
                .as_mapping_get("if")
                .and_then(saphyr::YamlOwned::as_str)
                .is_some_and(|condition| condition.contains("always()"))
            && step["with"]["retention-days"].as_integer() == Some(30)),
        "a mutant that survived is evidence, and evidence a failing job drops is evidence nobody \
         reads"
    );

    let script = common::repo::read("scripts/ci/mutation-diff.sh");
    for needle in ["--in-diff", "--list", "BUDGET", "mutation-verdict.py"] {
        assert!(script.contains(needle), "the script is missing `{needle}`");
    }
    assert!(
        !common::repo::read(".github/workflows/nightly.yml").contains("mutants"),
        "the nightly pass is gone; the diff pass above replaced it"
    );
}

#[test]
fn nightly_fuzzing_has_a_real_budget_and_the_corpus_survives_the_runner() {
    let text = common::repo::read(".github/workflows/nightly.yml");
    assert!(
        text.contains("-max_total_time=600"),
        "each parser gets ten minutes of exploration"
    );
    assert!(
        text.contains("gh run download"),
        "generated corpus must survive between runs"
    );
    assert!(
        text.contains("gh api --paginate"),
        "mutation artifacts must not hide the corpus on a later API page"
    );
    assert!(text.contains("source scripts/ci/fuzz-evidence.sh"));
    assert!(text.contains("\"status\":\"not_run\""));
}
