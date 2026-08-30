// SPDX-License-Identifier: MIT OR Apache-2.0
//! Copying a Gleam fixture project into a temporary directory and exporting it.
//!
//! The fixtures under `tests/fixtures/` are real Gleam projects, and a test
//! that built one in place would write `build/` into the repository and race
//! every other test doing the same. [`FixtureProject::copy`] takes a copy
//! first — everything but `build/`, which is output rather than input — so each
//! test owns its tree and `gleam` starts from a clean slate.
//!
//! `hello_ffi` is the fixture this exists for. It has no hex dependencies at
//! all, so `gleam export erlang-shipment` needs no network and no warmed cache:
//! its committed `manifest.toml` locks zero packages, and there is nothing to
//! resolve from hex. See the fixture policy in `docs/dev/testing.md`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::common::bounded::run_bounded;

/// How long `gleam export erlang-shipment` gets for a zero-dependency project.
///
/// It takes under a second on the development machine. The budget is wide
/// enough for a cold, loaded CI runner and finite so that a `gleam` waiting on
/// something — a lock, a package server it should not be talking to — is a
/// reported failure rather than a hung test binary.
pub const EXPORT_BUDGET: Duration = Duration::from_secs(180);

/// The directory holding the fixture projects.
///
/// `CARGO_MANIFEST_DIR` rather than the working directory, because a test
/// binary's working directory is not guaranteed to be the crate root.
pub fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// A copy of a fixture Gleam project, owned by one test.
#[derive(Clone, Debug)]
pub struct FixtureProject {
    dir: PathBuf,
}

impl FixtureProject {
    /// Copies `tests/fixtures/<name>` into `into`, skipping `build/`.
    ///
    /// The copy lands at `<into>/<name>`, so a caller can hold several
    /// fixtures, and the caller's own scratch directories, under one temporary
    /// root that is deleted when the test ends.
    ///
    /// # Panics
    ///
    /// If the fixture does not exist, or if any part of the copy fails. Both
    /// are bugs in the test tree rather than properties of the machine.
    pub fn copy(name: &str, into: &Path) -> Self {
        let source = fixtures_dir().join(name);
        assert!(
            source.is_dir(),
            "no fixture project at {}",
            source.display()
        );
        let dir = into.join(name);
        copy_tree(&source, &dir);
        Self { dir }
    }

    /// The root of the copied project, the directory holding `gleam.toml`.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Runs `gleam export erlang-shipment` and returns the directory it wrote.
    ///
    /// `gleam export erlang-shipment` takes no flags of its own — there is no
    /// `--no-print-progress` to pass — so it is run plainly and its output is
    /// only looked at when it fails.
    ///
    /// # Panics
    ///
    /// If `gleam` cannot be run, if it does not exit within [`EXPORT_BUDGET`],
    /// if it exits non-zero — the panic carries its whole standard error,
    /// because a truncated Gleam diagnostic is useless — or if it exits zero
    /// without writing `build/erlang-shipment`.
    pub fn export_shipment(&self) -> PathBuf {
        self.export_shipment_with(Path::new("gleam"))
    }

    /// [`FixtureProject::export_shipment`], running one particular `gleam`.
    ///
    /// A gated test resolves its programs through
    /// [`crate::common::tools::require_tools`] and then runs the path it was
    /// given, rather than trusting `PATH` a second time.
    ///
    /// # Panics
    ///
    /// As [`FixtureProject::export_shipment`].
    pub fn export_shipment_with(&self, gleam: &Path) -> PathBuf {
        let mut command = std::process::Command::new(gleam);
        command
            .args(["export", "erlang-shipment"])
            .current_dir(&self.dir);
        let output = run_bounded(
            &mut command,
            EXPORT_BUDGET,
            &format!("`{} export erlang-shipment`", gleam.display()),
        );

        assert!(
            output.status.success(),
            "`gleam export erlang-shipment` failed in {} with {}\n--- stdout ---\n{}\n--- stderr ---\n{}",
            self.dir.display(),
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );

        let shipment = self.dir.join("build/erlang-shipment");
        assert!(
            shipment.is_dir(),
            "`gleam export erlang-shipment` exited zero without writing {}",
            shipment.display()
        );
        shipment
    }
}

/// Copies a directory recursively, skipping any entry called `build`.
///
/// `build/` is `gleam`'s output directory. Copying it would carry one test's
/// compilation state into another's, which is the one thing the copy exists to
/// prevent.
fn copy_tree(source: &Path, target: &Path) {
    std::fs::create_dir_all(target)
        .unwrap_or_else(|error| panic!("cannot create {}: {error}", target.display()));
    let entries = std::fs::read_dir(source)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", source.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|error| {
            panic!("cannot read an entry of {}: {error}", source.display())
        });
        let name = entry.file_name();
        if name == "build" {
            continue;
        }
        let from = entry.path();
        let to = target.join(&name);
        if from.is_dir() {
            copy_tree(&from, &to);
        } else {
            std::fs::copy(&from, &to).unwrap_or_else(|error| {
                panic!(
                    "cannot copy {} to {}: {error}",
                    from.display(),
                    to.display()
                )
            });
        }
    }
}
