// SPDX-License-Identifier: MIT OR Apache-2.0
//! Writing a macOS artifact: the `__GINARY,__payload` section, and the
//! ad-hoc signature over it.
//!
//! There is no macOS toolchain on this host and no way to launch what this
//! produces here — see `docs/dev/log/D3.md` for exactly what only a macOS CI
//! runner can confirm. What *is* checkable on Linux, and what this file
//! checks, is structural: the section lands at the offset and size
//! `src/macho.rs` itself reports back, an `LC_CODE_SIGNATURE` load command
//! is present exactly when signing was asked for, and
//! [`ginary::payload::locate`] round-trips the exact bytes and digest that
//! went in. Per the plan, the committed real Mach-O fixture
//! (`tests/fixtures/macho/`) stands in for a darwin stub here: this is the
//! real coverage for `inject_and_sign`, since no genuine darwin stub can be
//! built on this machine.
// The command line half of the suite: `sign_macos` is a `cli`-gated module.
#![cfg(feature = "cli")]

mod common;

use common::macho::{
    CPU_TYPE_X86_64, MH_EXECUTE, fat_header, real_fixture_bytes, thin_header, with_payload_section,
};
use ginary::macho::{self, MachoError};
use ginary::payload::{PayloadVia, locate};
use ginary::sign_macos::{CodeSign, MacSignCfg, SignMacosError, inject_and_sign};
use ginary::trailer::{TRAILER_LEN, Trailer};

/// A digest whose bytes are all different, so a slice taken in the wrong
/// place is visible rather than accidentally right.
const DIGEST: [u8; 32] = [
    0x2a, 0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee,
    0xff, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
];

/// What `bundle.rs`'s darwin arm hands `inject_and_sign`: the 64-byte
/// trailer, `payload_offset` relative to the section it will end up inside,
/// immediately followed by the payload bytes.
fn payload_with_trailer(payload: &[u8], sha256: [u8; 32]) -> Vec<u8> {
    let mut bytes = Trailer {
        payload_offset: TRAILER_LEN,
        payload_len: payload.len() as u64,
        payload_sha256: sha256,
    }
    .to_bytes()
    .to_vec();
    bytes.extend_from_slice(payload);
    bytes
}

/// Opens `bytes` as a real file, for [`locate`], which reads a [`std::fs::File`].
fn open(bytes: &[u8]) -> (std::fs::File, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("out");
    std::fs::write(&path, bytes).expect("write");
    (std::fs::File::open(&path).expect("open"), dir)
}

#[test]
fn inject_and_sign_writes_a_section_that_locate_round_trips_unsigned() {
    let stub = real_fixture_bytes();
    let payload = b"the payload bytes a macOS artifact carries, unsigned run";
    let with_trailer = payload_with_trailer(payload, DIGEST);
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().join("artifact");

    let report = inject_and_sign(
        &stub,
        &with_trailer,
        &out,
        &MacSignCfg {
            codesign: CodeSign::None,
        },
    )
    .expect("injecting into a real, valid thin Mach-O stub succeeds");

    assert_eq!(report.cputype, "arm64");
    assert!(!report.signed, "CodeSign::None must not sign");
    assert_eq!(report.section_size, TRAILER_LEN + payload.len() as u64);

    let written = std::fs::read(&out).expect("the output was written");
    let facts = macho::read(&written).expect("the output is still a whole Mach-O");
    assert!(
        facts
            .sections
            .iter()
            .any(|(seg, sect, offset, size)| seg == "__GINARY"
                && sect == "__payload"
                && *offset == report.section_offset
                && *size == report.section_size),
        "expected a (__GINARY, __payload, {}, {}) section among {:?}",
        report.section_offset,
        report.section_size,
        facts.sections
    );
    assert!(
        !facts.has_code_signature,
        "no LC_CODE_SIGNATURE was asked for"
    );

    let (file, _dir2) = open(&written);
    let found = locate(&file)
        .expect("the written artifact locates")
        .expect("its payload is there");
    assert_eq!(found.via, PayloadVia::MachOSection);
    assert_eq!(found.len, payload.len() as u64);
    assert_eq!(found.sha256, DIGEST);
    assert_eq!(
        &written[found.offset as usize..(found.offset + found.len) as usize],
        payload,
        "locate's offset must point at the exact payload bytes that were injected"
    );
}

