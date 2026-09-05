// SPDX-License-Identifier: MIT OR Apache-2.0
//! A test that asks `git` what this repository ignores was run in a copy of
//! the tree that has no `.git`, and read the failure as an answer.
//!
//! **What went wrong.** Every `Mutation testing` shard of the `Nightly`
//! workflow has failed, and none of them failed on a mutant. They failed on
//! the unmutated baseline:
//!
//! ```text
//! FAILED   Unmutated baseline in 70s build + 144s test
//! fatal: not a git repository (or any of the parent directories): .git
//! fatal: not a git repository (or any of the parent directories): .git
//! test the_catalog_is_committed_and_the_tarballs_beside_it_are_not ... FAILED
//! thread '…' panicked at tests/smoke_matrix.rs:182:5:
//! and the 40 MB tarballs beside it are not
//! test result: FAILED. 9 passed; 1 failed; 0 ignored
//! error: test failed, to rerun pass `-p ginary --test smoke_matrix`
//! ```
//!
//! (`Mutation testing (payload)`, run `33953295452`,
//! <https://github.com/P4suta/ginary/actions/runs/33953295452/job/101271940898>.)
//!
//! **The input.** `cargo mutants` builds and tests inside a *copy* of the
//! tree, at `/tmp/cargo-mutants-ginary-<random>.tmp`, and by default the copy
//! carries no `.git`. `the_catalog_is_committed_and_the_tarballs_beside_it_are_not`
//! shells out to `git check-ignore -q <path>` and takes the exit status as the
//! answer: zero means ignored, non-zero means tracked. Outside a work tree
//! `git` exits `128` for every path, so every question answered "not
//! ignored" — which passed the assertion about `catalog.json` and failed the
//! one about the tarball beside it. The baseline failing means **no mutant was
//! ever run**, so the job's gate ("every mutant this module produces is
//! caught") has never been evaluated at all.
//!
//! **The correct behaviour.** The test asserts a property of *the repository*,
//! not of the code, and a copy with no `.git` genuinely cannot answer it. So
//! it stands down, out loud, naming the directory that could not answer and
//! why — `crate::common::tools::require_git_work_tree`, whose rule is written
//! as the table in `crate::common::tools::work_tree_gate`. A `git` that is
//! missing, or that is on `PATH` and will not start, still escalates under
//! `GINARY_REQUIRE_TOOLCHAIN`, because a job can install or fix one; a
//! directory that is not a checkout never does, because nobody can install
//! being one.
//!
//! The question the gate asks is `git -C <dir> rev-parse --show-toplevel`
//! compared against `<dir>`, and the comparison is the point: being *inside* a
//! work tree is not being one. A copy of this tree unpacked under any
//! directory that is itself a checkout — and `cargo mutants` unpacks under
//! `TMPDIR`, which nothing says is outside every checkout — is inside a work
//! tree that knows nothing about it, where `git ls-files` lists nothing and
//! succeeds and `git check-ignore` answers about the enclosing repository.
//! That is this same failure with the gate wide open, so the gate closes
//! there too; the price, stated rather than hidden, is that a checkout of this
//! repository nested inside a larger one stands down out loud instead of
//! answering from its parent.
//!
//! Excluding these tests from the `cargo mutants` run would go the other way:
//! it would stop the check running in the one place it *can* answer, and a
//! repository-property test that silently vanishes from CI is worse than one
//! that says why it stood down.
//!
//! Three tracked sources ask `git` about this repository —
//! `tests/smoke_matrix.rs`, `tests/common/portability.rs` and
//! `tests/common/homepath.rs` — and they are held together rather than one at
//! a time, by the scan below.

use std::path::{Path, PathBuf};

use crate::common::bounded::{run_bounded, wait_bounded};
use crate::common::portability::tracked_test_sources;
use crate::common::repo::root;
use crate::common::script::{ShimStep, program, shim_sidecar};
use crate::common::srcscan::literal_sites;
use crate::common::tools::{
    GIT_REDIRECTING_VARS, LS_FILES_BUDGET, REQUIRE_VAR, WorkTreeAnswer, WorkTreeGate,
    WorkTreeProbe, git_command, git_unrunnable_reason, probe_own_work_tree, require_git_work_tree,
    require_tools, require_working_git, toolchain_required, work_tree_gate, work_tree_skip_message,
};

