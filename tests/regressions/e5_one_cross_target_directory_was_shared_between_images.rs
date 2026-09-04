// SPDX-License-Identifier: MIT OR Apache-2.0
//! Three `cross` builds for three triples shared one `CARGO_TARGET_DIR`, and
//! the host build scripts compiled inside the first image could not run inside
//! the second.
//!
//! **What went wrong.** `cross` runs the compiler inside a per-triple
//! container, and the images do not share a glibc: the
//! `x86_64-unknown-linux-musl` image is a modern Ubuntu, the
//! `x86_64-unknown-linux-gnu` image is deliberately ancient so that the
//! artifact it produces has a low glibc floor. A build script is compiled *for
//! the host* and cached under the target directory, so pointing both builds at
//! one `CARGO_TARGET_DIR` hands the second image a `build-script-build` linked
//! against the first image's libc:
//!
//! ```text
//! /target/release/build/libc-.../build-script-build: /lib/x86_64-linux-gnu/libc.so.6:
//!   version `GLIBC_2.28' not found
//! ```
//!
//! (run <https://github.com/P4suta/ginary/actions/runs/33658759531>). The
//! `smoke-matrix` job died before it built a single stub, so neither the OTP
//! repack nor the matrix script it exists to run ever executed. The
//! `cross-build` job did not fail, because there each triple is a matrix row
//! with a runner, and therefore a target directory, of its own.
//!
//! **The input.** One step that invokes `cross build` for more than one triple
//! with a fixed `CARGO_TARGET_DIR`. `nightly.yml` carries a copy of the same
//! loop.
//!
//! **The correct behaviour.** A target directory belongs to one cross image.
//! A step that builds several triples gives each one its own — a
//! `CARGO_TARGET_DIR` that varies with the triple, or a separate `--target-dir`
//! per build. The comment in `ci.yml` already states the rule ("a build script
//! linked against this runner's glibc does not leak into the cross build;
//! hence a separate target dir"); it was one directory short of it.

use crate::common::repo::{WorkflowStep, workflow_steps, yaml_files_under};

/// The infixes that make a token a Rust target triple.
const TRIPLE_INFIXES: [&str; 3] = ["-unknown-linux-", "-pc-windows-", "-apple-darwin"];

/// Every literal Rust triple a step's script names, deduplicated, sorted.
///
/// A triple that arrives through `${{ matrix.triple }}` is deliberately not
/// one: the matrix row is a job, and every job gets its own runner and its own
/// target directory.
fn literal_triples(step: &WorkflowStep) -> Vec<String> {
    let mut found: Vec<String> = step
        .run
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.'))
        .filter(|token| TRIPLE_INFIXES.iter().any(|infix| token.contains(infix)))
        .map(str::to_owned)
        .collect();
    found.sort();
    found.dedup();
    found
}

/// The target directory a step's `cross build` invocations write into.
///
/// The step's own `env:` is only one of the three places it can be said, and
/// on GitHub it is the one place it *cannot* vary with the triple: `env:`
/// values are literal, never shell-expanded, so `CARGO_TARGET_DIR:
/// target/cross/$triple` would create a directory called `$triple`. A
/// per-image directory is therefore written inside the script — as an
/// assignment or an `export` around each build, or as an explicit
/// `--target-dir` — and *every* one the script names is read here, because a
/// script that starts with a per-triple directory and ends with a shared one
/// has the bug in its second half. Cargo's own default, `target`, is the
/// answer when nothing says otherwise.
fn target_directories(step: &WorkflowStep) -> Vec<String> {
    let mut named = Vec::new();
    for line in step.commands() {
        let mut rest = line.as_str();
        while let Some(value) = rest
            .split_once("CARGO_TARGET_DIR=")
            .map(|(_, after)| after)
            .or_else(|| {
                rest.split_once("--target-dir")
                    .map(|(_, after)| after.trim_start().trim_start_matches('='))
            })
        {
            named.push(
                value
                    .split_whitespace()
                    .next()
                    .unwrap_or_default()
                    .trim_matches(['"', '\''])
                    .to_owned(),
            );
            rest = value;
        }
    }
    if named.is_empty() {
        named.push(
            step.env
                .get("CARGO_TARGET_DIR")
                .cloned()
                .unwrap_or_else(|| "target".to_owned()),
        );
    }
    named
}

#[test]
fn no_step_builds_two_cross_images_into_one_target_directory() {
    let mut offenders: Vec<String> = Vec::new();
    let mut multi_triple_steps = 0usize;
    for workflow in yaml_files_under(".github/workflows") {
        for step in workflow_steps(&workflow) {
            if !step.run.contains("cross build") {
                continue;
            }
            let triples = literal_triples(&step);
            if triples.len() < 2 {
                continue;
            }
            multi_triple_steps += 1;
            // Every directory the script names, not the first one: a script
            // that gives the first build a per-triple directory and a later
            // one the shared default is the bug wearing a disguise. A
            // directory that varies with the triple contains a shell
            // expansion; a fixed one does not.
            for directory in target_directories(&step) {
                if !directory.contains('$') {
                    offenders.push(format!(
                        "{step} runs `cross build` for {} triples ({}) with target directory {} — \
                         one directory for {} container images",
                        triples.len(),
                        triples.join(", "),
                        directory,
                        triples.len()
                    ));
                }
            }
        }
    }
    assert!(
        multi_triple_steps > 0,
        "no step builds more than one triple through `cross` any more; this test has lost its \
         subject"
    );
    assert!(
        offenders.is_empty(),
        "a `cross` target directory belongs to one container image, and a host build script \
         cached in it does not run in another:\n{}",
        offenders.join("\n")
    );
}
