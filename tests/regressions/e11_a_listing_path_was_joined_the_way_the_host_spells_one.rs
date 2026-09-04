// SPDX-License-Identifier: MIT OR Apache-2.0
//! Five expectations built a path by joining a `/`-separated listing path onto
//! a host path, and got a spelling no platform produces.
//!
//! **What went wrong.** A test that wants the absolute path ginary walked to
//! writes the relative half the way `ginary.stage.json` carries it —
//! `lib/kernel-11.0.3/ebin/kernel.beam` — and reaches for `Path::join`. That
//! appends the *host's* separator between the two halves and leaves every
//! separator inside the relative half exactly as it was typed. On Windows the
//! result is the mixed spelling nothing writes:
//!
//! ```text
//! ---- no_directory_is_passed_to_the_runtime_where_a_module_belongs ----
//! assertion `left == right` failed
//!   left: ["C:\\Users\\RUNNER~1\\...\\out\\lib\\gleam_stdlib\\ebin\\gleam@list.beam", ...]
//!  right: ["C:\\Users\\RUNNER~1\\...\\out\\lib/gleam_stdlib/ebin/gleam@list.beam", ...]
//!
//! ---- a_manifest_env_may_not_take_over_a_name_the_launcher_derives ----
//! assertion `left == right` failed: the dump goes where the launcher put it
//!   left: ["/cache/hello\\erl_crash.dump"]
//!  right: ["/cache/hello/erl_crash.dump"]
//! ```
//!
//! (`Windows build and exit-code propagation`
//! <https://github.com/P4suta/ginary/actions/runs/33751715516/job/100636537290>:
//! `tests/strip.rs:227`, `:254` and `:280`, `tests/cli.rs:1123`, and
//! `tests/regressions/b1_manifest_env_overrode_the_launcher_s_own.rs:92`.)
//!
//! **The input.** Any host whose separator is not `/`. Note the second excerpt:
//! the *root* there is a literal `/cache/hello` written in the test, so the
//! defect is not that a Windows path arrived — it is that `Path::join` spelled
//! one join with the host separator and left the rest alone. A rule that
//! respells the whole string would be wrong for the opposite reason: `\` is an
//! ordinary character in a unix file name.
//!
//! **The correct behaviour.** `common::hostpath::joined_for` states the join
//! as a function of a named platform, so both spellings are asserted here and
//! the five expectations compose one of them rather than hoping `Path::join`
//! agrees with the code under test.

use crate::common::hostpath::{joined, joined_for, separator_for};
use ginary::target::Os;

#[test]
fn a_platform_joins_with_the_separator_it_spells_paths_with() {
    assert_eq!(
        [
            separator_for(Os::Linux),
            separator_for(Os::Macos),
            separator_for(Os::Windows),
        ],
        ['/', '/', '\\'],
    );
}

#[test]
fn every_separator_of_the_relative_half_is_respelled_and_not_just_the_join() {
    assert_eq!(
        joined_for(
            Os::Windows,
            r"C:\Users\RUNNER~1\out",
            "lib/kernel-11.0.3/ebin/kernel.beam"
        ),
        r"C:\Users\RUNNER~1\out\lib\kernel-11.0.3\ebin\kernel.beam",
        "the mixed spelling `out\\lib/kernel-11.0.3/...` is what `Path::join` produced and \
         what nothing on that platform writes"
    );
    assert_eq!(
        joined_for(Os::Linux, "/cache/hello", "erl_crash.dump"),
        "/cache/hello/erl_crash.dump"
    );
    assert_eq!(
        joined_for(Os::Windows, "/cache/hello", "erl_crash.dump"),
        r"/cache/hello\erl_crash.dump",
        "the root came from the caller and is left exactly as it was given; only the \
         relative half is a listing path"
    );
}

#[test]
fn a_unix_file_name_holding_a_backslash_is_not_respelled() {
    // The reason the rule takes an `os` rather than replacing blindly. `a\b`
    // is one file called `a\b` on unix and two components on Windows, and a
    // test that renamed it would be asserting about a different file.
    assert_eq!(
        joined_for(Os::Linux, "/srv", r"odd\name"),
        r"/srv/odd\name",
        "a backslash is an ordinary character in a unix file name"
    );
}

#[test]
fn the_empty_halves_do_not_grow_a_separator() {
    for os in [Os::Linux, Os::Macos, Os::Windows] {
        assert_eq!(
            joined_for(os, "", "lib/x"),
            if os == Os::Windows { r"lib\x" } else { "lib/x" },
            "an empty root leaves a relative path relative on {os:?}"
        );
        assert_eq!(joined_for(os, "/root", ""), "/root");
    }
}

#[test]
fn the_host_shorthand_is_the_rule_asked_about_this_machine() {
    let root = std::path::Path::new(if cfg!(windows) { r"C:\out" } else { "/out" });
    let answer = joined(root, "lib/notify/ebin/notify.beam");
    assert_eq!(
        answer,
        if cfg!(windows) {
            r"C:\out\lib\notify\ebin\notify.beam"
        } else {
            "/out/lib/notify/ebin/notify.beam"
        },
        "the shorthand is spelled out here rather than restated, or it could only fail by \
         ceasing to compile"
    );
    assert_eq!(
        answer,
        joined_for(
            ginary::platform::HOST,
            &root.display().to_string(),
            "lib/notify/ebin/notify.beam"
        ),
        "and it is the same rule asked about this machine"
    );
}
