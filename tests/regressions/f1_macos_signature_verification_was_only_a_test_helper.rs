// SPDX-License-Identifier: MIT OR Apache-2.0
//! Publication needs an independent verifier for the exact finished Mach-O bytes.
#![cfg(feature = "cli")]

use crate::common::{codesign, macho};
use ginary::sign_macos::{CodeSign, MacSignCfg, inject_and_sign, verify_ad_hoc};
use ginary::trailer::{TRAILER_LEN, Trailer};
use sha2::{Digest as _, Sha256};

fn artifact(digest: Option<[u8; 32]>) -> Vec<u8> {
    let payload = b"a complete payload whose digest is verified independently of its signed pages";
    let mut input = Trailer {
        payload_offset: TRAILER_LEN,
        payload_len: payload.len() as u64,
        payload_sha256: digest.unwrap_or_else(|| Sha256::digest(payload).into()),
    }
    .to_bytes()
    .to_vec();
    input.extend_from_slice(payload);
    let work = tempfile::tempdir().unwrap();
    let out = work.path().join("artifact");
    inject_and_sign(
        &macho::real_fixture_bytes(),
        &input,
        &out,
        &MacSignCfg {
            codesign: CodeSign::Adhoc,
        },
    )
    .unwrap();
    std::fs::read(out).unwrap()
}

#[test]
fn the_portable_validator_accepts_the_complete_signed_artifact() {
    verify_ad_hoc(&artifact(None)).unwrap();
}

#[test]
fn changed_code_pages_and_changed_hash_slots_are_refused() {
    let original = artifact(None);
    let signature = codesign::signature(&original).unwrap();
    for offset in [
        4096,
        signature.data_offset as usize - 65,
        original.len() - 1,
    ] {
        let mut damaged = original.clone();
        damaged[offset] ^= 0x80;
        let error = verify_ad_hoc(&damaged).unwrap_err();
        assert!(error.to_string().contains("code page"), "{offset}: {error}");
    }
}

#[test]
fn signature_metadata_cannot_change_what_is_verified() {
    let original = artifact(None);
    let signature = codesign::signature(&original).unwrap().data_offset as usize;
    // Superblob magic, length, count, slot type and directory offset.
    let mut offsets = vec![
        signature,
        signature + 4,
        signature + 8,
        signature + 12,
        signature + 16,
    ];
    // The signature itself is outside its own hashes, so each field needs its
    // own structural check rather than relying on the code-page comparisons.
    for offset in [
        0, 4, 8, 12, 16, 20, 24, 28, 32, 36, 37, 38, 39, 40, 44, 48, 52, 56, 64, 72, 80,
    ] {
        offsets.push(signature + 20 + offset);
    }
    for offset in offsets {
        let mut damaged = original.clone();
        damaged[offset] ^= 0x80;
        assert!(
            verify_ad_hoc(&damaged).is_err(),
            "unvalidated signature field at {offset}"
        );
    }
}

#[test]
fn truncation_extra_bytes_unsigned_files_and_impossible_command_bounds_are_refused() {
    let original = artifact(None);
    let signature = codesign::signature(&original).unwrap().data_offset as usize;
    for length in [
        0,
        4,
        12,
        31,
        32,
        40,
        signature - 1,
        signature,
        original.len() - 1,
    ] {
        assert!(
            verify_ad_hoc(&original[..length]).is_err(),
            "truncation at {length}"
        );
    }
    let mut extra = original.clone();
    extra.push(0);
    assert!(verify_ad_hoc(&extra).is_err());
    assert!(verify_ad_hoc(&macho::without_code_signature(&macho::real_fixture_bytes())).is_err());
    for offset in [16, 20] {
        let mut damaged = original.clone();
        damaged[offset..offset + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(verify_ad_hoc(&damaged).is_err());
    }
}

#[test]
fn correctly_signed_pages_do_not_hide_a_false_payload_digest() {
    let error = verify_ad_hoc(&artifact(Some([0; 32]))).unwrap_err();
    assert!(error.to_string().contains("payload digest"), "{error}");
}
