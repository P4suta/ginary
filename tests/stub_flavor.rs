// SPDX-License-Identifier: MIT OR Apache-2.0
//! What a binary with no trailer does, in each of the two flavors.
//!
//! One binary, two builds. `cargo build` produces the command line tool, and
//! `cargo build --no-default-features` produces the *stub*: the launcher, the
//! payload reader and nothing else — no clap, no TOML, none of the build-side
//! commands. A stub that is run directly is a stub nobody appended a payload
//! to, and the one thing it can usefully say is what it is and which target it
//! is for.
//!
//! The suite runs in both modes: plain `cargo test` covers the full flavor and
//! `mise run test:stub` covers the other, so this file asserts the branch its
//! own build compiled and neither claim is ever untested.

use assert_cmd::Command;
use ginary::target::Target;

/// The sentence a launcher stub with no payload prints.
fn stub_sentence() -> String {
    format!(
        "this is a ginary launcher stub for {}; it carries no payload and no CLI",
        Target::host()
    )
}

#[test]
fn the_launcher_owns_the_sentence_a_payloadless_stub_prints() {
    // Asserted in both flavors, which is the point of the seam: the binary
    // that prints this line is built only by `--no-default-features`, and a
    // sentence only that build could check would be a sentence nothing checks
    // on an ordinary `cargo test`.
    assert_eq!(
        ginary::launcher::no_payload_line(Target::host()),
        stub_sentence()
    );
}

#[test]
fn a_binary_with_no_trailer_answers_for_its_flavor() {
    let assert = Command::cargo_bin("ginary")
        .expect("the `ginary` binary is built for tests")
        .assert()
        .code(2);
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf-8");

    if cfg!(feature = "cli") {
        assert!(
            stderr.contains("Usage:"),
            "the full flavor is the command line tool: {stderr}"
        );
        assert!(
            !stderr.contains("carries no payload"),
            "a ginary with a CLI does not claim to have none: {stderr}"
        );
    } else {
        assert_eq!(
            stderr.trim_end(),
            stub_sentence(),
            "a stub says what it is and which target it is for"
        );
    }
}

#[test]
fn the_stub_sentence_names_the_target_the_marker_does() {
    // The two places a build says which machine it is for: the sentence a
    // payloadless stub prints, built from `Target::host()`, and the `t` field
    // `build.rs` writes into the embedded marker from Cargo's `TARGET`. A
    // build.rs whose table mapped this triple to the wrong name would put one
    // name in the marker and another in this line, so the assertion is that
    // they are the same name rather than that either is *a* name — which
    // `Target::host()` makes true by construction and would prove nothing.
    let host = Target::host();

    assert_eq!(
        ginary::stubid::TARGET_NAME,
        host.name(),
        "build.rs mapped this build's Rust triple to a target this binary does not run on"
    );
    let id = ginary::stubid::scan(&ginary::stubid::GINARY_STUB_ID)
        .expect("this build's own marker is a marker");
    assert_eq!(id.target, host, "and the marker in the binary agrees");
    assert!(
        stub_sentence().contains(&id.target.name()),
        "{}",
        stub_sentence()
    );
}
