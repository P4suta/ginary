// SPDX-License-Identifier: MIT OR Apache-2.0
//! A trailer claiming a payload of no bytes was reported as a file that is one
//! byte short.
//!
//! **What went wrong.** `Trailer::parse` folded the empty-payload case into
//! the geometry check by computing `payload_offset + payload_len.max(1) + 64`,
//! so a well-formed file whose trailer said `payload_len == 0` produced "the
//! trailer says the file is 6209 bytes long and it is 6208, so it was
//! truncated or something was appended to it". The file is not truncated and
//! the one byte named is an artefact of the `.max(1)`; this is a
//! launcher-facing diagnostic, and it pointed at the wrong fault.
//!
//! **The input.** A trailer with `payload_len` 0 and a file length that
//! matches it exactly, `payload_offset + 0 + 64`.
//!
//! **The correct behaviour.** `TrailerError::EmptyPayload`, whose message says
//! that the payload is zero bytes long and names no lengths at all.

use ginary::trailer::{TRAILER_LEN, Trailer, TrailerError};

/// A digest whose bytes are all different.
const DIGEST: [u8; 32] = [
    0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17,
    0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f, 0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27,
];

#[test]
fn a_trailer_that_claims_no_payload_says_so_rather_than_naming_a_missing_byte() {
    let trailer = Trailer {
        payload_offset: 4_096,
        payload_len: 0,
        payload_sha256: DIGEST,
    };
    let raw = trailer.to_bytes();
    let file_len = trailer.payload_offset + TRAILER_LEN;

    let error = Trailer::parse(&raw, file_len).expect_err("an empty payload is refused");

    assert!(
        matches!(error, TrailerError::EmptyPayload),
        "expected EmptyPayload, got {error:?}"
    );
    let message = error.to_string();
    assert!(
        message.contains("zero bytes"),
        "the message says what is wrong: {message}"
    );
    assert!(
        !message.contains("4161") && !message.contains("truncated"),
        "and does not diagnose a truncation that did not happen: {message}"
    );
}
