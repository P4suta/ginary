// SPDX-License-Identifier: MIT OR Apache-2.0
//! `cache dir --json` names the variable its fallback root came from, and the
//! test knew only the unix one.
//!
//! **What went wrong.** With nothing in the environment,
//! `ginary::cache::resolve` falls back to `${TMPDIR:-/tmp}/ginary-<uid>` and
//! `ginary::cache::resolve_windows` to `%TEMP%\ginary-<user>`. The `origin`
//! field of the JSON report says which, and it is correct on both platforms;
//! the assertion was written against one of them:
//!
//! ```text
//! ---- cache_dir_reports_the_temporary_fallback_when_nothing_is_set ----
//! assertion `left == right` failed
//!   left: String("TEMP fallback")
//!  right: String("TMPDIR fallback")
//! ```
//!
//! (`Windows build and exit-code propagation`
//! <https://github.com/P4suta/ginary/actions/runs/33751715516/job/100636537290>,
//! `tests/cli.rs:1423`.) The same test then sets `TMPDIR` and asserts the
//! resulting path is under it, which on Windows sets a variable nothing reads.
//!
//! **The input.** Any host whose temporary directory is not `$TMPDIR`.
//!
//! **The correct behaviour.** Which variable names the temporary directory is
//! a fact about an operating system, so it is
//! `ginary::platform::temp_dir_var`; the test sets that variable and composes
//! that word into the origin it expects.

use ginary::cache;
use ginary::platform::temp_dir_var;
use ginary::target::Os;

#[test]
fn the_temporary_directory_variable_is_the_one_the_platform_reads() {
    assert_eq!(
        [
            temp_dir_var(Os::Linux),
            temp_dir_var(Os::Macos),
            temp_dir_var(Os::Windows),
        ],
        ["TMPDIR", "TMPDIR", "TEMP"],
    );
}

#[test]
fn the_variable_the_rule_names_is_the_one_each_resolver_actually_reads() {
    // The rule and the two resolvers may not drift: a word composed into an
    // expectation has to be the word that produced the root.
    assert_eq!(
        temp_dir_var(Os::Linux),
        cache::TMPDIR_VAR,
        "the unix fallback reads TMPDIR"
    );
    assert_eq!(
        temp_dir_var(Os::Windows),
        cache::TEMP_VAR,
        "and the Windows one reads TEMP first"
    );
}

#[test]
fn setting_that_variable_moves_the_fallback_root_on_either_platform() {
    let pairs = |key: &str, value: &str| {
        cache::Env::from_pairs([(
            std::ffi::OsString::from(key),
            std::ffi::OsString::from(value),
        )])
    };

    let unix = cache::resolve(&pairs(temp_dir_var(Os::Linux), "/scratch"), 1000);
    assert!(unix.is_fallback, "{:?}", unix.origin);
    assert!(
        unix.root.starts_with("/scratch"),
        "the fallback lives under the variable the platform reads: {}",
        unix.root.display()
    );

    let windows = cache::resolve_windows(&pairs(temp_dir_var(Os::Windows), r"D:\scratch"), "ada");
    assert!(windows.is_fallback, "{:?}", windows.origin);
    assert!(
        windows.root.starts_with(r"D:\scratch"),
        "and so does the Windows one: {}",
        windows.root.display()
    );
}
