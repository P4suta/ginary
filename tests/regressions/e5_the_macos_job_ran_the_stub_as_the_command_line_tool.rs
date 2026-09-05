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

/// The variable cargo reads the output directory out of.
const TARGET_DIR: &str = "CARGO_TARGET_DIR";

/// Which flavor a `cargo build` step leaves at the default `target/release`
/// path, or `None` when the step writes somewhere else or does not build.
///
/// `--target <triple>` and `--target-dir <path>` both move the output, and so
/// does `CARGO_TARGET_DIR`. `cross` is never this: it always builds for a
/// named triple.
///
/// **Where that variable comes from is the script's business as much as the
/// YAML's.** Cargo reads it out of the environment, and a `run:` block puts it
/// there in two ways the workflow's `env:` mapping never mentions: an
/// assignment written on one command sets it for that command alone, and an
/// `export` sets it for every command after it in the same script. A rule that
/// read only `step.env` answered about a variable the build never saw, and the
/// answer it gave — "this stub landed on the default path" — rejected a
/// correct workflow. A guard that fails a good tree is a guard the next author
/// deletes, so all three spellings are read.
fn default_path_build(step: &WorkflowStep) -> Option<bool> {
    if step.env.contains_key(TARGET_DIR) {
        return None;
    }
    let mut flavor = None;
    // Whether an earlier command in this same script exported the variable, in
    // which case every build after it writes somewhere else.
    let mut exported = false;
    // `commands()` and not `run.lines()`: a `cargo build` wrapped over two
    // lines for width is one build, and reading its first line alone would
    // report the flavor of half a command.
    for line in step.commands() {
        let line = line.as_str();
        if exports_target_dir(line) {
            exported = true;
        }
        if !line.contains("cargo build") {
            continue;
        }
        if exported || assigns_target_dir_for_one_command(line) {
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

/// The name a `NAME=value` word assigns, when the word is a shell assignment.
///
/// A shell name is a letter or `_` followed by letters, digits and `_`, and
/// nothing else is an assignment: `--target-dir=x` is an option and
/// `a=b=c` assigns `a`.
fn assignment_name(word: &str) -> Option<&str> {
    let (name, _) = word.split_once('=')?;
    let mut characters = name.chars();
    let first = characters.next()?;
    if !first.is_ascii_alphabetic() && first != '_' {
        return None;
    }
    characters
        .all(|c| c.is_ascii_alphanumeric() || c == '_')
        .then_some(name)
}

/// Whether `command` puts [`TARGET_DIR`] into the environment of every command
/// after it in the same script.
///
/// `export CARGO_TARGET_DIR=..` does, and so does a bare assignment standing
/// as a command of its own — a shell keeps that one in the script's own
/// environment, which every later command inherits.
fn exports_target_dir(command: &str) -> bool {
    let mut words = command.split_whitespace();
    let Some(first) = words.next() else {
        return false;
    };
    if first == "export" {
        return words.any(|word| assignment_name(word) == Some(TARGET_DIR));
    }
    assignment_name(first) == Some(TARGET_DIR) && words.next().is_none()
}

/// Whether `command` assigns [`TARGET_DIR`] for the one command it runs.
///
/// The assignments a command carries are the words before the command name,
/// and they govern that command and nothing after it.
fn assigns_target_dir_for_one_command(command: &str) -> bool {
    command
        .split_whitespace()
        .map_while(assignment_name)
        .any(|name| name == TARGET_DIR)
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

/// A step that runs `script` with no `env:`, `uses:`, `with:` or `shell:` of
/// its own.
///
/// The scanners under test read a step's script and its environment and
/// nothing else, so every other field is a placeholder — spelled `<none>` so
/// that a failure message which prints one is obviously not naming a real job.
fn plain_step(script: &str) -> WorkflowStep {
    WorkflowStep {
        workflow: "<none>".to_owned(),
        job: "<none>".to_owned(),
        position: 1,
        id: String::new(),
        name: "<none>".to_owned(),
        cond: String::new(),
        run: script.to_owned(),
        uses: String::new(),
        shell: String::new(),
        with: std::collections::BTreeMap::new(),
        env: std::collections::BTreeMap::new(),
    }
}

#[test]
fn a_target_directory_the_script_sets_itself_moves_the_build_off_the_default_path() {
    // `CARGO_TARGET_DIR` is read from the environment, and a `run:` block can
    // put it there without the workflow's `env:` ever mentioning it: a
    // one-command assignment sets it for that command alone, and an `export`
    // sets it for every command after it in the same script. A rule that reads
    // only the YAML `env:` mapping answers about a variable the build never
    // saw, and the answer it gives — "this stub landed on the default path" —
    // rejects a workflow that is correct. A guard that fails a good tree is a
    // guard the next author deletes.
    assert_eq!(
        default_path_build(&plain_step(
            "CARGO_TARGET_DIR=target/stub cargo build --no-default-features --release\n"
        )),
        None,
        "a one-command assignment moves that build's output as surely as `--target-dir` does"
    );
    assert_eq!(
        default_path_build(&plain_step(
            "export CARGO_TARGET_DIR=target/stub\ncargo build --no-default-features --release\n"
        )),
        None,
        "and an `export` moves every build after it in the same script"
    );
    // The narrow half: an assignment written on one command governs that
    // command only, so the next `cargo build` is back on the default path and
    // its flavor is the one that matters.
    assert_eq!(
        default_path_build(&plain_step(
            "CARGO_TARGET_DIR=target/stub cargo build --no-default-features --release\n\
             cargo build --release --locked\n"
        )),
        Some(false),
        "a per-command assignment does not carry to the command after it"
    );
    // And the rule the file already had is unchanged: a step whose YAML `env:`
    // names the variable writes nowhere near the default path.
    let mut env = std::collections::BTreeMap::new();
    env.insert("CARGO_TARGET_DIR".to_owned(), "target/cross".to_owned());
    let mut step = plain_step("cargo build --release --locked\n");
    step.env = env;
    assert_eq!(
        default_path_build(&step),
        None,
        "the step environment is still read, and still moves the output"
    );
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
            id: String::new(),
            name: "<none>".to_owned(),
            cond: String::new(),
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

#[test]
fn a_line_a_shell_never_runs_is_not_a_build_and_not_a_consumer() {
    // Both scanners in this file read a `run:` block, and a `run:` block is
    // shell. Everything after an unquoted `#` is a comment the runner never
    // executes, so a rule that read one asserted about a command that does not
    // exist — in both directions. A commented-out stub build reports an
    // offender against a workflow whose real last build is the full tool, and a
    // commented-out `target/release/ginary build` invents a consumer for a job
    // that has none. E16 put the comment stripper in
    // `common::repo::shell_code` and gave `WorkflowStep::commands` its only
    // copy, for the reason the milestone states about lexers: two of them
    // drift.
    assert_eq!(
        default_path_build(&plain_step(
            "cargo build --release --locked\n# cargo build --no-default-features --release\n"
        )),
        Some(false),
        "a commented-out stub build is not the last thing that wrote `target/release/ginary`"
    );
    assert_eq!(
        default_path_build(&plain_step("# cargo build --release --locked\n")),
        None,
        "and a step whose only `cargo build` is commented out builds nothing at all"
    );
    // The other half of the same rule: a `#` inside a quoted word is an
    // ordinary character, so the command around it is still a command.
    assert_eq!(
        default_path_build(&plain_step(
            "cargo build --release --locked --config 'build.rustflags=[\"--cfg=a#b\"]'\n"
        )),
        Some(false),
        "a `#` inside a quoted word opens no comment"
    );
    assert!(
        !runs_the_command_line_tool(&plain_step(
            "# target/release/ginary build --target linux-x86_64-musl\n"
        )),
        "a commented-out invocation is not a step that runs the command line tool"
    );
    assert!(
        runs_the_command_line_tool(&plain_step(
            "target/release/ginary build --target linux-x86_64-musl # the cross build\n"
        )),
        "and a real invocation with a comment after it still is one"
    );
}
