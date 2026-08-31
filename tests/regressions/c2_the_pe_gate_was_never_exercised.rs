// SPDX-License-Identifier: MIT OR Apache-2.0
//! Nothing exercised the Windows half of `stub::verify`'s object gate.
//!
//! **What went wrong.** `check_object` has two branches that read a file
//! header, and only the ELF one was ever driven. The PE branch — the
//! `object::read::File::parse`, the `BinaryFormat::Pe` accept, `arch_of` and
//! the `describe_object` sentence — was reachable from no test on any machine
//! this suite runs on: `tests/stub.rs` used its Windows target for the
//! *search* and for the `NotFound` snapshot, and built its `ObjectMismatch`
//! by hand. Both outcomes were therefore unguarded. Dropping `pe` from
//! `object`'s feature list, or adding an architecture to `arch_of` and getting
//! it wrong, would have turned every Windows stub into `NotAnObject` or let a
//! stub for the wrong machine through, and the whole suite would still have
//! been green.
//!
//! **The input.** A hand-written PE32+ — see `common::stubfile::pe_bytes` —
//! carrying a marker for `windows-x86_64`, once with the matching COFF machine
//! and once with `IMAGE_FILE_MACHINE_ARM64`.
//!
//! **The correct behaviour.** The first verifies and reports the identity in
//! its marker; the second is `ObjectMismatch`, and the sentence says what the
//! file really is rather than repeating what its marker claimed.

#![cfg(unix)]
#![cfg(feature = "cli")]

use ginary::manifest::FORMAT_VERSION;
use ginary::stub::{self, StubError};
use ginary::stubid::Flavor;
use ginary::target::Target;

use crate::common::stubfile::{
    Marker, PE_MACHINE_AMD64, PE_MACHINE_ARM64, VERSION, pe_with_marker,
};

/// The one Windows target ginary names.
fn windows() -> Target {
    "windows-x86_64".parse().expect("a target name")
}

#[test]
fn a_pe_whose_machine_matches_its_marker_verifies() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let target = windows();
    let path = pe_with_marker(
        dir.path(),
        "ginary-stub.exe",
        PE_MACHINE_AMD64,
        &Marker::for_target(&target).flavor("stub").bytes(),
    );

    let id = stub::verify(&path, &target).expect("a PE for the machine its marker names is a stub");

    assert_eq!(id.version, VERSION);
    assert_eq!(id.target, target);
    assert_eq!(id.format_version, FORMAT_VERSION);
    assert_eq!(id.flavor, Flavor::Stub);
}

#[test]
fn a_pe_for_another_machine_is_refused_by_its_header() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let target = windows();
    let path = pe_with_marker(
        dir.path(),
        "liar.exe",
        PE_MACHINE_ARM64,
        &Marker::for_target(&target).bytes(),
    );

    let error = stub::verify(&path, &target).expect_err("the COFF machine is believed");

    assert!(
        matches!(&error, StubError::ObjectMismatch { path: named, want, .. }
            if *named == path && *want == target),
        "expected StubError::ObjectMismatch, got {error:?}"
    );
    let message = error.to_string();
    assert!(
        message.contains("a PE for aarch64"),
        "the message says what the file really is: {message}"
    );
}
