// SPDX-License-Identifier: MIT OR Apache-2.0
//! Unit test of the coverage-env helper itself.
//!
//! The measurement fix `preserve_coverage_env` performs -- re-injecting
//! `LLVM_PROFILE_FILE` into a hermetic spawn so the instrumented subprocess
//! writes coverage that merges into the run -- cannot be checked by asserting on
//! coverage numbers. What can be checked is the helper's own contract: it
//! threads exactly the profile file and nothing else onto a cleared command,
//! and it threads nothing when there is no profile file to thread. Both are
//! driven through the `preserve_coverage_env_value` seam, which takes the value
//! rather than reading the process environment, so the test mutates no global
//! state.

mod common;

use std::ffi::{OsStr, OsString};
use std::process::Command;

use common::coverage::preserve_coverage_env_value;

/// After a cleared environment, the helper threads exactly `LLVM_PROFILE_FILE`
/// with the given value and adds nothing else -- in particular no `PATH` and no
/// `ERL_*` variable the hermetic spawn scrubs.
#[test]
fn it_threads_only_the_profile_file_after_a_clear() {
    let mut command = Command::new("true");
    command.env_clear();
    preserve_coverage_env_value(&mut command, Some(OsStr::new("/cov/profile-%p-%m.profraw")));

    let envs: Vec<(OsString, Option<OsString>)> = command
        .get_envs()
        .map(|(key, value)| (key.to_os_string(), value.map(OsStr::to_os_string)))
        .collect();

    assert_eq!(
        envs,
        vec![(
            OsString::from("LLVM_PROFILE_FILE"),
            Some(OsString::from("/cov/profile-%p-%m.profraw")),
        )],
        "a cleared command must carry exactly the threaded profile file and nothing else"
    );
}
