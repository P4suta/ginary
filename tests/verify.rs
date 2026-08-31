// SPDX-License-Identifier: MIT OR Apache-2.0
//! The deep check of a packaged application.
//!
//! `ginary inspect --verify` re-hashes the payload against the trailer and
//! stops. Everything here is what that check cannot see: a file whose bytes no
//! longer match `ginary.index.json`, a file the index does not name, an index
//! row naming nothing, a native binary built for another machine, and one that
//! needs a library the artifact does not carry. A payload whose digest matches
//! can have every one of those, which is why the two commands are two
//! commands.
//!
//! It is also where the *kind* of a payload entry is checked, which is a rule
//! about position rather than about a name: `ginary.json` and
//! `ginary.index.json` are entries 0 and 1, a later entry on either is a
//! payload `payload::unpack` refuses, a directory entry is permitted and
//! carries nothing to check, and anything else is named for what it is. Those
//! four live in `tests/regressions/`, because each one was a defect.
//!
//! Three builders feed it. [`SyntheticArtifact`] is the clean artifact, whose
//! `erts-*/bin` programs are shell scripts and which therefore holds no ELF at
//! all. [`crate::common::repack`] lays the payload out entry by entry, so the
//! index can be made to disagree with the tree it describes, and copies this
//! test run's own binary in as a real object. The gated test at the end runs
//! the whole thing over a real `ginary build`.

mod common;

use std::path::Path;

use assert_cmd::Command;
use ginary::verify::{
    self, Issue, LOADER_PREFIX, NEEDED_ALLOWLIST, ObjectInfo, VERIFY_FORMAT_VERSION, VerifyOptions,
    VerifyReport,
};
use serde_json::Value;

use crate::common::artifact::{ArtifactOptions, SyntheticArtifact};
use crate::common::built::BuiltProject;
use crate::common::repack::{self, NATIVE_PATH, RepackOptions};
use crate::common::tools::require_tools;

/// A `Command` for the `ginary` binary.
fn ginary() -> Command {
    Command::cargo_bin("ginary").expect("the `ginary` binary is built for tests")
}

/// A temporary directory the test owns.
fn tempdir() -> tempfile::TempDir {
    tempfile::tempdir().expect("a temporary directory")
}

/// The issues of one report, as their rendered sentences.
fn sentences(report: &VerifyReport) -> Vec<String> {
    report.issues.iter().map(ToString::to_string).collect()
}

// ------------------------------------------------------- a clean file --

#[test]
fn a_clean_artifact_holds_no_objects_and_raises_nothing() {
    let dir = tempdir();
    let artifact = SyntheticArtifact::build(dir.path());

    let report = verify::verify(artifact.path()).expect("a whole artifact verifies");

    assert!(report.payload.ok(), "{:?}", report.payload);
    assert_eq!(report.issues, Vec::new());
    assert_eq!(
        report.objects,
        Vec::new(),
        "the synthetic runtime's programs are shell scripts, not ELF"
    );
    assert!(report.ok());
    assert_eq!(report.format_version, VERIFY_FORMAT_VERSION);
    assert_eq!(report.path, artifact.path().display().to_string());
}

#[test]
fn every_file_is_checked_against_the_index_and_not_only_the_native_ones() {
    let dir = tempdir();
    let artifact = SyntheticArtifact::build(dir.path());

    let report = verify::verify(artifact.path()).expect("a whole artifact verifies");

    let indexed = ginary::inspect::open(artifact.path())
        .expect("the artifact opens")
        .index
        .files
        .len();
    assert!(indexed > 0, "the synthetic tree has files");
    assert_eq!(
        report.files_checked, indexed,
        "verify is a full integrity check, not an ELF scan"
    );
}

#[test]
fn a_payload_that_no_longer_matches_the_trailer_is_not_ok() {
    let dir = tempdir();
    let artifact = SyntheticArtifact::build(dir.path());
    artifact.break_payload_tail();

    let report = verify::verify(artifact.path()).expect("a damaged payload is still reported");

    assert!(!report.payload.ok(), "{:?}", report.payload);
    assert_eq!(
        report.payload.expected,
        hex::encode(artifact.packed().sha256)
    );
    assert!(!report.ok());
}

// ---------------------------------------------------------- the index --

