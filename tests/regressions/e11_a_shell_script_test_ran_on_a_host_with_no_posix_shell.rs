// SPDX-License-Identifier: MIT OR Apache-2.0
//! Two tests run a shell script to make their claim, and on a host with no
//! POSIX shell one failed at spawn and the other ran the Windows Subsystem for
//! Linux launcher instead.
//!
//! **What went wrong.** Running the script is the right way to make both
//! claims. `c4_a_hook_token_was_pasted_unquoted` runs a build hook to prove
//! that an output directory holding a space arrives as *one* argument, and
//! `the_notice_script_exits_zero_under_the_shell_that_runs_it` runs the Release
//! workflow's notice block to prove the workflow of a repository with no
//! credentials is green. Reading either script and reasoning about it would
//! prove nothing. But neither can run where there is no such shell:
//!
//! ```text
//! ---- a_hook_is_handed_its_output_directory_as_one_word ----
//! the hook runs: HookProcess { package: "esqlite", source: Spawn {
//!   program: "/bin/sh", source: Os { code: 3, kind: NotFound, ... } } }
//!
//! ---- the_notice_script_exits_zero_under_the_shell_that_runs_it ----
//! the notice script exits Some(1) under `bash -e -o pipefail`, so the Release
//! workflow of a repository with no credentials is red. stderr:
//! ```
//!
//! (`Windows build and exit-code propagation`
//! <https://github.com/P4suta/ginary/actions/runs/33751715516/job/100636537290>,
//! `tests/regressions/c4_a_hook_token_was_pasted_unquoted.rs:69` and
//! `tests/release_workflow.rs:509`.)
//!
//! The second is the more interesting failure: exit `1` and *nothing on either
//! stream*. `Command::new("bash")` searches `PATH`, and on a Windows runner the
//! first `bash` on it is `C:\Windows\System32\bash.exe` — the WSL launcher,
//! which exits `1` in silence when no distribution is installed. The test was
//! running a program that is not a shell at all.
//!
//! **The input.** Any host with no `/bin/sh`.
//!
//! **The correct behaviour.** A machine with no POSIX shell genuinely cannot
//! answer either question, so this is a reported skip and not a gate lowered:
//! `common::tools::require_posix_shell` prints why and, like `require_tools`,
//! panics rather than skips under `GINARY_REQUIRE_TOOLCHAIN=1`. The shell it
//! probes is `ginary::native::HOOK_SHELL` by absolute path — the program the
//! hook rule names, and never a name resolved through `PATH`.
#![cfg(feature = "cli")]

use crate::common::tools::require_posix_shell;

#[test]
fn the_shell_the_suite_probes_is_the_one_the_hook_rule_names() {
    let Some(shell) = require_posix_shell() else {
        // The skip is the behaviour under test on such a host; there is
        // nothing further to assert here.
        return;
    };
    assert_eq!(
        shell,
        std::path::Path::new(ginary::native::HOOK_SHELL),
        "a hook is quoted for this shell, so this is the shell a hook test needs"
    );
    assert!(
        shell.is_absolute(),
        "an absolute path and never a `PATH` lookup: `bash` on a Windows runner resolves to \
         the WSL launcher, which is not a shell"
    );
}

#[test]
fn the_probe_answers_for_this_machine_rather_than_assuming_one() {
    let found = require_posix_shell().is_some();
    assert_eq!(
        found,
        std::path::Path::new(ginary::native::HOOK_SHELL).is_file(),
        "the probe is the question `is there a {} here`, asked and answered",
        ginary::native::HOOK_SHELL
    );
}
