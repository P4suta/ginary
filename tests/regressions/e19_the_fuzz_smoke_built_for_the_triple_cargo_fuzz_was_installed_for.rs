// SPDX-License-Identifier: MIT OR Apache-2.0
//! The fuzz smoke built for the triple cargo-fuzz's own binary was compiled
//! for, not for the one the runner runs.
//!
//! **What went wrong.** Every `Fuzz smoke` job the `Nightly` workflow has ever
//! run has failed, on all four targets, before a single input was tried:
//!
//! ```text
//! error: sanitizer is incompatible with statically linked libc, disable it
//!   using `-C target-feature=-crt-static`
//! error[E0463]: can't find crate for `core`
//!   = note: the `x86_64-unknown-linux-musl` target may not be installed
//! error: could not compile `cfg-if` (lib) due to 2 previous errors
//! Error: failed to build fuzz script: ASAN_OPTIONS="detect_odr_violation=0"
//!   RUSTFLAGS=" -Cpasses=sancov-module … -Zsanitizer=address …" "cargo"
//!   "build" "--manifest-path" "…/fuzz/Cargo.toml" "--target"
//!   "x86_64-unknown-linux-musl" "--release" …
//! ```
//!
//! (`Fuzz smoke (payload_read_manifest)`, run `33953295452`,
//! <https://github.com/P4suta/ginary/actions/runs/33953295452/job/101271940905>.)
//!
//! **The input.** No `--target` on the `cargo fuzz run` line. Nothing in this
//! repository sets `+crt-static`, exports a `RUSTFLAGS`, commits a
//! `.cargo/config.toml` or names a musl triple anywhere near the fuzz job —
//! all four were checked before this was written. The triple comes from
//! cargo-fuzz itself: with no `--target`, it builds for the triple **its own
//! binary was compiled for**, and the job installs cargo-fuzz through
//! `taiki-e/install-action`, which downloads a prebuilt archive. The log says
//! which one three lines above the failure:
//!
//! ```text
//! WARN The package cargo-fuzz v0.13.2 (x86_64-unknown-linux-musl) has been
//!   downloaded from github.com
//! ```
//!
//! So a gnu runner, given a musl-built helper, builds a musl artifact. musl
//! defaults to `+crt-static`, and rustc will not put a sanitizer on a
//! statically linked libc — which is the first error; the second is only the
//! consequence of the first, because that target's `core` was never installed.
//! `rustup target add x86_64-unknown-linux-musl` would therefore silence the
//! second error and leave the real one standing.
//!
//! **The correct behaviour.** The job names the triple it means. It runs on
//! `ubuntu-24.04`, its sanitizer needs a dynamically linked libc, and
//! `x86_64-unknown-linux-gnu` is both — so `--target` says so on the command
//! line rather than being inherited from how somebody's release archive was
//! built. The pin is here and in `tests/ci_matrix.rs`, so a `--target` deleted
//! as noise fails a test on a pull request instead of a job nobody watches at
//! 03:17 UTC.

use crate::common::repo::{
    WorkflowStep, option_value, parse_yaml, read, workflow_jobs, workflow_jobs_of, workflow_steps,
};

/// The workflow the fuzz smoke lives in.
const NIGHTLY: &str = ".github/workflows/nightly.yml";

/// The triple a sanitizer build on this runner can actually use.
const GNU: &str = "x86_64-unknown-linux-gnu";

/// Every `cargo fuzz` command the nightly workflow runs, with continuations
/// joined and comments removed.
fn fuzz_commands() -> Vec<String> {
    workflow_steps(NIGHTLY)
        .iter()
        .flat_map(WorkflowStep::commands)
        .filter(|command| command.starts_with("cargo fuzz"))
        .collect()
}

