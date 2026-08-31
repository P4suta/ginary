// SPDX-License-Identifier: MIT OR Apache-2.0
//! `ginary build --sbom-out` threw away the build report to report the SBOM.
//!
//! `write_build` wrote the document between the build and the report:
//!
//! ```text
//! let built = bundle::build(&options, &diag)?;
//! let sbom_path = if wanted { Some(write_sbom_for(&built.out, ...)?) } else { None };
//! ```
//!
//! A `--sbom-out` naming a path in a directory that does not exist therefore
//! ran the whole build — minutes of `gleam export`, a staged OTP, a `strip`
//! pass and zstd 19 — wrote the artifact, and then exited 1 having printed only
//! `cannot write the SBOM to …`. The artifact was on disk and its path was
//! nowhere, so a script could not tell the run from a build that failed.
//!
//! The correct behaviour is two things. A `--sbom-out` whose parent directory
//! is not there is refused *before* the build starts, because that is a mistake
//! in the command line and no work should be spent discovering it. And when the
//! document fails for a reason only the write can find, the build report is
//! written first, so the artifact's path survives.

use std::path::Path;

use crate::common::built::BuiltProject;
use crate::common::project::TempProject;
use crate::common::tools::require_tools;

/// The programs a build of the fixture needs.
const TOOLS: [&str; 3] = ["gleam", "erl", "strip"];

/// The fixture this file builds.
const APP: &str = "hello_ffi";

#[test]
fn a_destination_in_a_missing_directory_is_refused_before_the_build_runs() {
    let project = TempProject::named("nowhere");
    let destination = project.root().join("nope/bill.json");

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_ginary"))
        .arg("build")
        .arg("--sbom-out")
        .arg(&destination)
        .current_dir(project.root())
        .output()
        .expect("`ginary build` runs");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

    assert!(!output.status.success(), "{stderr}");
    assert!(
        stderr.contains("SBOM"),
        "the refusal is about the destination: {stderr}"
    );
    assert!(
        !stderr.contains("gleam"),
        "the export is what the refusal has to come before: {stderr}"
    );
}

#[test]
fn a_document_that_cannot_be_written_still_leaves_the_artifact_named() {
    let Some(_tools) = require_tools(&TOOLS) else {
        return;
    };
    let project = BuiltProject::copy(APP);
    // An existing *directory* is a destination whose parent is there and which
    // no writer can open: the failure the fail-fast check cannot see.
    let destination = project.root().join("occupied");
    std::fs::create_dir_all(&destination).expect("the directory");

    let output = project.build_with(&["--sbom-out", &destination.display().to_string()], &[]);
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let artifact: &Path = &project.artifact();

    assert!(
        !output.status.success(),
        "a document that could not be written is a failure\n{stdout}\n{stderr}"
    );
    assert!(artifact.is_file(), "the build wrote {}", artifact.display());
    assert!(
        stdout.contains(&artifact.display().to_string()),
        "the artifact exists and its path is nowhere\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}"
    );
}
