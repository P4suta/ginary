// SPDX-License-Identifier: MIT OR Apache-2.0
//! Four stub tests asked about `windows-aarch64`, which is not a target
//! ginary has a name for.
//!
//! **What went wrong.** `tests/stub.rs` needs "a target that is not this
//! one" twice: once for the target gate, which refuses a stub whose marker
//! names somebody else's target, and once for the header gate, which believes
//! the object header over the marker. Both reached for a helper that flips
//! the architecture and keeps everything else — `same_format_other_arch`, and
//! `foreign_target_for` beside it — a rule with exactly one answer on a
//! Windows host, and that answer is not in `ginary::target::ALL`:
//!
//! ```text
//! ---- a_stub_whose_marker_names_another_target_is_refused ----
//! expected StubError::TargetMismatch, got Marker { path: "...\\cross", source:
//!   UnknownTarget { name: "windows-aarch64",
//!                   source: Unsupported("windows-aarch64") } }
//!
//! ---- a_marker_that_disagrees_with_the_file_is_refused_by_the_header ----
//! expected StubError::ObjectMismatch, got Marker { path: "...\\liar", source:
//!   UnknownTarget { name: "windows-aarch64", ... } }
//!
//! ---- a_cross_build_with_no_stub_names_every_path_it_searched ----
//! the cross build says what is missing rather than that it is impossible:
//! error: cannot resolve the targets to build
//!   caused by: `windows-aarch64` is not a target; expected one of `host`, `all`,
//!   `linux-x86_64-gnu`, ..., `windows-x86_64`
//!
//! ---- the_build_command_takes_the_stub_it_is_given ----
//! `--stub` is a path the build reports on rather than an unknown flag: <the same>
//! ```
//!
//! (`Windows build and exit-code propagation`
//! <https://github.com/P4suta/ginary/actions/runs/33823103540/job/100869848230>,
//! `tests/stub.rs:467`, `:488`, `:736` and `:776`.) Every one of the four is
//! the same sentence: the marker parser and the command line both refuse the
//! name before the claim under test is reached, so four gates went unexercised
//! on the one platform whose PE branch nothing else covers.
//!
//! **The input.** Any host whose architecture is the only one its operating
//! system has a published target for — `windows-x86_64` today, and any future
//! single-architecture row of `ginary::target::ALL`.
//!
//! **The correct behaviour.** Two rules, because the two gates want two
//! different things.
//!
//! A test about the *target* gate wants a supported target that is not the
//! host; the container format is irrelevant, because the target gate is
//! reached before the object header is ever read. So the helper answers with
//! a member of `ginary::target::ALL`.
//!
//! A test about the *header* gate wants a file whose header disagrees with its
//! marker, and that is a claim about a *machine* rather than about a second
//! target. So the marker and the `want` both stay the host — supported on
//! every runner — and the fixture's own machine field is rewritten instead, by
//! `stubfile::for_other_machine`.

#![cfg(feature = "cli")]

use ginary::native::inspect_object_bytes;
use ginary::stub::{self, StubError};
use ginary::stubid;
use ginary::target::{ALL, Target};

use crate::common::stubfile::{
    Marker, for_other_machine, foreign_target_for, ginary_bin, other_supported_target, stub_copy_of,
};

#[test]
fn every_host_has_a_supported_target_that_is_not_itself() {
    for host in ALL {
        for (rule, other) in [
            ("other_supported_target", other_supported_target(host)),
            ("foreign_target_for", foreign_target_for(host)),
        ] {
            assert_ne!(
                other, host,
                "{rule}({host}) has to answer with a target that is not the host, or the test \
                 that uses it proves nothing"
            );
            assert_eq!(
                other.name().parse::<Target>(),
                Ok(other),
                "{rule}({host}) answered `{}`, which `ginary::target::ALL` does not carry, so a \
                 marker written from it does not scan back and a `--target` naming it is refused \
                 before the claim under test is reached",
                other.name()
            );
        }
    }
}

#[test]
fn a_marker_naming_the_other_target_scans_back_as_that_target() {
    for host in ALL {
        let other = other_supported_target(host);
        let bytes = Marker::for_target(&other).bytes();

        let id = stubid::scan(&bytes).unwrap_or_else(|error| {
            panic!("the marker a stub test writes for {host} has to scan back: {error}")
        });

        assert_eq!(
            id.target, other,
            "and it names the target it was written for"
        );
    }
}

#[test]
fn the_header_gate_fixture_is_this_hosts_object_for_the_other_machine() {
    let host = Target::host();
    let image = std::fs::read(ginary_bin()).expect("the ginary binary is readable");
    let before = inspect_object_bytes(&image).expect("the ginary binary is an object");

    let other = for_other_machine(&image);
    let after = inspect_object_bytes(&other).expect("and so is the rewritten copy");

    assert_eq!(
        before.machine,
        host.arch.as_str(),
        "the fixture starts as this host's own binary"
    );
    assert_eq!(
        before.format, after.format,
        "only the machine field is rewritten; the container format is what makes the header gate \
         reach the comparison at all"
    );
    assert_ne!(
        after.machine, before.machine,
        "the header gate's subject is a file whose header names a machine the marker does not"
    );
}

#[test]
fn a_file_whose_header_names_another_machine_is_refused_by_the_header() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let host = Target::host();
    let image = for_other_machine(&std::fs::read(ginary_bin()).expect("the ginary binary"));
    let path = stub_copy_of(dir.path(), "liar", &image, &Marker::host().bytes());

    let error = stub::verify(&path, &host).expect_err("the header is believed, not the marker");

    assert!(
        matches!(&error, StubError::ObjectMismatch { want, .. } if *want == host),
        "expected StubError::ObjectMismatch, got {error:?}"
    );
}
