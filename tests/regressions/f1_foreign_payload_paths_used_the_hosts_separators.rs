// SPDX-License-Identifier: MIT OR Apache-2.0
//! A foreign target's filename must not turn into another path on the reader.

use std::path::Path;

use crate::common::payload::{RawEntry, RawTar, sample_manifest, sha256, sha256_hex};
use ginary::manifest::{INDEX_NAME, Index, IndexFile, MANIFEST_NAME};
use ginary::target::Os;

#[test]
fn unix_target_names_keep_literal_backslashes_and_colons_on_every_host() {
    for os in [Os::Linux, Os::Macos] {
        for name in [r"lib/data\nested.txt", r"a\..\b", "C:relative", r"\rooted"] {
            assert_eq!(
                ginary::payload::destined_path_for(Path::new(name), os),
                Some(name.to_owned()),
                "{os} filename was interpreted using host separators: {name}"
            );
        }
        assert_eq!(
            ginary::payload::destined_path_for(Path::new("./lib//data/./file"), os),
            Some("lib/data/file".into())
        );
        for name in ["/rooted", "../escape", "lib/../escape", ".", ""] {
            assert_eq!(
                ginary::payload::destined_path_for(Path::new(name), os),
                None
            );
        }
    }
}

#[test]
fn windows_target_names_keep_their_target_rules_on_every_host() {
    assert_eq!(
        ginary::payload::destined_path_for(Path::new(r"LIB\data.txt"), Os::Windows),
        Some("lib/data.txt".into())
    );
    for name in [
        r"lib\..\escape",
        "C:relative",
        "lib/AUX.txt",
        "lib/trailing.",
    ] {
        assert_eq!(
            ginary::payload::destined_path_for(Path::new(name), Os::Windows),
            None,
            "{name}"
        );
    }
}

#[test]
fn legacy_destination_api_still_uses_native_path_components() {
    let expected = if cfg!(windows) {
        "lib/data/file"
    } else {
        r"lib/data\file"
    };
    assert_eq!(
        ginary::payload::destined_path(Path::new(r"lib/data\file")),
        Some(expected.into())
    );
}

#[cfg(feature = "cli")]
#[test]
fn verification_does_not_match_a_unix_backslash_name_to_a_slash_name() {
    let dir = tempfile::tempdir().expect("tempdir");
    let payload = raw_payload("lib/data/nested.txt", r"lib/data\nested.txt", Os::Linux);
    let artifact = dir.path().join("artifact");
    let trailer = ginary::trailer::Trailer {
        payload_offset: 0,
        payload_len: payload.len() as u64,
        payload_sha256: sha256(&payload),
    };
    let mut bytes = payload;
    bytes.extend_from_slice(&trailer.to_bytes());
    std::fs::write(&artifact, bytes).expect("artifact");
    let report = ginary::verify::verify(&artifact).expect("verify foreign artifact");
    assert!(
        report.issues.iter().any(|issue| matches!(issue, ginary::verify::Issue::IndexOrphan { path } if path == r"lib/data\nested.txt")),
        "verification silently matched a different Unix filename: {report:?}"
    );
    assert!(report.issues.iter().any(|issue| matches!(issue, ginary::verify::Issue::IndexMissing { path } if path == "lib/data/nested.txt")));
}

#[test]
fn extraction_refuses_target_components_the_host_would_reinterpret() {
    let target = if cfg!(windows) {
        Os::Linux
    } else {
        Os::Windows
    };
    let name = r"lib/data\nested.txt";
    let payload = raw_payload(name, name, target);
    let dir = tempfile::tempdir().expect("tempdir");
    let result = ginary::payload::unpack(
        payload.as_slice(),
        payload.len() as u64,
        &sha256(&payload),
        dir.path(),
    );
    assert!(
        matches!(
            result,
            Err(ginary::payload::PayloadError::UnsafePath { .. })
        ),
        "foreign extraction must refuse a filename the host would reinterpret: {result:?}"
    );
    assert!(!dir.path().join("lib/data/nested.txt").exists());
    assert!(
        !dir.path().join(MANIFEST_NAME).exists(),
        "failed extraction published a cache completion marker"
    );
}

#[cfg(windows)]
#[test]
fn foreign_unix_index_cannot_alias_two_host_destinations() {
    for names in [["Extra", "extra"], ["Extra/a", "extra/b"]] {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut manifest = sample_manifest();
        manifest.native.clear();
        let index = Index {
            files: names.into_iter().map(index_row).collect(),
        };
        let payload = RawTar::new()
            .push(RawEntry::file(
                MANIFEST_NAME,
                &serde_json::to_vec(&manifest).expect("manifest"),
            ))
            .push(RawEntry::file(
                INDEX_NAME,
                &serde_json::to_vec(&index).expect("index"),
            ))
            .push(RawEntry::file(names[0], b"data"))
            .push(RawEntry::file(names[1], b"data"))
            .build_zstd(1);
        let result = ginary::payload::unpack(
            payload.as_slice(),
            payload.len() as u64,
            &sha256(&payload),
            dir.path(),
        );
        assert!(
            matches!(
                result,
                Err(ginary::payload::PayloadError::DestinationConflict { .. })
            ),
            "host alias must be diagnosed before extraction: {result:?}"
        );
        assert!(!dir.path().join("Extra").exists());
        assert!(!dir.path().join(MANIFEST_NAME).exists());
    }
}

fn raw_payload(index_name: &str, tar_name: &str, os: Os) -> Vec<u8> {
    let mut manifest = sample_manifest();
    manifest.native.clear();
    manifest.target = match os {
        Os::Linux => "linux-x86_64-gnu",
        Os::Macos => "macos-x86_64",
        Os::Windows => "windows-x86_64",
    }
    .parse()
    .expect("target");
    let index = Index {
        files: vec![index_row(index_name)],
    };
    RawTar::new()
        .push(RawEntry::file(
            MANIFEST_NAME,
            &serde_json::to_vec(&manifest).expect("manifest"),
        ))
        .push(RawEntry::file(
            INDEX_NAME,
            &serde_json::to_vec(&index).expect("index"),
        ))
        .push(RawEntry::file(tar_name, b"data"))
        .build_zstd(1)
}

fn index_row(path: &str) -> IndexFile {
    IndexFile {
        path: path.into(),
        size: 4,
        mode: 0o644,
        sha256: sha256_hex(b"data"),
        category: ginary::assemble::Category::Other,
    }
}
