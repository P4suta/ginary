// SPDX-License-Identifier: MIT OR Apache-2.0
//! `sign_macos::shift_segment` shifted a section's file `offset` when its
//! containing segment moved, but never shifted the section's own `addr`
//! (virtual memory address) field.
//!
//! **What went wrong.** The section loop in `shift_segment` only rewrote the
//! `section_64::offset` field (at byte 48 of the section, a file offset). It
//! never touched `section_64::addr` (at byte 32, the section's VM address),
//! in either the `fileoff == 0` branch or the `else` branch, for any
//! segment. So once a segment's own `vmaddr` moved -- every segment other
//! than the one mapping the header -- every section inside it kept its
//! *original* `addr`, which now sits before the segment's new `vmaddr`:
//! outside the segment's own `[vmaddr, vmaddr + vmsize)` range. An invalid
//! Mach-O, independent of any live kernel.
//!
//! **The input.** `inject_and_sign` over the committed real Mach-O fixture
//! (`tests/fixtures/macho/`), in both the unsigned and the ad-hoc-signed
//! configurations -- both grow the load-command area and so both move every
//! later segment's `vmaddr`, reproducing the stale `addr` in that segment's
//! sections.
//!
//! **The correct behaviour.** Every `section_64` in the output declares an
//! `addr` that falls within its own segment's `[vmaddr, vmaddr + vmsize)`
//! range.

use crate::common::macho::real_fixture_bytes;
use ginary::sign_macos::{CodeSign, MacSignCfg, inject_and_sign};

/// One `section_64`, decoded field by field: just the `addr` this test
/// checks, plus enough to report a useful failure.
#[derive(Debug, Clone)]
struct Section {
    segname: String,
    sectname: String,
    addr: u64,
}

/// One `LC_SEGMENT_64` command, with its sections, decoded field by field --
/// mirroring exactly the layout `src/sign_macos.rs`'s `Writer` writes and
/// `src/macho.rs` reads.
#[derive(Debug, Clone)]
struct Segment {
    name: String,
    vmaddr: u64,
    vmsize: u64,
    sections: Vec<Section>,
}

const LC_SEGMENT_64: u32 = 0x19;
const SEGMENT_CMD_LEN: usize = 72;
const SECTION_LEN: usize = 80;

/// Every `LC_SEGMENT_64` command in `bytes`, with its sections, in
/// load-command order.
///
/// Field-by-field by hand rather than through `object` or `ginary::macho`:
/// `ginary::macho::MachoFacts::sections` reports a section's file offset and
/// size, not its VM address, so it cannot see the bug this test exists to
/// catch. The raw load commands are the only source for `addr`.
fn segments(bytes: &[u8]) -> Vec<Segment> {
    let u32_at = |at: usize| -> u32 { u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) };
    let u64_at = |at: usize| -> u64 { u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap()) };
    let name_at = |at: usize| -> String {
        let raw = &bytes[at..at + 16];
        let end = raw.iter().position(|&b| b == 0).unwrap_or(16);
        std::str::from_utf8(&raw[..end]).unwrap().to_owned()
    };

    let ncmds = u32_at(16);
    let mut out = Vec::new();
    let mut offset = 32usize;
    for _ in 0..ncmds {
        let cmd = u32_at(offset);
        let cmdsize = u32_at(offset + 4) as usize;
        if cmd == LC_SEGMENT_64 {
            let name = name_at(offset + 8);
            let vmaddr = u64_at(offset + 24);
            let vmsize = u64_at(offset + 32);
            let nsects = u32_at(offset + 64);
            let mut sections = Vec::new();
            for index in 0..nsects {
                let section_start = offset + SEGMENT_CMD_LEN + (index as usize) * SECTION_LEN;
                sections.push(Section {
                    sectname: name_at(section_start),
                    segname: name_at(section_start + 16),
                    addr: u64_at(section_start + 32),
                });
            }
            out.push(Segment {
                name,
                vmaddr,
                vmsize,
                sections,
            });
        }
        offset += cmdsize;
    }
    out
}

/// Asserts every section's `addr` falls within its own segment's
/// `[vmaddr, vmaddr + vmsize)` range.
fn assert_every_section_addr_is_inside_its_segment(segments: &[Segment]) {
    for segment in segments {
        let end = segment.vmaddr + segment.vmsize;
        for section in &segment.sections {
            assert!(
                section.addr >= segment.vmaddr && section.addr < end,
                "{},{}: addr {:#x} falls outside its segment {}'s range \
                 {:#x}..{:#x}",
                section.segname,
                section.sectname,
                section.addr,
                segment.name,
                segment.vmaddr,
                end
            );
        }
    }
}

#[test]
fn inject_and_sign_shifts_every_section_addr_with_its_segment_unsigned() {
    let stub = real_fixture_bytes();
    let payload = b"payload bytes, unsigned section-addr run";
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
    assert_every_section_addr_is_inside_its_segment(&segments(&written));
}

#[test]
fn inject_and_sign_shifts_every_section_addr_with_its_segment_signed() {
    let stub = real_fixture_bytes();
    let payload = b"payload bytes, signed section-addr run";
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
    assert_every_section_addr_is_inside_its_segment(&segments(&written));
}
