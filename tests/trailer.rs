// SPDX-License-Identifier: MIT OR Apache-2.0
//! The 64-byte trailer: the encoding, and every way a file can fail to hold
//! one.
//!
//! The distinction the whole binary turns on is asserted here. Bytes that are
//! not the magic are `Ok(None)` and mean *this copy is the command line tool*;
//! bytes that are the magic and then do not add up are an error and mean *this
//! is a broken application*, which must never present ginary's help text.

use std::fs::File;
use std::io::Write;

use ginary::trailer::{MAGIC, TRAILER_LEN, Trailer, TrailerError};
use proptest::prelude::*;

/// A digest whose bytes are all different, so a slice taken out of it in the
/// wrong place is visible rather than accidentally right.
const DIGEST: [u8; 32] = [
    0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17,
    0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f, 0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27,
];

fn sample() -> Trailer {
    Trailer {
        payload_offset: 4_096,
        payload_len: 2_048,
        payload_sha256: DIGEST,
    }
}

/// `payload_offset + payload_len + TRAILER_LEN`, the file length a trailer
/// describes.
fn file_len_for(trailer: &Trailer) -> u64 {
    trailer.payload_offset + trailer.payload_len + TRAILER_LEN
}

#[test]
fn a_trailer_round_trips_through_its_sixty_four_bytes() {
    let trailer = sample();
    let raw = trailer.to_bytes();

    let parsed = Trailer::parse(&raw, file_len_for(&trailer)).expect("a valid trailer parses");

    assert_eq!(parsed, Some(trailer));
}

#[test]
fn the_encoding_is_the_byte_layout_the_format_document_prints() {
    let raw = sample().to_bytes();

    assert_eq!(&raw[0..8], &MAGIC, "the magic is the first eight bytes");
    assert_eq!(
        &raw[8..16],
        &4_096u64.to_le_bytes(),
        "payload_offset is little-endian at offset 8"
    );
    assert_eq!(
        &raw[16..24],
        &2_048u64.to_le_bytes(),
        "payload_len is little-endian at offset 16"
    );
    assert_eq!(&raw[24..56], &DIGEST, "the digest is the 32 bytes at 24");
    assert_eq!(&raw[56..64], &[0u8; 8], "the reserved bytes are zero");
}

#[test]
fn bytes_that_do_not_start_with_the_magic_are_not_a_trailer_at_all() {
    let mut raw = sample().to_bytes();
    raw[0] = b'g';

    let parsed = Trailer::parse(&raw, file_len_for(&sample())).expect("no magic is not an error");

    assert_eq!(
        parsed, None,
        "a file without the magic is the ginary command line tool"
    );
}

#[test]
fn a_trailer_version_this_build_cannot_read_is_an_error_rather_than_the_command_line() {
    let mut raw = sample().to_bytes();
    raw[7] = 0x02;

    let error = Trailer::parse(&raw, file_len_for(&sample()))
        .expect_err("a future trailer version is refused");

    match error {
        TrailerError::UnsupportedVersion { found, supported } => {
            assert_eq!(found, 0x02);
            assert_eq!(supported, MAGIC[7]);
        }
        other => panic!("expected UnsupportedVersion, got {other:?}"),
    }
}

#[test]
fn reserved_bytes_that_are_not_zero_are_refused() {
    let mut raw = sample().to_bytes();
    raw[63] = 1;

    let error =
        Trailer::parse(&raw, file_len_for(&sample())).expect_err("a used reserved byte is refused");

    assert!(
        matches!(error, TrailerError::Reserved),
        "expected Reserved, got {error:?}"
    );
}

#[test]
fn a_payload_of_no_bytes_is_refused_and_is_not_diagnosed_as_a_truncation() {
    let trailer = Trailer {
        payload_len: 0,
        ..sample()
    };
    let raw = trailer.to_bytes();

    let error =
        Trailer::parse(&raw, file_len_for(&trailer)).expect_err("an empty payload is refused");

    assert!(
        matches!(error, TrailerError::EmptyPayload),
        "expected EmptyPayload, got {error:?}"
    );
    let message = error.to_string();
    assert_eq!(
        message,
        "the trailer says the payload is zero bytes long, so this artifact carries no application",
        "the message names the fault, and no length: nothing here was truncated"
    );
}

