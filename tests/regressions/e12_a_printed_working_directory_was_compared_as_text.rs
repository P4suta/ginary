// SPDX-License-Identifier: MIT OR Apache-2.0
//! The two "a real thing really works" tests compared a directory a running
//! program printed against a directory the test had built, as strings.
//!
//! **What went wrong.** The `hello_ffi` fixture prints `cwd=` and whatever
//! `file:get_cwd/0` gives it, and both tests that run it assert the artifact
//! started where the user is rather than where the runtime unpacked:
//!
//! ```rust,ignore
//! stdout.contains(&format!("cwd={}", cwd.display()))
//! ```
//!
//! On unix the two spellings are the same string and the assertion is exact.
//! On Windows they are three transformations apart, and every one of them is a
//! different way of writing one directory: the runtime prints a lower-case
//! drive letter and forward separators, `std::fs::canonicalize` returns the
//! verbatim `\\?\` form with backslashes, and `%TEMP%` on a GitHub runner is
//! the 8.3 name `C:\Users\RUNNER~1\…` whose long form is `…\runneradmin\…`.
//!
//! ```text
//! ---- the_built_artifact_runs_the_application_with_no_erlang_on_the_machine ----
//! the application must start where the user is, not where the runtime unpacked:
//! …args=3 a b
//! hello from priv
//! cwd=c:/Users/RUNNER~1/AppData/Local/Temp/.tmpGAJaYB/args-cwd
//!
//! ---- a_staged_hello_ffi_prints_its_arguments_and_its_priv_file ----
//! the application did not start in the directory it was given:
//! …cwd=c:/Users/RUNNER~1/AppData/Local/Temp/.tmpyueybk/run/cwd
//! ```
//!
//! (`Windows build and exit-code propagation`
//! <https://github.com/P4suta/ginary/actions/runs/33823103540/job/100869848230>,
//! `tests/e2e_hello.rs:117` and `tests/stage_run.rs:104`.) Both runs are
//! correct: `args=3 a b` and `hello from priv` are on the same line, the exit
//! code is 3, and the directory printed *is* the one the test asked for. What
//! failed is the comparison.
//!
//! **The input.** Any host that spells one directory more than one way. Two of
//! the three differences are Windows path syntax and can be held here; the
//! third is not text at all — only the filesystem knows that `RUNNER~1` and
//! `runneradmin` are one directory — and a symbolic link makes the same
//! two-spellings-one-directory case on this machine.
//!
//! **The correct behaviour.** A test comparing a path a program printed with a
//! path it built asks whether they name one directory. Where the filesystem
//! can answer, it answers, which settles the short name, the verbatim prefix,
//! the letter case and any link in one step; where it cannot — a spelling
//! recorded from another platform — the pure rule decides it, so the answer is
//! testable from here.

use std::path::Path;

use ginary::target::Os;

use crate::common::hostpath::{names_the_same_directory, same_directory_text};

/// The account a GitHub Windows runner runs as, in the 8.3 spelling `%TEMP%`
/// carries.
///
/// The recorded spellings below are assembled from segments rather than
/// written out in one piece, and the reason is the rule
/// `e7_the_home_directory_scan_only_worked_on_one_machine` states: no tracked
/// file under `src/`, `tests/`, `scripts/` or `.github/` may name a person's
/// absolute home path in code. These four strings *are* one runner's home
/// path — that is what makes them the evidence — so the technique is the one
/// that file uses for its own input: the bytes are joined at run time, and
/// no line of source spells `/Users/<a person>`.
const RUNNER_ACCOUNT: &str = "RUNNER~1";

/// `drive`, the runner's temporary directory and `tail`, joined with
/// `separator`.
fn runner_temp(drive: &str, separator: char, tail: &[&str]) -> String {
    let mut path = String::from(drive);
    for segment in ["Users", RUNNER_ACCOUNT, "AppData", "Local", "Temp"]
        .iter()
        .chain(tail)
    {
        path.push(separator);
        path.push_str(segment);
    }
    path
}

