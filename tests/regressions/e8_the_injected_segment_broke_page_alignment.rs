// SPDX-License-Identifier: MIT OR Apache-2.0
//! The `__GINARY,__payload` segment was inserted by shifting every following
//! byte forward by the load-command growth (152 bytes), which is not a
//! multiple of a page, so every segment after `__TEXT` lost page alignment and
//! `__TEXT`'s page-rounded `vmsize` swallowed `__DATA_CONST` and `__DATA` — a
//! shape a real kernel refuses to map, independent of the signature.
//!
//! **What went wrong.** `Writer::plan` computed `header_growth = 152` (one new
//! segment-and-section command, less the dropped `LC_CODE_SIGNATURE`) and
//! `Writer::build` placed `pre_linkedit` immediately after the grown load
//! commands, so every `fileoff` and `vmaddr` after `__TEXT` moved by 152. On
//! arm64 the page size is `0x4000`; `152 % 0x4000 != 0`, so `__DATA_CONST`,
//! `__DATA`, `__GINARY` and `__LINKEDIT` all landed at `fileoff % 0x4000 ==
//! 152`, and `round_page(__TEXT.vmsize) = round_page(0x8098) = 0xC000` made
//! `__TEXT` span `0x100000000..0x10000C000`, overlapping both `__DATA_CONST`
//! (`0x100008098`) and `__DATA` (`0x10000C098`).
//!
//! **What it should do.** Content is shifted by a whole page: the load-command
//! growth is rounded up to `SEGMENT_PAGE_ALIGN` and the gap between the load
//! commands and the first byte of content is padded to make up the difference,
//! so every segment stays page-aligned, no rounded `vmsize` range overlaps its
//! neighbour, and the injected segment is emitted in increasing `vmaddr`
//! order.
//!
//! This is pure arithmetic over the load commands the writer emits — no macOS
//! toolchain and no launch — so it holds on Linux.
// The signer is a `cli`-gated module.
#![cfg(feature = "cli")]

use crate::common::codesign;
use crate::common::macho::real_fixture_bytes;

use ginary::sign_macos::{CodeSign, MacSignCfg, inject_and_sign};
use ginary::trailer::{TRAILER_LEN, Trailer};

/// The page a macOS segment's `vmaddr` and `fileoff` are aligned to, and the
/// unit `vmsize` is rounded up to when a kernel maps it: Apple Silicon's page.
const PAGE: u64 = 0x4000;

/// The payload digest the trailer carries; any 32 bytes will do.
const DIGEST: [u8; 32] = [0x5a; 32];

/// Rounds `value` up to the next multiple of [`PAGE`].
fn round_page(value: u64) -> u64 {
    value.div_ceil(PAGE) * PAGE
}

/// Signs the committed real arm64 fixture around a payload and returns the
/// finished bytes.
fn signed(payload_len: usize) -> Vec<u8> {
    let stub = real_fixture_bytes();
    let payload = vec![0x41u8; payload_len];
    let mut with_trailer = Trailer {
        payload_offset: TRAILER_LEN,
        payload_len: payload.len() as u64,
        payload_sha256: DIGEST,
    }
    .to_bytes()
    .to_vec();
    with_trailer.extend_from_slice(&payload);

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
    .expect("a real, valid thin Mach-O stub can be injected into and signed");
    std::fs::read(&out).expect("the output was written")
}

#[test]
fn every_segment_the_writer_emits_is_page_aligned() {
    let written = signed(5000);
    for segment in codesign::segments(&written) {
        // `__PAGEZERO` maps nothing and sits at address 0; it is the one
        // segment a kernel does not hold to this rule.
        if segment.name == "__PAGEZERO" {
            continue;
        }
        assert_eq!(
            segment.vmaddr % PAGE,
            0,
            "segment {} has vmaddr {:#x}, which is not on a {:#x} page boundary",
            segment.name,
            segment.vmaddr,
            PAGE
        );
        assert_eq!(
            segment.fileoff % PAGE,
            0,
            "segment {} has fileoff {:#x}, which is not on a {:#x} page boundary",
            segment.name,
            segment.fileoff,
            PAGE
        );
    }
}

#[test]
fn no_page_rounded_segment_range_overlaps_its_neighbour() {
    let written = signed(5000);
    let mut ranges: Vec<(String, u64, u64)> = codesign::segments(&written)
        .into_iter()
        .filter(|seg| seg.name != "__PAGEZERO" && seg.vmsize > 0)
        .map(|seg| (seg.name, seg.vmaddr, seg.vmaddr + round_page(seg.vmsize)))
        .collect();
    ranges.sort_by_key(|(_, start, _)| *start);
    for pair in ranges.windows(2) {
        let (ref lo_name, _lo_start, lo_end) = pair[0];
        let (ref hi_name, hi_start, _hi_end) = pair[1];
        assert!(
            lo_end <= hi_start,
            "segment {lo_name} rounds up to end at {lo_end:#x}, past where {hi_name} begins \
             ({hi_start:#x})"
        );
    }
}

#[test]
fn the_segments_are_emitted_in_increasing_vmaddr_order() {
    let written = signed(5000);
    let vmaddrs: Vec<(String, u64)> = codesign::segments(&written)
        .into_iter()
        .map(|seg| (seg.name, seg.vmaddr))
        .collect();
    for pair in vmaddrs.windows(2) {
        assert!(
            pair[0].1 <= pair[1].1,
            "segment {} (vmaddr {:#x}) is emitted before {} (vmaddr {:#x}); the load-command \
             order must be non-decreasing in vmaddr",
            pair[0].0,
            pair[0].1,
            pair[1].0,
            pair[1].1
        );
    }
}
