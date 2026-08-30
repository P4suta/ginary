// SPDX-License-Identifier: MIT OR Apache-2.0
//! An application's `ebin` or `priv` being itself a symlink was followed.
//!
//! **What went wrong.** `copy_tree` checked every entry it found *inside* a
//! directory against the application boundary, but never the two directories it
//! was called with. It began with `create_dir(to)` and `read_dir(from)`, and
//! `read_dir` follows a symlink, so an application whose `priv` (or `ebin`) was
//! a link to somewhere else on the build machine had that directory copied into
//! the artifact whole. Staging exited zero and said nothing, which is precisely
//! what `AssembleError::UnsafeSymlink` exists to prevent — its own rustdoc says
//! a link out of the application directory would "pull an arbitrary file of the
//! build machine into the artifact".
//!
//! **The input.** A shipment application whose `priv` is a symlink to a
//! directory outside the shipment, and one whose `ebin` is.
//!
//! **The correct behaviour.** Both are `AssembleError::UnsafeSymlink` naming
//! the link, and nothing outside the application reaches the staged tree.

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

    /// The application directory of the one shipment application.
    fn app(&self) -> PathBuf {
        self.shipment.join("notify")
    }

    /// A directory outside the shipment holding one file nobody asked for.
    fn outside(&self, name: &str) -> PathBuf {
        let dir = self.dir.path().join(name);
        std::fs::create_dir_all(&dir).expect("the outside directory");
        std::fs::write(dir.join("secrets.txt"), b"not part of the app\n").expect("the secret");
        dir
    }

    /// Stages `notify` into `<tmp>/out`.
    fn stage(&self) -> Result<assemble::StagedRoot, AssembleError> {
        let otp = ginary::otp::inspect_root(&self.otp).expect("a usable fake OTP root");
        let set = app_dependency_closure(&self.shipment, &otp.lib, &["notify".to_owned()], &[])
            .expect("the closure resolves");
        assemble::stage(&set, &otp, &StageOptions::default(), &self.out())
    }

    /// Where the staging root goes.
    fn out(&self) -> PathBuf {
        self.dir.path().join("out")
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

#[cfg(unix)]
#[test]
fn a_priv_that_is_a_symlink_out_of_the_application_is_refused() {
    let trees = Trees::new();
    let outside = trees.outside("outside_priv");
    std::fs::remove_file(trees.app().join("priv/greeting.txt")).expect("the priv file");
    std::fs::remove_dir(trees.app().join("priv")).expect("the priv directory");
    std::os::unix::fs::symlink(&outside, trees.app().join("priv")).expect("the symlink");

    let error = trees
        .stage()
        .expect_err("a priv that leaves the application is refused rather than followed");

    match &error {
        AssembleError::UnsafeSymlink { path, .. } => {
            assert_eq!(path, &trees.app().join("priv"));
        }
        other => panic!("expected UnsafeSymlink, got {other:?}"),
    }
    assert!(
        !walk(&trees.out())
            .iter()
            .any(|path| path.contains("secrets")),
        "a file that was never inside the application must not reach the artifact"
    );
}

#[cfg(unix)]
#[test]
fn an_ebin_that_is_a_symlink_out_of_the_application_is_refused() {
    let trees = Trees::new();
    let outside = trees.outside("outside_ebin");
    let ebin = trees.app().join("ebin");
    for entry in std::fs::read_dir(&ebin).expect("the ebin") {
        let path = entry.expect("an entry").path();
        std::fs::copy(&path, outside.join(path.file_name().expect("a name"))).expect("the copy");
    }
    std::fs::remove_dir_all(&ebin).expect("the ebin directory");
    std::os::unix::fs::symlink(&outside, &ebin).expect("the symlink");

    let error = trees
        .stage()
        .expect_err("an ebin that leaves the application is refused rather than followed");

    match &error {
        AssembleError::UnsafeSymlink { path, .. } => assert_eq!(path, &ebin),
        other => panic!("expected UnsafeSymlink, got {other:?}"),
    }
    assert!(
        !walk(&trees.out())
            .iter()
            .any(|path| path.contains("secrets")),
        "a file that was never inside the application must not reach the artifact"
    );
}
