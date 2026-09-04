// SPDX-License-Identifier: MIT OR Apache-2.0
//! `payload::locate`: the one abstraction the launcher, `ginary inspect`,
//! `ginary verify` and `cache::ensure_extracted` read a packaged
//! application's payload through, whichever container produced it.
//!
//! Not gated behind the `cli` feature: `locate` is on the launcher path, so
//! this test binary has to build under `--no-default-features` as well as
//! the default build.
//!
//! ## Payload section geometry (a RED-phase design decision)
//!
//! The plan text describes the Mach-O section's inner trailer as "its first
//! 64 bytes are the same trailer struct, with `payload_offset` relative to
//! the section start", and separately gives the validation
//! `offset_rel + len + 64 == section_size`. Taken completely literally
//! those two sentences disagree about where the trailer physically sits
//! (byte 0 of the section, versus the last 64 bytes of a region whose
//! arithmetic mirrors the EOF trailer's). This test file commits to one
//! reading, so that RED and GREEN agree on what "correct" means:
//!
//! - the section's bytes are the 64-byte trailer at offset 0, immediately
//!   followed by the payload — nothing else, so `section_size == 64 +
//!   payload_len`;
//! - the trailer's own `payload_offset` field, read from inside the
//!   section, is fixed at `64` (`TRAILER_LEN`) by that layout: it names
//!   where the payload starts, relative to the section;
//! - `locate` converts that to an absolute file offset as
//!   `section_file_offset + 64`;
//! - the geometry check reuses [`Trailer::parse`] itself, passing
//!   `section_size + TRAILER_LEN` in place of a file length, which is
//!   exactly what makes `payload_offset(64) + payload_len + TRAILER_LEN`
//!   land on `section_size + TRAILER_LEN` for a well-formed section — the
//!   same equation the plan text states, applied to a region one trailer
//!   length larger than the section itself rather than to the section's own
//!   size. A section smaller than 64 bytes, or one whose first 64 bytes are
//!   not the trailer magic at all, is [`TrailerError::Section`]; a
//!   well-formed trailer whose `payload_offset`/`payload_len` disagree with
//!   the section's actual size is the existing [`TrailerError::Geometry`].
//!
//! `tests/common/macho.rs::payload_section_body` builds exactly this layout,
//! so every fixture in this file that carries a `__GINARY,__payload` section
//! agrees with it.

mod common;

use std::fs::File;
use std::io::Write;

use common::macho::{CPU_TYPE_ARM64, fat_header, with_payload_section, with_section};
use ginary::payload::{PayloadLoc, PayloadVia, locate};
use ginary::trailer::{TRAILER_LEN, Trailer, TrailerError};

/// A digest whose bytes are all different, so a slice taken in the wrong
/// place is visible rather than accidentally right.
const DIGEST: [u8; 32] = [
    0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17,
    0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f, 0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27,
];

/// Writes `bytes` to a fresh temporary file and returns it, opened for
/// reading, alongside the [`tempfile::TempDir`] that must outlive it.
fn file_of(bytes: &[u8]) -> (File, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("artifact");
    std::fs::write(&path, bytes).expect("write the fixture");
    (File::open(&path).expect("open the fixture"), dir)
}

/// An EOF-trailered "artifact": arbitrary stub bytes, a payload, and the
/// trailer describing it — the shape every ELF and PE artifact already is.
fn eof_artifact(stub: &[u8], payload: &[u8], sha256: [u8; 32]) -> Vec<u8> {
    let mut bytes = stub.to_vec();
    let payload_offset = bytes.len() as u64;
    bytes.extend_from_slice(payload);
    let trailer = Trailer {
        payload_offset,
        payload_len: payload.len() as u64,
        payload_sha256: sha256,
    };
    bytes.extend_from_slice(&trailer.to_bytes());
    bytes
}

#[test]
fn locate_finds_the_eof_trailer_of_a_plain_artifact() {
    let stub = b"stub bytes go here, arbitrary length";
    let payload = b"a payload's worth of bytes";
    let bytes = eof_artifact(stub, payload, DIGEST);
    let (file, _dir) = file_of(&bytes);

    let found = locate(&file)
        .expect("a well-formed trailer locates")
        .expect("it is there");

    assert_eq!(
        found,
        PayloadLoc {
            offset: stub.len() as u64, // the stub's length, not a magic number
            len: payload.len() as u64,
            sha256: DIGEST,
            via: PayloadVia::EofTrailer,
        }
    );
}

