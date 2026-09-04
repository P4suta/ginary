// SPDX-License-Identifier: MIT OR Apache-2.0
//! Three `tests/cli.rs` assertions read a planted program's argument log by a
//! name only its unix spelling produces, and reported that stripping had never
//! started.
//!
//! **What went wrong.** E10 taught `common::script` to plant a program the
//! host can start: `erl` on unix and a compiled `erl.exe` on Windows. Its
//! sidecar files are named after the program file, so the log is `erl.argv`
//! there and `erl.exe.argv` here — `common::script::shim_sidecar` says so and
//! `FakeOtpRoot::erl_argv` composes it correctly. `tests/cli.rs` has a second
//! reader of the same file that spells the name itself, and it read a file
//! that is not there:
//!
//! ```text
//! ---- stage_strips_by_default_and_prints_the_strip_table ----
//! stripping is on by default, so the runtime was started
//!
//! ---- stage_with_strip_beams_only_leaves_the_native_binaries_alone ----
//! assertion failed: !erl_argv(&otp).is_empty()
//!
//! ---- stage_runs_the_otp_roots_own_erl_with_the_beam_lib_one_liner ----
//!   left: []
//!  right: ["-noshell", "-env", "ERL_CRASH_DUMP", ...]
//! ```
//!
//! (`Windows build and exit-code propagation`
//! <https://github.com/P4suta/ginary/actions/runs/33751715516/job/100636537290>,
//! `tests/cli.rs:1097`, `:1123` and `:1189`.)
//!
//! The failure mode is the dangerous one: a missing log reads as an empty
//! argument vector, which is exactly what "the runtime was never started"
//! looks like. Two of the three assertions are the *negative* claim
//! `--no-strip` makes, and they would have passed on that host whatever ginary
//! did.
//!
//! **The input.** Any host where a program's file name is not the name it was
//! asked for.
//!
//! **The correct behaviour.** One composition, in `common::script`, that every
//! reader of the log uses: `argv_log_path`.

use crate::common::script::{argv_log_path, shim_file_name, shim_sidecar};
use ginary::target::Os;

#[test]
fn the_argv_log_is_named_after_the_file_the_program_was_planted_as() {
    let dir = std::path::Path::new("/otp/bin");
    assert_eq!(
        argv_log_path(dir, "erl", Os::Linux),
        dir.join("erl.argv"),
        "a unix program is `erl`, so its log is `erl.argv`"
    );
    assert_eq!(argv_log_path(dir, "erl", Os::Macos), dir.join("erl.argv"));
    assert_eq!(
        argv_log_path(dir, "erl", Os::Windows),
        dir.join("erl.exe.argv"),
        "and a Windows program is `erl.exe`, so its log is `erl.exe.argv`"
    );
}

#[test]
fn it_is_the_composition_the_planter_already_states_and_not_a_second_rule() {
    for os in [Os::Linux, Os::Macos, Os::Windows] {
        let dir = std::path::Path::new("/otp/bin");
        assert_eq!(
            argv_log_path(dir, "erl", os),
            shim_sidecar(&dir.join(shim_file_name("erl", os)), "argv"),
            "the log is `shim_sidecar` of `shim_file_name`, on every platform"
        );
    }
}
