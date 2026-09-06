// SPDX-License-Identifier: MIT OR Apache-2.0
//! Two platforms were proved on real runners and the documents went on calling
//! them unproven.
//!
//! **What went wrong.** The `macos` job of `.github/workflows/ci.yml` has, on
//! every push since E1 reached a green runner, packaged a `hello_ffi`
//! artifact, started it, and run `codesign --verify --strict` over ginary's own
//! output on both `macos-15-intel` and `macos-14`. Meanwhile `README.md` — the
//! document a person deciding whether to adopt ginary reads first — said
//! macOS support "has never been run on a Mac", and `docs/dev/v1-readiness.md`
//! — the fail-closed checklist that decides whether ginary is v1 — carried the
//! macOS launch as `CI-gated`, which it defines as "never marked done". The
//! same held for Windows in the other direction: E23 made that job package and
//! launch an artifact, and the same two documents kept the sentence saying no
//! Windows machine ever had.
//!
//! Both directions of that are a defect, and the second is the worse one. A
//! checklist whose rule is fail-closed is only honest while its items are
//! *read* from the tree; an item that stays deferred after the evidence exists
//! understates the project exactly as badly as an item marked done without it,
//! and it does so in the document that exists to prevent hand-waving.
//!
//! **The input.** Any tree where a CI job packages an application and starts
//! it while the documents describe that platform as unlaunched, or the reverse.
//!
//! **The correct behaviour.** The claim is derivable from the workflow, so it
//! is derived rather than trusted, for both platforms and in both directions.
//! A job that packages an artifact and then executes it is a launch; a job that
//! only builds is not.

use crate::common::repo::{WorkflowStep, read, workflow_steps};

/// The CI workflow both jobs live in.
const CI: &str = ".github/workflows/ci.yml";

/// The documents that describe where each platform's support stands.
const DOCUMENTS: [&str; 2] = ["README.md", "docs/dev/v1-readiness.md"];

/// Every step of one job of the CI workflow, in file order.
fn steps_of(job: &str) -> Vec<WorkflowStep> {
    workflow_steps(CI)
        .into_iter()
        .filter(|step| step.job == job)
        .collect()
}

/// The variables a script assigns a path under the build output directory to.
///
/// `build/ginary` is where `ginary build` leaves an artifact, in either
/// spelling of a path separator, and a job that has the path in a variable is
/// a job that can start it. Reading the assignment rather than the artifact's
/// file name keeps the rule working across the two jobs' two shells and across
/// the macOS job's `${{ matrix.target }}`, which is not a name any scanner can
/// resolve.
fn artifact_variables(script: &str) -> Vec<String> {
    let mut found = Vec::new();
    for line in script.lines() {
        let line = line.trim();
        let Some((left, right)) = line.split_once('=') else {
            continue;
        };
        if !(right.contains("build/ginary/") || right.contains("build\\ginary\\")) {
            continue;
        }
        let name = left.trim().trim_start_matches('$').trim();
        if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            found.push(name.to_owned());
        }
    }
    found
}

/// Whether one command line starts one of `variables` as a program.
///
/// The variable has to be the command, not an argument: `codesign --verify
/// "$artifact"` reads the file and starts nothing, and a rule that could not
/// tell those apart would have reported the macOS job as launching an artifact
/// on the two runs where it verified a signature over one that never ran.
///
/// A line is split on the separators a shell runs commands in sequence with,
/// because `set +e; "$artifact" 3; test $? -eq 3` is three commands and the
/// middle one is the launch.
fn starts_one_of(command: &str, variables: &[String]) -> bool {
    command
        .split([';', '\n'])
        .flat_map(|part| part.split("&&"))
        .flat_map(|part| part.split("||"))
        .any(|part| {
            let part = part.trim();
            // pwsh's call operator, the one way a script runs a program it
            // holds the path of in a variable.
            let part = part.strip_prefix("& ").unwrap_or(part).trim_start();
            let Some(word) = part.split_whitespace().next() else {
                return false;
            };
            let word = word.trim_matches(['"', '\'']);
            let Some(name) = word.strip_prefix('$') else {
                return false;
            };
            let name = name.trim_matches(['{', '}']);
            variables.iter().any(|variable| variable == name)
        })
}