#[test]
fn a_file_whose_bytes_are_not_the_indexed_ones_is_named() {
    let dir = tempdir();
    let artifact = repack::build(
        dir.path(),
        &RepackOptions {
            corrupt: vec![(
                "lib/hello/priv/greeting.txt".to_owned(),
                b"goodbye, world\n".to_vec(),
            )],
            ..RepackOptions::default()
        },
    );

    let report = verify::verify(artifact.path()).expect("the artifact opens");

    assert!(
        report.payload.ok(),
        "the trailer describes the payload that was written: this is the defect the payload \
         hash cannot see"
    );
    let expected = artifact
        .index()
        .files
        .iter()
        .find(|file| file.path == "lib/hello/priv/greeting.txt")
        .expect("the file is indexed")
        .sha256
        .clone();
    let indexed_size = artifact
        .index()
        .files
        .iter()
        .find(|file| file.path == "lib/hello/priv/greeting.txt")
        .expect("the file is indexed")
        .size;
    // Two findings rather than one: the replacement bytes are two longer than
    // the ones the row was written from, so the row's length is wrong as well
    // as its digest, and each column is named on its own.
    assert_eq!(
        report.issues,
        vec![
            Issue::IndexMismatch {
                path: "lib/hello/priv/greeting.txt".to_owned(),
                expected,
                actual: hex::encode(<sha2::Sha256 as sha2::Digest>::digest(b"goodbye, world\n")),
            },
            Issue::IndexSizeMismatch {
                path: "lib/hello/priv/greeting.txt".to_owned(),
                expected: indexed_size,
                actual: b"goodbye, world\n".len() as u64,
            },
        ]
    );
    assert!(!report.ok());
}

#[test]
fn a_file_the_index_does_not_name_is_an_orphan() {
    let dir = tempdir();
    let artifact = repack::build(
        dir.path(),
        &RepackOptions {
            drop_from_index: vec!["lib/hello/priv/greeting.txt".to_owned()],
            ..RepackOptions::default()
        },
    );

    let report = verify::verify(artifact.path()).expect("the artifact opens");

    assert_eq!(
        report.issues,
        vec![Issue::IndexOrphan {
            path: "lib/hello/priv/greeting.txt".to_owned(),
        }]
    );
}

#[test]
fn an_index_row_the_payload_does_not_carry_is_missing() {
    let dir = tempdir();
    let artifact = repack::build(
        dir.path(),
        &RepackOptions {
            ghost_index_rows: vec!["lib/hello/ebin/never_packed.beam".to_owned()],
            ..RepackOptions::default()
        },
    );

    let report = verify::verify(artifact.path()).expect("the artifact opens");

    assert_eq!(
        report.issues,
        vec![Issue::IndexMissing {
            path: "lib/hello/ebin/never_packed.beam".to_owned(),
        }]
    );
}

// -------------------------------------------------------- the objects --

#[test]
fn a_real_elf_in_the_payload_is_listed_with_what_it_needs() {
    let dir = tempdir();
    let artifact = repack::build(
        dir.path(),
        &RepackOptions {
            artifact: repack::with_native_object(),
            ..RepackOptions::default()
        },
    );
    let host = ginary::elf::inspect_bytes(&repack::test_binary()).expect("the test binary is ELF");

    let report = verify::verify(artifact.path()).expect("the artifact opens");

    assert_eq!(
        report.objects,
        vec![ObjectInfo {
            path: NATIVE_PATH.to_owned(),
            machine: host.machine.clone(),
            class: host.class,
            kind: host.kind,
            interp: host.interp.clone(),
            needed: host.needed.clone(),
            glibc_max: host.glibc_max.clone(),
            issues: Vec::new(),
        }]
    );
    assert!(report.ok(), "{:?}", report.issues);
}

#[test]
fn an_object_built_for_another_machine_is_a_mismatch() {
    let dir = tempdir();
    let foreign = repack::foreign_target();
    let artifact = repack::build(
        dir.path(),
        &RepackOptions {
            artifact: repack::with_native_object(),
            target: Some(foreign),
            ..RepackOptions::default()
        },
    );

    let report = verify::verify(artifact.path()).expect("the artifact opens");

    assert_eq!(
        report.issues,
        vec![Issue::MachineMismatch {
            path: NATIVE_PATH.to_owned(),
            found: ginary::target::Target::host().arch.as_str().to_owned(),
            expected: foreign.arch.as_str().to_owned(),
        }]
    );
    assert_eq!(report.objects.len(), 1);
    assert_eq!(report.objects[0].issues, report.issues);
}

