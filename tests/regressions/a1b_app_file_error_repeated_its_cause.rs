// SPDX-License-Identifier: MIT OR Apache-2.0
//! `ClosureError::AppFile` said the same thing three times.
//!
//! **What went wrong.** The variant's `Display` interpolated `{source}` *and*
//! its `Error::source` returned that same error, so `src/main.rs`, which
//! prints one line per link of the chain, repeated the parse failure three
//! times and the path twice. The one sentence a reader needs — which file
//! ginary could not read — was buried in three renderings of the same thing.
//!
//! **The input.** A dependency whose `.app` file is truncated mid-term, read
//! through `ginary closure`.
//!
//! **The correct behaviour.** Each link describes its own layer: the closure
//! error names the file, and the reason is reachable through `source()`, which
//! is where the chain printer looks for it.

use std::error::Error as _;

use assert_cmd::Command;
use ginary::closure::{ClosureError, app_dependency_closure};

use crate::common::fake_otp::{FakeOtp, FakeShipment};

#[test]
fn the_closure_error_names_the_file_once_and_leaves_the_reason_to_its_source() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let shipment = FakeShipment::new()
        .app("app", "1.0.0", &["dep"])
        .app("dep", "0.4.0", &[])
        .build_in(dir.path().join("shipment"));
    std::fs::create_dir_all(dir.path().join("otp")).expect("the otp root");
    let otp = FakeOtp::new().build_in(dir.path().join("otp"));
    let broken = shipment.app_file("dep");
    std::fs::write(&broken, b"{application, dep,\n").expect("the truncated file");

    let error = match app_dependency_closure(&shipment.root, &otp.lib(), &["app".to_owned()], &[]) {
        Ok(set) => panic!("a truncated `.app` file must fail, got {:?}", set.names()),
        Err(error) => error,
    };
    assert!(
        matches!(&error, ClosureError::AppFile { path, .. } if path == &broken),
        "{error:?}"
    );

    let rendered = error.to_string();
    assert_eq!(
        rendered.matches(&broken.display().to_string()).count(),
        1,
        "the file is named once, not once per layer:\n{rendered}"
    );
    assert!(
        !rendered.contains("expected a term"),
        "the parse failure belongs to the cause, not to this layer:\n{rendered}"
    );
    let cause = error.source().expect("the reason is still reachable");
    assert!(
        cause.to_string().contains("expected a term"),
        "nothing may be lost by shortening the message: {cause}"
    );
}

#[test]
fn the_command_prints_the_file_on_its_own_line() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let shipment = FakeShipment::new()
        .app("app", "1.0.0", &["dep"])
        .app("dep", "0.4.0", &[])
        .build_in(dir.path().join("shipment"));
    std::fs::create_dir_all(dir.path().join("otp")).expect("the otp root");
    let otp = FakeOtp::new().build_in(dir.path().join("otp"));
    let broken = shipment.app_file("dep");
    std::fs::write(&broken, b"{application, dep,\n").expect("the truncated file");

    let assert = Command::cargo_bin("ginary")
        .expect("the `ginary` binary is built for tests")
        .arg("closure")
        .arg(&shipment.root)
        .arg("--otp-root")
        .arg(&otp.root)
        .args(["--root", "app"])
        .assert()
        .code(1);
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf-8");

    assert_eq!(
        stderr.lines().next(),
        Some(
            format!(
                "error: cannot read the application file `{}`",
                broken.display()
            )
            .as_str()
        ),
        "the first line says which file, and nothing else:\n{stderr}"
    );
    assert_eq!(
        stderr.matches("cannot read the application file").count(),
        1,
        "one layer, one line:\n{stderr}"
    );
}
