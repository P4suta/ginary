// SPDX-License-Identifier: MIT OR Apache-2.0
//! `doctor`'s crypto report looked for `priv/lib/crypto.so` and read it as an
//! ELF, so on Windows it answered "this installation has no crypto" about an
//! installation that has one.
//!
//! **What went wrong.** `crypto_report` is how `ginary doctor` answers the one
//! portability question a packaged application really turns on: does the
//! runtime's `crypto` NIF need an OpenSSL the target machine has to supply.
//! Two things in it were fixed to unix — the file name, and the header the
//! list of needed libraries is read out of — and the function returns `None`
//! when either fails, so a healthy Windows installation was reported as
//! carrying no crypto at all:
//!
//! ```text
//! ---- the_host_installation_answers_the_crypto_question ----
//! a real OTP carries crypto
//! ```
//!
//! (`Windows build and exit-code propagation`
//! <https://github.com/P4suta/ginary/actions/runs/33751715516/job/100636537290>,
//! `tests/doctor.rs:579`.)
//!
//! This is the silent skip `CLAUDE.md` forbids, wearing a `None`: the question
//! `doctor` exists to answer went unanswered and unremarked on a whole
//! platform.
//!
//! **The input.** Any OTP installation whose NIFs are not ELF. A Windows OTP
//! spells the file `priv/lib/crypto.dll` and it is a PE; macOS spells it
//! `crypto.so`, as OTP builds NIFs there, and it is a Mach-O.
//!
//! **The correct behaviour.** The file name is a fact about a platform,
//! `ginary::platform::crypto_nif`, and `crypto_report_for` takes the operating
//! system it is asking about so that both answers are asserted on one machine.
#![cfg(feature = "cli")]

use std::path::Path;

use ginary::doctor::crypto_report_for;
use ginary::platform::crypto_nif;
use ginary::target::{Arch, Libc, Os, Target};

use crate::common::native::object_for;

/// Builds `<root>/lib/crypto-5.9.2/<`[`crypto_nif`]`>` holding an object for
/// `target`, and answers the root.
fn installation_with_crypto(dir: &Path, target: Target) -> std::path::PathBuf {
    let root = dir.join("otp");
    let nif = root.join("lib/crypto-5.9.2").join(crypto_nif(target.os));
    std::fs::create_dir_all(nif.parent().expect("a parent")).expect("the crypto priv directory");
    std::fs::write(&nif, object_for(&target)).expect("a crypto NIF");
    root
}

#[test]
fn the_nif_is_named_the_way_the_platform_names_a_shared_library() {
    assert_eq!(
        [
            crypto_nif(Os::Linux),
            crypto_nif(Os::Macos),
            crypto_nif(Os::Windows),
        ],
        [
            "priv/lib/crypto.so",
            "priv/lib/crypto.so",
            "priv/lib/crypto.dll"
        ],
        "OTP builds NIFs with the `.so` suffix on macOS too; only Windows differs"
    );
}

#[test]
fn a_windows_installation_answers_the_crypto_question() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let windows = Target::new(Os::Windows, Arch::X86_64, Libc::None);
    let root = installation_with_crypto(dir.path(), windows);

    let report = crypto_report_for(Os::Windows, &root)
        .expect("an installation carrying a crypto NIF has a crypto report");

    assert!(
        report.path.starts_with(&root),
        "the report names the file it read: {:?}",
        report.path
    );
    assert!(
        report.path.ends_with("crypto.dll"),
        "and it is the file that platform ships: {:?}",
        report.path
    );
}

#[test]
fn a_linux_installation_still_answers_it_the_way_it_always_did() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let linux = Target::new(Os::Linux, Arch::X86_64, Libc::Gnu);
    let root = installation_with_crypto(dir.path(), linux);

    let report =
        crypto_report_for(Os::Linux, &root).expect("a Linux installation carries a crypto NIF");

    assert!(report.path.ends_with("crypto.so"), "{:?}", report.path);
}

#[test]
fn an_installation_that_carries_no_crypto_still_answers_none() {
    // The other side of the rule: `None` has to keep meaning "there is no
    // crypto here", which is what a runtime assembled from ERTS binaries
    // alone legitimately looks like.
    let dir = tempfile::tempdir().expect("a temporary directory");
    let root = dir.path().join("bare");
    std::fs::create_dir_all(root.join("lib")).expect("an empty lib directory");

    for os in [Os::Linux, Os::Macos, Os::Windows] {
        assert!(crypto_report_for(os, &root).is_none(), "{os:?}");
    }
}
