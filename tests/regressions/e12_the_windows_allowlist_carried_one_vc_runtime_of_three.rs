// SPDX-License-Identifier: MIT OR Apache-2.0
//! The Windows allowlist named one of the three files the Visual C++
//! redistributable installs, so a healthy artifact reported two findings.
//!
//! **What went wrong.** `verify::WINDOWS_NEEDED_ALLOWLIST` is "the libraries a
//! Windows target guarantees", and it carries `VCRUNTIME140.dll`. The C++
//! runtime that ships in the same package, and that the official Erlang/OTP
//! Windows build links, is three files: `VCRUNTIME140.dll`,
//! `VCRUNTIME140_1.dll` — the exception-handling half, split out for x64 —
//! and `MSVCP140.dll`, the C++ standard library. `ginary verify` on a real
//! Windows artifact reported the two that were missing:
//!
//! ```text
//! ---- a_real_artifact_verifies_clean ----
//! ObjectInfo { path: "erts-17.0.5/bin/beam.smp.dll", …
//!   needed: [… "MSVCP140.dll", "VCRUNTIME140.dll", "VCRUNTIME140_1.dll", …],
//!   issues: [UnexpectedNeeded { needed: "MSVCP140.dll" },
//!            UnexpectedNeeded { needed: "VCRUNTIME140_1.dll" }] }
//! ```
//!
//! (`Windows build and exit-code propagation`
//! <https://github.com/P4suta/ginary/actions/runs/33823103540/job/100869848230>.)
//! The test that printed it does not compare against an empty finding list —
//! it recomputes the expectation with the same allowlist, so both sides agreed
//! and the gap stayed invisible — but `ginary verify` run by a user on a
//! Windows artifact reports two findings that are not defects, which is
//! exactly the noise that teaches a reader to ignore the report.
//!
//! Admitting them is not weakening the gate. The list's own documentation says
//! it names what the platform guarantees, and a machine that has
//! `VCRUNTIME140.dll` has the other two: they are one redistributable, and
//! Erlang/OTP's own Windows installer requires it. `doctor` asks the same
//! question of a `crypto` NIF through its own copy of the list and has the
//! same gap.
//!
//! **The input.** Any Windows artifact carrying an upstream `beam.smp.dll`.
//!
//! **The correct behaviour.** The three files of one redistributable are one
//! entry in the allowlist's mind: naming one and reporting the others says
//! something about the machine that is not true.

#![cfg(feature = "cli")]

use std::path::Path;

use ginary::doctor::{self, CryptoReport};
use ginary::target::Os;
use ginary::verify::{self, needed_is_allowed, platform_allowlist};

use crate::common::fake_otp::FakeOtp;
use crate::common::native::pe_with_imports;
use crate::common::stubfile::PE_MACHINE_AMD64;

/// The three files the Visual C++ redistributable installs, as an import
/// table spells them.
const VC_REDISTRIBUTABLE: [&str; 3] = ["VCRUNTIME140.dll", "VCRUNTIME140_1.dll", "MSVCP140.dll"];

#[test]
fn the_whole_vc_redistributable_is_a_library_a_windows_target_guarantees() {
    let allowlist = platform_allowlist(Os::Windows);
    for name in VC_REDISTRIBUTABLE {
        assert!(
            needed_is_allowed(name, allowlist),
            "`{name}` ships in the same redistributable as the entries beside it, so an artifact \
             needing it needs nothing the platform does not already promise: {allowlist:?}"
        );
    }
}

#[test]
fn the_case_a_pe_import_table_uses_does_not_change_the_answer() {
    // The list is matched case-insensitively, which is what makes it a rule
    // about a Windows file name rather than about one linker's output.
    let allowlist = platform_allowlist(Os::Windows);
    for name in ["vcruntime140_1.dll", "MSVCP140.DLL", "msvcp140.dll"] {
        assert!(
            needed_is_allowed(name, allowlist),
            "a Windows filesystem does not distinguish these spellings: {name}"
        );
    }
}

#[test]
fn the_debug_c_runtime_is_still_reported() {
    // The guard. The debug CRT is not redistributable and is present only
    // where Visual Studio is, so an artifact needing one is the finding this
    // check exists for and must stay a finding.
    let allowlist = platform_allowlist(Os::Windows);
    for name in [
        "MSVCP140D.dll",
        "VCRUNTIME140D.dll",
        "VCRUNTIME140_1D.dll",
        "ucrtbased.dll",
    ] {
        assert!(
            !needed_is_allowed(name, allowlist),
            "`{name}` is the debug C runtime: it is not redistributable and no user's machine \
             is promised to have it"
        );
    }
}

