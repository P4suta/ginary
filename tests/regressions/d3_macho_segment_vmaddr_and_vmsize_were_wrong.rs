// SPDX-License-Identifier: MIT OR Apache-2.0
//! `sign_macos::Writer` never updated a segment's `vmaddr`/`vmsize` when
//! inserting the `__GINARY,__payload` segment.
//!
//! **What went wrong.** `Writer::plan` shifted every segment's `fileoff` (and
//! the `symtab`/`dysymtab`/`dyld_info`/`linkedit_data` offsets that point into
//! `__LINKEDIT`) by the header growth and, for `__LINKEDIT` itself, by the
//! payload segment's own size — but it left every segment's `vmaddr` and
//! `vmsize` completely untouched, and reused the *original* `__LINKEDIT`
//! `vmaddr` verbatim for the new `__GINARY` segment. Two concrete, checkable
//! consequences followed: the new `__GINARY` segment and the (now relocated)
//! `__LINKEDIT` segment declared the identical `vmaddr`, an invalid Mach-O
//! regardless of any live kernel; and `__TEXT`'s `filesize` grew to cover the
//! longer load-command area while its `vmsize` did not, so `filesize` ended
//! up larger than `vmsize` — itself an invalid segment, since a segment may
//! not claim more file-backed bytes than its own VM region holds.
//!
//! **The input.** `inject_and_sign` over the committed real Mach-O fixture
//! (`tests/fixtures/macho/`), in both the unsigned and the ad-hoc-signed
//! configurations — both grow the load command area, so both reproduce the
//! collision and the `filesize > vmsize` violation.
//!
//! **The correct behaviour.** Every `LC_SEGMENT_64` command in the output
//! declares a distinct, non-overlapping `vmaddr` range, and no segment's
//! `filesize` exceeds its own `vmsize`.

use crate::common::macho::real_fixture_bytes;
use ginary::sign_macos::{CodeSign, MacSignCfg, inject_and_sign};

/// One `LC_SEGMENT_64` command, decoded field by field, mirroring exactly the
/// layout `src/sign_macos.rs::Writer` writes and `src/macho.rs` reads.
#[derive(Debug, Clone)]
struct Segment {
    name: String,
    vmaddr: u64,
    vmsize: u64,
    filesize: u64,
}

const LC_SEGMENT_64: u32 = 0x19;

/// Every `LC_SEGMENT_64` command in `bytes`, in load-command order.
///
/// Field-by-field by hand rather than through `object` or `ginary::macho`:
/// this test exists specifically to catch `vmaddr`/`vmsize` bugs that
/// `ginary::macho::MachoFacts` does not report at all (it exposes sections,
/// not segment geometry), so it has to read the raw load commands itself.
fn segments(bytes: &[u8]) -> Vec<Segment> {
    let u32_at = |at: usize| -> u32 { u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) };
    let u64_at = |at: usize| -> u64 { u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap()) };

    let ncmds = u32_at(16);
    let mut out = Vec::new();
    let mut offset = 32usize;
    for _ in 0..ncmds {
        let cmd = u32_at(offset);
        let cmdsize = u32_at(offset + 4) as usize;
        if cmd == LC_SEGMENT_64 {
            let name_bytes = &bytes[offset + 8..offset + 24];
            let end = name_bytes.iter().position(|&b| b == 0).unwrap_or(16);
            let name = std::str::from_utf8(&name_bytes[..end]).unwrap().to_owned();
            out.push(Segment {
                name,
                vmaddr: u64_at(offset + 24),
                vmsize: u64_at(offset + 32),
                filesize: u64_at(offset + 48),
            });
        }
        offset += cmdsize;
    }
    out
}

/// Asserts the two structural invariants every `LC_SEGMENT_64` in a valid
/// Mach-O holds, over `segments`.
fn assert_geometry_is_valid(segments: &[Segment]) {
    for segment in segments {
        assert!(
            segment.filesize <= segment.vmsize,
            "{}: filesize {} exceeds vmsize {} -- a segment may not claim more \
             file-backed bytes than its own VM region",
            segment.name,
            segment.filesize,
            segment.vmsize
        );
    }

    let mut by_addr: Vec<&Segment> = segments.iter().filter(|s| s.vmsize > 0).collect();
    by_addr.sort_by_key(|s| s.vmaddr);
    for pair in by_addr.windows(2) {
        let [a, b] = pair else { unreachable!() };
        assert!(
            a.vmaddr != b.vmaddr,
            "{} and {} declare the identical vmaddr {:#x}",
            a.name,
            b.name,
            a.vmaddr
        );
        assert!(
            a.vmaddr + a.vmsize <= b.vmaddr,
            "{} ({:#x}..{:#x}) overlaps {} ({:#x}..)",
            a.name,
            a.vmaddr,
            a.vmaddr + a.vmsize,
            b.name,
            b.vmaddr
        );
    }
}

#[test]
fn inject_and_sign_produces_distinct_non_overlapping_segments_unsigned() {
    let stub = real_fixture_bytes();
    let payload = b"payload bytes, unsigned geometry run";
    let mut with_trailer = ginary::trailer::Trailer {
        payload_offset: ginary::trailer::TRAILER_LEN,
        payload_len: payload.len() as u64,
        payload_sha256: [0u8; 32],
    }
    .to_bytes()
    .to_vec();
    with_trailer.extend_from_slice(payload);
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().join("artifact");

    inject_and_sign(
        &stub,
        &with_trailer,
        &out,
        &MacSignCfg {
            codesign: CodeSign::None,
        },
    )
    .expect("injecting into the real fixture succeeds");

    let written = std::fs::read(&out).expect("the output was written");
    assert_geometry_is_valid(&segments(&written));
}

#[test]
fn inject_and_sign_produces_distinct_non_overlapping_segments_signed() {
    let stub = real_fixture_bytes();
    let payload = b"payload bytes, signed geometry run";
    let mut with_trailer = ginary::trailer::Trailer {
        payload_offset: ginary::trailer::TRAILER_LEN,
        payload_len: payload.len() as u64,
        payload_sha256: [0u8; 32],
    }
    .to_bytes()
    .to_vec();
    with_trailer.extend_from_slice(payload);
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().join("artifact");

    inject_and_sign(
        &stub,
        &with_trailer,
        &out,
        &MacSignCfg {
            codesign: CodeSign::Adhoc,
        },
    )
    .expect("injecting and signing the real fixture succeeds");

    let written = std::fs::read(&out).expect("the output was written");
    assert_geometry_is_valid(&segments(&written));
}
