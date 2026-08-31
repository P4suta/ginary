// SPDX-License-Identifier: MIT OR Apache-2.0
//! `erts_extra_bins` entries were joined onto two paths without being checked.
//!
//! **What went wrong.** `[tools.ginary] erts_extra_bins` and its command line
//! twin `--extra-bin` name programs to copy out of the runtime's
//! `erts-<vsn>/bin`. Both halves of that copy were built by joining the name
//! onto a directory — `otp.erts_bin.join(name)` for the source and
//! `bin.join(name)` for the destination — and nothing checked that the name
//! was a file name. `ProjectConfig` validates the project `name` for exactly
//! this reason, and `closure` validates `otp_applications`, but the two lists
//! of program names were passed through as typed. A `gleam.toml` holding
//! `erts_extra_bins = ["../../../victim"]` therefore made `ginary build`
//! truncate a file outside the staging root — outside the project — and the
//! build reported success.
//!
//! **The input.** Three: a staged tree asked for `../../../victim`, a
//! `gleam.toml` whose table holds the same name, and a `--extra-bin` flag
//! holding it.
//!
//! **The correct behaviour.** A program name is a file name. Anything else is
//! refused by the configuration, by the merge and by staging itself, and the
//! file outside the tree is untouched.

use std::path::Path;

use ginary::assemble::{self, AssembleError, StageOptions};
use ginary::config::{BuildFlags, BuildOptions, ConfigError, ProjectConfig};
use ginary::otp::OtpInfo;

use crate::common::fake_otp::{FakeOtp, FakeOtpRoot, FakeShipment, FakeShipmentRoot};

/// The name that walks out of `erts-<vsn>/bin` in both directions.
const ESCAPING_BIN: &str = "../../../victim";

/// What the file outside the tree holds, before and after.
const VICTIM: &[u8] = b"IMPORTANT USER DATA\n";

/// A shipment and a runtime side by side, with the output under `work`.
struct Trees {
    dir: tempfile::TempDir,
    shipment: FakeShipmentRoot,
    otp: FakeOtpRoot,
}

impl Trees {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let shipment = FakeShipment::new()
            .app("notify", "1.0.0", &[])
            .build_in(dir.path().join("shipment"));
        std::fs::create_dir_all(dir.path().join("otp")).expect("the otp root");
        let otp = FakeOtp::new().build_in(dir.path().join("otp"));
        std::fs::create_dir_all(dir.path().join("work")).expect("the work directory");
        Self { dir, shipment, otp }
    }

    /// The directory the staging root and its temporary sibling live in.
    fn work(&self) -> std::path::PathBuf {
        self.dir.path().join("work")
    }

    fn otp_info(&self) -> OtpInfo {
        ginary::otp::inspect_root(&self.otp.root).expect("the fake root is a usable OTP root")
    }

    fn stage(&self, opts: &StageOptions) -> Result<(), AssembleError> {
        let roots = ["notify".to_owned()];
        let set = ginary::closure::app_dependency_closure(
            &self.shipment.root,
            &self.otp.lib(),
            &roots,
            &[],
        )
        .expect("the closure should resolve");
        assemble::stage(&set, &self.otp_info(), opts, &self.work().join("out")).map(|_| ())
    }
}

#[test]
fn an_extra_binary_name_that_is_a_path_is_refused_and_touches_nothing_outside_the_tree() {
    let trees = Trees::new();

    // The source the copy would read: `erts-<vsn>/bin/../../../victim` is
    // `<tmp>/victim`, so the copy has something to succeed with.
    let source = trees.dir.path().join("victim");
    std::fs::write(&source, b"a program\n").expect("the source");
    // The destination it would write: the staging root is built beside
    // `<work>/out`, so `bin/../../../victim` lands in `<work>`.
    let victim = trees.work().join("victim");
    std::fs::write(&victim, VICTIM).expect("the file outside the tree");

    let result = trees.stage(&StageOptions {
        extra_bins: vec![ESCAPING_BIN.to_owned()],
        ..StageOptions::default()
    });

    // The damage first: the whole point is that the file outside the staging
    // root is not touched, whatever staging then reports.
    assert_eq!(
        std::fs::read(&victim).expect("the file outside the tree is still there"),
        VICTIM,
        "staging wrote through a name that is not a file name"
    );
    let error = result.expect_err("a program name that is a path is refused rather than copied");
    match &error {
        AssembleError::UnusableExtraBinary { name } => assert_eq!(name, ESCAPING_BIN),
        other => panic!("expected UnusableExtraBinary, got {other:?}"),
    }
}

#[test]
fn a_table_whose_erts_extra_bins_holds_a_path_is_refused_by_name_and_file() {
    let manifest = Path::new("/w/app/gleam.toml");
    let text =
        format!("name = \"app\"\n\n[tools.ginary]\nerts_extra_bins = [\"{ESCAPING_BIN}\"]\n");

    let error = ProjectConfig::from_toml(&text, manifest)
        .expect_err("a program name that is a path is not a program name");

    match &error {
        ConfigError::ExtraBin { path, name } => {
            assert_eq!(path, manifest);
            assert_eq!(name, ESCAPING_BIN);
        }
        other => panic!("expected ConfigError::ExtraBin, got {other:?}"),
    }
    let message = error.to_string();
    assert!(
        message.contains("erts_extra_bins") && message.contains(ESCAPING_BIN),
        "the message must name the key and the value: {message}"
    );
}

#[test]
fn an_extra_bin_flag_that_is_a_path_is_refused_by_the_merge() {
    let root = Path::new("/w/app");
    let config = ProjectConfig::from_toml("name = \"app\"\n", &root.join("gleam.toml"))
        .expect("a manifest with nothing in it but a name");
    let flags = BuildFlags {
        start: root.to_path_buf(),
        extra_bins: vec![ESCAPING_BIN.to_owned()],
        ..BuildFlags::default()
    };

    let error = BuildOptions::merge(root, &config, &flags)
        .expect_err("a flag that is a path is refused with the table");

    match &error {
        ConfigError::ExtraBinFlag { name } => assert_eq!(name, ESCAPING_BIN),
        other => panic!("expected ConfigError::ExtraBinFlag, got {other:?}"),
    }
    assert!(
        error.to_string().contains("--extra-bin"),
        "the message must name the flag the user typed: {error}"
    );
}
