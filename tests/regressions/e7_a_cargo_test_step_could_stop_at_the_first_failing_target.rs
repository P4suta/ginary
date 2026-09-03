// SPDX-License-Identifier: MIT OR Apache-2.0
//! `--no-fail-fast` was argued twice and asserted nowhere, so the next step
//! written without it would report the first slice of a failure set again.
//!
//! **What went wrong.** Twice now a live run has cost a milestone a complete
//! report. E5 read three failures out of `tests/e2e_cross.rs` and never saw
//! the eleven targets after it; E7 read seven library failures off the first
//! Windows-native run and never saw the rest, because `cargo test` stops at
//! the first *target* that fails and abandons every one that follows. Both
//! times the fix was the same flag on the step, and both times the flag went
//! in as a comment beside a `run:` line with nothing asserting it. A flag
//! that only a comment defends is a flag the next step does without.
//!
//! **The input.** Any new `cargo test` step in `.github/workflows/`, or a
//! rewrite of an existing one that drops the flag. Nothing in the suite
//! notices: the run is green either way, and the cost is only visible on the
//! day something fails, which is the day the whole report was wanted.
//!
//! **The correct behaviour.** Every command CI runs that invokes `cargo test`
//! carries `--no-fail-fast`, including the one that selects a single test by
//! name — a rule with an exception is a rule the next author has to relitigate
//! rather than read. One run, one full report.

use crate::common::repo::{read, shell_scripts_under, workflow_jobs, yaml_files_under};

/// The flag that makes one run produce the whole failure set.
const FLAG: &str = "--no-fail-fast";

/// Whether `command` runs `cargo test` without [`FLAG`].
///
/// Pure, and deliberately syntactic: the question is about the text of a
/// command line, and the scanner that answers it is calibrated below on the
/// two shapes the workflows actually hold — a single line, and a continuation
/// the YAML reader has already joined.
fn stops_at_the_first_failure(command: &str) -> bool {
    command.contains("cargo test") && !command.contains(FLAG)
}

#[test]
fn every_cargo_test_ci_runs_reports_every_target_that_failed() {
    assert!(
        stops_at_the_first_failure("cargo test --locked --test regressions some_test_name"),
        "the scanner has to see a bare `cargo test` as one that stops early, or it passes on \
         every tree including a broken one"
    );
    assert!(
        !stops_at_the_first_failure("cargo test --locked --no-fail-fast --test stub"),
        "and it has to leave alone a command that already carries the flag"
    );

    let mut offenders: Vec<String> = Vec::new();
    let mut checked = 0usize;
    for workflow in yaml_files_under(".github/workflows") {
        for job in workflow_jobs(&workflow) {
            for command in &job.commands {
                if !command.contains("cargo test") {
                    continue;
                }
                checked += 1;
                if stops_at_the_first_failure(command) {
                    offenders.push(format!("{workflow}: job `{}` runs `{command}`", job.id));
                }
            }
        }
    }
    for script in shell_scripts_under("scripts")
        .into_iter()
        .chain(shell_scripts_under(".github"))
    {
        // A backslash continuation carries the rest of the command onto the
        // next line, and the flag may be on either half, so the halves are
        // joined before the lines are counted — the same join
        // `repo::workflow_jobs` does to a `run:` block.
        for (index, command) in read(&script).replace("\\\n", " ").lines().enumerate() {
            if !command.contains("cargo test") {
                continue;
            }
            checked += 1;
            if stops_at_the_first_failure(command) {
                offenders.push(format!("{script}:{} runs `{}`", index + 1, command.trim()));
            }
        }
    }

    assert!(
        checked > 0,
        "no workflow or script runs `cargo test` any more, so this rule is measuring nothing"
    );
    assert!(
        offenders.is_empty(),
        "a `cargo test` CI runs without {FLAG} stops at the first target that fails and abandons \
         every one after it, so the failures it reports are the first slice of a set rather than \
         the whole of one. That has cost two milestones a complete report already:\n{}",
        offenders.join("\n")
    );
}
