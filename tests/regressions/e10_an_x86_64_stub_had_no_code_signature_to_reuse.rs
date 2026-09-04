// SPDX-License-Identifier: MIT OR Apache-2.0
//! Packaging a macOS artifact failed outright when the stub carried no
//! `LC_CODE_SIGNATURE`, which is the ordinary state of an x86_64 Mach-O.
//!
//! **What went wrong.** The macOS x86_64 job packaged a `hello_ffi` artifact
//! and got an error instead of a file:
//!
//! ```text
//! error: cannot write the macOS payload section
//!   caused by: cannot ad-hoc sign a Mach-O with no LC_CODE_SIGNATURE to reuse;
//!              its load-command area cannot grow without relocating code
//! ```
//!
//! (`macOS (macos-15-intel, macos-x86_64)`
//! <https://github.com/P4suta/ginary/actions/runs/33739517757/job/100597889308>.)
//!
//! The arm64 job passed on the same code. The difference is the platform
//! linker: it ad-hoc signs every arm64 image it produces, so an arm64 stub
//! always arrives with a command to reuse, and an x86_64 stub often does not.
//!
//! **The input.** Any thin 64-bit Mach-O stub with no `LC_CODE_SIGNATURE` load
//! command — here the committed arm64 fixture with the one its linker left
//! removed again, which is a real image in exactly that state.
//!
//! **The correct behaviour.** An `LC_CODE_SIGNATURE` is a
//! `linkedit_data_command`: sixteen bytes. The segment-plus-section command E9
//! proved impossible is a hundred and fifty-two, and the slack a linker leaves
//! between the last load command and the first section is forty. Sixteen fits
//! in forty *without moving anything*, because those bytes belong to no
//! command and no section. So the command is added, `ncmds` and `sizeofcmds`
//! grow by one command, and every other byte before `__LINKEDIT` stays exactly
//! where the linker put it. Only a stub whose slack is genuinely too small is
//! refused, and the refusal names both numbers.

#![cfg(feature = "cli")]

use crate::common::macho::{command_counts, real_fixture_bytes, without_code_signature};
use ginary::sign_macos::{
    CODE_SIGNATURE_COMMAND_LEN, CodeSign, CodeSignatureSlot, MacSignCfg, inject_and_sign,
    load_command_slack,
};
use ginary::trailer::{TRAILER_LEN, Trailer};

/// A digest whose bytes are all different, so a slice taken in the wrong place
/// is visible rather than accidentally right.
const DIGEST: [u8; 32] = [
    0x2a, 0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee,
    0xff, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
];

#[test]
fn a_stub_with_no_code_signature_is_packaged_rather_than_refused() {
    let stub = without_code_signature(&real_fixture_bytes());
    let (ncmds, sizeofcmds) = command_counts(&stub);
    let payload = b"the payload the x86_64 job could not write";
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

    let report = inject_and_sign(
        &stub,
        &with_trailer,
        &out,
        &MacSignCfg {
            codesign: CodeSign::Adhoc,
        },
    )
    .expect("a stub with sixteen spare bytes can be given the command it lacks");

    assert_eq!(report.code_signature, Some(CodeSignatureSlot::Added));
    let written = std::fs::read(&out).expect("the artifact was written");
    assert_eq!(
        command_counts(&written),
        (
            ncmds + 1,
            sizeofcmds + u32::try_from(CODE_SIGNATURE_COMMAND_LEN).expect("16 fits"),
        ),
        "exactly one sixteen-byte load command more than the stub had"
    );
    let before = load_command_slack(&stub).expect("the stub measures");
    let after = load_command_slack(&written).expect("and so does the artifact");
    assert_eq!(
        (after.first_content_offset, after.free),
        (
            before.first_content_offset,
            before.free - CODE_SIGNATURE_COMMAND_LEN,
        ),
        "the command was written into the slack: sixteen bytes fewer are free, and the first \
         section is still at the file offset the linker gave it"
    );
}
