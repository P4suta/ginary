// SPDX-License-Identifier: MIT OR Apache-2.0
//! Verification must reject artifacts the target cannot execute or extract.
use crate::common::artifact::SyntheticArtifact;
use crate::common::native;
use crate::common::repack::{self, AppendedEntry, RepackOptions};

#[test]
fn a_same_architecture_object_for_another_os_is_not_portable() {
    for bytes in [
        native::pe_bytes(0x8664, true),
        native::macho_bytes(native::MACHO_CPU_X86_64, native::MACHO_TYPE_DYLIB),
    ] {
        let dir = tempfile::tempdir().expect("tempdir");
        let artifact = repack::build(
            dir.path(),
            &RepackOptions {
                target: Some("linux-x86_64-gnu".parse().expect("target")),
                appended: vec![AppendedEntry::file("lib/hello/priv/foreign", &bytes).indexed()],
                ..Default::default()
            },
        );
        let report = ginary::verify::verify(artifact.path()).expect("read artifact");
        assert!(
            !report.ok(),
            "same architecture cannot make another OS's binary portable: {report:?}"
        );
        assert!(
            report
                .issues
                .iter()
                .any(|issue| issue.to_string().contains("format")),
            "{report:?}"
        );
    }
}

#[test]
fn a_file_and_its_descendant_conflict_in_either_order() {
    for reverse in [false, true] {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut appended = vec![
            AppendedEntry::file("extra", b"parent").indexed(),
            AppendedEntry::file("extra/child", b"child").indexed(),
        ];
        if reverse {
            appended.reverse();
        }
        let artifact = repack::build(
            dir.path(),
            &RepackOptions {
                appended,
                ..Default::default()
            },
        );
        let report = ginary::verify::verify(artifact.path()).expect("read artifact");
        assert!(
            !report.ok(),
            "a file cannot be a parent directory: {report:?}"
        );
    }
}

#[test]
fn an_explicit_directory_cannot_replace_a_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = repack::build(
        dir.path(),
        &RepackOptions {
            appended: vec![
                AppendedEntry::file("extra", b"file").indexed(),
                AppendedEntry::directory("extra"),
            ],
            ..Default::default()
        },
    );
    let report = ginary::verify::verify(artifact.path()).expect("read artifact");
    assert!(!report.ok(), "{report:?}");
}

#[test]
fn repeated_index_rows_are_diagnosed_before_the_map_can_collapse_them() {
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = repack::build(
        dir.path(),
        &RepackOptions {
            ghost_index_rows: vec!["lib/hello/priv/greeting.txt".into()],
            ..Default::default()
        },
    );
    let report = ginary::verify::verify(artifact.path()).expect("read artifact");
    assert!(
        report
            .issues
            .iter()
            .any(|issue| issue.to_string().contains("duplicate index")),
        "{report:?}"
    );
    assert_unpack_refuses(artifact.path(), dir.path());
}

fn assert_unpack_refuses(artifact: &std::path::Path, directory: &std::path::Path) {
    use std::io::{Seek, SeekFrom};
    let info = ginary::inspect::open(artifact).expect("front matter");
    let mut file = std::fs::File::open(artifact).expect("artifact");
    file.seek(SeekFrom::Start(info.trailer.payload_offset))
        .expect("payload offset");
    let result = ginary::payload::unpack(
        file,
        info.payload_len,
        &info.trailer.payload_sha256,
        &directory.join("extract-check"),
    );
    assert!(
        result.is_err(),
        "verification and extraction must both refuse this payload"
    );
}

#[test]
fn corrupt_payload_is_reported_as_integrity_failure_not_zero_issues() {
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = SyntheticArtifact::build(dir.path());
    artifact.break_payload_tail();
    let output = assert_cmd::Command::cargo_bin("ginary")
        .expect("binary")
        .arg("verify")
        .arg(artifact.path())
        .assert()
        .code(1)
        .get_output()
        .clone();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("0 issue(s)"), "{stderr}");
    assert!(stderr.contains("payload"), "{stderr}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("not run"),
        "library/text report must name the unperformed checks: {stdout}"
    );
}

#[test]
fn verification_json_distinguishes_integrity_failure_from_unperformed_contents() {
    let dir = tempfile::tempdir().unwrap();
    let artifact = SyntheticArtifact::build(dir.path());
    for damaged in [false, true] {
        if damaged {
            artifact.break_payload_tail();
        }
        let output = assert_cmd::Command::cargo_bin("ginary")
            .unwrap()
            .arg("verify")
            .arg(artifact.path())
            .arg("--json")
            .output()
            .unwrap();
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(output.status.success(), !damaged);
        assert_eq!(
            report["checks"]["integrity"],
            if damaged { "failed" } else { "passed" }
        );
        assert_eq!(
            report["checks"]["contents"],
            if damaged { "not_run" } else { "passed" }
        );
    }
}

#[test]
fn an_unreadable_artifact_still_produces_json_explaining_the_incomplete_checks() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("broken");
    std::fs::write(&path, b"not a ginary artifact").unwrap();
    let output = assert_cmd::Command::cargo_bin("ginary")
        .unwrap()
        .arg("verify")
        .arg(&path)
        .arg("--json")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout)
        .expect("an unreadable artifact needs a machine-readable failure report");
    assert_eq!(report["format_version"], 2);
    assert_eq!(report["checks"]["integrity"], "incomplete");
    assert_eq!(report["checks"]["contents"], "not_run");
    assert!(report["error"].is_string());
    assert!(
        report["issues"].is_null(),
        "unknown issues must not be reported as an empty list"
    );
}

