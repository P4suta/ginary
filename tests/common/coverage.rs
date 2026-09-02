// SPDX-License-Identifier: MIT OR Apache-2.0
//! Threading the coverage profile file through a hermetic spawn.
//!
//! The launcher path runs in the spawned artifact subprocess -- a copy of this
//! test run's own `ginary` binary with a payload appended -- not in the test
//! process. Under `cargo llvm-cov` the instrumented binary writes its coverage
//! to the file named by `LLVM_PROFILE_FILE`; a subprocess that inherits that
//! variable writes its own `*.profraw`, which `cargo llvm-cov report` then
//! merges. The artifact-run helpers call [`std::process::Command::env_clear`]
//! for a hermetic environment, which drops `LLVM_PROFILE_FILE` with everything
//! else, so the subprocess writes no profile and its execution of `launcher`,
//! `launch`, `cache` and `selfexe` is invisible to coverage.
//!
//! [`preserve_coverage_env`] is the one seam that undoes exactly that one loss:
//! called *after* `env_clear`, it re-injects `LLVM_PROFILE_FILE` when the
//! parent has one and does nothing otherwise, so a non-coverage run stays fully
//! hermetic. `PATH` and the `ERL_*` family the hermetic spawn scrubs are never
//! touched.

use std::ffi::OsStr;
use std::process::Command;

/// The variable `cargo llvm-cov` points at the per-process profile file.
///
/// A subprocess that inherits it writes its own `*.profraw`, which the report
/// step merges with the parent's.
pub const LLVM_PROFILE_FILE: &str = "LLVM_PROFILE_FILE";

/// Re-injects `LLVM_PROFILE_FILE` into a command whose environment was cleared.
///
/// Call this *after* [`std::process::Command::env_clear`] on every spawn of the
/// ginary artifact or stub, so the instrumented subprocess writes coverage that
/// merges into the run. When the parent process carries no `LLVM_PROFILE_FILE`
/// -- an ordinary `cargo test` -- nothing is added and the spawn stays
/// hermetic.
pub fn preserve_coverage_env(command: &mut Command) {
    preserve_coverage_env_value(command, std::env::var_os(LLVM_PROFILE_FILE).as_deref());
}

/// The seam [`preserve_coverage_env`] is written over: the profile-file value
/// is passed in rather than read from the environment, so a unit test can drive
/// both the present and the absent case without mutating the process
/// environment.
///
/// `Some(value)` threads `LLVM_PROFILE_FILE=value` onto `command`; `None` adds
/// nothing.
pub fn preserve_coverage_env_value(command: &mut Command, value: Option<&OsStr>) {
    // Thread exactly `LLVM_PROFILE_FILE` when the parent carries one, and touch
    // nothing else: an absent value leaves the cleared command fully hermetic
    // (no PATH, no ERL_* re-introduced), while a present value lets the
    // instrumented subprocess write its own `*.profraw` for the report step to
    // merge.
    if let Some(value) = value {
        command.env(LLVM_PROFILE_FILE, value);
    }
}

/// The [`preserve_coverage_env`] contract for an [`assert_cmd::Command`].
///
/// The CLI tests spawn `ginary` through `assert_cmd`, whose `Command` is a
/// distinct type from [`std::process::Command`] and cannot be passed to
/// [`preserve_coverage_env`]. This overload does the same one thing for it:
/// after the test has called `env_clear`, it re-injects `LLVM_PROFILE_FILE`
/// when the parent carries one, so the instrumented `ginary` subprocess writes
/// coverage that the report step merges, and adds nothing when the parent has
/// none.
pub fn preserve_coverage_env_assert(command: &mut assert_cmd::Command) {
    if let Some(value) = std::env::var_os(LLVM_PROFILE_FILE) {
        command.env(LLVM_PROFILE_FILE, value);
    }
}