#[test]
fn locate_returns_none_for_a_file_with_no_trailer_and_no_macho_magic() {
    let (file, _dir) = file_of(b"just a plain file, not an executable of any kind");

    let found = locate(&file).expect("no trailer is not an error");

    assert_eq!(found, None);
}

#[test]
fn locate_returns_none_for_a_thin_macho_with_no_payload_section() {
    let built = with_section(
        CPU_TYPE_ARM64,
        "__TEXT",
        "__text",
        b"\x00\x00\x00\x00",
        false,
    );
    let (file, _dir) = file_of(&built.bytes);

    let found = locate(&file).expect("a Mach-O with no __GINARY,__payload section is not an error");

    assert_eq!(found, None);
}

#[test]
fn locate_finds_the_macho_section_with_the_right_absolute_offset() {
    let payload = b"the payload bytes a macOS artifact carries";
    let built = with_payload_section(CPU_TYPE_ARM64, payload, DIGEST);
    let (file, _dir) = file_of(&built.bytes);

    let found = locate(&file)
        .expect("a well-formed __GINARY,__payload section locates")
        .expect("it is there");

    assert_eq!(
        found,
        PayloadLoc {
            offset: built.section_offset + TRAILER_LEN,
            len: payload.len() as u64,
            sha256: DIGEST,
            via: PayloadVia::MachOSection,
        }
    );
}

#[test]
fn locate_reports_a_typed_error_for_a_section_whose_trailer_offset_is_not_the_fixed_one() {
    // `docs/format.md` fixes the inner trailer's own `payload_offset` at
    // exactly `TRAILER_LEN`: the payload immediately follows the trailer and
    // nothing else is in the section. `payload_offset + payload_len +
    // TRAILER_LEN == region_len` has other solutions than that one — here,
    // `payload_offset = 0` and `payload_len = section_size` satisfies the
    // same equation while pointing `PayloadLoc::offset` at the trailer's own
    // bytes instead of skipping past them. `locate` must reject this rather
    // than silently accept it.
    let mut body = Trailer {
        payload_offset: 0,
        payload_len: 0, // patched below, once the section's size is known
        payload_sha256: DIGEST,
    }
    .to_bytes()
    .to_vec();
    body.extend_from_slice(b"12345");
    let section_size = body.len() as u64;
    body[16..24].copy_from_slice(&section_size.to_le_bytes()); // payload_len := section_size
    let built = with_section(CPU_TYPE_ARM64, "__GINARY", "__payload", &body, false);
    let (file, _dir) = file_of(&built.bytes);

    let error = locate(&file)
        .expect_err("a payload_offset other than TRAILER_LEN must not locate successfully");

    assert!(
        matches!(error, TrailerError::Section { .. }),
        "expected TrailerError::Section, got {error:?}"
    );
}

#[test]
fn locate_reports_geometry_for_a_section_whose_declared_length_disagrees_with_its_size() {
    // A trailer that claims a payload of 999 bytes, sitting in a section
    // that actually holds the trailer plus five bytes of "payload". The
    // section's own byte count is honest; the trailer inside it lies.
    let mut body = Trailer {
        payload_offset: TRAILER_LEN,
        payload_len: 999,
        payload_sha256: DIGEST,
    }
    .to_bytes()
    .to_vec();
    body.extend_from_slice(b"12345");
    let built = with_section(CPU_TYPE_ARM64, "__GINARY", "__payload", &body, false);
    let (file, _dir) = file_of(&built.bytes);

    let error = locate(&file).expect_err("999 declared bytes do not fit in a 5-byte remainder");

    assert!(
        matches!(error, TrailerError::Geometry { .. }),
        "expected TrailerError::Geometry, got {error:?}"
    );
}

#[test]
fn locate_reports_a_typed_error_for_a_section_too_small_to_hold_a_trailer() {
    // Thirty bytes: not even enough for the 64-byte trailer struct, let
    // alone a payload after it.
    let built = with_section(CPU_TYPE_ARM64, "__GINARY", "__payload", &[0u8; 30], false);
    let (file, _dir) = file_of(&built.bytes);

    let error = locate(&file).expect_err("30 bytes cannot hold a 64-byte trailer");

    assert!(
        matches!(error, TrailerError::Section { .. }),
        "expected TrailerError::Section, got {error:?}"
    );
}

#[test]
fn locate_reports_a_typed_error_for_a_section_whose_first_bytes_are_not_a_trailer() {
    let mut body = vec![0u8; 64]; // no GINARY magic at all
    body.extend_from_slice(b"payload");
    let built = with_section(CPU_TYPE_ARM64, "__GINARY", "__payload", &body, false);
    let (file, _dir) = file_of(&built.bytes);

    let error = locate(&file).expect_err("a section whose first 64 bytes carry no magic");

    assert!(
        matches!(error, TrailerError::Section { .. }),
        "expected TrailerError::Section, got {error:?}"
    );
}

