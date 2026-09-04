// SPDX-License-Identifier: MIT OR Apache-2.0
//! Two doctor tests planted `crypto.so` and asked the host probe, which on
//! Windows looks for `crypto.dll`.
//!
//! **What went wrong.** E11 split the NIF's name into
//! `ginary::platform::crypto_nif` and gave `doctor::crypto_report_for` an
//! `os` parameter, so that both answers could be asserted on one machine. The
//! two tests that build a fake installation and then ask about it were not
//! moved with it: they write the file by hand, under the unix name, on every
//! host, and then call `doctor::crypto_report`, which is the *host's* answer:
//!
//! ```rust,ignore
//! .app_with("crypto", "5.9.2", |app| app.priv_file("lib/crypto.so", &test_binary()))
//! …
//! assert!(doctor::crypto_report(&full.root).is_some(), "the installation carries a crypto NIF");
//! ```
//!
//! ```text
//! ---- crypto_is_reported_exactly_when_the_installation_carries_it ----
//! panicked at tests\doctor.rs:511:5: the installation carries a crypto NIF
//!
//! ---- the_crypto_nif_is_found_and_read ----
//! panicked at tests\doctor.rs:532:51: the installation has a crypto NIF
//! ```
//!
//! (`Windows build and exit-code propagation`
//! <https://github.com/P4suta/ginary/actions/runs/33823103540/job/100869848230>.)
//! The probe was right and the fixture was wrong, which is the reverse of what
//! the failure reads like: an installation that carries no NIF is exactly what
//! a Windows tree with a `crypto.so` in it *is*.
//!
//! **The input.** Any host whose NIF suffix is not `.so`.
//!
//! **The correct behaviour.** The fixture composes the name from the same rule
//! the probe reads it from, so the two cannot drift, and the builder takes the
//! `os` it is building for — which lets the answer for all three be asserted
//! here rather than only the host's on whichever runner happens to be up.

#![cfg(feature = "cli")]

use ginary::doctor;
use ginary::platform::{self, HOST};
use ginary::target::Os;

use crate::common::fake_otp::{CRYPTO_APP, FakeOtp, crypto_nif_under_priv};
use crate::common::repack::test_binary;

/// The three operating systems ginary packages for.
const EVERY_OS: [Os; 3] = [Os::Linux, Os::Macos, Os::Windows];

#[test]
fn the_fixture_plants_the_nif_the_probe_for_that_platform_looks_for() {
    for os in EVERY_OS {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let otp = FakeOtp::new()
            .with_crypto_for(os, &test_binary())
            .build_in(dir.path());

        let report = doctor::crypto_report_for(os, &otp.root).unwrap_or_else(|| {
            panic!(
                "a {os} installation carrying `{}` has a crypto NIF",
                platform::crypto_nif(os)
            )
        });

        assert_eq!(
            report.path,
            otp.app_dir(CRYPTO_APP).join(platform::crypto_nif(os)),
            "and it is the file the platform rule names"
        );
    }
}

#[test]
fn the_fixture_the_host_probe_is_pointed_at_is_the_hosts_own_spelling() {
    // The failing pair in one sentence: `doctor::crypto_report` asks about the
    // host, so a fixture built for it has to be built for the host too.
    let dir = tempfile::tempdir().expect("a temporary directory");
    let otp = FakeOtp::new()
        .with_crypto_for(HOST, &test_binary())
        .build_in(dir.path());

    assert!(
        doctor::crypto_report(&otp.root).is_some(),
        "the installation carries a crypto NIF, spelled `{}`",
        platform::crypto_nif(HOST)
    );
}

#[test]
fn one_platforms_nif_is_not_another_platforms() {
    // The negative half, and the reason the positive one is not vacuous: a
    // builder that planted every name would satisfy the tests above without
    // fixing anything.
    let dir = tempfile::tempdir().expect("a temporary directory");
    let otp = FakeOtp::new()
        .with_crypto_for(Os::Windows, &test_binary())
        .build_in(dir.path());

    assert_eq!(
        doctor::crypto_report_for(Os::Linux, &otp.root),
        None,
        "a tree holding `{}` and nothing else carries no NIF a Linux artifact could load",
        platform::crypto_nif(Os::Windows)
    );
}

#[test]
fn the_relative_name_the_builder_uses_is_the_platform_rule_without_its_priv() {
    for os in EVERY_OS {
        assert_eq!(
            format!("priv/{}", crypto_nif_under_priv(os)),
            platform::crypto_nif(os),
            "the fixture's half and the probe's half are one rule: {os}"
        );
    }
    assert_eq!(crypto_nif_under_priv(Os::Windows), "lib/crypto.dll");
    assert_eq!(crypto_nif_under_priv(Os::Linux), "lib/crypto.so");
}
