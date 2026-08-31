// SPDX-License-Identifier: MIT OR Apache-2.0
//! The pieces of the build that are testable without building.
//!
//! `bundle::build` needs a Gleam project, an OTP installation and `strip`, so
//! the whole of it lives in `tests/e2e_hello.rs` behind a toolchain gate. Three
//! things do not, and they are here because each is a rule a machine with no
//! toolchain must still hold:
//!
//! * the stub refusal — a copy of a *packaged* application handed to a build
//!   would produce an artifact with two payloads and one trailer, and
//!   [`ginary::bundle::check_stub`] is the seam that refuses it;
//! * the work directory's name, which is what `--keep-staging` prints and what
//!   a later build recognises as the residue of a killed one;
//! * the report's two rendered forms, which are pure functions of numbers a
//!   test can choose.

mod common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use ginary::assemble::Category;
use ginary::bundle::{self, BuildReport, BundleError, WORK_DIR_PREFIX, WORK_STAGE_NAME};
use ginary::config::{BuildFlags, BuildOptions, ProjectConfig};
use ginary::diag::Diag;
use ginary::report::{CategorySize, NeedsSummary, SizeReport};
use ginary::strip::StripReport;

use crate::common::artifact::SyntheticArtifact;
use crate::common::payload::sample_manifest;
use crate::common::project::TempProject;

/// This test run's own `ginary` binary: a stub with no trailer.
fn ginary_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_ginary"))
}

/// The options a build of `project` would run with, `--skip-export` set.
///
/// `--skip-export` is what keeps the two build tests off the toolchain: the
/// project has no shipment, so the phase after the stub check fails at once
/// rather than running `gleam`.
fn build_options(project: &TempProject) -> BuildOptions {
    let config = ProjectConfig::read(&project.manifest()).expect("the fixture manifest parses");
    let flags = BuildFlags {
        start: project.root().to_path_buf(),
        skip_export: true,
        ..BuildFlags::default()
    };
    BuildOptions::merge(project.root(), &config, &flags).expect("the defaults merge")
}

// ------------------------------------------------------------ the stub --

#[test]
fn the_plain_ginary_binary_is_an_acceptable_stub() {
    let path = ginary_bin();
    let expected = std::fs::metadata(&path).expect("stat the binary").len();

    let len = bundle::check_stub(&path).expect("the command line tool is the stub");

    assert_eq!(
        len, expected,
        "the stub's length is the payload's offset, so it is the whole file"
    );
}

#[test]
fn a_stub_that_already_carries_a_trailer_is_refused() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let artifact = SyntheticArtifact::build(dir.path());

    let error = bundle::check_stub(artifact.path())
        .expect_err("a packaged application may not be used as a stub");

    let BundleError::BundledStub { path } = &error else {
        panic!("expected BundleError::BundledStub, got {error:?}");
    };
    assert_eq!(path, artifact.path());
    let message = error.to_string();
    assert!(
        message.contains("a bundled executable cannot build; install plain ginary"),
        "the message must say what to do instead: {message}"
    );
}

#[test]
fn a_stub_that_is_not_there_is_an_io_error_naming_it() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let missing = dir.path().join("no-such-ginary");

    let error = bundle::check_stub(&missing).expect_err("a missing stub is an error");

    assert!(
        matches!(error, BundleError::Io { .. }),
        "expected BundleError::Io, got {error:?}"
    );
    assert!(
        error.to_string().contains("no-such-ginary"),
        "the message must name the file: {error}"
    );
}

#[test]
fn a_build_refuses_a_bundled_stub_before_it_looks_at_the_project() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let artifact = SyntheticArtifact::build(dir.path());
    // A project with no shipment at all: `--skip-export` over it is the error
    // the build reports *second*, so a build that reached it is a build that
    // did not check the stub first.
    let project = TempProject::named("hello");
    let options = build_options(&project);

    let error = bundle::build_with_stub(&options, artifact.path(), &Diag::disabled())
        .expect_err("a packaged application may not be used as a stub");

    let BundleError::BundledStub { path } = &error else {
        panic!(
            "expected BundleError::BundledStub before the shipment is looked for, got {error:?}"
        );
    };
    assert_eq!(path, artifact.path());
}