#[test]
fn the_allowlist_is_the_documented_set_and_matches_the_loader_by_prefix() {
    assert_eq!(
        NEEDED_ALLOWLIST,
        [
            "libc.so.6",
            "libm.so.6",
            "libpthread.so.0",
            "libdl.so.2",
            "librt.so.1",
            "libgcc_s.so.1",
            "libstdc++.so.6",
            "libtinfo.so.6",
        ]
    );
    assert_eq!(LOADER_PREFIX, "ld-linux-");

    for allowed in [
        "libc.so.6",
        "libgcc_s.so.1",
        "ld-linux-x86-64.so.2",
        "ld-linux-aarch64.so.1",
    ] {
        assert!(
            verify::needed_is_allowed(allowed, &NEEDED_ALLOWLIST),
            "{allowed} is part of a glibc system"
        );
    }
    for refused in ["libssl.so.3", "libsqlite3.so.0", "libc.so", "ld-musl.so.1"] {
        assert!(
            !verify::needed_is_allowed(refused, &NEEDED_ALLOWLIST),
            "{refused} is not something an artifact may assume"
        );
    }
}

#[test]
fn the_default_allowlist_raises_nothing_for_a_binary_this_toolchain_linked() {
    let dir = tempdir();
    let artifact = repack::build(
        dir.path(),
        &RepackOptions {
            artifact: repack::with_native_object(),
            ..RepackOptions::default()
        },
    );

    let report = verify::verify(artifact.path()).expect("the artifact opens");

    assert!(
        !report
            .issues
            .iter()
            .any(|issue| matches!(issue, Issue::UnexpectedNeeded { .. })),
        "{:?}",
        report.issues
    );
}

#[test]
fn a_needed_outside_the_allowlist_is_reported() {
    let dir = tempdir();
    let artifact = repack::build(
        dir.path(),
        &RepackOptions {
            artifact: repack::with_native_object(),
            ..RepackOptions::default()
        },
    );
    let host = ginary::elf::inspect_bytes(&repack::test_binary()).expect("the test binary is ELF");
    assert!(
        !host.needed.is_empty(),
        "this test needs a dynamically linked test binary"
    );

    // The narrowed allowlist is the seam: every ELF a test can produce on the
    // machine it runs on links against libraries that are on the real list, so
    // an implementation that accepted everything and a correct one would agree.
    let report = verify::verify_with(
        artifact.path(),
        &VerifyOptions {
            allowlist: &[],
            ..VerifyOptions::default()
        },
    )
    .expect("the artifact opens");

    assert_eq!(
        report.issues,
        host.needed
            .iter()
            .map(|needed| Issue::UnexpectedNeeded {
                path: NATIVE_PATH.to_owned(),
                needed: needed.clone(),
            })
            .collect::<Vec<_>>()
    );
}

#[test]
fn a_file_that_begins_with_the_elf_magic_and_is_not_one_is_named() {
    // Not skipped: a file that looks like native code and does not parse as
    // native code is a damaged artifact or a hostile one, and either way the
    // reader decides.
    let dir = tempdir();
    let mut bogus = ginary::elf::ELF_MAGIC.to_vec();
    bogus.extend_from_slice(b"not really an object at all\n");
    let artifact = repack::build(
        dir.path(),
        &RepackOptions {
            artifact: ArtifactOptions {
                extra_files: vec![(
                    "lib/hello/priv/lib/broken.so".to_owned(),
                    0o644,
                    bogus,
                    ginary::assemble::Category::Priv,
                )],
                ..ArtifactOptions::default()
            },
            ..RepackOptions::default()
        },
    );

    let report = verify::verify(artifact.path()).expect("the artifact opens");

    assert_eq!(
        report.objects,
        Vec::new(),
        "there is nothing an object row could say about it"
    );
    assert!(
        report.issues.iter().any(|issue| matches!(
            issue,
            Issue::UnreadableObject { path, .. } if path == "lib/hello/priv/lib/broken.so"
        )),
        "{:#?}",
        report.issues
    );
    assert!(!report.ok());
}

