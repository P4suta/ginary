// SPDX-License-Identifier: MIT OR Apache-2.0
//! An `ebin` or `priv` that is a symlink to an excluded directory was staged.
//!
//! **What went wrong.** A1c closed one half of this. `copy_subtree` began to
//! stat its `from` before reading it, so an `ebin` or a `priv` that was itself
//! a link out of the application became `AssembleError::UnsafeSymlink` instead
//! of a directory of the build machine copied into the artifact. The other half
//! stayed open: `resolve_link` holds a link to the *application* directory,
//! which is the boundary for a link to a file, while a link to a *directory*
//! found inside an `ebin` or `priv` is held to that `ebin` or `priv` so that
//! `ebin/sources -> ../src` cannot step around
//! `assemble::EXCLUDED_APP_DIRS`. The `ebin` and the `priv` themselves were
//! held to the weaker of the two boundaries, so an application whose `priv`
//! *is* a link to its own `src` passed the check: the target is inside the
//! application, and the whole of a directory that is never staged was copied
//! into the artifact under the name `priv`. Staging exited zero and said
//! nothing, which is the same silent leak A1c's fix exists to prevent, with the
//! source tree in place of the build machine.
//!
//! **The input.** A shipment application whose `priv` is a symlink to its own
//! `src`; one whose `priv` is a symlink to a directory *inside* `src`; and one
//! whose `ebin` is a symlink to `src`.
//!
//! **The correct behaviour.** All three are refused, naming the link and the
//! excluded directory it reaches, and nothing that lives under an excluded
//! directory reaches the staged tree. A link to a sibling directory that is
//! *not* excluded — the case A1c deliberately allows — still stages.

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
    /// Writes both trees. `notify` is the only shipment application, and it
    /// has the `src` directory a real `gleam export erlang-shipment` leaves
    /// beside `ebin`.
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let shipment = FakeShipment::new()
            .app_with("notify", "1.0.0", |app| {
                app.priv_file("greeting.txt", b"hello from priv\n")
            })
            .build_in(dir.path().join("shipment"));
        let app = shipment.root.join("notify");
        std::fs::create_dir_all(app.join("src/data")).expect("the src directory");
        std::fs::write(app.join("src/notify.erl"), b"-module(notify).\n").expect("the source");
        std::fs::write(app.join("src/data/table.bin"), b"generated\n").expect("the source data");
        std::fs::create_dir_all(app.join("assets")).expect("the assets directory");
        std::fs::write(app.join("assets/logo.txt"), b"a shared asset\n").expect("the asset");
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

    /// Replaces the application's `priv` with a symlink to `target`.
    #[cfg(unix)]
    fn link_priv_to(&self, target: &str) {
        let priv_dir = self.app().join("priv");
        std::fs::remove_file(priv_dir.join("greeting.txt")).expect("the priv file");
        std::fs::remove_dir(&priv_dir).expect("the priv directory");
        std::os::unix::fs::symlink(target, &priv_dir).expect("the symlink");
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

/// Asserts that nothing the excluded `src` directory holds was staged.
fn assert_no_source_staged(out: &Path) {
    let staged = walk(out);
    assert!(
        !staged.iter().any(|path| path.ends_with("notify.erl")),
        "a file under an excluded directory must not reach the artifact, staged: {staged:?}"
    );
    assert!(
        !staged.iter().any(|path| path.ends_with("table.bin")),
        "a file under an excluded directory must not reach the artifact, staged: {staged:?}"
    );
}

#[cfg(unix)]
#[test]
fn a_priv_that_is_a_symlink_to_the_source_directory_is_refused() {
    let trees = Trees::new();
    trees.link_priv_to("src");

    let error = trees
        .stage()
        .expect_err("a priv that is a link to an excluded directory is refused, not followed");

    match &error {
        AssembleError::ExcludedSymlinkTarget {
            path,
            target,
            excluded,
        } => {
            assert_eq!(path, &trees.app().join("priv"));
            assert!(
                target.ends_with("src"),
                "the target is the link's: {target:?}"
            );
            assert_eq!(excluded, "src");
        }
        other => panic!("expected ExcludedSymlinkTarget, got {other:?}"),
    }
    assert_no_source_staged(&trees.out());
}

#[cfg(unix)]
#[test]
fn a_priv_that_is_a_symlink_inside_the_source_directory_is_refused() {
    let trees = Trees::new();
    trees.link_priv_to("src/data");

    let error = trees
        .stage()
        .expect_err("a priv that is a link into an excluded directory is refused, not followed");

    match &error {
        AssembleError::ExcludedSymlinkTarget { path, excluded, .. } => {
            assert_eq!(path, &trees.app().join("priv"));
            assert_eq!(
                excluded, "src",
                "the excluded directory the link reaches is named, not the last component"
            );
        }
        other => panic!("expected ExcludedSymlinkTarget, got {other:?}"),
    }
    assert_no_source_staged(&trees.out());
}

#[cfg(unix)]
#[test]
fn an_ebin_that_is_a_symlink_to_the_source_directory_is_refused() {
    let trees = Trees::new();
    let ebin = trees.app().join("ebin");
    let src = trees.app().join("src");
    for entry in std::fs::read_dir(&ebin).expect("the ebin") {
        let path = entry.expect("an entry").path();
        std::fs::copy(&path, src.join(path.file_name().expect("a name"))).expect("the copy");
    }
    std::fs::remove_dir_all(&ebin).expect("the ebin directory");
    std::os::unix::fs::symlink("src", &ebin).expect("the symlink");

    let error = trees
        .stage()
        .expect_err("an ebin that is a link to an excluded directory is refused, not followed");

    match &error {
        AssembleError::ExcludedSymlinkTarget { path, excluded, .. } => {
            assert_eq!(path, &ebin);
            assert_eq!(excluded, "src");
        }
        other => panic!("expected ExcludedSymlinkTarget, got {other:?}"),
    }
    assert_no_source_staged(&trees.out());
}

#[cfg(unix)]
#[test]
fn a_priv_that_is_a_symlink_to_a_directory_that_is_not_excluded_still_stages() {
    let trees = Trees::new();
    trees.link_priv_to("assets");

    let staged = trees
        .stage()
        .expect("a link to a staged sibling directory is allowed");

    assert!(
        walk(staged.root()).contains(&"lib/notify/priv/logo.txt".to_owned()),
        "the linked directory is staged as priv, staged: {:?}",
        walk(staged.root())
    );
    assert_no_source_staged(&trees.out());
}
