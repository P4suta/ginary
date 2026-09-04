// SPDX-License-Identifier: MIT OR Apache-2.0
//! The ad-hoc signature a macOS artifact carries was computed over bytes that
//! were then overwritten, so the kernel killed every artifact ginary built
//! before it reached `main`.
//!
//! **What went wrong.** Both macOS jobs — arm64 and x86_64 — died with exit
//! code 137 at the step that runs the artifact and then verifies its
//! signature. `codesign` never ran: the artifact's own run is line 10, and
//! that is the line that died.
//!
//! ```text
//! /Users/runner/work/_temp/84fd6172-....sh: line 10:  7695 Killed: 9   "$artifact" 0 hello world
//! ##[error]Process completed with exit code 137.
//! ```
//!
//! 137 is 128+9. `Killed: 9` before a program prints anything is what the
//! kernel does to a Mach-O whose `CodeDirectory` does not match the pages it
//! is mapping: a page whose hash disagrees is a page it will not map, and an
//! *invalid* signature is worse than no signature on both architectures.
//!
//! (`macOS build, launch and signature (macos-14, macos-aarch64)`
//! <https://github.com/P4suta/ginary/actions/runs/33712111530/job/100513644659>
//! and `(macos-15-intel, macos-x86_64)`
//! <https://github.com/P4suta/ginary/actions/runs/33712111530/job/100513644714>.)
//!
//! **The input.** Every signed macOS artifact. `Writer::build` takes the page
//! hashes over the body, and *then* patches four fields into it — the
//! `LC_CODE_SIGNATURE` command's `dataoff` and `datasize`, and `__LINKEDIT`'s
//! `vmsize` and `filesize`. All four live in the load-command area, which is
//! page 0, so slot 0 describes a page that no longer exists.
//!
//! **The correct behaviour.** The signature is computed over the finished
//! file: the section injected, every offset patched, and only then the pages
//! hashed. ADR 0016 says the `__GINARY,__payload` section exists so that the
//! signature can cover the payload; covering it means hashing the bytes that
//! are actually there when the file is closed.
// The signer is a `cli`-gated module: a launcher-only build has nothing here
// to run.
#![cfg(feature = "cli")]

use crate::common::codesign;
use crate::common::macho::real_fixture_bytes;

use ginary::sign_macos::{CodeSign, MacSignCfg, inject_and_sign};
use ginary::trailer::{TRAILER_LEN, Trailer};

/// The payload digest the trailer carries; any 32 bytes will do.
const DIGEST: [u8; 32] = [0x5a; 32];

#[test]
fn every_code_slot_is_the_hash_of_the_page_it_stands_for() {
    let stub = real_fixture_bytes();
    let payload = b"a payload the CodeDirectory has to be taken over";
    let mut with_trailer = Trailer {
        payload_offset: TRAILER_LEN,
        payload_len: payload.len() as u64,
        payload_sha256: DIGEST,
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
    .expect("a real, valid thin Mach-O stub can be injected into and signed");

    let written = std::fs::read(&out).expect("the output was written");
    let signature = codesign::signature(&written).expect("a signed artifact carries a signature");

    assert_eq!(
        codesign::first_bad_slot(&written, &signature.code_directory),
        None,
        "the slot named here is a page the kernel will refuse to map, and it reports that by \
         killing the process with SIGKILL rather than by saying so"
    );
}
