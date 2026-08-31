// SPDX-License-Identifier: MIT OR Apache-2.0
//! The identity marker: what this build embeds, and what the scanner makes of
//! bytes that hold none, two, or one that is malformed.
//!
//! Almost every case here is a byte vector rather than a file. `stubid::scan`
//! reads bytes and nothing else, so a fixture built by hand pins the record
//! exactly — the needle at a known offset, a field with a value no build would
//! produce, a padding byte that is not zero. The one case that must be a real
//! file is the first: the claim that *this* binary carries exactly one marker
//! is a claim about what the compiler and the linker did, and nothing a test
//! fabricates can stand in for it.

mod common;

use ginary::manifest::FORMAT_VERSION;
use ginary::stubid::{self, Flavor, GINARY_STUB_ID, MARKER_LEN, StubId, StubIdError};
use ginary::target::{ParseTargetError, Target};

use crate::common::stubfile::{
    Marker, ginary_bin, marker_from_body, needle, noise, offsets, with_markers,
};

/// The flavor a binary built with the current feature set carries.
///
/// `full` when the `cli` feature is on, `stub` when it is not. The test suite
/// runs both ways — plain `cargo test` and `mise run test:stub` — and this is
/// the one value that differs between them.
const THIS_FLAVOR: Flavor = if cfg!(feature = "cli") {
    Flavor::Full
} else {
    Flavor::Stub
};

/// The identity this build's own binary should report.
fn this_binary() -> StubId {
    StubId {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        target: Target::host(),
        format_version: FORMAT_VERSION,
        flavor: THIS_FLAVOR,
    }
}

// ------------------------------------------------------- the embedded id --

#[test]
fn the_binary_this_test_run_built_carries_exactly_one_marker() {
    let bytes = std::fs::read(ginary_bin()).expect("the ginary binary is readable");

    let found = offsets(&bytes);

    assert_eq!(
        found.len(),
        1,
        "a ginary binary carries one identity marker, at one offset: {found:?}"
    );
}

#[test]
fn scanning_this_builds_own_binary_reports_its_identity() {
    let bytes = std::fs::read(ginary_bin()).expect("the ginary binary is readable");

    let id = stubid::scan(&bytes).expect("the ginary binary is a stub");

    assert_eq!(id, this_binary());
}

#[test]
fn the_embedded_constant_scans_to_the_same_identity_as_the_binary() {
    // The constant and the binary are one claim seen twice: if the linker
    // dropped the static, the file scan above fails; if the const fn wrote the
    // wrong text, both fail with the same message.
    let id = stubid::scan(&GINARY_STUB_ID).expect("the constant is a marker");

    assert_eq!(id, this_binary());
}

#[test]
fn the_marker_begins_with_the_needle_and_the_four_fields() {
    let expected = format!(
        "v={};t={};f={FORMAT_VERSION};k={THIS_FLAVOR}",
        env!("CARGO_PKG_VERSION"),
        Target::host().name()
    );
    let head = needle();

    assert_eq!(
        &GINARY_STUB_ID[..head.len()],
        &head[..],
        "the marker opens with GINARY-STUB-ID and a NUL"
    );
    let body_end = head.len() + expected.len();
    assert_eq!(
        std::str::from_utf8(&GINARY_STUB_ID[head.len()..body_end]).expect("the body is text"),
        expected
    );
    assert_eq!(
        GINARY_STUB_ID[body_end], 0,
        "the body is closed by a NUL of its own"
    );
}

#[test]
fn the_marker_is_zero_after_the_terminating_nul() {
    let head = needle();
    let body_end = GINARY_STUB_ID[head.len()..]
        .iter()
        .position(|byte| *byte == 0)
        .map(|offset| head.len() + offset)
        .expect("the body is terminated");

    let padding = &GINARY_STUB_ID[body_end + 1..];

    assert_eq!(GINARY_STUB_ID.len(), MARKER_LEN);
    assert!(
        padding.iter().all(|byte| *byte == 0),
        "the padding decides whether two builds of one ginary are the same bytes: {padding:?}"
    );
}

// ------------------------------------------------------------ the scanner --

#[test]
fn bytes_with_no_marker_are_not_a_stub() {
    let bytes = noise(64 * 1024, 0xf00d);

    let error = stubid::scan(&bytes).expect_err("noise is not a ginary binary");

    assert_eq!(error, StubIdError::NotAStub);
}

#[test]
fn a_buffer_shorter_than_the_needle_is_not_a_stub() {
    let error = stubid::scan(b"GINARY").expect_err("six bytes are not a marker");

    assert_eq!(error, StubIdError::NotAStub);
}

#[test]
fn two_markers_are_ambiguous_even_when_they_agree() {
    let marker = Marker::host().bytes();
    let bytes = with_markers(&[marker, marker]);

    let error = stubid::scan(&bytes).expect_err("a file with two identities has none");

    assert_eq!(error, StubIdError::Ambiguous { count: 2 });
}

#[test]
fn a_marker_that_runs_past_the_end_is_truncated() {
    let mut bytes = noise(512, 0x1234);
    let offset = bytes.len();
    bytes.extend_from_slice(&Marker::host().bytes()[..MARKER_LEN - 1]);

    let error = stubid::scan(&bytes).expect_err("a marker is 128 bytes or it is not one");

    assert_eq!(error, StubIdError::Truncated { offset });
}