#[test]
fn a_geometry_error_names_the_length_it_expected_and_the_length_the_file_has() {
    let trailer = sample();
    let raw = trailer.to_bytes();
    let actual_len = file_len_for(&trailer) - 1;

    let error = Trailer::parse(&raw, actual_len).expect_err("a truncated file is refused");

    match error {
        TrailerError::Geometry { expected, actual } => {
            assert_eq!(expected, 6_208, "4096 + 2048 + 64");
            assert_eq!(actual, 6_207);
            let message = error.to_string();
            assert!(
                message.contains("6208") && message.contains("6207"),
                "the message names both lengths: {message}"
            );
        }
        other => panic!("expected Geometry, got {other:?}"),
    }
}

#[test]
fn an_offset_and_length_that_would_overflow_are_refused_rather_than_wrapping() {
    let trailer = Trailer {
        payload_offset: u64::MAX,
        payload_len: 64,
        payload_sha256: DIGEST,
    };
    let raw = trailer.to_bytes();
    // The file length is the one the *wrapping* sum produces:
    // `u64::MAX + 64 + 64 == 127`. A parser that wrapped would find this
    // trailer describes this file exactly and hand a launcher an offset of
    // `u64::MAX`; only a saturating sum still refuses it.
    let wrapped = u64::MAX
        .wrapping_add(trailer.payload_len)
        .wrapping_add(TRAILER_LEN);
    assert_eq!(wrapped, 127, "the length a wrapping sum would accept");

    let error = Trailer::parse(&raw, wrapped).expect_err("an overflowing geometry is refused");

    match error {
        TrailerError::Geometry { expected, actual } => {
            assert_eq!(
                expected,
                u64::MAX,
                "the sum saturates rather than wrapping, and the message says so"
            );
            assert_eq!(actual, 127);
        }
        other => panic!("expected Geometry, got {other:?}"),
    }
}

#[test]
fn a_file_shorter_than_the_trailer_holds_no_trailer() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("too-short");
    std::fs::write(&path, vec![0u8; (TRAILER_LEN - 1) as usize]).expect("write");
    let file = File::open(&path).expect("open");

    let parsed = Trailer::read_from(&file).expect("a short file is not an error");

    assert_eq!(parsed, None);
}

#[test]
fn read_from_reads_the_last_sixty_four_bytes_of_a_file() {
    let trailer = Trailer {
        payload_offset: 16,
        payload_len: 8,
        payload_sha256: DIGEST,
    };
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("artifact");
    let mut file = File::create(&path).expect("create");
    file.write_all(&[0xaa; 16]).expect("stub bytes");
    file.write_all(&[0xbb; 8]).expect("payload bytes");
    file.write_all(&trailer.to_bytes()).expect("trailer bytes");
    file.sync_all().expect("sync");
    drop(file);

    let opened = File::open(&path).expect("open");
    let parsed = Trailer::read_from(&opened).expect("the file holds a trailer");

    assert_eq!(parsed, Some(trailer));
}

#[test]
fn the_cache_key_is_the_first_eight_bytes_of_the_digest_in_lower_case_hexadecimal() {
    let key = sample().cache_key();

    assert_eq!(key, "0123456789abcdef");
    assert_eq!(key.len(), 16, "eight bytes, two characters each");
}

proptest! {
    /// The bytes at the end of an artifact are bytes ginary did not write: a
    /// virus scanner, an installer or a truncated download can put anything
    /// there. Whatever they are, the answer is a value or an error.
    #[test]
    fn parse_never_panics_on_arbitrary_bytes(
        raw in proptest::array::uniform32(any::<u8>()),
        tail in proptest::array::uniform32(any::<u8>()),
        file_len in any::<u64>(),
    ) {
        let mut bytes = [0u8; 64];
        bytes[..32].copy_from_slice(&raw);
        bytes[32..].copy_from_slice(&tail);
        let _ = Trailer::parse(&bytes, file_len);
    }

    /// The branch a random vector cannot reach: bytes that *are* the magic and
    /// then are rubbish, which is what a corrupted artifact looks like.
    #[test]
    fn parse_never_panics_on_the_magic_followed_by_rubbish(
        rest in proptest::collection::vec(any::<u8>(), 56..=56),
        file_len in any::<u64>(),
    ) {
        let mut bytes = [0u8; 64];
        bytes[..8].copy_from_slice(&MAGIC);
        bytes[8..].copy_from_slice(&rest);
        let _ = Trailer::parse(&bytes, file_len);
    }
}
