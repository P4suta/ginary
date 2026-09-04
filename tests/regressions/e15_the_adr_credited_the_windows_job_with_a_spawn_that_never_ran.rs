// SPDX-License-Identifier: MIT OR Apache-2.0
//! Correcting one sentence of ADR 0015 replaced a true claim with a false one.
//!
//! **What went wrong.** The Consequences section of
//! `docs/adr/0015-windows-launcher-stays-resident.md` used to say that "the
//! spawn, the job object, the console handler, the share-mode lock and
//! `erl.exe` itself have never run anywhere". The first Windows runner made
//! part of that stale — `erl.exe` ran, and the suite ran natively — so E15
//! rewrote it as "the spawn, the job object, the console handler and the
//! share-mode lock run nowhere but the `windows` job of
//! `.github/workflows/ci.yml`". That overshot. The `windows` job runs two
//! `cargo build`s, one `cargo test` and the exit-code probe; it starts no
//! packaged artifact, and no test in the tree constructs a `LaunchPlan` and
//! calls `launch_windows::run`. Of the four mechanisms named, exactly two are
//! reached by that job: `SharedLock`, through the `cfg(windows)` regression
//! tests, and `win32::process_is_alive`, through `cache::sweep`'s. The spawn,
//! the job object and the console handler are reached only by a real Windows
//! artifact starting a real runtime, which nothing in CI does — as the same
//! paragraph then conceded two sentences later, still owing "the end-to-end
//! run of a real artifact".
//!
//! **The input.** Reading the ADR. An unsafe exception and a resident-launcher
//! design that describe themselves as *exercised* invite exactly the review a
//! `#[allow(unsafe_code)]` is supposed to get and then answer it with a job
//! that never called the code.
//!
//! **The correct behaviour.** The claim is derivable, so it is derived: while
//! no test calls `launch_windows::run` and no step of the `windows` job starts
//! a packaged artifact, the ADR has to keep recording the spawn, the job
//! object and the console handler as unrun. The day one of those premises
//! stops holding, this test fails and points at the sentence to update.

use crate::common::repo::{read, root, workflow_steps};

/// The ADR this file holds to the tree.
const ADR: &str = "docs/adr/0015-windows-launcher-stays-resident.md";

/// The module the spawn, the job object and the console handler live in.
const MODULE: &str = "src/launch_windows.rs";

/// The one call site of `launch_windows::run`, reached by a launching
/// artifact and by nothing else.
const ONLY_CALLER: &str = "src/launcher.rs";

/// The call this file asks the tree about, spelled as a call rather than as a
/// name so that prose mentioning the function is not read as running it.
const THE_SPAWN: &str = "launch_windows::run(";

/// Every Rust source under `tests/`, as a repository-relative path.
///
/// The whole tree: the helpers under `tests/common/` are compiled into every
/// test binary, so a spawn started from one of those would run just as much as
/// one written in a test file.
fn test_sources() -> Vec<String> {
    let root = root();
    let mut found = Vec::new();
    let mut pending = vec![root.join("tests")];
    while let Some(directory) = pending.pop() {
        let entries = std::fs::read_dir(&directory)
            .unwrap_or_else(|error| panic!("cannot list {}: {error}", directory.display()));
        for entry in entries {
            let path = entry.expect("a directory entry").path();
            if path.is_dir() {
                pending.push(path);
                continue;
            }
            if path.extension().is_none_or(|suffix| suffix != "rs") {
                continue;
            }
            found.push(
                path.strip_prefix(&root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        }
    }
    found.sort();
    found
}

/// Every file that could make the spawn run under `cargo test`.
///
/// Two ways in: a test that calls `launch_windows::run` itself — the module is
/// `pub`, so an integration test on a Windows host could — or a `#[test]`
/// inside the module. Neither exists, and both are what the ADR's claim
/// rests on.
fn what_would_run_the_spawn() -> Vec<String> {
    let mut reachable = Vec::new();
    let myself = file!().replace('\\', "/");
    for relative in test_sources() {
        // This file spells the call in order to look for it, which is not a
        // call. `file!()` is the scanner's own path, so the exclusion cannot
        // go stale under a rename.
        if relative == myself {
            continue;
        }
        if read(&relative).contains(THE_SPAWN) {
            reachable.push(relative);
        }
    }
    if read(MODULE).contains("#[test]") {
        reachable.push(MODULE.to_owned());
    }
    for relative in ["src/launcher.rs", "src/lib.rs", "src/launch.rs"] {
        if relative != ONLY_CALLER && read(relative).contains(THE_SPAWN) {
            reachable.push(relative.to_owned());
        }
    }
    reachable
}

/// Every command the `windows` job of the CI workflow runs.
fn windows_job_commands() -> Vec<String> {
    workflow_steps(".github/workflows/ci.yml")
        .into_iter()
        .filter(|step| step.job == "windows")
        .flat_map(|step| step.commands())
        .filter(|command| !command.is_empty())
        .collect()
}

#[test]
fn no_test_and_no_ci_step_reaches_the_windows_spawn() {
    let reachable = what_would_run_the_spawn();
    assert!(
        reachable.is_empty(),
        "`{THE_SPAWN}..)` is reachable from the suite now — {reachable:?}. That is a better \
         tree than the one this test was written against, and it means the sentence in {ADR} \
         recording the spawn as unrun has to be rewritten around what those tests prove"
    );

    let commands = windows_job_commands();
    assert!(
        !commands.is_empty(),
        "the `windows` job of .github/workflows/ci.yml runs nothing, so this test is measuring \
         a job that no longer exists"
    );
    let starts_an_artifact: Vec<&String> = commands
        .iter()
        .filter(|command| command.contains("ginary.exe") || command.contains("release\\ginary"))
        .collect();
    assert!(
        starts_an_artifact.is_empty(),
        "the `windows` job starts a packaged artifact now — {starts_an_artifact:?} — which is \
         the end-to-end run {ADR} says that milestone still owes. Both statements in the ADR \
         change together"
    );
}

/// One paragraph of hard-wrapped prose as a single line.
///
/// Every needle below is a sentence, and this file's prose is wrapped at about
/// 100 columns, so a match that read the file as written would be defeated by
/// a reflow — which is exactly how a claim drifts back in unnoticed.
fn flowed(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn the_adr_credits_the_windows_job_with_what_it_actually_runs() {
    let adr = flowed(&read(ADR));
    for forbidden in [
        "the console handler and the share-mode lock run nowhere but the",
        "console handler and the share-mode lock run nowhere",
    ] {
        assert!(
            !adr.contains(forbidden),
            "{ADR} credits the `windows` job with the spawn, the job object and the console \
             handler. That job runs two `cargo build`s, one `cargo test` and the exit-code \
             probe: nothing in it constructs a `LaunchPlan`, and the same paragraph goes on to \
             say the end-to-end run of a real artifact is still owed. Offending text: \
             `{forbidden}`"
        );
    }
    assert!(
        adr.contains("The spawn, the job object and the console handler have still never run"),
        "{ADR} has to keep saying that the spawn, the job object and the console handler have \
         run nowhere, because {} calls `{THE_SPAWN}..)` and no test does",
        ONLY_CALLER
    );
    for needle in ["share-mode lock", "win32::process_is_alive"] {
        assert!(
            adr.contains(needle),
            "{ADR} names what the `windows` job does reach of this decision, and `{needle}` is \
             one of the two: a reader who is told the spawn is unrun has to be told what is not"
        );
    }
}
