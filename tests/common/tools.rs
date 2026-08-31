// SPDX-License-Identifier: MIT OR Apache-2.0
//! Gating a test on the external programs it needs.
//!
//! A test that needs `erl` cannot run on a machine without Erlang, and a test
//! that quietly passes on such a machine is worse than one that does not run at
//! all. [`require_tools`] makes the choice explicit: it skips, loudly, unless
//! [`REQUIRE_VAR`] says the toolchain is supposed to be there, and then it
//! fails instead.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The variable that turns a skip into a failure.
///
/// CI sets it on the job that installs Erlang and Gleam, so a broken toolchain
/// there cannot look like a green run.
pub const REQUIRE_VAR: &str = "GINARY_REQUIRE_TOOLCHAIN";

/// The programs a gated test asked for, with the path each was found at.
#[derive(Clone, Debug)]
pub struct Toolchain {
    /// Program name to the absolute path `PATH` resolved it to.
    tools: BTreeMap<String, PathBuf>,
}

impl Toolchain {
    /// The path of a program the test asked for.
    ///
    /// # Panics
    ///
    /// If `name` was not in the list passed to [`require_tools`]. That is a bug
    /// in the test, not a property of the machine.
    pub fn path(&self, name: &str) -> &Path {
        match self.tools.get(name) {
            Some(path) => path,
            None => panic!("`{name}` was not requested from require_tools"),
        }
    }
}

/// Finds every named program on `PATH`, or reports a skip.
///
/// Returns `Some(Toolchain)` when all of them are present. When one is missing
/// it prints `skipping: <tool> not on PATH` on standard error and returns
/// `None`, so the caller returns and the test passes without pretending to have
/// covered anything — unless `GINARY_REQUIRE_TOOLCHAIN=1`, in which case the
/// missing program is a panic.
///
/// # Panics
///
/// If a program is missing and [`REQUIRE_VAR`] is `1`.
pub fn require_tools(names: &[&str]) -> Option<Toolchain> {
    let path_var = std::env::var_os("PATH");
    let mut tools = BTreeMap::new();

    for name in names {
        match ginary::process::find_in_path(name, path_var.as_deref()) {
            Some(path) => {
                tools.insert((*name).to_owned(), path);
            }
            None => {
                let required = std::env::var_os(REQUIRE_VAR).is_some_and(|value| value == "1");
                assert!(
                    !required,
                    "`{name}` is not on PATH and {REQUIRE_VAR}=1 forbids skipping"
                );
                eprintln!("skipping: {name} not on PATH");
                return None;
            }
        }
    }

    Some(Toolchain { tools })
}