#[test]
fn every_cargo_fuzz_command_names_a_target_rather_than_taking_the_default() {
    let commands = fuzz_commands();
    assert!(
        !commands.is_empty(),
        "the nightly workflow runs a fuzz smoke; a run of this test over no commands would pass \
         by finding nothing"
    );
    for command in &commands {
        assert!(
            option_value(command, "--target").is_some(),
            "cargo-fuzz's default target is the triple its own binary was built for, and this \
             job installs a prebuilt one. A command that does not say which triple it means is \
             asking whoever packaged cargo-fuzz:\n{command}"
        );
    }
}

#[test]
fn the_triple_the_fuzz_smoke_names_is_one_a_sanitizer_can_be_built_for() {
    for command in fuzz_commands() {
        assert_eq!(
            option_value(&command, "--target"),
            Some(GNU.to_owned()),
            "a sanitizer needs a dynamically linked libc, and `-musl` targets default to \
             `+crt-static`. `{GNU}` is the runner's own triple and links libc \
             dynamically:\n{command}"
        );
    }
}

#[test]
fn the_job_still_installs_the_prebuilt_that_made_the_default_wrong() {
    let steps = workflow_steps(NIGHTLY);
    let installs = steps
        .iter()
        .filter(|step| step.job == "fuzz" && step.uses.starts_with("taiki-e/install-action"))
        .count();

    assert_eq!(
        installs,
        1,
        "this is the premise of the fix rather than the fix. The job installs cargo-fuzz from a \
         prebuilt archive whose own triple is musl, which is *why* the default target cannot be \
         trusted. If this ever stops being true the reason for the `--target` changes and the \
         comment beside it has to change with it:\n{}",
        read(NIGHTLY)
    );
}

/// The runner whose own triple is [`GNU`].
const RUNNER: &str = "ubuntu-24.04";

#[test]
fn the_fuzz_job_asks_for_the_runner_whose_own_triple_is_the_one_it_pins() {
    let labels: Vec<Vec<String>> = workflow_jobs(NIGHTLY)
        .into_iter()
        .filter(|job| job.id == "fuzz")
        .map(|job| job.runs_on)
        .collect();

    assert_eq!(
        labels,
        vec![vec![RUNNER.to_owned()]],
        "`{GNU}` is the right triple only because the job is assigned an x86-64 Linux runner, \
         whose own triple it is. Read off the `fuzz` job rather than searched for in the file: \
         two other jobs of this workflow ask for `{RUNNER}` too, so a search passes however this \
         job is scheduled, and `ubuntu-24.04-arm` even contains the needle while making the \
         pinned triple a cross build with no `rustup target add` behind it — which is the \
         original failure again by another route"
    );
}

/// Two jobs, one of them the `fuzz` job, asking for different runners.
///
/// The shape the searched-for-in-the-file assertion could not tell apart: the
/// needle `ubuntu-24.04` is in this document twice over, once as a prefix of
/// the arm label the `fuzz` job actually asks for.
const TWO_RUNNERS: &str = r#"
name: Nightly
jobs:
  mutants:
    runs-on: ubuntu-24.04
    steps:
      - run: cargo mutants
  fuzz:
    runs-on: ubuntu-24.04-arm
    steps:
      - run: cargo fuzz run t --target x86_64-unknown-linux-gnu
"#;

#[test]
fn a_runner_label_is_read_off_the_job_that_asks_for_it_and_matched_whole() {
    let parsed = parse_yaml(TWO_RUNNERS).expect("the fixture is valid YAML");
    let labels: Vec<(String, Vec<String>)> = workflow_jobs_of("<fixture>", &parsed)
        .into_iter()
        .map(|job| (job.id, job.runs_on))
        .collect();

    assert_eq!(
        labels,
        vec![
            ("mutants".to_owned(), vec![RUNNER.to_owned()]),
            ("fuzz".to_owned(), vec!["ubuntu-24.04-arm".to_owned()]),
        ],
        "the calibration for the assertion above: each job's own labels, and the arm label read \
         as itself rather than as the label it has as a prefix. A reader that answered \
         `{RUNNER}` for the `fuzz` job here would pass the whole-file search and miss the cross \
         build:\n{TWO_RUNNERS}"
    );
}
