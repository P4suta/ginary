// SPDX-License-Identifier: MIT OR Apache-2.0
//! The macOS job built the launcher-only stub over the command line binary and
//! then asked the stub to package an artifact.
//!
//! **What went wrong.** Both flavors of ginary are one Cargo target, so both
//! land at `target/release/ginary`. The `macos` job of `ci.yml` ran
//! `cargo build --release` (the full tool), then `cargo build
//! --no-default-features --release` (the stub) into the same path, and then
//! invoked `"$GITHUB_WORKSPACE/target/release/ginary" build`. By then that file
//! was the stub, which parses no arguments by design:
//!
//! ```text
//! this is a ginary launcher stub for macos-aarch64; it carries no payload and no CLI
//! ##[error]Process completed with exit code 2.
//! ```
//!
//! (run <https://github.com/P4suta/ginary/actions/runs/33658759531>). So the
//! job that closes the D3 "awaits a Mac runner" gap never packaged an artifact,
//! never ran one, and never reached `codesign --verify` — the three things it
//! exists for.
//!
//! **The input.** Any job that builds both flavors into the default target
//! directory and then executes `target/release/ginary` as a command line tool.
//!
//! **The correct behaviour.** When a step runs the command line tool out of
//! the target directory, the most recent build to have written that path must
//! be the full-feature one. Building the stub somewhere else (`--target-dir`,
//! `CARGO_TARGET_DIR`) and building it before the full binary both satisfy
//! that; leaving the stub in place does not.

use crate::common::repo::{WorkflowStep, workflow_steps};

/// The workflows whose jobs both build ginary and run it.
const WORKFLOWS: [&str; 3] = [
    ".github/workflows/ci.yml",
    ".github/workflows/distribute.yml",
    ".github/workflows/nightly.yml",
];

/// The subcommands that prove a step is running the *command line tool*
/// rather than copying a file or launching a packaged artifact.
const SUBCOMMANDS: [&str; 6] = ["build", "inspect", "otp", "doctor", "stage", "verify"];

/// Which flavor a `cargo build` step leaves at the default `target/release`
/// path, or `None` when the step writes somewhere else or does not build.
///
/// `--target <triple>` and `--target-dir <path>` both move the output, and so
/// does a `CARGO_TARGET_DIR` in the step's environment. `cross` is never this:
/// it always builds for a named triple.
fn default_path_build(step: &WorkflowStep) -> Option<bool> {
    if step.env.contains_key("CARGO_TARGET_DIR") {
        return None;
    }
    let mut flavor = None;
    // `commands()` and not `run.lines()`: a `cargo build` wrapped over two
    // lines for width is one build, and reading its first line alone would
    // report the flavor of half a command.
    for line in step.commands() {
        let line = line.as_str();
        if !line.contains("cargo build") {
            continue;
        }
        if line.contains("--target-dir") || line.contains("--target ") || line.contains("--target=")
        {
            continue;
        }
        flavor = Some(line.contains("--no-default-features"));
    }
    flavor
}

/// Whether a step invokes `target/release/ginary` as the command line tool.
fn runs_the_command_line_tool(step: &WorkflowStep) -> bool {
    step.commands().iter().any(|line| {
        let Some((before, after)) = line.split_once("target/release/ginary") else {
            return false;
        };
        if before.trim_start().starts_with("cp ") {
            return false;
        }
        let word = after
            .trim_start_matches(['"', '\'', ' ', '\\'])
            .split_whitespace()
            .next()
            .unwrap_or_default();
        SUBCOMMANDS.contains(&word)
    })
}

#[test]
fn a_build_wrapped_over_two_lines_is_still_one_build() {
    // The rule below reads one command per line, and every workflow in this
    // tree happens to write each `cargo build` on one. That is a fact about
    // today's formatting, not about the rule, so the rule is asserted against
    // formatting it does not yet see: rewrapping a build for width moves
    // `--no-default-features` or `--target-dir` onto a continuation line, and
    // a scanner reading physical lines would then answer about half a command
    // — reporting a stub build as a full one, or losing the build entirely.
    let wrapped = |run: &str| {
        default_path_build(&WorkflowStep {
            workflow: "<none>".to_owned(),
            job: "<none>".to_owned(),
            position: 1,
            name: "<none>".to_owned(),
            run: run.to_owned(),
            uses: String::new(),
            shell: String::new(),
            with: std::collections::BTreeMap::new(),
            env: std::collections::BTreeMap::new(),
        })
    };
    assert_eq!(
        wrapped(
            "cargo build --release --locked \\
  --no-default-features
"
        ),
        Some(true),
        "a stub build wrapped after `--locked` is a stub build"
    );
    assert_eq!(
        wrapped(
            "cargo build --no-default-features --release \\
  --target-dir target/stub
"
        ),
        None,
        "a build whose `--target-dir` is on the continuation line does not write the default path"
    );
    assert_eq!(
        wrapped(
            "cargo build --release --locked
"
        ),
        Some(false),
        "and an unwrapped full build is unchanged by any of this"
    );
}

#[test]
fn the_build_a_job_runs_is_the_one_it_last_built() {
    let mut offenders: Vec<String> = Vec::new();
    let mut consumers = 0usize;
    for workflow in WORKFLOWS {
        let steps = workflow_steps(workflow);
        assert!(!steps.is_empty(), "{workflow} declares no steps at all");
        for (index, step) in steps.iter().enumerate() {
            if !runs_the_command_line_tool(step) {
                continue;
            }
            consumers += 1;
            let last = steps[..index]
                .iter()
                .filter(|earlier| earlier.job == step.job)
                .filter_map(|earlier| default_path_build(earlier).map(|stub| (earlier, stub)))
                .next_back();
            match last {
                Some((earlier, true)) => offenders.push(format!(
                    "{step} runs `target/release/ginary`, but {earlier} last wrote that path with \
                     `--no-default-features`: it is the launcher stub, which parses no arguments"
                )),
                Some((_, false)) => {}
                None => offenders.push(format!(
                    "{step} runs `target/release/ginary`, and no earlier step of that job built it"
                )),
            }
        }
    }
    assert!(
        consumers > 0,
        "no job in {WORKFLOWS:?} runs the command line tool out of the target directory any \
         more; this test has lost its subject"
    );
    assert!(
        offenders.is_empty(),
        "both flavors of ginary are one Cargo target and share `target/release/ginary`:\n{}",
        offenders.join("\n")
    );
}