/// The spelling the runtime printed for `tests/e2e_hello.rs`.
///
/// Verbatim prefix, drive-letter case and separator, and nothing else: the
/// short name is the same on both sides here, because the third difference is
/// not a text rule.
fn e2e_printed() -> String {
    runner_temp("c:", '/', &[".tmpGAJaYB", "args-cwd"])
}

/// What `std::fs::canonicalize` gave the same test for the same directory.
fn e2e_canonical() -> String {
    format!(
        r"\\?\{}",
        runner_temp("C:", '\\', &[".tmpGAJaYB", "args-cwd"])
    )
}

/// The spelling `tests/stage_run.rs` read, which differs by one
/// transformation fewer: the staged run built its directory itself and never
/// canonicalised it, so there is no verbatim prefix on the expected side.
fn stage_printed() -> String {
    runner_temp("c:", '/', &[".tmpyueybk", "run", "cwd"])
}

/// The directory that test created.
fn stage_built() -> String {
    runner_temp("C:", '\\', &[".tmpyueybk", "run", "cwd"])
}

#[test]
fn the_spellings_two_windows_runs_produced_name_one_directory() {
    for (printed, expected) in [
        (e2e_printed(), e2e_canonical()),
        (stage_printed(), stage_built()),
    ] {
        assert!(
            same_directory_text(Os::Windows, &printed, &expected),
            "one directory, spelled by the runtime and by the test:\n  {printed}\n  {expected}"
        );
    }
}

#[test]
fn two_directories_are_still_two_directories() {
    // The guard that keeps the rule from being "true": folding the separator
    // and the drive letter is all it may do.
    for (left, right) in [
        (e2e_printed(), stage_printed()),
        (r"C:\a\b".to_owned(), r"D:\a\b".to_owned()),
        (r"C:\a\b".to_owned(), r"C:\a\B".to_owned()),
        (r"C:\a\b".to_owned(), r"C:\a".to_owned()),
    ] {
        assert!(
            !same_directory_text(Os::Windows, &left, &right),
            "these are two directories:\n  {left}\n  {right}"
        );
    }
}

#[test]
fn a_backslash_in_a_unix_name_is_a_character_and_not_a_separator() {
    // The reason the rule is a function of `os`: `a\b` is one file on Linux
    // and two components on Windows, so a blanket fold would call two unix
    // files one file.
    assert!(!same_directory_text(Os::Linux, r"/tmp/a\b", "/tmp/a/b"));
    assert!(same_directory_text(Os::Linux, "/tmp/a/b", "/tmp/a/b"));
}

#[test]
#[cfg(unix)]
fn two_spellings_of_one_real_directory_are_one_directory() {
    // The half no text rule can settle, made on this machine: a link and its
    // target are one directory, exactly as `RUNNER~1` and `runneradmin` are.
    let dir = tempfile::tempdir().expect("a temporary directory");
    let real = dir.path().join("cwd");
    std::fs::create_dir_all(&real).expect("the directory the program starts in");
    let link = dir.path().join("link");
    std::os::unix::fs::symlink(&real, &link).expect("a second spelling of it");

    assert!(
        names_the_same_directory(&link.display().to_string(), &real),
        "the filesystem knows these are one directory:\n  {}\n  {}",
        link.display(),
        real.display()
    );
    assert!(
        !names_the_same_directory(&dir.path().display().to_string(), &real),
        "and it knows these are not"
    );
}

#[test]
fn a_directory_is_the_directory_it_is() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let real = dir.path().join("cwd");
    std::fs::create_dir_all(&real).expect("the directory the program starts in");

    assert!(
        names_the_same_directory(&real.display().to_string(), &real),
        "the rule still holds for the case that already worked"
    );
    assert!(
        !names_the_same_directory("cwd", Path::new("other")),
        "and two names the filesystem cannot resolve are still compared, rather than being \
         called equal because neither could be looked up"
    );
}
