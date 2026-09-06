// SPDX-License-Identifier: MIT OR Apache-2.0
//! The job the readiness checklist books the Windows launch against never
//! started a ginary artifact.
//!
//! **What went wrong.** `docs/dev/v1-readiness.md` carries a row reading
//! "Windows artifact **launch**, exit-code propagation | `ci.yml` `windows`
//! job", and its deferred-items section says that job "Asserts `halt(3)`
//! reaches `%ERRORLEVEL%`, the exit-code propagation the D2 wine gap left
//! unproven". The step it means runs
//!
//! ```text
//! & $erl -noshell -eval "halt(3)"
//! ```
//!
//! which is `erl.exe` itself. No artifact is packaged, no launcher runs, and
//! `launch_windows::run` — the code the row is about — is not in the command
//! at all. What the step proves is that Erlang's `halt/1` reaches a shell on
//! Windows, which is a platform fact the launcher *rests* on and not the
//! launcher's own behaviour. The macOS job in the same file gets this right:
//! its step is named "Package, run and verify a hello_ffi artifact".
//!
//! Nothing was hidden. `tests/ci_matrix.rs` has always carried
//! `the_windows_job_asserts_exit_code_propagation`, whose message says the job
//! "proves an exit code crosses the launcher" — but whose assertion was
//!
//! ```rust,ignore
//! job.contains("halt(3)") || job.contains("ERRORLEVEL") || job.contains("exit-code")
//! ```
//!
//! a substring, which the `erl`-only step satisfies. A fail-closed checklist
//! that attributes a proof to a step which does not perform it is a defect by
//! this repository's own rule, and the test that should have caught it was
//! written loosely enough to agree. E23 replaced that assertion with the same
//! rule this file applies, shared through `common::repo`, so the looseness is
//! gone from the original site and not merely out-argued beside it.
//!
//! **The input.** The workflow as committed. No runner is needed to see it:
//! the step's own text says what it starts.
//!
//! **The correct behaviour.** The job packages the `hello_ffi` fixture with
//! the ginary it just built and runs the *artifact*, comparing the code it
//! leaves against the one the application asked for — the shape the macOS job
//! already has. The bare `erl` probe stays, because the platform fact is worth
//! reading in the log when the launcher check fails, but it is a precondition
//! and no longer the proof.
//!
//! E23 proves the launcher's own half of the same contract on a real Windows
//! host — `tests/launcher.rs` runs there now, and its exit-code claims go
//! through `launch_windows::run`. What stays CI's is the other half: that a
//! *real* `erl.exe` running `halt(3)` inside a real packaged artifact reaches
//! `%ERRORLEVEL%`. That needs a runtime, and this job is where one is.

use crate::common::repo::{
    WorkflowStep, compares_a_captured_exit_code, names_path, workflow_steps,
};

/// The workflow the Windows job lives in.
const CI: &str = ".github/workflows/ci.yml";

/// The fixture the job has to package, the one the macOS job packages.
const FIXTURE: &str = "hello_ffi";

/// The directory `ginary build` writes an artifact into, and therefore the
/// only path a step that runs one can name.
///
/// Spelled with a forward slash and matched through [`names_path`], because a
/// `pwsh` step is free to write it either way and both reach the same
/// directory.
const ARTIFACT_DIR: &str = "build/ginary";

/// The code the packaged application asks for, and therefore the number a step
/// that reads `%ERRORLEVEL%` has to agree with.
const HALT_CODE: i32 = 3;

#[test]
fn the_rule_this_file_applies_reads_a_comparison_and_not_a_digit() {
    let mentions = "& $artifact 3\nWrite-Host \"ran it 3 times\"\n";
    assert!(
        !compares_a_captured_exit_code(mentions, HALT_CODE),
        "a step that only mentions {HALT_CODE} must not satisfy the rule, or the rule is the \
         substring this file exists to replace"
    );
    let captures = "& $artifact 3\n$halted = $LASTEXITCODE\nif ($halted -ne 3) { exit 1 }\n";
    assert!(
        compares_a_captured_exit_code(captures, HALT_CODE),
        "the shape the job is written in must satisfy the rule, or the rule cannot be met at all"
    );
    assert!(
        names_path("$a = Join-Path $p 'build\\ginary\\hello.exe'", ARTIFACT_DIR),
        "a step is free to spell `{ARTIFACT_DIR}` with backslashes — it is a Windows path in a \
         Windows shell — and a rule that reads only one separator is a rule about how the step \
         was typed"
    );
}

/// Every step of the `windows` job.
fn windows_steps() -> Vec<WorkflowStep> {
    let steps: Vec<WorkflowStep> = workflow_steps(CI)
        .into_iter()
        .filter(|step| step.job == "windows")
        .collect();
    assert!(
        !steps.is_empty(),
        "the `windows` job of {CI} has no steps, so this scan has lost its subject"
    );
    steps
}

#[test]
fn the_windows_job_packages_the_fixture_it_is_going_to_run() {
    let steps = windows_steps();
    let packaging: Vec<&WorkflowStep> = steps
        .iter()
        .filter(|step| step.run.contains(FIXTURE) && step.run.contains("build"))
        .collect();
    assert!(
        !packaging.is_empty(),
        "no step of the `windows` job packages `{FIXTURE}`, so nothing it does afterwards can be \
         about a ginary artifact. The macOS job's `Package, run and verify a hello_ffi artifact` \
         is the shape:\n{}",
        rendered(&steps)
    );
}

#[test]
fn the_windows_job_runs_the_artifact_and_reads_the_code_it_left() {
    let steps = windows_steps();
    let running: Vec<&WorkflowStep> = steps
        .iter()
        .filter(|step| names_path(&step.run, ARTIFACT_DIR))
        .collect();
    assert!(
        !running.is_empty(),
        "no step of the `windows` job names `{ARTIFACT_DIR}`, so no packaged application is ever \
         started there and `launch_windows::run` is not exercised by this job at all:\n{}",
        rendered(&steps)
    );
    assert!(
        running
            .iter()
            .any(|step| compares_a_captured_exit_code(&step.run, HALT_CODE)),
        "a step runs the artifact but none compares the code it left against {HALT_CODE}: the \
         claim `docs/dev/v1-readiness.md` books against this job is that `halt({HALT_CODE})` \
         reaches `%ERRORLEVEL%`, and only a comparison proves it:\n{}",
        rendered(&running)
    );
}

/// The steps, one line each, for a message a reader can act on.
fn rendered(steps: &[impl std::borrow::Borrow<WorkflowStep>]) -> String {
    steps
        .iter()
        .map(|step| {
            let step: &WorkflowStep = step.borrow();
            format!("  step {} ({})", step.position, step.name)
        })
        .collect::<Vec<_>>()
        .join("\n")
}
