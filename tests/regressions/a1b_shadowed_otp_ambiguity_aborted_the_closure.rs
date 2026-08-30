// SPDX-License-Identifier: MIT OR Apache-2.0
//! A stale second OTP version aborted a closure that never read the OTP copy.
//!
//! **What went wrong.** `locate` asked the OTP library for a version *before*
//! it looked in the shipment, so `ClosureError::AmbiguousOtpApp` escaped for an
//! application the shipment shadows — one whose OTP directories the closure
//! would never have read. The documented resolution order is the shipment
//! first and the OTP library second, and a shipment hit makes the OTP side
//! irrelevant to the artifact, so a leftover `crypto-5.9.3` beside
//! `crypto-5.9.2` failed a build it could not have affected.
//!
//! **The input.** A shipment holding its own `crypto` beside an OTP `lib`
//! holding both `crypto-5.9.2` and `crypto-5.9.3`.
//!
//! **The correct behaviour.** The closure succeeds, `crypto` is read from the
//! shipment, and both ignored OTP directories are named in a warning. The
//! ambiguity is still an error when the shipment does *not* hold the
//! application, which `tests/closure.rs` pins.

use std::path::Path;

use ginary::closure::{AppSource, app_dependency_closure};

use crate::common::fake_otp::{FakeOtp, FakeShipment};

/// Copies `<lib>/<from>` to `<lib>/<to>`, `.app` file and all.
fn copy_app_dir(lib: &Path, from: &str, to: &str) {
    let ebin = lib.join(to).join("ebin");
    std::fs::create_dir_all(&ebin).expect("the copied ebin");
    for entry in std::fs::read_dir(lib.join(from).join("ebin")).expect("the source ebin") {
        let entry = entry.expect("a readable entry");
        std::fs::copy(entry.path(), ebin.join(entry.file_name())).expect("the copied file");
    }
}

#[test]
fn a_shadowed_application_with_two_otp_versions_is_a_warning_not_an_error() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let shipment = FakeShipment::new()
        .app("app", "1.0.0", &["crypto"])
        .app("crypto", "9.9.9", &[])
        .build_in(dir.path().join("shipment"));
    std::fs::create_dir_all(dir.path().join("otp")).expect("the otp root");
    let otp = FakeOtp::new()
        .app("crypto", "5.9.2", &["kernel", "stdlib"])
        .build_in(dir.path().join("otp"));
    copy_app_dir(&otp.lib(), "crypto-5.9.2", "crypto-5.9.3");

    let set = match app_dependency_closure(&shipment.root, &otp.lib(), &["app".to_owned()], &[]) {
        Ok(set) => set,
        Err(error) => panic!("the shipment copy wins, so the OTP versions cannot matter: {error}"),
    };

    let crypto = set.get("crypto").expect("`crypto` is in the closure");
    assert_eq!(crypto.source, AppSource::Shipment);
    assert_eq!(crypto.vsn, "9.9.9");
    assert_eq!(set.warnings.len(), 1, "{:?}", set.warnings);
    let warning = &set.warnings[0];
    assert!(
        warning.contains("crypto-5.9.2") && warning.contains("crypto-5.9.3"),
        "the warning must name every OTP directory that was ignored:\n{warning}"
    );
    assert!(
        warning.contains(
            &shipment
                .app_dir("crypto")
                .join("ebin")
                .display()
                .to_string()
        ),
        "the warning must name the copy that was used:\n{warning}"
    );
}