#[test]
fn inject_and_sign_applies_an_adhoc_signature_when_asked() {
    let stub = real_fixture_bytes();
    let payload = b"the payload bytes a macOS artifact carries, signed run";
    let with_trailer = payload_with_trailer(payload, DIGEST);
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().join("artifact");

    let report = inject_and_sign(
        &stub,
        &with_trailer,
        &out,
        &MacSignCfg {
            codesign: CodeSign::Adhoc,
        },
    )
    .expect("injecting and ad-hoc signing a real, valid thin Mach-O stub succeeds");

    assert!(report.signed, "CodeSign::Adhoc must sign");

    let written = std::fs::read(&out).expect("the output was written");
    let facts = macho::read(&written).expect("the signed output is still a whole Mach-O");
    assert!(
        facts.has_code_signature,
        "an ad-hoc signature adds an LC_CODE_SIGNATURE load command"
    );

    let (file, _dir2) = open(&written);
    let found = locate(&file)
        .expect("the signed artifact still locates")
        .expect("its payload is there");
    assert_eq!(found.sha256, DIGEST);
}

#[test]
fn inject_and_sign_refuses_a_fat_stub() {
    let stub = fat_header(&[(0x0100_0007, 0), (0x0100_000c, 0)]);
    let payload = payload_with_trailer(b"anything", DIGEST);
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().join("artifact");

    let error = inject_and_sign(
        &stub,
        &payload,
        &out,
        &MacSignCfg {
            codesign: CodeSign::None,
        },
    )
    .expect_err("a fat Mach-O has no single architecture to inject into");

    assert!(
        matches!(error, SignMacosError::Fat),
        "expected SignMacosError::Fat, got {error:?}"
    );
}

#[test]
fn inject_and_sign_refuses_a_stub_that_is_not_a_macho_at_all() {
    let stub = b"this is not a Mach-O file of any kind".to_vec();
    let payload = payload_with_trailer(b"anything", DIGEST);
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().join("artifact");

    let error = inject_and_sign(
        &stub,
        &payload,
        &out,
        &MacSignCfg {
            codesign: CodeSign::None,
        },
    )
    .expect_err("not a Mach-O");

    assert!(
        matches!(error, SignMacosError::NotAMachO { .. }),
        "expected SignMacosError::NotAMachO, got {error:?}"
    );
}

#[test]
fn inject_and_sign_refuses_a_stub_that_already_carries_the_section() {
    let already = with_payload_section(CPU_TYPE_X86_64, b"already here", DIGEST);
    let payload = payload_with_trailer(b"a second payload", DIGEST);
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().join("artifact");

    let error = inject_and_sign(
        &already.bytes,
        &payload,
        &out,
        &MacSignCfg {
            codesign: CodeSign::None,
        },
    )
    .expect_err("a payload may not be injected twice");

    assert!(
        matches!(error, SignMacosError::AlreadySectioned),
        "expected SignMacosError::AlreadySectioned, got {error:?}"
    );
}

#[test]
fn inject_and_sign_reports_the_cputype_of_a_fabricated_x86_64_stub() {
    let stub = thin_header(CPU_TYPE_X86_64, MH_EXECUTE);
    let payload = payload_with_trailer(b"an x86_64 payload", DIGEST);
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().join("artifact");

    let report = inject_and_sign(
        &stub,
        &payload,
        &out,
        &MacSignCfg {
            codesign: CodeSign::None,
        },
    )
    .expect("injecting into a minimal, valid x86_64 thin Mach-O succeeds");

    assert_eq!(report.cputype, "x86_64");
}

#[test]
fn inject_and_sign_never_panics_on_a_truncated_thin_magic() {
    let stub = ginary::macho::MH_MAGIC_64.to_le_bytes().to_vec();
    let payload = payload_with_trailer(b"anything", DIGEST);
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().join("artifact");

    let error = inject_and_sign(
        &stub,
        &payload,
        &out,
        &MacSignCfg {
            codesign: CodeSign::None,
        },
    )
    .expect_err("four magic bytes and nothing else do not parse");

    assert!(
        matches!(
            &error,
            SignMacosError::NotAMachO { source } if matches!(source, MachoError::Parse { .. })
        ),
        "expected NotAMachO(Parse), got {error:?}"
    );
}