#[test]
fn a_build_with_a_plain_stub_gets_as_far_as_the_missing_shipment() {
    let project = TempProject::named("hello");
    let options = build_options(&project);

    let error = bundle::build_with_stub(&options, &ginary_bin(), &Diag::disabled())
        .expect_err("a project with no shipment cannot be built with --skip-export");

    assert!(
        matches!(error, BundleError::Gleam(_)),
        "the stub is fine, so the next phase is what fails: {error:?}"
    );
}

// -------------------------------------------------- the work directory --

#[test]
fn the_work_directory_is_under_the_projects_build_ginary_and_names_the_pid() {
    let project = Path::new("/w/app");

    let root = bundle::work_root(project, 4242);

    assert_eq!(
        root,
        Path::new("/w/app/build/ginary/.work-4242/root"),
        "staging belongs to the project, whatever --out says"
    );
    assert!(
        root.starts_with(project),
        "an artifact written to /usr/local/bin must not stage there"
    );
    assert_eq!(WORK_DIR_PREFIX, ".work-");
    assert_eq!(WORK_STAGE_NAME, "root");
}

#[test]
fn two_concurrent_builds_of_one_project_stage_in_different_directories() {
    let project = Path::new("/w/app");

    assert_ne!(
        bundle::work_root(project, 1),
        bundle::work_root(project, 2),
        "the process id is what keeps two builds of one project apart"
    );
}

// ---------------------------------------------------------- the report --

/// A report over numbers chosen so that every column is distinguishable.
fn synthetic_report() -> BuildReport {
    let mut categories = BTreeMap::new();
    categories.insert(
        Category::ErtsBinary,
        CategorySize {
            files: 4,
            bytes_before: 41_675_352,
            bytes_after: 11_742_936,
        },
    );
    categories.insert(
        Category::GleamBeam,
        CategorySize {
            files: 3,
            bytes_before: 1_382_144,
            bytes_after: 511_072,
        },
    );

    BuildReport {
        app: "hello_ffi".to_owned(),
        out: PathBuf::from("build/ginary/hello_ffi"),
        stub_len: 5_242_880,
        payload_len: 9_437_184,
        total_len: 5_242_880 + 9_437_184 + 64,
        sha256: "a".repeat(64),
        strip: StripReport::disabled(),
        size_report: SizeReport {
            categories,
            total_before: 43_057_496,
            total_after: 12_254_008,
            elf_deps: Vec::new(),
            needs_summary: NeedsSummary {
                needed: ["libc.so.6".to_owned(), "libm.so.6".to_owned()]
                    .into_iter()
                    .collect(),
                glibc_max: Some("2.38".to_owned()),
            },
            warnings: Vec::new(),
        },
        manifest: sample_manifest(),
        staging: None,
        warnings: Vec::new(),
        explain: None,
    }
}

#[test]
fn the_artifact_line_names_the_file_and_its_three_parts() {
    let report = synthetic_report();

    assert_eq!(
        report.artifact_line(),
        "artifact: build/ginary/hello_ffi (5242880 stub + 9437184 payload + 64 trailer)"
    );
}

#[test]
fn the_build_report_is_the_size_table_and_then_the_artifact_line() {
    let report = synthetic_report();

    insta::assert_snapshot!("build_report_text", report.render_text());
}

#[test]
fn the_rendered_report_ends_with_the_artifact_line() {
    let report = synthetic_report();

    let text = report.render_text();

    assert!(
        text.ends_with(&format!("{}\n", report.artifact_line())),
        "the rendered report must end with the line the caller can quote:\n{text}"
    );
    // The arithmetic behind `total_len` is not asserted here, because the
    // three numbers of a hand-built report are three literals: a test over
    // them restates the fixture to itself. `tests/e2e_hello.rs` asserts it
    // against a real build, where `--report json` and the artifact's own size
    // on disk are independent of each other.
}

#[test]
fn a_work_directory_that_could_not_be_removed_is_printed_with_the_report() {
    let mut report = synthetic_report();
    report.warnings = vec!["build/ginary/.work-1 could not be removed: no".to_owned()];

    let text = report.render_text();

    assert!(
        text.contains("warning: build/ginary/.work-1 could not be removed: no"),
        "a build that left staging behind has to say so:\n{text}"
    );
    assert!(
        text.ends_with(&format!("{}\n", report.artifact_line())),
        "the artifact line stays last, so a caller can still quote it:\n{text}"
    );
}