#[test]
fn a_body_with_no_terminating_nul_is_truncated() {
    // 113 bytes of body: the needle is 15, so the last byte of the marker is
    // the last byte of the body and there is nowhere for the NUL to go.
    let body = vec![b'v'; MARKER_LEN - needle().len()];
    let mut marker = [0u8; MARKER_LEN];
    marker[..needle().len()].copy_from_slice(&needle());
    marker[needle().len()..].copy_from_slice(&body);
    let bytes = with_markers(&[marker]);
    let offset = offsets(&bytes)[0];

    let error = stubid::scan(&bytes).expect_err("an unterminated body is not a record");

    assert_eq!(error, StubIdError::Truncated { offset });
}

#[test]
fn a_body_that_is_not_utf8_is_refused() {
    let mut body = Marker::host().body().into_bytes();
    body.push(0xff);
    let bytes = with_markers(&[marker_from_body(&body)]);
    let offset = offsets(&bytes)[0];

    let error = stubid::scan(&bytes).expect_err("a marker is text");

    assert_eq!(error, StubIdError::NotUtf8 { offset });
}

#[test]
fn a_marker_with_a_missing_field_names_it() {
    let bytes = with_markers(&[marker_from_body(b"v=0.1.0;f=1;k=full")]);

    let error = stubid::scan(&bytes).expect_err("a marker names four fields");

    assert_eq!(error, StubIdError::MissingField { field: "t" });
}

#[test]
fn a_marker_with_an_unknown_field_names_it() {
    let body = format!("{};x=1", Marker::host().body());
    let bytes = with_markers(&[marker_from_body(body.as_bytes())]);

    let error = stubid::scan(&bytes).expect_err("an unknown field is not ignored");

    assert_eq!(
        error,
        StubIdError::UnknownKey {
            key: "x".to_owned()
        }
    );
}

#[test]
fn a_marker_naming_a_target_ginary_has_no_name_for_is_typed() {
    let bytes = with_markers(&[Marker::host().target("plan9-mips").bytes()]);

    let error = stubid::scan(&bytes).expect_err("`plan9-mips` is not a target");

    assert_eq!(
        error,
        StubIdError::UnknownTarget {
            name: "plan9-mips".to_owned(),
            source: ParseTargetError::Unsupported("plan9-mips".to_owned()),
        }
    );
}

#[test]
fn a_format_version_that_is_not_a_number_is_typed() {
    let bytes = with_markers(&[Marker::host().format("one").bytes()]);

    let error = stubid::scan(&bytes).expect_err("the format version is a number");

    assert_eq!(
        error,
        StubIdError::BadFormatVersion {
            value: "one".to_owned()
        }
    );
}

#[test]
fn a_flavor_that_is_neither_full_nor_stub_is_typed() {
    let bytes = with_markers(&[Marker::host().flavor("lite").bytes()]);

    let error = stubid::scan(&bytes).expect_err("there are two flavors");

    assert_eq!(
        error,
        StubIdError::UnknownFlavor {
            value: "lite".to_owned()
        }
    );
}

#[test]
fn a_marker_whose_padding_is_not_zero_is_refused() {
    let mut marker = Marker::host().bytes();
    marker[MARKER_LEN - 1] = b'!';
    let bytes = with_markers(&[marker]);
    let offset = offsets(&bytes)[0];

    let error = stubid::scan(&bytes).expect_err("the padding is part of the record");

    assert_eq!(
        error,
        StubIdError::NotPadded {
            offset,
            byte: MARKER_LEN - 1,
        }
    );
}

#[test]
fn a_marker_of_the_other_flavor_scans_as_that_flavor() {
    // The one field that differs between `cargo build` and
    // `cargo build --no-default-features`, read from a marker rather than from
    // this build, so that the assertion holds in both modes.
    let bytes = with_markers(&[Marker::host().flavor("stub").bytes()]);

    let id = stubid::scan(&bytes).expect("a stub-flavored marker is a marker");

    assert_eq!(id.flavor, Flavor::Stub);
    assert_eq!(id.flavor.to_string(), "stub");
    assert_eq!(Flavor::Full.as_str(), "full");
}

#[test]
fn the_scanner_reads_a_marker_for_a_target_that_is_not_the_host() {
    let target: Target = "windows-x86_64".parse().expect("a target name");
    let bytes = with_markers(&[Marker::for_target(&target).flavor("stub").bytes()]);

    let id = stubid::scan(&bytes).expect("a cross stub's marker is readable here");

    assert_eq!(
        id,
        StubId {
            version: env!("CARGO_PKG_VERSION").to_owned(),
            target,
            format_version: FORMAT_VERSION,
            flavor: Flavor::Stub,
        }
    );
}

#[test]
fn the_marker_errors_say_what_is_wrong_with_the_file() {
    let cases = [
        StubIdError::NotAStub,
        StubIdError::Ambiguous { count: 2 },
        StubIdError::Truncated { offset: 4096 },
        StubIdError::NotUtf8 { offset: 4096 },
        StubIdError::MissingField { field: "t" },
        StubIdError::UnknownKey {
            key: "x".to_owned(),
        },
        StubIdError::UnknownTarget {
            name: "plan9-mips".to_owned(),
            source: ParseTargetError::Unsupported("plan9-mips".to_owned()),
        },
        StubIdError::BadFormatVersion {
            value: "one".to_owned(),
        },
        StubIdError::UnknownFlavor {
            value: "lite".to_owned(),
        },
        StubIdError::NotPadded {
            offset: 4096,
            byte: 127,
        },
    ];

    let rendered = cases
        .iter()
        .map(std::string::ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n");

    insta::assert_snapshot!("stub_id_error_messages", rendered);
}