#[test]
fn no_other_platforms_allowlist_gained_a_windows_library() {
    for os in [Os::Linux, Os::Macos] {
        for name in VC_REDISTRIBUTABLE {
            assert!(
                !needed_is_allowed(name, platform_allowlist(os)),
                "the allowlist is a statement about one target: {os} does not promise `{name}`"
            );
        }
    }
    assert!(
        verify::WINDOWS_NEEDED_ALLOWLIST.len() > verify::MACOS_NEEDED_ALLOWLIST.len(),
        "and the Windows list is still the Windows list"
    );
}

// ---------------------------------------------------- and `doctor`'s copy --

/// The `crypto` NIF `doctor` reads, as a Windows installation spells it,
/// importing `libraries` and nothing else.
///
/// `doctor::crypto_report_for` reads a file rather than taking a name, so its
/// copy of the rule is reachable only through an object that really imports
/// something: `verify`'s half can be asked directly and this half cannot.
fn windows_crypto_report(dir: &Path, libraries: &[&str]) -> CryptoReport {
    let otp = FakeOtp::new()
        .with_crypto_for(Os::Windows, &pe_with_imports(PE_MACHINE_AMD64, libraries))
        .build_in(dir);
    doctor::crypto_report_for(Os::Windows, &otp.root)
        .expect("the fixture carries the crypto NIF a Windows installation spells")
}

#[test]
fn a_windows_nif_needing_only_the_redistributable_reports_openssl_linked_in() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let mut libraries = vec!["KERNEL32.dll"];
    libraries.extend(VC_REDISTRIBUTABLE);

    let report = windows_crypto_report(dir.path(), &libraries);

    assert_eq!(
        report.needed, libraries,
        "the fixture has to import what it was asked to, or the verdict below is a statement \
         about an empty list"
    );
    assert!(
        report.statically_linked_openssl,
        "a NIF needing the kernel interface and one redistributable needs nothing a Windows \
         machine does not already have, so its OpenSSL was linked in: {report:?}"
    );
}

#[test]
fn a_windows_nif_that_loads_openssl_is_still_reported_as_loading_it() {
    // The guard. `statically_linked_openssl` is the whole question the report
    // exists to answer, and a rule that admitted anything would answer `true`
    // for the case it was written to catch.
    let dir = tempfile::tempdir().expect("a temporary directory");
    let libraries = ["KERNEL32.dll", "VCRUNTIME140.dll", "LIBCRYPTO-3-X64.dll"];

    let report = windows_crypto_report(dir.path(), &libraries);

    assert!(
        !report.statically_linked_openssl,
        "`LIBCRYPTO-3-X64.dll` is OpenSSL itself: a machine without it cannot load this NIF, \
         which is exactly what the report is asked: {report:?}"
    );
}

#[test]
fn a_windows_nif_needing_the_debug_runtime_is_not_a_statically_linked_one() {
    // The second guard, and the one that keeps this rule from growing into
    // "anything whose name begins with MSVC": the debug C runtime is not
    // redistributable, so a NIF needing it needs something no user's machine
    // is promised to have.
    let dir = tempfile::tempdir().expect("a temporary directory");
    let libraries = ["KERNEL32.dll", "MSVCP140D.dll"];

    let report = windows_crypto_report(dir.path(), &libraries);

    assert!(
        !report.statically_linked_openssl,
        "`MSVCP140D.dll` exists only where Visual Studio is installed: {report:?}"
    );
}

#[test]
fn the_universal_crt_family_is_still_one_library_to_the_report() {
    // The `api-ms-win-crt-*` forwarding libraries are the Universal CRT split
    // across several dozen files, and the rule that admits them is beside the
    // one this milestone widened; asserting it here is what keeps the two
    // from being merged into one list somebody has to enumerate.
    let dir = tempfile::tempdir().expect("a temporary directory");
    let libraries = [
        "KERNEL32.dll",
        "api-ms-win-crt-runtime-l1-1-0.dll",
        "MSVCP140.dll",
    ];

    let report = windows_crypto_report(dir.path(), &libraries);

    assert!(
        report.statically_linked_openssl,
        "the Universal CRT ships with Windows whichever of its files a NIF names: {report:?}"
    );
}