#[test]
fn an_object_larger_than_the_bound_is_reported_rather_than_read() {
    // The bound is injected for the reason the allowlist is: the real one is a
    // hundred megabytes, and a test that packed that much would be a test
    // nobody runs.
    let dir = tempdir();
    let artifact = repack::build(
        dir.path(),
        &RepackOptions {
            artifact: repack::with_native_object(),
            ..RepackOptions::default()
        },
    );

    let report = verify::verify_with(
        artifact.path(),
        &VerifyOptions {
            max_object_bytes: 64,
            ..VerifyOptions::default()
        },
    )
    .expect("the artifact opens");

    assert_eq!(
        report.objects,
        Vec::new(),
        "nothing this large is read into memory to be described"
    );
    assert!(
        report.issues.iter().any(|issue| matches!(
            issue,
            Issue::UnreadableObject { path, message }
                if path == NATIVE_PATH && message.contains("64")
        )),
        "{:#?}",
        report.issues
    );
    assert!(
        !report
            .issues
            .iter()
            .any(|issue| matches!(issue, Issue::IndexMismatch { .. })),
        "the entry is still hashed against the index on the way past: {:#?}",
        report.issues
    );
}

#[test]
fn the_issue_table_is_this() {
    let report = VerifyReport {
        format_version: VERIFY_FORMAT_VERSION,
        path: "/w/hello".to_owned(),
        payload: ginary::inspect::Verification {
            expected: "a".repeat(64),
            actual: "a".repeat(64),
        },
        files_checked: 9,
        objects: vec![ObjectInfo {
            path: NATIVE_PATH.to_owned(),
            machine: "x86_64".to_owned(),
            class: 64,
            kind: ginary::elf::ElfKind::SharedObject,
            interp: Some("/lib64/ld-linux-x86-64.so.2".to_owned()),
            needed: vec!["libc.so.6".to_owned(), "libssl.so.3".to_owned()],
            glibc_max: Some("2.38".to_owned()),
            issues: vec![Issue::UnexpectedNeeded {
                path: NATIVE_PATH.to_owned(),
                needed: "libssl.so.3".to_owned(),
            }],
        }],
        issues: vec![
            Issue::MachineMismatch {
                path: "lib/hello/priv/lib/nif.so".to_owned(),
                found: "aarch64".to_owned(),
                expected: "x86_64".to_owned(),
            },
            Issue::UnexpectedNeeded {
                path: NATIVE_PATH.to_owned(),
                needed: "libssl.so.3".to_owned(),
            },
            Issue::IndexMismatch {
                path: "lib/hello/priv/greeting.txt".to_owned(),
                expected: "b".repeat(64),
                actual: "c".repeat(64),
            },
            Issue::IndexOrphan {
                path: "lib/hello/ebin/extra.beam".to_owned(),
            },
            Issue::IndexMissing {
                path: "lib/hello/ebin/gone.beam".to_owned(),
            },
        ],
    };

    insta::assert_snapshot!("verify_issue_table", report.render_text());
}

// -------------------------------------------------------- the command --

#[test]
fn verify_exits_zero_and_says_so_for_a_clean_artifact() {
    let dir = tempdir();
    let artifact = SyntheticArtifact::build(dir.path());

    let assert = ginary()
        .arg("verify")
        .arg(artifact.path())
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8");

    assert!(stdout.contains("payload:"), "{stdout}");
    assert!(
        !stdout.contains("issues:"),
        "a clean artifact has no block:\n{stdout}"
    );
}

#[test]
fn verify_exits_one_and_names_the_file_that_does_not_match() {
    let dir = tempdir();
    let artifact = repack::build(
        dir.path(),
        &RepackOptions {
            corrupt: vec![(
                "lib/hello/priv/greeting.txt".to_owned(),
                b"goodbye, world\n".to_vec(),
            )],
            ..RepackOptions::default()
        },
    );

    let assert = ginary().arg("verify").arg(artifact.path()).assert().code(1);
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8");

    assert!(stdout.contains("issues:"), "{stdout}");
    assert!(stdout.contains("lib/hello/priv/greeting.txt"), "{stdout}");
    assert!(stdout.contains("the index says"), "{stdout}");
}

