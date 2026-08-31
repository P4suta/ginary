// SPDX-License-Identifier: MIT OR Apache-2.0
//! Reading a packaged application from the outside.
//!
//! Two halves. The rendering is pure — an `ArtifactInfo` a test writes by
//! hand, a report and a launch plan derived from it — so the exact text is
//! pinned by snapshots and nothing is read from disk. The rest runs against a
//! `SyntheticArtifact`: a real payload, a real trailer and this test run's own
//! `ginary` binary as the stub, with the two ways a file can stop being the
//! artifact it says it is.
//!
//! Nothing here extracts anything. Inspecting a stranger's artifact must write
//! nothing at all, and `--verify` streams the payload past a hasher rather
//! than unpacking it.
// The command line half of the suite: every claim in this file is about a
// module the `cli` feature carries, so a `--no-default-features` build has
// nothing here to run. See `docs/dev/log/C2.md`.
#![cfg(feature = "cli")]

mod common;

use std::path::{Path, PathBuf};

use ginary::inspect::{
    self, ArtifactInfo, InspectError, LARGEST_FILES, PLACEHOLDER_APP_DIR, PLACEHOLDER_ROOT,
};
use ginary::manifest::{Index, IndexFile, LaunchSpec, OtpProvenance};
use ginary::trailer::Trailer;

use crate::common::artifact::SyntheticArtifact;
use crate::common::payload::{sample_launch, sample_manifest};

/// The stub length of the hand-built artifact.
const STUB_LEN: u64 = 1000;

/// The payload length of the hand-built artifact.
const PAYLOAD_LEN: u64 = 500;

/// An `ArtifactInfo` over numbers and files a test chose.
///
/// Every value is deliberately distinguishable: no two index files share a
/// size except the pair that pins the tie-break, and the three lengths add up
/// the way a real file's do.
fn hand_built() -> ArtifactInfo {
    ArtifactInfo {
        path: PathBuf::from("/w/hello"),
        trailer: Trailer {
            payload_offset: STUB_LEN,
            payload_len: PAYLOAD_LEN,
            payload_sha256: [0xab; 32],
        },
        manifest: sample_manifest(),
        index: Index {
            files: vec![
                index_file("bin/no_dot_erlang.boot", 300),
                index_file("erts-17.0.5/bin/beam.smp", 900),
                index_file("lib/hello/ebin/hello.app", 300),
                index_file("lib/hello/ebin/hello.beam", 700),
            ],
        },
        stub_len: STUB_LEN,
        payload_len: PAYLOAD_LEN,
        total_len: STUB_LEN + PAYLOAD_LEN + ginary::trailer::TRAILER_LEN,
    }
}

/// One index entry; the digest and the category are not what these tests read.
fn index_file(path: &str, size: u64) -> IndexFile {
    IndexFile {
        path: path.to_owned(),
        size,
        mode: 0o644,
        sha256: "0".repeat(64),
        category: ginary::assemble::Category::Other,
    }
}

// ---------------------------------------------------------- the report --

#[test]
fn the_text_report_names_every_field_a_reader_asks_for() {
    insta::assert_snapshot!("inspect_text", hand_built().render_text());
}

#[test]
fn the_largest_files_are_the_biggest_first_and_then_in_path_order() {
    let info = hand_built();

    let names: Vec<&str> = info
        .largest_files(LARGEST_FILES)
        .iter()
        .map(|file| file.path.as_str())
        .collect();

    assert_eq!(
        names,
        [
            "erts-17.0.5/bin/beam.smp",
            "lib/hello/ebin/hello.beam",
            // Two files of 300 bytes: path order decides, so the list is the
            // same on every machine that reads the same artifact.
            "bin/no_dot_erlang.boot",
            "lib/hello/ebin/hello.app",
        ]
    );
}

#[test]
fn the_largest_files_stop_at_the_count_that_was_asked_for() {
    let info = hand_built();

    assert_eq!(info.largest_files(2).len(), 2);
    assert_eq!(
        info.largest_files(0).len(),
        0,
        "asking for none must not be read as asking for all"
    );
    assert_eq!(
        info.largest_files(100).len(),
        4,
        "asking for more than there are is not an error"
    );
}

// ----------------------------------------------------- the launch plan --

#[test]
fn the_launch_plan_is_the_launchers_own_against_a_placeholder_root() {
    let info = hand_built();

    let plan = inspect::launch_plan(
        &info,
        Path::new(PLACEHOLDER_ROOT),
        Path::new(PLACEHOLDER_APP_DIR),
    )
    .expect("the sample manifest launches");

    insta::assert_snapshot!("inspect_launch_plan", inspect::render_launch_plan(&plan));
}

#[test]
fn a_manifest_the_launcher_would_refuse_is_reported_rather_than_printed() {
    let mut info = hand_built();
    info.manifest.launch = LaunchSpec {
        pa: vec!["../../escape/ebin".to_owned()],
        ..sample_launch()
    };

    let error = inspect::launch_plan(
        &info,
        Path::new(PLACEHOLDER_ROOT),
        Path::new(PLACEHOLDER_APP_DIR),
    )
    .expect_err("a path that leaves the extracted root is not a plan");

    assert!(
        matches!(error, InspectError::Launch { .. }),
        "expected InspectError::Launch, got {error:?}"
    );
}

