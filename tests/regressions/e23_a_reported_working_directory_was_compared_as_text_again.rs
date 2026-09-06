// SPDX-License-Identifier: MIT OR Apache-2.0
//! Un-gating a launcher claim moved a text comparison onto the platform where
//! it is wrong.
//!
//! **What went wrong.** E12 established the rule: a directory a running program
//! *reports* and a directory a test *built* are two strings that may name one
//! directory, and on Windows they routinely do. `%TEMP%` on a GitHub runner is
//! the 8.3 spelling `C:\Users\RUNNER~1\...` while the long name is
//! `C:\Users\runneradmin\...`; the drive letter's case, the separators and the
//! verbatim `\\?\` prefix differ too. `common::hostpath::names_the_same_directory`
//! reconciles all of it by asking the filesystem, and
//! `tests/regressions/e12_a_printed_working_directory_was_compared_as_text.rs`
//! is where that rule is held to its own claims.
//!
//! E23 removed the `#![cfg(unix)]` from `tests/launcher.rs`, and
//! `the_runtime_starts_in_the_callers_working_directory` — written years before
//! that file ever compiled for Windows, and correct while it did not — compared
//! the reported directory against a canonicalised one **as text**:
//!
//! ```rust,ignore
//! let expected = plain_path(&canonicalize(artifact.dir())?).display().to_string();
//! assert_eq!(run.cwd().as_deref(), Some(expected.as_str()), "...");
//! ```
//!
//! It passed on the development host, whose user name is short enough to have
//! no 8.3 alias, and failed on the first GitHub runner it ever reached:
//!
//! ```text
//!   left: Some("C:\\Users\\RUNNER~1\\AppData\\Local\\Temp\\.tmpVCyusT")
//!  right: Some("C:\\Users\\runneradmin\\AppData\\Local\\Temp\\.tmpVCyusT")
//! ```
//!
//! One directory, two spellings, and an `assert_eq!` that cannot tell.
//!
//! **The input.** Every tracked source under `tests/`. No runner is needed:
//! whether a test settles a reported directory by the rule or by string
//! equality is visible in the file.
//!
//! **The correct behaviour.** E12's regression proved the *rule*; it did not
//! stop a fourth site from bypassing it, which is why the defect could come
//! back through a file E12 never touched. So this is a scan rather than a
//! case: a test that reads a directory a program reported settles it through
//! `names_the_same_directory`, wherever that test lives and whichever platform
//! it was written for.

use crate::common::portability::tracked_test_sources;

/// The two ways a test gets hold of a directory a running program reported.
///
/// `printed_cwd(` parses it out of the application's standard output;
/// `.cwd()` is what `common::artifact::Run` answers with after reading the
/// `cwd:` line the launch fixture prints.
const READS_A_REPORTED_DIRECTORY: [&str; 2] = ["printed_cwd(", ".cwd()"];

/// The one rule that settles two spellings of one directory.
const THE_RULE: &str = "names_the_same_directory";

/// The helpers, which implement the rule rather than apply it.
///
/// `common::hostpath` *is* the rule, and `common::artifact` is where `.cwd()`
/// is defined. A scan that read them would be reading its own subject.
const HELPERS: &str = "tests/common/";

#[test]
fn the_rule_this_file_applies_tells_the_two_shapes_apart() {
    // The instrument, calibrated on the shape that failed and the shape that
    // replaced it, so a change to the needles cannot quietly stop matching.
    let as_text = "\
    let expected = plain_path(&canonicalize(artifact.dir())?).display().to_string();\n\
    assert_eq!(run.cwd().as_deref(), Some(expected.as_str()), \"...\");\n";
    assert!(
        reads_a_reported_directory(as_text) && !as_text.contains(THE_RULE),
        "the comparison that reached a runner has to be the one this scan catches"
    );

    let by_rule = "\
    let reported = run.cwd().expect(\"a working directory\");\n\
    assert!(names_the_same_directory(&reported, artifact.dir()), \"...\");\n";
    assert!(
        reads_a_reported_directory(by_rule) && by_rule.contains(THE_RULE),
        "and the shape that replaced it has to satisfy the scan"
    );

    assert!(
        !reads_a_reported_directory("let dir = tempfile::tempdir()?;\n"),
        "a test that never asks a program where it is is not this scan's subject"
    );
}

/// Whether `text` gets hold of a directory a running program reported.
fn reads_a_reported_directory(text: &str) -> bool {
    READS_A_REPORTED_DIRECTORY
        .iter()
        .any(|needle| text.contains(needle))
}

#[test]
fn every_test_that_reads_a_reported_directory_settles_it_by_the_rule() {
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
    assert!(
        sources.files.len() > 40,
        "only {} tracked test sources were read; the scan has lost its subject",
        sources.files.len()
    );

    // This file spells both the reads and the rule in order to look for them.
    // `file!()` is the scanner's own path, so the exclusion cannot go stale
    // under a rename.
    let myself = file!().replace('\\', "/");

    let mut offenders: Vec<&str> = Vec::new();
    let mut settled = 0usize;
    for (name, text) in &sources.files {
        if name == &myself || name.starts_with(HELPERS) {
            continue;
        }
        if !reads_a_reported_directory(text) {
            continue;
        }
        if text.contains(THE_RULE) {
            settled += 1;
        } else {
            offenders.push(name);
        }
    }

    assert!(
        offenders.is_empty(),
        "a directory a running program reported and a directory a test built are two strings that \
         name one directory as often as not — the 8.3 `%TEMP%` of a Windows runner, a drive \
         letter's case, a `/tmp` that is a symlink. Settle it with `{THE_RULE}` rather than by \
         string equality:\n{}",
        offenders.join("\n")
    );
    assert!(
        settled >= 3,
        "only {settled} tracked tests read a reported directory at all; this scan has lost its \
         subject and would pass over the defect it exists to catch"
    );
}
