// SPDX-License-Identifier: MIT OR Apache-2.0
//! Gating a test on the external programs it needs.
//!
//! A test that needs `erl` cannot run on a machine without Erlang, and a test
//! that quietly passes on such a machine is worse than one that does not run at
//! all. [`require_tools`] makes the choice explicit: it skips, loudly, unless
//! [`REQUIRE_VAR`] says the toolchain is supposed to be there, and then it
//! fails instead.
//!
//! [`REQUIRE_VAR`] is a claim about *the toolchain an artifact is built with*:
//! `gleam`, `erl`, `strip`, `docker`. It is not a claim about every program a
//! test could want, and reading it as one is how `actionlint` — a lint, wanted
//! by one job and installed by no runner — failed three CI jobs that had a
//! complete toolchain. So this module holds a second gate,
//! [`require_actionlint`], with a variable of its own, exactly as E6 split
//! `GINARY_REQUIRE_STUBS` out for the cross-built stubs. The rule for adding a
//! third is the rule these two follow: a gate is a claim somebody has to be
//! able to *make true*, so it belongs to whichever job installs the thing.

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

/// The variable that says `actionlint` is supposed to be on this machine, so
/// a missing one is a failure rather than a skip.
///
/// Deliberately *not* [`REQUIRE_VAR`]. `GINARY_REQUIRE_TOOLCHAIN` says the
/// toolchain a runtime is packaged with is installed — `gleam`, `erl`,
/// `strip`, `docker` — and `actionlint` is none of those: it is a lint over
/// the workflow files, it has nothing to do with whether a runtime can be
/// packaged, and no hosted runner ships it. Three jobs set the toolchain
/// variable and ran the `regressions` target; all three panicked on a machine
/// whose toolchain was complete. See
/// `tests/regressions/e7_actionlint_was_required_of_every_toolchain_job.rs`.
///
/// Exactly one job sets this one: `lint` in `.github/workflows/ci.yml`, which
/// installs the tool and selects the gated test by name. A gate no job can
/// satisfy is a test that never runs; a gate every job claims is a job that
/// fails for a tool it was never given.
pub const REQUIRE_ACTIONLINT_VAR: &str = "GINARY_REQUIRE_ACTIONLINT";

/// The `actionlint` binary, or a reported skip.
///
/// The same rule [`require_tools`] follows, against [`REQUIRE_ACTIONLINT_VAR`]
/// rather than [`REQUIRE_VAR`]: `Some(path)` when the program is on `PATH`, a
/// printed `skipping:` and `None` when it is not, and a panic when it is not
/// and the caller's job promised it.
///
/// # Panics
///
/// If `actionlint` is missing and [`REQUIRE_ACTIONLINT_VAR`] is `1`.
pub fn require_actionlint() -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH");
    match ginary::process::find_in_path(ACTIONLINT, path_var.as_deref()) {
        Some(path) => Some(path),
        None => {
            let required =
                std::env::var_os(REQUIRE_ACTIONLINT_VAR).is_some_and(|value| value == "1");
            assert!(
                !required,
                "`{ACTIONLINT}` is not on PATH and {REQUIRE_ACTIONLINT_VAR}=1 forbids skipping. \
                 The job that sets it is the job that installs the tool"
            );
            eprintln!("skipping: {ACTIONLINT} not on PATH");
            None
        }
    }
}

/// The program [`require_actionlint`] looks for.
pub const ACTIONLINT: &str = "actionlint";
