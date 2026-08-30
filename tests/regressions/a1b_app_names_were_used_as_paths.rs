// SPDX-License-Identifier: MIT OR Apache-2.0
//! An application name out of an `.app` file was interpolated into a path.
//!
//! **What went wrong.** Every lookup built its paths by interpolating the name
//! it was given — `<shipment>/<name>/ebin/<name>.app` and
//! `<otp_lib>/<name>-<vsn>` — and nothing checked that the name was a
//! directory name. `{applications, ['../../escape']}` parses, so the closure
//! walked out of both trees, and an absolute name such as `'/etc'` left them
//! altogether, because `Path::join` with an absolute path discards the prefix.
//! Assembly would then have been handed an `ebin` outside the shipment and the
//! OTP library.
//!
//! **The input.** A dependency named `../../escape`, one named `/etc`, and an
//! empty `--root`.
//!
//! **The correct behaviour.** A name that is not a directory name is rejected
//! before it is used, with `ClosureError::InvalidAppName` naming the chain that
//! asked for it, whether it came from a seed or from an `.app` file.

use ginary::closure::{ClosureError, app_dependency_closure};

use crate::common::fake_otp::{FakeOtp, FakeShipment};

/// Builds both trees with one application depending on `dependency`.
fn closure_over(dependency: &str, roots: &[&str]) -> Result<Vec<String>, ClosureError> {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let shipment = FakeShipment::new()
        .app("app", "1.0.0", &[dependency])
        .build_in(dir.path().join("shipment"));
    std::fs::create_dir_all(dir.path().join("otp")).expect("the otp root");
    let otp = FakeOtp::new().build_in(dir.path().join("otp"));
    let roots: Vec<String> = roots.iter().map(|name| (*name).to_owned()).collect();

    app_dependency_closure(&shipment.root, &otp.lib(), &roots, &[]).map(|set| set.names())
}

#[test]
fn a_dependency_name_that_is_a_path_is_rejected() {
    for name in ["../../escape", "/etc", "a/b"] {
        match closure_over(name, &["app"]) {
            Err(ClosureError::InvalidAppName { name: found, .. }) => assert_eq!(found, name),
            other => panic!("`{name}` must be refused as a name, got {other:?}"),
        }
    }
}

#[test]
fn the_rejected_name_carries_the_chain_that_asked_for_it() {
    match closure_over("../../escape", &["app"]) {
        Err(ClosureError::InvalidAppName { requested_by, .. }) => {
            assert_eq!(requested_by, ["app", "../../escape"]);
        }
        other => panic!("expected InvalidAppName, got {other:?}"),
    }
}

#[test]
fn an_empty_root_is_rejected_rather_than_looked_up() {
    match closure_over("kernel", &[""]) {
        Err(ClosureError::InvalidAppName { name, requested_by }) => {
            assert_eq!(name, "");
            assert_eq!(requested_by, [""]);
        }
        other => panic!("expected InvalidAppName, got {other:?}"),
    }
}