/// The name every test that shells out to `git` about this repository has to
/// go through.
const GATE: &str = "require_git_work_tree";

/// A directory of exactly the shape `cargo mutants` builds in.
///
/// The name is the one from the failing job's log, so a reader who greps for
/// it lands on both. What makes it answer "not a work tree" is that it is a
/// fresh temporary directory rather than the name it carries.
fn a_copy_of_the_tree() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("cargo-mutants-ginary-")
        .suffix(".tmp")
        .tempdir()
        .expect("a temporary directory")
}

/// A directory that really is a checkout, or the reason there is none.
///
/// The positive control the over-correction guards need, and it is built
/// instead of being read off [`root`] for the reason this whole file is
/// about: under `cargo mutants` the directory this suite runs from *is* the
/// copy with no `.git`, so a guard that asserted "the tree I was compiled
/// from is a checkout" would fail in precisely the job E19 exists to make
/// green. `git init` answers the same question — does the probe say
/// `OwnWorkTree` where the answer is yes — from anywhere.
///
/// `--quiet` keeps the hint about the default branch name off the log, and
/// no user identity is configured because nothing here commits.
///
/// Both ways `git init` can fail to answer are reasons rather than panics, and
/// the child is spawned by hand instead of through
/// [`run_bounded`](crate::common::bounded::run_bounded) for the second of
/// them: `run_bounded` turns a program that cannot be started into a panic,
/// and here — as in `probe_own_work_tree`, for the same reason — that is
/// something to report rather than a defect in this repository.
fn make_a_checkout(git: &Path) -> Result<tempfile::TempDir, String> {
    let checkout = tempfile::Builder::new()
        .prefix("ginary-e19-checkout-")
        .tempdir()
        .expect("a temporary directory");
    // Through `git_command`, so an inherited `GIT_DIR` cannot make this
    // `git init` create its repository somewhere the test never looks — which
    // it does: with `GIT_DIR` set, `git init` targets that path and the
    // fixture directory stays empty.
    let spawned = git_command(git)
        .arg("-C")
        .arg(checkout.path())
        .args(["init", "--quiet"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn();
    let child = match spawned {
        Ok(child) => child,
        Err(error) => return Err(git_unrunnable_reason(git, &error)),
    };
    let output = wait_bounded(child, LS_FILES_BUDGET, "git init");
    if !output.status.success() {
        return Err(format!(
            "`{}` could not make the checkout in {} that this file holds the probe against: \
             `git init` exited with {} and said `{}`",
            git.display(),
            checkout.path().display(),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(checkout)
}

/// The positive control, or a reported stand-down.
///
/// The fixture is the one thing in this file that needs a `git` that *works*
/// rather than one that merely exists, and [`require_tools`] cannot promise
/// that: it opens on any `git` on `PATH`. A `git` that is there and refuses
/// every invocation — a typo in `~/.gitconfig`, a bind-mounted `HOME` owned by
/// another uid — would otherwise turn four tests in this file red for a fact
/// about the machine rather than about their subject, while every other
/// repository-property test in the suite stands down cleanly in the same
/// condition.
///
/// So it follows the rule its own subject follows: a reported skip, escalated
/// under [`REQUIRE_VAR`] exactly as [`WorkTreeGate::FailGitRefused`] is,
/// because a `git` that will not run is something a job can be made to fix and
/// the jobs that run this target set the variable.
///
/// # Panics
///
/// If the fixture cannot be built and [`REQUIRE_VAR`] is `1`.
fn a_real_checkout(git: &Path) -> Option<tempfile::TempDir> {
    match make_a_checkout(git) {
        Ok(checkout) => Some(checkout),
        Err(reason) => {
            assert!(
                !toolchain_required(),
                "{reason}, and {REQUIRE_VAR}=1 forbids skipping"
            );
            eprintln!("skipping: {reason}");
            None
        }
    }
}

/// A `git` on `PATH` that runs, says `message`, and refuses with `128`.
///
/// The two conditions this file has no other way to reach: a `git` that is
/// found, starts, and will not answer. `128` is the status git itself uses
/// both for `not a git repository` and for every refusal, which is why the
/// probe has to read what was said rather than the status.
fn a_git_that_refuses(dir: &Path, message: &str) -> PathBuf {
    let git = program(
        dir,
        "git",
        &[ShimStep::PrintStderrFile, ShimStep::Exit(128)],
    );
    std::fs::write(shim_sidecar(&git, "stderr"), message).expect("the shim's stderr file");
    git
}

/// What a `git` says when it will not look at a repository this uid does not
/// own — the routine case for a checkout bind-mounted into a container, and
/// the one a reader has to be shown to know what to do about it.
const DUBIOUS_OWNERSHIP: &str =
    "fatal: detected dubious ownership in repository at '/srv/checkout/ginary'";

// --------------------------------------------------------- the probe --

#[test]
fn a_directory_that_is_not_a_checkout_is_not_a_git_work_tree() {
    let Some(git) = require_working_git() else {
        return;
    };
    let copy = a_copy_of_the_tree();

    assert_eq!(
        probe_own_work_tree(&git, copy.path()).probe,
        WorkTreeProbe::NotItsOwnWorkTree,
        "`git -C {} rev-parse --show-toplevel` fails with `not a git repository` outside a \
         checkout, and a probe that reports otherwise is the defect: it is what let `git \
         check-ignore`'s exit 128 be read as `not ignored`",
        copy.path().display()
    );
}

#[test]
fn a_directory_that_is_a_checkout_is_a_git_work_tree() {
    let Some(tools) = require_tools(&["git"]) else {
        return;
    };
    let Some(checkout) = a_real_checkout(tools.path("git")) else {
        return;
    };

    assert_eq!(
        probe_own_work_tree(tools.path("git"), checkout.path()).probe,
        WorkTreeProbe::OwnWorkTree,
        "the over-correction this pins: a probe that answered `false` everywhere would turn \
         every repository-property test into a permanent skip, which is the check deleted \
         rather than the check gated. {} is a checkout, and it is its own",
        checkout.path().display()
    );
}

#[test]
fn a_git_that_ran_and_refused_to_look_is_not_a_directory_that_is_not_a_checkout() {
    let bin = tempfile::tempdir().expect("a temporary directory");
    let git = a_git_that_refuses(bin.path(), DUBIOUS_OWNERSHIP);

    assert_eq!(
        probe_own_work_tree(&git, &root()).probe,
        WorkTreeProbe::GitRefused,
        "`git` exits 128 both where a directory belongs to no repository and where it refuses to \
         look at one, so a probe that reads every failure as an answer reports an ownership \
         refusal — or a typo in `~/.gitconfig`, which reaches every `git` child alike — as the \
         `cargo mutants` copy of the tree. {} still carries its `.git`",
        root().display()
    );
}

#[test]
fn a_refusal_is_reported_in_gits_own_words_and_escalates_where_a_missing_checkout_does_not() {
    let bin = tempfile::tempdir().expect("a temporary directory");
    let git = a_git_that_refuses(bin.path(), DUBIOUS_OWNERSHIP);
    let answer = probe_own_work_tree(&git, &root());

    assert!(
        answer.detail.contains(DUBIOUS_OWNERSHIP),
        "the sentence that tells the reader to run `git config --global --add safe.directory` is \
         `git`'s own, and a probe that returns a bare `false` throws it away: {}",
        answer.detail
    );
    assert!(
        answer.detail.contains(&git.display().to_string())
            && !answer.detail.contains("cargo mutants"),
        "and it names the program that refused rather than borrowing the other skip's \
         explanation: {}",
        answer.detail
    );
    assert_eq!(
        work_tree_gate(WorkTreeProbe::GitRefused, true),
        WorkTreeGate::FailGitRefused,
        "nobody can install being a checkout, which is why that row never escalates. An \
         ownership refusal is the other kind of thing entirely — one `git config --global --add \
         safe.directory` in the job makes it untrue — so a job that promises a working toolchain \
         and gets a `git` that will not look at this repository fails instead of standing down"
    );
}

#[test]
fn the_one_failure_that_is_an_answer_is_the_one_that_says_so() {
    let bin = tempfile::tempdir().expect("a temporary directory");
    let git = a_git_that_refuses(
        bin.path(),
        "fatal: not a git repository (or any of the parent directories): .git",
    );

    assert_eq!(
        probe_own_work_tree(&git, &root()),
        WorkTreeAnswer {
            probe: WorkTreeProbe::NotItsOwnWorkTree,
            detail: String::new(),
        },
        "the calibration for the refusal above: the two are the same exit status and differ only \
         in what `git` said, so a probe that called every 128 a refusal would turn the mutants \
         copy — the whole subject of this milestone — into a toolchain failure under \
         {REQUIRE_VAR}=1, which is the over-correction"
    );
}

#[test]
fn a_git_that_cannot_build_the_positive_control_is_reported_rather_than_asserted_away() {
    let bin = tempfile::tempdir().expect("a temporary directory");
    let git = a_git_that_refuses(bin.path(), DUBIOUS_OWNERSHIP);

    let reason = make_a_checkout(&git).expect_err(
        "a `git` that is on PATH and refuses every invocation cannot make a checkout, and \
         `require_tools` cannot promise otherwise: it opens on any `git` on PATH. Asserting the \
         fixture into existence turns four tests in this file red for a fact about the machine, \
         while every other repository-property test in the suite stands down cleanly in exactly \
         that condition",
    );
    assert!(
        reason.contains(&git.display().to_string())
            && reason.contains(DUBIOUS_OWNERSHIP)
            && reason.contains("128"),
        "and the stand-down names the program, quotes what it said and gives its status, so the \
         reader fixes a `git` rather than looking for a defect here: {reason}"
    );
}

#[test]
fn the_positive_control_is_really_built_where_git_works() {
    // The name is the whole of it: *where `git` works*. The guard below asks
    // whether the fixture builder reports a working `git` unable, and that
    // question only exists once there is a working `git` to report on. A `git`
    // that refuses every question — `detected dubious ownership` is the common
    // shape — makes the builder's "unable" a true answer, and failing on a true
    // answer would be this file testing the machine it runs on. So the health
    // check first, and the guard against it.
    let Some(git) = require_working_git() else {
        return;
    };
    let made = make_a_checkout(&git);

    let checkout = made.as_ref().expect(
        "the over-correction the test above could hide: a fixture builder that reported every \
         `git` unable would stand every guard in this file down and report a clean run, which is \
         the check deleted rather than gated. The `git` this ran with answered `--version`, so \
         its refusal here is the builder's to explain",
    );
    assert_eq!(
        probe_own_work_tree(&git, checkout.path()).probe,
        WorkTreeProbe::OwnWorkTree,
        "and what it built is a checkout: {}",
        checkout.path().display()
    );
}

#[test]
fn a_git_this_suite_starts_cannot_be_pointed_at_another_repository() {
    let Some(git) = require_working_git() else {
        return;
    };
    // The positive control first: the variable being removed has to be one
    // that would have changed the answer. `GIT_DIR` overrides `-C`, so a
    // `git init` that inherits it creates its repository at that path and
    // leaves the directory it was pointed at empty — which is the fixture
    // builder above being redirected by whatever shell ran `cargo test`.
    let elsewhere = a_copy_of_the_tree();
    let target = a_copy_of_the_tree();
    let redirected = elsewhere.path().join("redirected.git");
    let output = run_bounded(
        std::process::Command::new(&git)
            .env("GIT_DIR", &redirected)
            .arg("-C")
            .arg(target.path())
            .args(["init", "--quiet"]),
        LS_FILES_BUDGET,
        "git init under GIT_DIR",
    );
    assert!(
        output.status.success() && redirected.is_dir() && !target.path().join(".git").exists(),
        "the premise of the removal: `GIT_DIR` sends `git init -C {}` to {} instead. If this \
         ever stops being true the removal is still harmless, but the reason written beside it \
         is not:\n{}",
        target.path().display(),
        redirected.display(),
        String::from_utf8_lossy(&output.stderr).trim()
    );

    let removed: Vec<&str> = GIT_REDIRECTING_VARS
        .iter()
        .copied()
        .filter(|name| {
            git_command(&git)
                .get_envs()
                .any(|(key, value)| key == std::ffi::OsStr::new(name) && value.is_none())
        })
        .collect();
    assert_eq!(
        removed,
        GIT_REDIRECTING_VARS.to_vec(),
        "and every one of them is removed from every `git` this suite starts, so a question \
         about this repository is answered about this repository whatever the caller's shell \
         was last in"
    );
}

// ---------------------------------------------------------- the gate --

#[test]
fn the_gate_stands_down_where_the_tree_is_a_copy_rather_than_a_checkout() {
    let copy = a_copy_of_the_tree();

    assert!(
        require_git_work_tree(copy.path()).is_none(),
        "a question about what the repository tracks has no answer in {}, so the gate closes \
         and the test says so instead of asserting against `git`'s error status",
        copy.path().display()
    );
}

#[test]
fn a_copy_of_the_tree_unpacked_inside_a_foreign_checkout_still_cannot_answer() {
    let Some(tools) = require_tools(&["git"]) else {
        return;
    };
    let Some(enclosing) = a_real_checkout(tools.path("git")) else {
        return;
    };
    let copy = enclosing.path().join("cargo-mutants-ginary-XcVxbW.tmp");
    std::fs::create_dir(&copy).expect("a directory inside the checkout");

    assert!(
        require_git_work_tree(&copy).is_none(),
        "\"inside a work tree\" is not \"is this tree\". {} is inside a checkout of something \
         else, so `git ls-files` lists nothing there and `git check-ignore` answers about the \
         enclosing repository — the E19 failure again, by a route a gate that asked only \
         `--is-inside-work-tree` would open. Where `cargo mutants` puts its copy is `TMPDIR`, \
         and nothing says `TMPDIR` is outside every checkout",
        copy.display()
    );
}

#[test]
fn the_gate_opens_in_a_checkout() {
    let Some(tools) = require_tools(&["git"]) else {
        return;
    };
    let Some(checkout) = a_real_checkout(tools.path("git")) else {
        return;
    };

    assert!(
        require_git_work_tree(checkout.path()).is_some(),
        "the gate has to keep the check running where it can answer, and closing everywhere \
         would be the check deleted rather than gated: {} is a work tree with a `git` on PATH",
        checkout.path().display()
    );
}

#[test]
fn the_gate_opens_on_this_repository_wherever_git_says_this_repository_is_one() {
    let Some(tools) = require_tools(&["git"]) else {
        return;
    };
    // The claim is an implication, and it is asked of `git` rather than of the
    // filesystem, so the one thing that can make this test stand down is the
    // one thing that can make the gate close. A `.git` that is *there* is not
    // enough: `git` refuses to answer about a repository owned by another uid
    // (`safe.directory`, git 2.35.2 and later), which is routine for a
    // checkout bind-mounted into a container, and there every other
    // repository-property test in this file stands down while an assertion of
    // the bare "the gate opens here" would turn the whole target red for a
    // reason outside its subject.
    let answer = probe_own_work_tree(tools.path("git"), &root());
    if answer.probe != WorkTreeProbe::OwnWorkTree {
        eprintln!(
            "skipping: `git` does not call {} a work tree of its own ({answer:?}), so this run \
             is the copy of the tree the gate exists for, or a `git` that cannot answer about \
             it — and either way there is no repository here to open the gate on",
            root().display()
        );
        return;
    }

    assert!(
        require_git_work_tree(&root()).is_some(),
        "this is the half of the rule that keeps the repository-property tests running: \
         wherever `git` says {} is a checkout of its own — every developer machine and every CI \
         job but the mutants one — the gate opens on it",
        root().display()
    );
}

#[test]
fn a_directory_that_cannot_answer_never_escalates_under_the_toolchain_variable() {
    let rows = [
        (WorkTreeProbe::NoGit, false),
        (WorkTreeProbe::NoGit, true),
        (WorkTreeProbe::GitUnrunnable, false),
        (WorkTreeProbe::GitUnrunnable, true),
        (WorkTreeProbe::GitRefused, false),
        (WorkTreeProbe::GitRefused, true),
        (WorkTreeProbe::NotItsOwnWorkTree, false),
        (WorkTreeProbe::NotItsOwnWorkTree, true),
        (WorkTreeProbe::OwnWorkTree, false),
        (WorkTreeProbe::OwnWorkTree, true),
    ];
    let decided: Vec<((WorkTreeProbe, bool), WorkTreeGate)> = rows
        .into_iter()
        .map(|row| (row, work_tree_gate(row.0, row.1)))
        .collect();

    assert_eq!(
        decided,
        vec![
            ((WorkTreeProbe::NoGit, false), WorkTreeGate::SkipNoGit),
            ((WorkTreeProbe::NoGit, true), WorkTreeGate::FailNoGit),
            (
                (WorkTreeProbe::GitUnrunnable, false),
                WorkTreeGate::SkipGitUnrunnable
            ),
            (
                (WorkTreeProbe::GitUnrunnable, true),
                WorkTreeGate::FailGitUnrunnable
            ),
            (
                (WorkTreeProbe::GitRefused, false),
                WorkTreeGate::SkipGitRefused
            ),
            (
                (WorkTreeProbe::GitRefused, true),
                WorkTreeGate::FailGitRefused
            ),
            (
                (WorkTreeProbe::NotItsOwnWorkTree, false),
                WorkTreeGate::SkipNotAWorkTree
            ),
            (
                (WorkTreeProbe::NotItsOwnWorkTree, true),
                WorkTreeGate::SkipNotAWorkTree
            ),
            ((WorkTreeProbe::OwnWorkTree, false), WorkTreeGate::Open),
            ((WorkTreeProbe::OwnWorkTree, true), WorkTreeGate::Open),
        ],
        "the halves of the gate are deliberately asymmetric. A `git` that is missing, a `git` \
         that is there and will not start, and a `git` that runs and refuses to look, all three \
         escalate under GINARY_REQUIRE_TOOLCHAIN, because that variable is a claim somebody can \
         make true by installing or fixing something — an ownership refusal is one `git config \
         --global --add safe.directory` in the job. A directory that is not a checkout of its \
         own does not, ever, because nobody can install being one — and the job that hits that \
         row is the mutants job, which sets the variable and has a perfectly good `git`"
    );
}

#[test]
fn a_git_that_will_not_start_is_reported_as_the_program_rather_than_the_directory() {
    let error = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
    let reason = git_unrunnable_reason(Path::new("/usr/bin/git"), &error);

    assert!(
        reason.contains("/usr/bin/git") && reason.contains(&error.to_string()),
        "a `git` on PATH that cannot be executed — the wrong mode, a broken interpreter — is a \
         fact about the program, and the skip has to name it and quote the operating system. \
         Reported as `this directory is not inside a git work tree` it is a confident diagnosis \
         of the wrong thing, pointing the reader at `cargo mutants`: {reason}"
    );
    assert!(
        !reason.contains("cargo mutants"),
        "and it does not borrow the other skip's explanation: {reason}"
    );
}

#[test]
fn the_skip_names_the_directory_and_says_what_could_not_be_asked() {
    // A fixed path rather than a real temporary one: the snapshot pins the
    // sentence, and a name that changes every run would pin nothing. It is the
    // directory out of the failing job's own log.
    let rendered = work_tree_skip_message(Path::new("/tmp/cargo-mutants-ginary-XcVxbW.tmp"));

    insta::with_settings!({snapshot_path => "../snapshots", prepend_module_to_snapshot => false}, {
        insta::assert_snapshot!("regressions__e19_work_tree_skip_message", rendered);
    });
}

// ---------------------------------------------------------- the scan --

/// Every 1-based line of `source` that names the `git` program in code.
///
/// The literal `"git"` covers all three spellings this repository uses to
/// reach it — `Command::new("git")`, `require_tools(&["git"])` and
/// `tools.path("git")` — and [`literal_sites`] drops comment lines, so a
/// sentence about `git` in a doc comment is prose rather than a call.
fn git_program_sites(source: &str) -> Vec<usize> {
    literal_sites(source, "\"git\"")
}

/// The tracked sources that ask `git` about this repository today.
///
/// Named rather than counted, so that the scan below is held against a subject
/// it is known to have. See
/// `tests/regressions/e16_a_glibc_only_assertion_ran_under_a_linux_gate.rs`,
/// whose scan over the same helper carries the other half of this guard.
const KNOWN_CALLERS: [&str; 3] = [
    "tests/common/homepath.rs",
    "tests/common/portability.rs",
    "tests/smoke_matrix.rs",
];

/// The calibration fixture: one line of each shape the scan has to tell apart.
const CALIBRATION: &str = r#"
use std::process::Command;
// A comment naming "git" is prose, not a call.
fn a() { Command::new("git").arg("status"); }
fn b() { require_tools(&["git"]); }
fn c() { tools.path("git"); }
fn d() { Command::new("gitk"); }
"#;

#[test]
fn only_a_code_line_that_names_the_git_program_is_a_site() {
    assert_eq!(
        git_program_sites(CALIBRATION),
        vec![4, 5, 6],
        "the three spellings that start a `git` are sites; the comment is prose and `gitk` is a \
         different program:\n{CALIBRATION}"
    );
}

#[test]
fn every_tracked_test_that_asks_git_about_this_repository_goes_through_the_gate() {
    let Some(sources) = tracked_test_sources() else {
        eprintln!("skipping: `git ls-files` did not answer, so `tracked` would be a guess");
        return;
    };
    assert!(
        sources.unreadable.is_empty(),
        "a tracked source the scan cannot read is a file it has no answer for, and reporting it \
         as clean is the silent skip CLAUDE.md forbids:\n{}",
        sources.unreadable.join("\n")
    );
    // Two guards on the scan's own subject, before anything is concluded from
    // it. A scan that finds nothing reports the whole repository clean, and
    // `git ls-files` answers with an empty list and an exit status of zero
    // whenever it is asked from a directory the repository does not cover —
    // which is the very condition this milestone is about.
    assert!(
        sources.files.len() > 40,
        "only {} tracked test sources were read; the scan has lost its subject",
        sources.files.len()
    );
    let unseen: Vec<&str> = KNOWN_CALLERS
        .iter()
        .filter(|caller| {
            !sources
                .files
                .iter()
                .any(|(name, text)| name == *caller && !git_program_sites(text).is_empty())
        })
        .copied()
        .collect();
    assert_eq!(
        unseen,
        Vec::<&str>::new(),
        "the positive control: these three sources start a `git` today, so a scan that does not \
         see them there is a scan whose answer about every other file means nothing. It fails \
         from either end — a tracked list that no longer covers `tests/`, or a site scanner that \
         stopped recognising the call — and both report a clean repository"
    );

    let ungated: Vec<String> = sources
        .files
        .iter()
        .filter(|(_, text)| !git_program_sites(text).is_empty())
        .filter(|(_, text)| literal_sites(text, GATE).is_empty())
        .map(|(name, _)| name.clone())
        .collect();

    assert_eq!(
        ungated,
        Vec::<String>::new(),
        "these sources start a `git` and never name `{GATE}`, so each carries its own answer to \
         the question of what a copy of the tree can be asked — which is how one of them came to \
         read `fatal: not a git repository` as `not ignored`. One gate, so the three of them \
         cannot drift apart"
    );
}