#[test]
fn locate_reports_fat_cleanly_for_a_fat_macho() {
    let bytes = fat_header(&[(0x0100_0007, 0), (0x0100_000c, 0)]);
    let (file, _dir) = file_of(&bytes);

    let error = locate(&file).expect_err("a fat Mach-O carries no single payload section");

    assert!(
        matches!(error, TrailerError::Fat),
        "expected TrailerError::Fat, got {error:?}"
    );
}

#[test]
fn locate_prefers_the_eof_trailer_over_reading_the_file_as_a_macho() {
    // A Mach-O carrying a real __GINARY,__payload section, but with a
    // well-formed EOF trailer *also* appended after it (which is not a shape
    // a real build produces, but is exactly the case that proves which of
    // the two `locate` tries first). The EOF trailer's own answer must win,
    // unchanged, for every existing ELF and PE artifact to keep working.
    let macho_payload = b"the macho section's payload";
    let built = with_payload_section(CPU_TYPE_ARM64, macho_payload, DIGEST);
    let eof_payload = b"a different payload, at the end of the file";
    let mut bytes = built.bytes;
    let eof_offset = bytes.len() as u64;
    bytes.extend_from_slice(eof_payload);
    let other_digest = [0xffu8; 32];
    bytes.extend_from_slice(
        &Trailer {
            payload_offset: eof_offset,
            payload_len: eof_payload.len() as u64,
            payload_sha256: other_digest,
        }
        .to_bytes(),
    );
    let (file, _dir) = file_of(&bytes);

    let found = locate(&file)
        .expect("the eof trailer is well-formed")
        .expect("it is there");

    assert_eq!(found.via, PayloadVia::EofTrailer);
    assert_eq!(found.offset, eof_offset);
    assert_eq!(found.len, eof_payload.len() as u64);
    assert_eq!(found.sha256, other_digest);
}

#[test]
fn a_flushed_write_is_visible_to_locate_on_the_same_open_file() {
    // Not a `locate` claim by itself, but the guarantee every caller above
    // relies on: `file_of` opens the file it just wrote, and `locate` reads
    // from that same handle.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("artifact");
    let mut writer = File::create(&path).expect("create");
    writer.write_all(b"hello").expect("write");
    writer.flush().expect("flush");
    drop(writer);

    let file = File::open(&path).expect("open");
    assert_eq!(locate(&file).expect("no trailer, no macho magic"), None);
}

#[test]
fn a_signed_macho_whose_dataoff_points_past_eof_is_not_a_read_error() {
    // A thin Mach-O with a valid header and an `LC_CODE_SIGNATURE` whose
    // `dataoff` names an offset past the end of the file — the shape a
    // truncated or malformed signed image has. `locate` must treat the
    // out-of-range signature as "no ginary trailer here" and fall through
    // to the section lookup (which finds nothing), returning `None`, not
    // raise a spurious `TrailerError::Io` from reading past EOF. The comment
    // beside the code-signature branch promises exactly this fall-through.
    let mut built = with_section(CPU_TYPE_ARM64, "__TEXT", "__text", b"a little code", true);
    let (codesig_offset, _size) = built.codesig.expect("with_section wrote a code signature");

    // The `LC_CODE_SIGNATURE` command sits after the one segment+section:
    // header (32) + segment_command_64 (72) + section_64 (80) = 184, and its
    // `dataoff` field is 8 bytes into the 16-byte command.
    const DATAOFF_FIELD: usize = 32 + 72 + 80 + 8;
    assert_eq!(
        u32::from_le_bytes(
            built.bytes[DATAOFF_FIELD..DATAOFF_FIELD + 4]
                .try_into()
                .unwrap()
        ) as u64,
        codesig_offset,
        "the field we are about to overwrite is the code-signature dataoff",
    );
    let past_eof: u32 = 0xffff_ff00;
    assert!(
        u64::from(past_eof) > built.bytes.len() as u64,
        "dataoff is past EOF"
    );
    built.bytes[DATAOFF_FIELD..DATAOFF_FIELD + 4].copy_from_slice(&past_eof.to_le_bytes());

    let (file, _dir) = file_of(&built.bytes);
    let found = locate(&file)
        .expect("an out-of-range signature dataoff is not a read error; it falls through");
    assert_eq!(found, None, "no ginary payload, so locate finds none");
}