// ------------------------------------------------------- a real file --

#[test]
fn opening_an_artifact_reads_its_trailer_manifest_and_index() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let artifact = SyntheticArtifact::build(dir.path());

    let info = inspect::open(artifact.path()).expect("the artifact is readable");

    assert_eq!(info.path, artifact.path());
    assert_eq!(&info.trailer, artifact.trailer());
    assert_eq!(&info.manifest, artifact.manifest());
    assert_eq!(info.stub_len, artifact.stub_len());
    assert_eq!(info.payload_len, artifact.packed().len);
    assert_eq!(info.total_len, artifact.file_len());
    assert!(
        !info.index.files.is_empty(),
        "the index is entry 1 of the payload and lists what was staged"
    );
}

#[test]
fn opening_a_file_with_no_trailer_says_it_is_not_an_artifact() {
    let plain = PathBuf::from(env!("CARGO_BIN_EXE_ginary"));

    let error = inspect::open(&plain).expect_err("the command line tool is not an artifact");

    let InspectError::NoTrailer { path } = &error else {
        panic!("expected InspectError::NoTrailer, got {error:?}");
    };
    assert_eq!(path, &plain);
    assert!(
        error.to_string().contains("no ginary trailer"),
        "the message is what `ginary inspect` prints: {error}"
    );
}

#[test]
fn opening_an_artifact_whose_trailer_lies_about_the_file_reports_the_trailer() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let artifact = SyntheticArtifact::build(dir.path());
    artifact.break_geometry();

    let error = inspect::open(artifact.path()).expect_err("a trailer that lies is not readable");

    assert!(
        matches!(error, InspectError::Trailer { .. }),
        "expected InspectError::Trailer, got {error:?}"
    );
}

#[test]
fn opening_a_file_that_is_not_there_names_it() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let missing = dir.path().join("nothing");

    let error = inspect::open(&missing).expect_err("a missing file is an error");

    assert!(
        matches!(error, InspectError::Io { .. }),
        "expected InspectError::Io, got {error:?}"
    );
    assert!(
        error.to_string().contains("nothing"),
        "the message must name the file: {error}"
    );
}

// ---------------------------------------------------------- verifying --

#[test]
fn verifying_an_intact_artifact_finds_the_digest_the_trailer_carries() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let artifact = SyntheticArtifact::build(dir.path());
    let info = inspect::open(artifact.path()).expect("the artifact is readable");

    let verification = inspect::verify(&info).expect("the payload can be re-hashed");

    assert!(
        verification.ok(),
        "an artifact nothing touched must verify: expected {} got {}",
        verification.expected,
        verification.actual
    );
    assert_eq!(
        verification.expected,
        hex::encode(artifact.packed().sha256),
        "the expected digest is the trailer's, in lower-case hexadecimal"
    );
    assert_eq!(verification.actual, verification.expected);
}

#[test]
fn verifying_an_artifact_whose_payload_changed_reports_both_digests() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let artifact = SyntheticArtifact::build(dir.path());
    let expected = hex::encode(artifact.packed().sha256);
    artifact.break_payload_tail();
    let info = inspect::open(artifact.path()).expect("the manifest is still readable");

    let verification = inspect::verify(&info).expect("the payload can still be re-hashed");

    assert!(
        !verification.ok(),
        "a payload with a flipped byte must not verify"
    );
    assert_eq!(
        verification.expected, expected,
        "the trailer is unchanged, so the expected digest is the one it was built with"
    );
    assert_ne!(
        verification.actual, expected,
        "the actual digest is the one the bytes on disk produce"
    );
}

#[test]
fn a_corrupted_artifact_still_says_what_it_claims_to_be() {
    // Verification is a separate question from readability: `ginary inspect`
    // without `--verify` must still print the manifest of a file that would
    // fail `--verify`, because that is how a user finds out what the file was
    // supposed to be.
    let dir = tempfile::tempdir().expect("a temporary directory");
    let artifact = SyntheticArtifact::build(dir.path());
    let manifest = artifact.manifest().clone();
    artifact.break_payload_tail();

    let info = inspect::open(artifact.path()).expect("the front of the payload is intact");

    assert_eq!(info.manifest, manifest);
}

#[test]
fn an_artifact_that_recorded_no_provenance_says_unknown_rather_than_guessing() {
    // What an artifact built before C1 carries: the block is absent, serde
    // fills in the default, and the report says nobody read the runtime
    // rather than printing a linkage it made up.
    let mut info = hand_built();
    info.manifest.otp = OtpProvenance::default();

    let text = info.render_text();

    assert!(
        text.contains("runtime:       unknown"),
        "an unrecorded linkage is `unknown`:\n{text}"
    );
    assert!(
        text.contains("runtime from:  unknown"),
        "and so is an unrecorded source:\n{text}"
    );
}
