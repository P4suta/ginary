// SPDX-License-Identifier: MIT OR Apache-2.0
//! Finding the Gleam project and running `gleam export erlang-shipment`.
//!
//! This is the only module that runs `gleam`, and it runs it exactly once per
//! build. Everything downstream — the closure, assembly, the payload — reads
//! the directory the export wrote and never the project again.
//!
//! Two rules shape it. The project is found by walking *up* from where the
//! user is, the way every other project tool behaves, so `ginary build` in a
//! subdirectory builds the project that subdirectory belongs to. And when
//! `gleam` fails, its standard error is passed through verbatim: a Gleam type
//! error is a message the Gleam compiler has already written well, and
//! summarising it would only lose the part the user needs.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::diag::Diag;
use crate::process::ProcessError;

/// The program this module runs.
pub const PROGRAM: &str = "gleam";

/// The file whose presence marks a Gleam project root.
pub const MANIFEST_NAME: &str = "gleam.toml";

/// Where `gleam export erlang-shipment` writes, relative to the project root.
pub const SHIPMENT_DIR: &str = "build/erlang-shipment";

/// The arguments the export is run with.
pub const EXPORT_ARGS: [&str; 2] = ["export", "erlang-shipment"];

/// How long the export gets before it is killed and reported.
///
/// Ten minutes: a cold project with dependencies to compile is minutes on a
/// loaded machine, and a `gleam` waiting on a lock or on a package server is a
/// failed build rather than a hung one.
pub const EXPORT_TIMEOUT: Duration = Duration::from_secs(600);

/// How long `gleam --version` gets.
///
/// The version is decoration on the manifest rather than something a build
/// depends on, so the budget is short and a failure is [`None`].
pub const VERSION_TIMEOUT: Duration = Duration::from_secs(10);

/// A directory holding a `gleam.toml`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectDir {
    root: PathBuf,
}

impl ProjectDir {
    /// Names a project root directly, without searching for it.
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// The project root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// `<root>/gleam.toml`.
    pub fn manifest(&self) -> PathBuf {
        self.root.join(MANIFEST_NAME)
    }

    /// `<root>/build/erlang-shipment`.
    pub fn shipment(&self) -> PathBuf {
        self.root.join(SHIPMENT_DIR)
    }
}

/// Walks up from `start` to the nearest directory holding a `gleam.toml`.
///
/// # Errors
///
/// [`GleamError::NoProject`] when neither `start` nor any of its parents holds
/// one, naming the directory the search began in.
pub fn find_project(start: &Path) -> Result<ProjectDir, GleamError> {
    for directory in start.ancestors() {
        if directory.join(MANIFEST_NAME).is_file() {
            return Ok(ProjectDir::new(directory.to_path_buf()));
        }
    }
    Err(GleamError::NoProject {
        start: start.to_path_buf(),
    })
}

/// Runs `gleam export erlang-shipment` in the project and returns its output
/// directory.
///
/// # Errors
///
/// [`GleamError::NotOnPath`] when there is no `gleam`, [`GleamError::Export`]
/// carrying its standard error verbatim when it fails, and
/// [`GleamError::NoShipment`] when it exits zero without writing the
/// directory.
pub fn export_shipment(project: &ProjectDir, diag: &Diag) -> Result<PathBuf, GleamError> {
    let program = find_gleam().ok_or(GleamError::NotOnPath)?;
    let _phase = diag.phase("export");
    diag.kv("export", &[("program", &program.display().to_string())]);

    let args: Vec<&str> = EXPORT_ARGS.to_vec();
    let output = crate::process::run_in_dir_with_timeout(
        &program,
        &args,
        Some(project.root()),
        EXPORT_TIMEOUT,
    )?;
    if !output.success {
        // Verbatim: a Gleam type error is a diagnostic the compiler has
        // already written well, and summarising it would lose the half the
        // user needs.
        return Err(GleamError::Export {
            dir: project.root().to_path_buf(),
            stderr: output.stderr,
        });
    }

    let shipment = project.shipment();
    if !shipment.is_dir() {
        return Err(GleamError::NoShipment { path: shipment });
    }
    Ok(shipment)
}

/// The shipment directory `--skip-export` reuses.
///
/// # Errors
///
/// [`GleamError::MissingShipment`], whose message says how to produce one,
/// when the directory is not there.
pub fn existing_shipment(project: &ProjectDir) -> Result<PathBuf, GleamError> {
    let shipment = project.shipment();
    if shipment.is_dir() {
        Ok(shipment)
    } else {
        Err(GleamError::MissingShipment { path: shipment })
    }
}

/// The version of the `gleam` on `PATH`, for the manifest.
///
/// [`None`] whenever the question cannot be answered — no `gleam`, a `gleam`
/// that fails, or output this does not recognise — because the version is
/// recorded in the artifact and never acted on.
pub fn gleam_version() -> Option<String> {
    let program = find_gleam()?;
    let output =
        crate::process::run_with_timeout(&program, &["--version"], VERSION_TIMEOUT).ok()?;
    if !output.success {
        return None;
    }
    parse_version(&output.stdout)
}

/// The `gleam` on `PATH`, or [`None`] when there is none.
fn find_gleam() -> Option<PathBuf> {
    crate::process::find_in_path(PROGRAM, std::env::var_os("PATH").as_deref())
}

/// The version out of `gleam --version` output.
///
/// `gleam 1.18.1` is `Some("1.18.1")`. The pure half of [`gleam_version`], so
/// that the parsing is testable without a Gleam installation.
pub fn parse_version(output: &str) -> Option<String> {
    let mut words = output.lines().next()?.split_whitespace();
    if words.next()? != PROGRAM {
        return None;
    }
    let version = words.next()?;
    version
        .starts_with(|c: char| c.is_ascii_digit())
        .then(|| version.to_owned())
}

/// Why the project or its export is not usable.
#[derive(Debug, thiserror::Error)]
pub enum GleamError {
    /// Neither the starting directory nor any parent holds a `gleam.toml`.
    #[error(
        "no {MANIFEST_NAME} in {start} or any parent directory: run `ginary build` inside a \
         Gleam project"
    )]
    NoProject {
        /// Where the upward search began.
        start: PathBuf,
    },
    /// There is no `gleam` to run.
    #[error(
        "`{PROGRAM}` is not on PATH: install Gleam, or pass --skip-export to use an export that \
         is already there"
    )]
    NotOnPath,
    /// `gleam` ran and failed. Its diagnosis travels verbatim.
    #[error("`{PROGRAM} export erlang-shipment` failed in {dir}:\n{stderr}")]
    Export {
        /// The project the export ran in.
        dir: PathBuf,
        /// Everything `gleam` wrote to standard error.
        stderr: String,
    },
    /// `gleam` exited zero and wrote nothing.
    #[error("`{PROGRAM} export erlang-shipment` exited zero without writing {path}")]
    NoShipment {
        /// The directory that should have been written.
        path: PathBuf,
    },
    /// `--skip-export` was given and there is nothing to reuse.
    #[error(
        "no shipment at {path}: --skip-export reuses an export that is already there; run \
         `{PROGRAM} export erlang-shipment` first, or drop --skip-export"
    )]
    MissingShipment {
        /// The directory that is not there.
        path: PathBuf,
    },
    /// `gleam` could not be started, or did not finish in time.
    #[error("cannot run `{PROGRAM}`")]
    Process(#[from] ProcessError),
}
