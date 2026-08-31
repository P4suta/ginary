// SPDX-License-Identifier: MIT OR Apache-2.0
//! `catalog::pack_runtime` promised a zeroed mtime in four places and set none.
//!
//! Its own doc comment, `docs/adr/0013-local-first-otp-catalog.md`, the mise
//! task and `tests/common/catalog.rs` all said "sorted paths, zeroed mtime,
//! uid and gid". The code set `tar::HeaderMode::Deterministic` and stopped
//! there, and tar-rs deliberately does *not* zero the mtime in that mode — it
//! writes `DETERMINISTIC_TIMESTAMP`, 1153704088 — which `src/payload.rs`
//! already knew and worked around:
//!
//! ```rust
//! header.set_metadata_in_mode(&file.metadata()?, tar::HeaderMode::Deterministic);
//! header.set_mtime(0);
//! ```
//!
//! So the shipped `dist/otp/otp-29.0.5-linux-x86_64-musl-static.tar.zst` held
//! `-rwxr-xr-x 0/0 3561 2006-07-24 10:21 Install`: uid and gid zeroed as
//! promised, and a 2006 timestamp on every entry.
//!
//! Determinism and a zero mtime are two claims, so they are two assertions
//! here: two packs of one tree are byte-identical, *and* every header carries
//! mtime 0 with uid and gid 0.
#![cfg(feature = "cli")]

use std::io::Read;
use std::path::Path;

use ginary::catalog::{self, RepackOptions, RepackSelector};
use ginary::diag::Diag;
use ginary::download::Net;
use ginary::elf::{ElfInfo, ElfKind};

use crate::common::catalog::{FakeUpstream, UPSTREAM_TAG};

/// What the fixture emulator reads back as: a fully static x86-64 build.
fn static_x86_64() -> ElfInfo {
    ElfInfo {
        class: 64,
        kind: ElfKind::Executable,
        machine: "x86_64".to_owned(),
        interp: None,
        needed: Vec::new(),
        glibc_max: None,
        is_pie: false,
        stripped: true,
    }
}

/// Runs one repack of the fixture upstream asset into `out`.
fn repack_into(out: &Path, upstream_dir: &Path) -> std::path::PathBuf {
    let report = catalog::repack_with(
        &RepackOptions {
            upstream_tag: UPSTREAM_TAG.to_owned(),
            selectors: vec![RepackSelector {
                target: "linux-x86_64-musl".to_owned(),
                variant: "static".to_owned(),
            }],
            out: out.to_path_buf(),
            upstream_dir: Some(upstream_dir.to_path_buf()),
            source_date_epoch: Some(1_756_598_400),
        },
        &Net::offline(),
        &Diag::disabled(),
        |_| Ok(static_x86_64()),
    )
    .expect("a pre-downloaded asset needs no network");
    report.outcomes[0].tarball.clone()
}

/// Every entry of a `.tar.zst`, as (path, mtime, uid, gid, mode).
fn entries(tarball: &Path) -> Vec<(String, u64, u64, u64, u32)> {
    let file = std::fs::File::open(tarball).expect("the repacked tarball");
    let mut decoded = Vec::new();
    zstd::stream::read::Decoder::new(file)
        .expect("the tarball is one zstd stream")
        .read_to_end(&mut decoded)
        .expect("the stream decodes");

    let mut archive = tar::Archive::new(std::io::Cursor::new(decoded));
    archive
        .entries()
        .expect("the archive lists its entries")
        .map(|entry| {
            let entry = entry.expect("an entry");
            let header = entry.header();
            (
                entry.path().expect("a path").display().to_string(),
                header.mtime().expect("an mtime"),
                header.uid().expect("a uid"),
                header.gid().expect("a gid"),
                header.mode().expect("a mode"),
            )
        })
        .collect()
}

#[test]
fn every_entry_of_a_repacked_runtime_carries_a_zero_mtime_and_no_owner() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let upstream = FakeUpstream::build("erlang-29.0.5", &[]);
    let upstream_dir = dir.path().join("upstream");
    upstream.write_in(&upstream_dir, "erlang-29.0.5-x64.tar.gz");

    let tarball = repack_into(&dir.path().join("dist"), &upstream_dir);
    let entries = entries(&tarball);

    assert!(!entries.is_empty(), "the runtime is not an empty archive");
    for (path, mtime, uid, gid, mode) in &entries {
        assert_eq!(
            *mtime, 0,
            "`{path}` carries the build machine's clock, so two repacks of one upstream asset \
             are not the same bytes"
        );
        assert_eq!(*uid, 0, "`{path}` names an owner");
        assert_eq!(*gid, 0, "`{path}` names a group");
        assert!(
            *mode == 0o755 || *mode == 0o644,
            "`{path}` carries the umask it was unpacked under: {mode:o}"
        );
    }
}

#[test]
fn two_repacks_of_one_upstream_asset_are_the_same_bytes() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let upstream = FakeUpstream::build("erlang-29.0.5", &[]);
    let upstream_dir = dir.path().join("upstream");
    upstream.write_in(&upstream_dir, "erlang-29.0.5-x64.tar.gz");

    let first = std::fs::read(repack_into(&dir.path().join("one"), &upstream_dir))
        .expect("the first tarball");
    let second = std::fs::read(repack_into(&dir.path().join("two"), &upstream_dir))
        .expect("the second tarball");

    assert_eq!(
        first, second,
        "a repack is reproducible, or a catalogue digest means nothing"
    );
}
