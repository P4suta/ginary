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
//! **The correct behaviour.** The claim is derivable, so it is derived, and in
//! both directions: while no test calls `launch_windows::run` and no step of
//! the `windows` job starts a packaged artifact, the ADR has to record the
//! spawn, the job object and the console handler as unrun; once one of those
//! premises stops holding it has to stop saying so, and name what reaches
//! them instead.
//!
//! **E23 is the day that happened.** The `windows` job now packages a
//! `hello_ffi` artifact against the runtime `setup-beam` installs and starts
//! it, which is the only thing that constructs a `LaunchPlan` and calls
//! `launch_windows::run` anywhere. This file did what it was written to do: it
//! went red on the workflow change and pointed at the sentence to rewrite. It
//! stays, aimed the other way, because a claim that drifted twice can drift
//! back.

use crate::common::repo::{WorkflowStep, read, root, workflow_steps};

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

/// The name of the packaged application the `windows` job builds and starts.
///
/// The same fixture the macOS job packages, so that what the two runners prove
/// differs in the operating system and in nothing else.
const ARTIFACT: &str = "hello_ffi-windows-x86_64";

/// Every step of the `windows` job of the CI workflow, in file order.
fn windows_steps() -> Vec<WorkflowStep> {
    workflow_steps(".github/workflows/ci.yml")
        .into_iter()
        .filter(|step| step.job == "windows")
        .collect()
}

/// The variable a pwsh script assigns the packaged artifact's path to.
///
/// Reading the *mechanism* rather than a substring of a path is the whole
/// difference between this scanner and the one it replaces. The first version
/// asked whether any command of the job mentioned `ginary.exe`, which is true
/// of `cp target/stub/release/ginary.exe …` and of the line that resolves the
/// command line tool — neither of which starts anything. A job that copies a
/// file is not a job that launches an application, and a rule that cannot tell
/// them apart reports the ADR as stale on a tree where it is exact.
fn artifact_variable(script: &str) -> Option<String> {
    for line in script.lines() {
        let line = line.trim();
        let Some((left, right)) = line.split_once('=') else {
            continue;
        };
        if !right.contains(ARTIFACT) {
            continue;
        }
        let name = left.trim();
        if let Some(bare) = name.strip_prefix('$')
            && !bare.is_empty()
            && bare.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return Some(name.to_owned());
        }
    }
    None
}

/// Whether the `windows` job packages an application and then starts it.
///
/// Two facts, both required. A job that builds an artifact and never runs it
/// is what this repository had before E23, and it leaves `launch_windows::run`
/// unexecuted however much it builds; a job that runs something it did not
/// package is not running this launcher.
fn windows_job_starts_a_packaged_artifact() -> bool {
    let steps = windows_steps();
    let packages = steps.iter().any(|step| {
        step.commands()
            .iter()
            .any(|command| command.contains("build --target windows-x86_64"))
    });
    let starts = steps.iter().any(|step| {
        let Some(variable) = artifact_variable(&step.run) else {
            return false;
        };
        // `& $artifact …` — pwsh's call operator, the one way a script runs a
        // program it has a path to.
        step.commands().iter().any(|command| {
            command
                .trim()
                .strip_prefix("& ")
                .and_then(|rest| rest.split_whitespace().next())
                .is_some_and(|word| word == variable)
        })
    });
    packages && starts
}

#[test]
fn the_scanner_tells_a_copy_from_a_launch() {
    // The half that made the old rule wrong. Both of these name a ginary
    // binary and neither starts an application.
    assert_eq!(
        artifact_variable("cp target/stub/release/ginary.exe target/stubs/ginary-stub.exe\n"),
        None,
        "a copy assigns nothing and starts nothing"
    );
    assert_eq!(
        artifact_variable(
            "$ginary = Join-Path $env:GITHUB_WORKSPACE 'target\\release\\ginary.exe'"
        ),
        None,
        "the path of the command line tool is not the path of a packaged application"
    );
    assert_eq!(
        artifact_variable(
            "$artifact = Join-Path $project 'build\\ginary\\hello_ffi-windows-x86_64.exe'"
        )
        .as_deref(),
        Some("$artifact"),
        "and the one assignment that does name the packaged application is found"
    );
}

#[test]
fn the_windows_job_reaches_the_spawn_and_the_adr_says_which_step_does() {
    let reachable = what_would_run_the_spawn();
    assert!(
        reachable.is_empty(),
        "`{THE_SPAWN}..)` is reachable from the suite now — {reachable:?}. That is a better \
         tree than the one this test was written against, and it means the sentence in {ADR} \
         naming the CI job as the only thing that reaches the spawn has to be rewritten around \
         what those tests prove"
    );
    assert!(
        !windows_steps().is_empty(),
        "the `windows` job of .github/workflows/ci.yml runs nothing, so this test is measuring \
         a job that no longer exists"
    );
    assert!(
        windows_job_starts_a_packaged_artifact(),
        "the `windows` job no longer packages an application and starts it. That step is the \
         only execution the spawn, the job object and the console control handler have \
         anywhere, and {ADR} rests an `#[allow(unsafe_code)]` on describing them accurately: \
         without it the ADR has to go back to recording all three as unrun"
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
    let exercised = windows_job_starts_a_packaged_artifact();

    // The sentence this file was written to defend, and the reason it is now
    // forbidden rather than required: while nothing started a packaged
    // artifact it was the exact truth, and the moment something does it is a
    // stale claim about an `#[allow(unsafe_code)]`. Both readings are wrong at
    // the other time, so the premise decides which one the ADR must carry.
    let unrun = "The spawn, the job object and the console handler have still never run";
    if exercised {
        assert!(
            !adr.contains(unrun),
            "the `windows` job packages an application and starts it, so {ADR} may no longer \
             say `{unrun}`: that step is the first execution those three mechanisms have ever \
             had"
        );
        for needle in [
            // Which job, so the claim can be re-read rather than believed.
            "`windows` job",
            // And what it does there, so a job that goes back to building
            // without launching is a visible difference rather than a silent
            // one.
            "hello_ffi",
        ] {
            assert!(
                adr.contains(needle),
                "{ADR} rests an `#[allow(unsafe_code)]` on saying what exercises the spawn, the \
                 job object and the console control handler, and it does not mention `{needle}`"
            );
        }
    } else {
        assert!(
            adr.contains(unrun),
            "nothing in the tree starts a packaged Windows artifact, so {ADR} has to say so: \
             `{unrun}`"
        );
    }

    for forbidden in [
        "the console handler and the share-mode lock run nowhere but the",
        "console handler and the share-mode lock run nowhere",
    ] {
        assert!(
            !adr.contains(forbidden),
            "{ADR} credits the `windows` job with more than one sentence can carry. What that \
             job reaches has to be named step by step, because the same paragraph once claimed \
             a spawn it never ran. Offending text: `{forbidden}`"
        );
    }
    for needle in ["share-mode lock", "win32::process_is_alive"] {
        assert!(
            adr.contains(needle),
            "{ADR} names what the `windows` job reaches of this decision, and `{needle}` is one \
             of them: a reader told about the spawn has to be told about the rest"
        );
    }
}
