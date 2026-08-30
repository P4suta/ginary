// SPDX-License-Identifier: MIT OR Apache-2.0
//! An ambiguous `optional_applications` entry aborted the whole closure.
//!
//! **What went wrong.** The optional edge was probed with the same fallible
//! `locate` the required edges use, and the probe's `?` let
//! `ClosureError::AmbiguousOtpApp` escape. An `optional_applications` entry
//! that does not resolve is never an error — that is what makes it optional —
//! but a broken OTP library turned one into a hard failure of a build that
//! does not need the application at all.
//!
//! **The input.** An application whose `optional_applications` names `crypto`,
//! against an OTP `lib` holding both `crypto-5.9.2` and `crypto-5.9.3`.
//!
//! **The correct behaviour.** The optional dependency counts as unresolvable:
//! it lands in `AppSet::skipped_optional` with its requester, a warning says
//! why it was skipped and names the candidate directories, and the closure
//! succeeds.

use std::path::Path;

use ginary::closure::app_dependency_closure;

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
fn an_optional_application_with_two_otp_versions_is_skipped_not_an_error() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let shipment = FakeShipment::new()
        .app_with("app", "1.0.0", |app| app.optional(&["crypto"]))
        .build_in(dir.path().join("shipment"));
    std::fs::create_dir_all(dir.path().join("otp")).expect("the otp root");
    let otp = FakeOtp::new()
        .app("crypto", "5.9.2", &["kernel", "stdlib"])
        .build_in(dir.path().join("otp"));
    copy_app_dir(&otp.lib(), "crypto-5.9.2", "crypto-5.9.3");

    let set = match app_dependency_closure(&shipment.root, &otp.lib(), &["app".to_owned()], &[]) {
        Ok(set) => set,
        Err(error) => {
            panic!("an optional dependency that does not resolve is never an error: {error}")
        }
    };

    assert_eq!(set.names(), ["app", "kernel", "stdlib"]);
    assert_eq!(
        set.skipped_optional,
        vec![("crypto".to_owned(), "app".to_owned())]
    );
    assert_eq!(set.warnings.len(), 1, "{:?}", set.warnings);
    let warning = &set.warnings[0];
    assert!(
        warning.contains("crypto-5.9.2") && warning.contains("crypto-5.9.3"),
        "skipping is a reported decision, and the report names the candidates:\n{warning}"
    );
}