#[test]
fn inspect_verify_stays_the_payload_hash_alone() {
    let dir = tempdir();
    let artifact = repack::build(
        dir.path(),
        &RepackOptions {
            corrupt: vec![(
                "lib/hello/priv/greeting.txt".to_owned(),
                b"goodbye, world\n".to_vec(),
            )],
            ..RepackOptions::default()
        },
    );

    // The cheap check passes: the payload is the one the trailer describes.
    let assert = ginary()
        .args(["inspect", "--verify"])
        .arg(artifact.path())
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8");
    assert!(stdout.contains("verify: ok"), "{stdout}");

    // The deep one does not, and it is the one that names the file.
    let deep = ginary().arg("verify").arg(artifact.path()).assert().code(1);
    let deep_stdout = String::from_utf8(deep.get_output().stdout.clone()).expect("utf-8");
    assert!(
        deep_stdout.contains("lib/hello/priv/greeting.txt"),
        "{deep_stdout}"
    );
}

#[test]
fn verify_json_carries_the_documented_keys() {
    let dir = tempdir();
    let artifact = repack::build(
        dir.path(),
        &RepackOptions {
            artifact: repack::with_native_object(),
            ..RepackOptions::default()
        },
    );

    let assert = ginary()
        .args(["verify", "--json"])
        .arg(artifact.path())
        .assert()
        .success();
    let value: Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("the output is JSON");

    assert_eq!(value["format_version"], VERIFY_FORMAT_VERSION);
    assert_eq!(value["payload"]["expected"], value["payload"]["actual"]);
    assert_eq!(value["issues"].as_array().expect("issues").len(), 0);
    let objects = value["objects"].as_array().expect("objects");
    assert_eq!(objects.len(), 1);
    assert_eq!(objects[0]["path"], NATIVE_PATH);
    assert!(objects[0]["needed"].is_array(), "{}", objects[0]);
}

#[test]
fn verify_json_names_the_issue_kind() {
    let dir = tempdir();
    let artifact = repack::build(
        dir.path(),
        &RepackOptions {
            drop_from_index: vec!["lib/hello/priv/greeting.txt".to_owned()],
            ..RepackOptions::default()
        },
    );

    let assert = ginary()
        .args(["verify", "--json"])
        .arg(artifact.path())
        .assert()
        .code(1);
    let value: Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("the output is JSON");

    assert_eq!(value["issues"][0]["issue"], "index_orphan");
    assert_eq!(value["issues"][0]["path"], "lib/hello/priv/greeting.txt");
}

// ---------------------------------------------------- a real artifact --

#[test]
fn a_real_artifact_verifies_clean() {
    let Some(_tools) = require_tools(&["gleam", "erl", "strip"]) else {
        return;
    };
    let project = BuiltProject::copy("hello_ffi");
    let output = project.build();
    assert!(
        output.status.success(),
        "the build failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let report = verify::verify(&project.artifact()).expect("a real artifact verifies");

    assert!(report.payload.ok(), "{:?}", report.payload);
    assert_eq!(sentences(&report), Vec::<String>::new());
    assert!(
        report.objects.len() > 1,
        "a real runtime carries beam.smp and its friends: {:?}",
        report
            .objects
            .iter()
            .map(|object| object.path.as_str())
            .collect::<Vec<_>>()
    );
    assert!(
        report
            .objects
            .iter()
            .any(|object| object.path.ends_with("/beam.smp")),
        "{:?}",
        report.objects
    );
    assert!(report.ok());
}

#[test]
fn a_shipment_named_by_the_environment_verifies_clean() {
    let Some(path) = std::env::var_os("GINARY_TEST_ARTIFACT").map(std::path::PathBuf::from) else {
        eprintln!("skipping: GINARY_TEST_ARTIFACT is not set");
        return;
    };
    assert!(
        Path::new(&path).is_file(),
        "GINARY_TEST_ARTIFACT names {}, which is not a file",
        path.display()
    );

    let report = verify::verify(&path).expect("the named artifact verifies");

    assert_eq!(sentences(&report), Vec::<String>::new());
    assert!(report.ok());
}
