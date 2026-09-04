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

use crate::common::repo::{read, shell_code, shell_scripts_under, workflow_jobs, yaml_files_under};

/// The flag that makes one run produce the whole failure set.
const FLAG: &str = "--no-fail-fast";

/// Whether `command` runs `cargo test` without [`FLAG`].
///
/// Pure, and deliberately syntactic: the question is about the text of a
/// command line, and the scanner that answers it is calibrated below on the
/// shapes the workflows actually hold — a single line, a continuation the YAML
/// reader has already joined, and a line with a comment after it.
///
/// The comment is removed first, by [`shell_code`]. Everything after an
/// unquoted `#` is prose the shell never executes, and reading it would accept
/// a step that *documents* the flag and does not pass it — which is exactly
/// the shape of the two lost reports this file records, since both times the
/// flag went in as a comment beside the `run:` line.
fn stops_at_the_first_failure(command: &str) -> bool {
    let code = shell_code(command);
    code.contains("cargo test") && !code.contains(FLAG)
}

#[test]
fn a_flag_that_only_a_shell_comment_carries_is_not_on_the_command_line() {
    // `#` starts a comment in every shell CI runs a `run:` block under, so
    // everything after it is prose the shell never sees. A scanner that reads
    // the whole line accepts a step that documents the flag and does not pass
    // it — which is precisely the shape this file exists to refuse, since the
    // two milestones it records lost their reports to a flag that lived only
    // in a comment beside the command.
    assert!(
        stops_at_the_first_failure("cargo test --locked # --no-fail-fast"),
        "a commented flag is not an argument: the shell runs `cargo test --locked`, which stops \
         at the first target that fails"
    );
    assert!(
        stops_at_the_first_failure("cargo test --locked   #--no-fail-fast one day"),
        "and the comment need not be spaced away from the `#` for the shell to drop it"
    );
    // The narrow half of the same rule: a `#` inside a quoted argument is a
    // character, not a comment, so the flag before it still counts.
    assert!(
        !stops_at_the_first_failure("cargo test --locked --no-fail-fast -- --skip 'a#b'"),
        "a `#` inside a quoted argument opens no comment, and the flag ahead of it is real"
    );
    // And a line that is nothing but a comment mentioning `cargo test` is not
    // a command at all, so it is not an offender either.
    assert!(
        !stops_at_the_first_failure("# cargo test is run by the step below"),
        "a line the shell never runs cannot stop at anything"
    );
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
