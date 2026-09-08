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
    let workflow = common::repo::yaml(".github/workflows/nightly.yml");
    for name in [
        "mutation-plan",
        "mutants",
        "mutation-gate",
        "fuzz",
        "formal",
        "smoke-matrix",
    ] {
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

#[test]
fn every_mutation_shard_fits_even_when_every_build_and_test_uses_its_entire_budget() {
    let plan = common::nightly::mutants_plan();
    let budget = common::nightly::mutation_budget();
    let measured = common::nightly::measured_mutants();
    for shard in &plan.shards {
        let count = measured.modules[&shard.module].div_ceil(shard.shards);
        let test_seconds: u64 = shard.timeout.as_ref().unwrap().parse().unwrap();
        let minutes = measured.baseline_minutes
            + (count * (test_seconds + budget.build_timeout_seconds)).div_ceil(60)
            + 15;
        assert!(
            minutes <= plan.timeout_minutes,
            "{} would consume {minutes} minutes",
            shard.row
        );
    }
    let current: serde_json::Value =
        serde_json::from_str(&common::repo::read(common::nightly::CURRENT_MUTANT_COUNTS)).unwrap();
    for shard in &plan.shards {
        let count = current["modules"][&shard.module].as_u64().unwrap();
        assert!(
            count.div_ceil(shard.shards) <= budget.max_mutants_per_shard,
            "{} cannot fit the current integrated source enumeration",
            shard.module
        );
    }
}

#[test]
fn mutation_reconciliation_uses_the_original_plan_and_every_native_jobs_evidence() {
    use common::repo::{option_value, workflow_steps, yaml};

    let workflow = yaml(common::nightly::NIGHTLY);
    let steps = workflow_steps(common::nightly::NIGHTLY);
    let invocation = |job: &str, verb: &str| {
        let prefix = format!("python3 scripts/ci/mutation.py {verb} ");
        let calls: Vec<_> = steps
            .iter()
            .filter(|step| step.job == job)
            .flat_map(|step| {
                step.commands()
                    .into_iter()
                    .map(move |command| (step, command))
            })
            .filter(|(_, command)| command.starts_with(&prefix))
            .collect();
        assert_eq!(calls.len(), 1, "{job} must invoke {verb} exactly once");
        calls.into_iter().next().unwrap()
    };
    let (planner, planning) = invocation("mutation-plan", "plan");
    let (runner, running) = invocation("mutants", "run");
    let (gate, finalizing) = invocation("mutation-gate", "finalize");
    // Like common::deps, read the committed section/key spelling so this
    // contract runs when the CLI-only toml dependency is disabled as well.
    let manifest = common::repo::read("tools/mutation-plan/Cargo.toml");
    let binary_name = manifest
        .lines()
        .map(common::repo::shell_code)
        .map(str::trim)
        .skip_while(|line| *line != "[package]")
        .skip(1)
        .take_while(|line| !line.starts_with('['))
        .find_map(|line| {
            let (key, value) = line.split_once('=')?;
            if key.trim() != "name" {
                return None;
            }
            value.trim().strip_prefix('"')?.strip_suffix('"')
        })
        .filter(|name| !name.is_empty())
        .expect("the standalone manifest declares its binary name in [package]");
    assert_eq!(
        option_value(&planning, "--planner"),
        Some(format!("tools/mutation-plan/target/debug/{binary_name}")),
        "the planning command must execute the standalone binary that Cargo actually builds"
    );
    assert!(
        !planner.id.is_empty(),
        "the matrix needs a real step output"
    );
    assert_eq!(
        workflow["jobs"]["mutation-plan"]["outputs"]["matrix"]
            .as_str()
            .unwrap()
            .split_whitespace()
            .collect::<String>(),
        format!("${{{{steps.{}.outputs.matrix}}}}", planner.id)
    );
    let planned = option_value(&planning, "--output").unwrap();
    let bundle = format!("{planned}/plan.json");
    assert_eq!(option_value(&running, "--bundle"), Some(bundle.clone()));
    assert_eq!(option_value(&finalizing, "--bundle"), Some(bundle));
    assert_eq!(
        option_value(&running, "--job").as_deref(),
        Some("$MUTATION_JOB")
    );
    assert_eq!(runner.env["MUTATION_JOB"], "${{ matrix.id }}");

    let plan_upload = steps
        .iter()
        .find(|step| {
            step.job == "mutation-plan" && step.uses.starts_with("actions/upload-artifact@")
        })
        .expect("the original plan is retained");
    assert_eq!(plan_upload.with["path"], planned);
    for job in ["mutants", "mutation-gate"] {
        let download = steps
            .iter()
            .find(|step| {
                step.job == job
                    && step.uses.starts_with("actions/download-artifact@")
                    && step.with.get("name") == plan_upload.with.get("name")
            })
            .expect("each consumer downloads the same original plan artifact");
        assert_eq!(download.with["path"], planned);
    }
    let native_upload = steps
        .iter()
        .find(|step| step.job == "mutants" && step.uses.starts_with("actions/upload-artifact@"))
        .expect("native raw outcomes are retained");
    assert_eq!(native_upload.with["name"], "mutants-${{ matrix.id }}");
    assert_eq!(
        option_value(&running, "--output").as_deref(),
        Some(native_upload.with["path"].as_str())
    );
    let evidence_download = steps
        .iter()
        .find(|step| {
            step.job == "mutation-gate"
                && step.uses.starts_with("actions/download-artifact@")
                && step
                    .with
                    .get("pattern")
                    .is_some_and(|pattern| pattern == "mutants-*")
        })
        .expect("reconciliation reads every native job, including failed ones");
    assert_ne!(
        evidence_download
            .with
            .get("merge-multiple")
            .map(String::as_str),
        Some("true"),
        "per-job raw outcomes must retain their identities instead of overwriting each other"
    );
    assert_eq!(
        option_value(&finalizing, "--evidence").as_deref(),
        Some(evidence_download.with["path"].as_str())
    );
    let final_job = &workflow["jobs"]["mutation-gate"];
    let mut dependencies: Vec<_> = final_job["needs"]
        .as_sequence()
        .unwrap()
        .iter()
        .map(|name| name.as_str().unwrap())
        .collect();
    dependencies.sort_unstable();
    assert_eq!(dependencies, ["mutants", "mutation-plan"]);
    assert_eq!(
        final_job["if"]
            .as_str()
            .unwrap()
            .split_whitespace()
            .collect::<String>(),
        "${{always()}}",
        "a failed or cancelled native job must still be reconciled"
    );
    for step in [gate, evidence_download, native_upload, plan_upload] {
        assert_eq!(
            step.cond.split_whitespace().collect::<String>(),
            "${{always()}}",
            "{step} must not disappear after an earlier failure"
        );
        assert!(
            !step.run.contains("|| true"),
            "reconciliation cannot hide failure"
        );
    }
    let final_steps = final_job["steps"].as_sequence().unwrap();
    let final_step = final_steps
        .iter()
        .find(|step| {
            step.as_mapping_get("name")
                .and_then(saphyr::YamlOwned::as_str)
                == Some(gate.name.as_str())
        })
        .unwrap();
    for node in [final_step, final_job] {
        assert_ne!(
            node.as_mapping_get("continue-on-error")
                .and_then(saphyr::YamlOwned::as_bool),
            Some(true)
        );
    }
    let summary_upload = steps
        .iter()
        .find(|step| {
            step.job == "mutation-gate" && step.uses.starts_with("actions/upload-artifact@")
        })
        .expect("the reconciled verdict is retained");
    assert_eq!(
        option_value(&finalizing, "--output").as_deref(),
        Some(summary_upload.with["path"].as_str())
    );
}
