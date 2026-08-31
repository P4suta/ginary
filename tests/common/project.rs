// SPDX-License-Identifier: MIT OR Apache-2.0
//! A `gleam.toml` in a temporary directory, and the tree around it.
//!
//! `tests/config.rs` reads its manifests as text, so it needs nothing from
//! here. `tests/gleam.rs` does: [`crate::common::project::TempProject`] is
//! what an upward search walks over, what `--skip-export` looks for a shipment
//! in, and what a `ginary build` run outside a project must fail to find.
//!
//! Nothing here writes Gleam source. A project that has to *compile* is
//! `tests/fixtures/hello_ffi`, copied by
//! [`crate::common::fixture::FixtureProject`]; this builder exists for the
//! tests that only care about where `gleam.toml` is.

use std::path::{Path, PathBuf};

/// A directory holding a `gleam.toml`, with a temporary root of its own.
#[derive(Debug)]
pub struct TempProject {
    dir: tempfile::TempDir,
    root: PathBuf,
}

impl TempProject {
    /// Writes `manifest` as `<tmp>/project/gleam.toml`.
    ///
    /// The project is one level below the temporary root on purpose: a test
    /// that walks *up* from the project has somewhere to walk to, and a test
    /// that starts outside a project has a directory that is not one.
    ///
    /// # Panics
    ///
    /// If the temporary directory or the manifest cannot be written.
    pub fn new(manifest: &str) -> Self {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let root = dir.path().join("project");
        std::fs::create_dir_all(&root).expect("the project directory");
        std::fs::write(root.join("gleam.toml"), manifest).expect("the manifest");
        Self { dir, root }
    }

    /// A project whose manifest declares `name` and nothing else.
    pub fn named(name: &str) -> Self {
        Self::new(&format!("name = \"{name}\"\nversion = \"0.1.0\"\n"))
    }

    /// The temporary root, which is the project's parent.
    pub fn outside(&self) -> &Path {
        self.dir.path()
    }

    /// The project root, the directory holding `gleam.toml`.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// `<root>/gleam.toml`.
    pub fn manifest(&self) -> PathBuf {
        self.root.join("gleam.toml")
    }

    /// Creates `<root>/<relative>` as a directory and returns it.
    ///
    /// # Panics
    ///
    /// If the directory cannot be created.
    pub fn subdir(&self, relative: &str) -> PathBuf {
        let path = self.root.join(relative);
        std::fs::create_dir_all(&path).expect("a subdirectory of the project");
        path
    }

    /// Creates an empty `<root>/build/erlang-shipment` and returns it.
    ///
    /// What `--skip-export` reuses. It holds no applications, because the
    /// tests that use it ask whether the directory was found rather than what
    /// is in it.
    ///
    /// # Panics
    ///
    /// If the directory cannot be created.
    pub fn empty_shipment(&self) -> PathBuf {
        self.subdir("build/erlang-shipment")
    }
}

/// The directory holding the `gleam.toml` fixtures.
pub fn config_fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/config")
}

/// Reads one `tests/fixtures/config/<name>` as text.
///
/// # Panics
///
/// If the fixture is not there, which is a bug in the test tree.
pub fn config_fixture(name: &str) -> String {
    let path = config_fixtures().join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()))
}