#[test]
fn a_valid_digest_does_not_hide_an_incomplete_archive_scan() {
    use ginary::verify::{CheckOutcome, VerificationChecks};
    use sha2::Digest;

    let dir = tempfile::tempdir().unwrap();
    let artifact = SyntheticArtifact::build(dir.path());
    let info = ginary::inspect::open(artifact.path()).unwrap();
    let mut bytes = std::fs::read(artifact.path()).unwrap();
    let offset = info.trailer.payload_offset as usize;
    let end = offset + info.trailer.payload_len as usize;
    let mut tar = zstd::stream::decode_all(&bytes[offset..end]).unwrap();
    // Leave the two front-matter entries intact, then invalidate the first
    // content header's checksum. The recomputed outer digest still passes.
    let mut header_offset = 0;
    for _ in 0..2 {
        let header = tar::Header::from_byte_slice(&tar[header_offset..header_offset + 512]);
        header_offset += 512 + (header.size().unwrap() as usize).div_ceil(512) * 512;
    }
    tar[header_offset] ^= 1;
    let payload = zstd::stream::encode_all(tar.as_slice(), 1).unwrap();
    let trailer = ginary::trailer::Trailer {
        payload_offset: offset as u64,
        payload_len: payload.len() as u64,
        payload_sha256: sha2::Sha256::digest(&payload).into(),
    };
    bytes.truncate(offset);
    bytes.extend_from_slice(&payload);
    bytes.extend_from_slice(&trailer.to_bytes());
    std::fs::write(artifact.path(), bytes).unwrap();

    let error = ginary::verify::verify_detailed(artifact.path()).unwrap_err();
    assert_eq!(
        error.checks(),
        VerificationChecks {
            integrity: CheckOutcome::Passed,
            contents: CheckOutcome::Incomplete,
        }
    );
    let output = assert_cmd::Command::cargo_bin("ginary")
        .unwrap()
        .arg("verify")
        .arg(artifact.path())
        .arg("--json")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["checks"]["integrity"], "passed");
    assert_eq!(report["checks"]["contents"], "incomplete");
    assert!(report["issues"].is_null());
    assert!(!report["causes"].as_array().unwrap().is_empty());
    let diagnosis = ginary::diagnose::summarize(Some(artifact.path()), None, None);
    assert!(!diagnosis.complete);
    let document = serde_json::to_value(&diagnosis).unwrap();
    assert_eq!(document["artifact"]["checks"]["contents"], "incomplete");
    assert_eq!(document["artifact"]["payload_ok"], true);
    assert!(document["artifact"]["issues"].is_null());
}

#[test]
fn diagnosis_does_not_report_unknown_contents_as_zero_findings() {
    let dir = tempfile::tempdir().unwrap();
    let artifact = SyntheticArtifact::build(dir.path());
    for stage in ["passed", "failed", "incomplete"] {
        if stage == "failed" {
            artifact.break_payload_tail();
        }
        if stage == "incomplete" {
            std::fs::write(artifact.path(), b"broken front matter").unwrap();
        }
        let diagnosis = ginary::diagnose::summarize(Some(artifact.path()), None, None);
        let document = serde_json::to_value(&diagnosis).unwrap();
        assert_eq!(document["artifact"]["checks"]["integrity"], stage);
        assert_eq!(diagnosis.complete, stage == "passed");
        if stage == "passed" {
            assert_eq!(document["artifact"]["issues"], 0);
        } else {
            assert!(document["artifact"]["issues"].is_null());
            let text = diagnosis.render_text();
            assert!(text.contains("not run"), "{text}");
            assert!(!text.contains("0 issues"), "{text}");
        }
    }
}

#[test]
fn windows_destinations_cannot_hide_collisions_with_different_case() {
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = repack::build(
        dir.path(),
        &RepackOptions {
            target: Some("windows-x86_64".parse().unwrap()),
            appended: vec![
                AppendedEntry::file("Extra", b"one").indexed(),
                AppendedEntry::file("extra", b"two").indexed(),
            ],
            ..Default::default()
        },
    );
    let report = ginary::verify::verify(artifact.path()).unwrap();
    assert!(
        report
            .issues
            .iter()
            .any(|issue| matches!(issue, ginary::verify::Issue::DestinationConflict { .. })),
        "{report:?}"
    );
}

#[test]
fn windows_reserved_front_matter_is_case_insensitive() {
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = repack::build(
        dir.path(),
        &RepackOptions {
            target: Some("windows-x86_64".parse().unwrap()),
            appended: vec![AppendedEntry::file("GINARY.JSON/child", b"one").indexed()],
            ..Default::default()
        },
    );
    let report = ginary::verify::verify(artifact.path()).unwrap();
    assert!(
        report
            .issues
            .iter()
            .any(|issue| matches!(issue, ginary::verify::Issue::ReservedEntry { .. })),
        "{report:?}"
    );
}
