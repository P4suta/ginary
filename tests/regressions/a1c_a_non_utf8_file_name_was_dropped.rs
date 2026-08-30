// SPDX-License-Identifier: MIT OR Apache-2.0
//! A file whose name is not valid UTF-8 vanished from the staged tree.
//!
//! **What went wrong.** Three places in `src/assemble.rs` turned a path into
//! text with `to_str()` and treated `None` as "nothing to do": the recursive
//! copy skipped the entry with a bare `continue`, the ERTS exclusion list
//! dropped the program from `--explain`, and the listing filtered the component
//! out of the path it recorded. Staging therefore reported success, the listing
//! never mentioned the file, and the application failed at run time looking for
//! a `priv` file that had been in its source tree all along. CLAUDE.md forbids
//! exactly this: skipping is a reported decision or an error, never a default.
//!
//! **The input.** A `priv` file called `caf\xe9.dat`, which is Latin-1 rather
//! than UTF-8, and a program of the same name in the runtime's `erts-*/bin`.
//!
//! **The correct behaviour.** Staging fails, naming the path it could not
//! represent, rather than producing an artifact that is quietly missing a file.

#![cfg(unix)]

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt as _;
use std::path::{Path, PathBuf};

use ginary::assemble::{self, AssembleError, StageOptions};
use ginary::closure::app_dependency_closure;

use crate::common::fake_otp::{FakeOtp, FakeShipment};

/// The name no `&str` can hold: `café.dat` in Latin-1.
const LATIN1_NAME: &[u8] = b"caf\xe9.dat";

/// A shipment holding one application, a runtime, and an output directory.
struct Trees {
    dir: tempfile::TempDir,
    shipment: PathBuf,
    otp: PathBuf,
}

impl Trees {
    /// Writes both trees. `notify` is the only shipment application.
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let shipment = FakeShipment::new()
            .app_with("notify", "1.0.0", |app| {
                app.priv_file("greeting.txt", b"hello from priv\n")
            })
            .build_in(dir.path().join("shipment"));
        std::fs::create_dir_all(dir.path().join("otp")).expect("the otp root");
        let otp = FakeOtp::new().build_in(dir.path().join("otp"));
        Self {
            shipment: shipment.root.clone(),
            otp: otp.root.clone(),
            dir,
        }
    }

    /// Stages `notify` into `<tmp>/out`.
    fn stage(&self) -> Result<assemble::StagedRoot, AssembleError> {
        let otp = ginary::otp::inspect_root(&self.otp).expect("a usable fake OTP root");
        let set = app_dependency_closure(&self.shipment, &otp.lib, &["notify".to_owned()], &[])
            .expect("the closure resolves");
        assemble::stage(
            &set,
            &otp,
            &StageOptions::default(),
            &self.dir.path().join("out"),
        )
    }
}

/// The path of a file called [`LATIN1_NAME`] inside `dir`.
fn latin1_path(dir: &Path) -> PathBuf {
    dir.join(OsStr::from_bytes(LATIN1_NAME))
}

#[test]
fn a_priv_file_whose_name_is_not_utf8_fails_the_staging() {
    let trees = Trees::new();
    let path = latin1_path(&trees.shipment.join("notify/priv"));
    std::fs::write(&path, b"a file the application reads\n").expect("the latin-1 file");

    let error = trees
        .stage()
        .expect_err("a file that cannot be named is refused rather than dropped");

    let message = error.to_string();
    assert!(
        message.contains("caf"),
        "the error has to name the file it could not represent: {message}"
    );
    assert!(
        message.to_lowercase().contains("utf-8"),
        "the error has to say why the name is a problem: {message}"
    );
}

#[test]
fn a_program_in_the_runtime_bin_whose_name_is_not_utf8_fails_the_staging() {
    let trees = Trees::new();
    let bin = trees.otp.join(format!(
        "erts-{}/bin",
        crate::common::fake_otp::DEFAULT_ERTS_VSN
    ));
    std::fs::write(latin1_path(&bin), b"#!/bin/sh\nexit 0\n").expect("the latin-1 program");

    let error = trees
        .stage()
        .expect_err("a program that cannot be named is refused rather than dropped");

    let message = error.to_string();
    assert!(
        message.contains("caf"),
        "the exclusion list has to name what it could not report: {message}"
    );
}