/// One command with every run of whitespace collapsed to a single space.
///
/// `WorkflowStep::commands` joins a backslash continuation by replacing the
/// `\\\n` with one space, and the next line arrives with its own indentation
/// still on it. So the macOS job's
///
/// ```text
/// "$WORKSPACE/target/release/ginary" build \\
///   --target "${{ matrix.target }}"
/// ```
///
/// becomes `... build   --target ...` with three spaces, and a rule looking
/// for `build --target ` finds nothing and reports the one job that has
/// packaged and launched an artifact since E1 as a job that never has. Reading
/// a command as its words is the fix; matching a substring of one particular
/// wrapping is the defect.
fn words(command: &str) -> String {
    command.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Whether one job packages an application and then starts it.
fn job_launches_an_artifact(job: &str) -> bool {
    let steps = steps_of(job);
    assert!(
        !steps.is_empty(),
        "the `{job}` job of {CI} declares no steps, so this rule is measuring a job that no \
         longer exists"
    );
    let packages = steps.iter().any(|step| {
        step.commands()
            .iter()
            .any(|command| words(command).contains("build --target "))
    });
    let starts = steps.iter().any(|step| {
        let variables = artifact_variables(&step.run);
        !variables.is_empty()
            && step
                .commands()
                .iter()
                .any(|command| starts_one_of(&words(command), &variables))
    });
    packages && starts
}

/// One document as a single line, so that a needle spanning a hard wrap is
/// still found.
///
/// Every sentence below is longer than the 100 columns this repository wraps
/// prose at, so a match read against the file as written would be defeated by
/// a reflow — which is exactly how a stale claim survives an edit.
fn flowed(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The sentences that say a platform has never been launched, as each document
/// spelled them before E23.
///
/// Verbatim from the tree they were removed from: a defect record that
/// paraphrases what it forbids cannot tell whether the paraphrase came back.
const NEVER_LAUNCHED: [(&str, &str); 6] = [
    (
        "windows",
        "no Windows machine has ever started a packaged application",
    ),
    (
        "windows",
        "no Windows machine has run a packaged application yet",
    ),
    (
        "windows",
        "no Windows machine has started a packaged application and propagated",
    ),
    ("macos", "has never been run on a Mac"),
    ("macos", "nothing built for macOS has run anywhere"),
    ("macos", "no Mach-O has ever been executed"),
];

#[test]
fn the_scanner_tells_a_launch_from_a_build_and_from_a_read() {
    let variables = vec!["artifact".to_owned()];
    assert!(
        starts_one_of("\"$artifact\" 0 hello world", &variables),
        "a quoted variable standing as the command is a launch"
    );
    assert!(
        starts_one_of("& $artifact 3 | Out-Null", &variables),
        "and so is pwsh's call operator applied to one"
    );
    assert!(
        starts_one_of("set +e; \"$artifact\" 3; test $? -eq 3; set -e", &variables),
        "a launch in the middle of a line of three commands is still a launch"
    );
    assert!(
        !starts_one_of(
            "codesign --verify --strict --verbose=4 \"$artifact\"",
            &variables
        ),
        "reading a file is not starting it; the macOS job verified a signature over an artifact \
         that segfaulted on exec for two whole runs"
    );
    assert!(
        !starts_one_of("otool -l \"$artifact\" | grep -E \"$fields\"", &variables),
        "and neither is dumping its load commands"
    );
    assert_eq!(
        words("\"$W/target/release/ginary\" build   --target \"${{ matrix.target }}\""),
        "\"$W/target/release/ginary\" build --target \"${{ matrix.target }}\"",
        "a build wrapped over two lines arrives with the continuation's indentation in the \
         middle of it, and a rule that matches one particular wrapping is a rule about \
         formatting"
    );
    assert_eq!(
        artifact_variables("artifact=\"$work/hello_ffi/build/ginary/hello_ffi-x\"\n"),
        vec!["artifact".to_owned()],
        "the bash assignment the macOS job makes is found"
    );
    assert_eq!(
        artifact_variables(
            "$artifact = Join-Path $p 'build\\ginary\\hello_ffi-windows-x86_64.exe'"
        ),
        vec!["artifact".to_owned()],
        "and so is the pwsh one the Windows job makes"
    );
    assert!(
        artifact_variables("stub=\"$GITHUB_WORKSPACE/target/stubs/ginary-stub-1-x\"\n").is_empty(),
        "a stub is not an artifact: it is what an artifact is built from"
    );
}

#[test]
fn a_platform_ci_launches_is_not_described_as_one_nothing_has_launched() {
    for (job, sentence) in NEVER_LAUNCHED {
        let launches = job_launches_an_artifact(job);
        for document in DOCUMENTS {
            let text = flowed(&read(document));
            if launches {
                assert!(
                    !text.contains(sentence),
                    "the `{job}` job of {CI} packages an application and starts it on every \
                     push, and {document} still says `{sentence}`. A document that understates \
                     what is proved costs a reader the platform they were evaluating, and \
                     v1-readiness is fail-closed in the other direction only"
                );
            } else {
                assert!(
                    text.contains(sentence),
                    "nothing in the `{job}` job starts a packaged application any more, so \
                     {document} has to go back to saying so: `{sentence}`"
                );
            }
        }
    }
}

#[test]
fn each_launched_platform_names_the_runner_that_launched_it() {
    // The other half: removing the stale sentence is not the same as recording
    // what replaced it. A reader who is told a platform is proved has to be
    // able to go and read the job that proves it.
    for (job, runners) in [
        ("windows", &["windows-2022"][..]),
        ("macos", &["macos-15-intel", "macos-14"][..]),
    ] {
        if !job_launches_an_artifact(job) {
            continue;
        }
        for document in DOCUMENTS {
            let text = read(document);
            for runner in runners {
                assert!(
                    text.contains(runner),
                    "{document} says the `{job}` launch is proved and never names `{runner}`, \
                     the image it is proved on"
                );
            }
        }
    }
}
