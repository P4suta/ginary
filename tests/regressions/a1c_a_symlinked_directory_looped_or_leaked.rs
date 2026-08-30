// SPDX-License-Identifier: MIT OR Apache-2.0
//! A symlinked *directory* inside an application was recursed into blindly.
//!
//! **What went wrong.** The recursive copy resolved a symlink, asked
//! `resolved.is_dir()`, and recursed — with no record of the directories it had
//! already entered and no second look at the structural exclusion. Two things
//! followed. A self-referential link (`priv/loop -> .`) recursed until the
//! destination path hit `ENAMETOOLONG`, and the error that came out named an
//! innocent file rather than the link, because the copy's `Io` error carried
//! its *source* path and the failure was on the destination. And a link across
//! the application (`ebin/sources -> ../src`) staged the application's sources,
//! which `docs/dev/architecture.md` and two tests all say cannot happen — none
//! of them used a symlink, so the hole was untested.
//!
//! **The input.** `priv/loop` pointing at `.`, and `ebin/sources` pointing at
//! the application's own `src`.
//!
//! **The correct behaviour.** The first is refused as a cycle naming the link;
//! the second is refused because the target is outside the subtree being
//! copied. Neither produces a staged tree.

#![cfg(unix)]

use std::path::{Path, PathBuf};

use ginary::assemble::{self, AssembleError, StageOptions};
use ginary::closure::app_dependency_closure;

use crate::common::fake_otp::{FakeOtp, FakeShipment};

/// A shipment holding one application, a runtime, and an output directory.
struct Trees {
    dir: tempfile::TempDir,
    shipment: PathBuf,
    otp: PathBuf,
}

impl Trees {
    /// Writes both trees. `notify` is the only shipment application, and it has
    /// a `src` directory, as a real shipment application does.
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let shipment = FakeShipment::new()
            .app_with("notify", "1.0.0", |app| {
                app.priv_file("greeting.txt", b"hello from priv\n")
            })
            .build_in(dir.path().join("shipment"));
        std::fs::create_dir_all(shipment.root.join("notify/src")).expect("the src directory");
        std::fs::write(
            shipment.root.join("notify/src/leak.gleam"),
            b"pub fn main() { Nil }\n",
        )
        .expect("the source file");
        std::fs::create_dir_all(dir.path().join("otp")).expect("the otp root");
        let otp = FakeOtp::new().build_in(dir.path().join("otp"));
        Self {
            shipment: shipment.root.clone(),
            otp: otp.root.clone(),
            dir,
        }
    }

    /// The application directory of the one shipment application.
    fn app(&self) -> PathBuf {
        self.shipment.join("notify")
    }

    /// Where the staging root goes.
    fn out(&self) -> PathBuf {
        self.dir.path().join("out")
    }

    /// Stages `notify` into `<tmp>/out`.
    fn stage(&self) -> Result<assemble::StagedRoot, AssembleError> {
        let otp = ginary::otp::inspect_root(&self.otp).expect("a usable fake OTP root");
        let set = app_dependency_closure(&self.shipment, &otp.lib, &["notify".to_owned()], &[])
            .expect("the closure resolves");
        assemble::stage(&set, &otp, &StageOptions::default(), &self.out())
    }
}

/// Every file under a directory, as `/`-separated relative paths.
fn walk(root: &Path) -> Vec<String> {
    let mut found = Vec::new();
    if !root.exists() {
        return found;
    }
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("a readable directory") {
            let path = entry.expect("a readable entry").path();
            if path.is_dir() {
                stack.push(path);
            } else {
                found.push(
                    path.strip_prefix(root)
                        .expect("under the root")
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
        }
    }
    found.sort();
    found
}

#[test]
fn a_symlink_that_points_at_its_own_directory_is_refused_by_name() {
    let trees = Trees::new();
    std::os::unix::fs::symlink(".", trees.app().join("priv/loop")).expect("the symlink");

    let error = trees
        .stage()
        .expect_err("a link that loops is refused rather than recursed into");

    let message = error.to_string();
    assert!(
        message.contains("loop"),
        "the error has to name the link that loops, not a file it happened to reach: {message}"
    );
    assert!(
        !message.contains("too long"),
        "the loop is caught before the filesystem complains about the path length: {message}"
    );
}

#[test]
fn a_symlink_out_of_the_ebin_into_the_sources_is_refused() {
    let trees = Trees::new();
    std::os::unix::fs::symlink("../src", trees.app().join("ebin/sources")).expect("the symlink");

    let error = trees
        .stage()
        .expect_err("a link into the application's sources is refused rather than followed");

    match &error {
        AssembleError::UnsafeSymlink { path, .. } => {
            assert_eq!(path, &trees.app().join("ebin/sources"));
        }
        other => panic!("expected UnsafeSymlink, got {other:?}"),
    }
    assert!(
        !walk(&trees.out())
            .iter()
            .any(|path| path.ends_with(".gleam")),
        "`src` never travels, whatever a symlink says"
    );
}
