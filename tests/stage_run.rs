// SPDX-License-Identifier: MIT OR Apache-2.0
//! Booting a staged root.
//!
//! Everything else in the suite asserts on trees and values. This file is the
//! only place that asks the question the whole project is about: does the
//! directory `assemble::stage` writes actually run? It stages the zero-hex
//! fixture `hello_ffi` against the host OTP installation and launches it
//! through [`crate::common::erl::run_staged`], which is a hermetic subset of
//! the launch contract ADR 0003 records — it clears the environment where the
//! launcher will scrub a denylist from an inherited one.
//!
//! Every test is gated on `gleam` and `erl`, and a machine without them reports
//! a skip. `GINARY_REQUIRE_TOOLCHAIN=1` turns that skip into a failure, which
//! is what CI sets: this is the coverage that would be worth the most and cost
//! the least to lose silently.

mod common;

use std::path::Path;

use ginary::assemble::{Category, StageOptions, StagedRoot};
use ginary::closure::app_dependency_closure;
use tempfile::TempDir;

use crate::common::erl::{crash_dump_path, run_cwd, run_staged};
use crate::common::fixture::FixtureProject;
use crate::common::tools::{Toolchain, require_tools};

/// The application the fixture ships, and the `-root` the closure starts from.
const APP: &str = "hello_ffi";

/// A staged `hello_ffi`, and the temporary directory holding everything.
struct Staged {
    dir: TempDir,
    root: StagedRoot,
}

impl Staged {
    /// The staged tree.
    fn root(&self) -> &Path {
        self.root.root()
    }

    /// A fresh `HOME` for one run, so two runs cannot see each other.
    fn home(&self, name: &str) -> std::path::PathBuf {
        let home = self.dir.path().join(name);
        std::fs::create_dir_all(&home).expect("a home directory");
        home
    }
}

/// Exports the fixture, resolves the closure and stages it.
///
/// The three steps `ginary build` will run in this order, with nothing faked:
/// a real `gleam export erlang-shipment`, the real host OTP root, and the real
/// assembly. A failure in any of them is a failure of this test.
fn stage_hello_ffi(tools: &Toolchain) -> Staged {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let project = FixtureProject::copy(APP, dir.path());
    let shipment = project.export_shipment_with(tools.path("gleam"));

    let otp = ginary::otp::discover(None).expect("the host OTP installation");
    let set = app_dependency_closure(&shipment, &otp.lib, &[APP.to_owned()], &[])
        .expect("the fixture's closure resolves");

    let root = ginary::assemble::stage(
        &set,
        &otp,
        &StageOptions::default(),
        &dir.path().join("staged"),
    )
    .expect("the fixture stages");

    Staged { dir, root }
}

#[test]
fn a_staged_hello_ffi_prints_its_arguments_and_its_priv_file() {
    let Some(tools) = require_tools(&["gleam", "erl"]) else {
        return;
    };
    let staged = stage_hello_ffi(&tools);
    let home = staged.home("run");

    let output = run_staged(staged.root(), APP, &["3", "a", "b"], &home);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        stdout.contains("args=3 a b"),
        "`-extra` did not reach init:get_plain_arguments/0:\n{stdout}\n--- stderr ---\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains("hello from priv"),
        "code:priv_dir/1 did not find the staged priv:\n{stdout}"
    );
    assert!(
        stdout.contains(&format!("cwd={}", run_cwd(&home).display())),
        "the application did not start in the directory it was given:\n{stdout}"
    );
    assert_eq!(
        output.status.code(),
        Some(3),
        "the first argument is the exit code"
    );
}

#[test]
fn a_staged_hello_ffi_exits_zero_when_the_first_argument_is_zero() {
    let Some(tools) = require_tools(&["gleam", "erl"]) else {
        return;
    };
    let staged = stage_hello_ffi(&tools);

    let output = run_staged(staged.root(), APP, &["0"], &staged.home("run"));

    assert_eq!(output.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&output.stdout).contains("args=0"));
}

#[test]
fn a_crash_exits_one_and_leaves_no_dump_in_the_working_directory() {
    let Some(tools) = require_tools(&["gleam", "erl"]) else {
        return;
    };
    let staged = stage_hello_ffi(&tools);
    let home = staged.home("run");

    let output = run_staged(staged.root(), APP, &["--crash"], &home);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(output.status.code(), Some(1), "stderr:\n{stderr}");
    assert!(
        stderr.contains("runtime error"),
        "Gleam's own error report did not reach standard error:\n{stderr}"
    );
    assert!(
        !run_cwd(&home).join("erl_crash.dump").exists(),
        "a crash dump in the user's working directory is litter; ERL_CRASH_DUMP points into HOME"
    );
    if crash_dump_path(&home).exists() {
        // Not required — the runtime writes one only for some failures — but
        // if there is one it belongs where the launch contract put it.
        assert!(crash_dump_path(&home).is_file());
    }
}

#[test]
fn the_staged_root_holds_no_sources_and_the_kernel_the_boot_file_names() {
    let Some(tools) = require_tools(&["gleam", "erl"]) else {
        return;
    };
    let staged = stage_hello_ffi(&tools);

    // Scoped to the top level of an application, which is the rule assembly
    // actually has: `ebin` and `priv` are copied and the rest of the
    // application directory is not. A name matched at any depth would forbid
    // `priv/mibs/*.bin`, which a real `snmp` needs at run time.
    for app in staged.root.apps() {
        for excluded in ginary::assemble::EXCLUDED_APP_DIRS {
            let prefix = format!("{}/{excluded}/", app.dir);
            assert!(
                !staged
                    .root
                    .files()
                    .iter()
                    .any(|file| file.path.starts_with(&prefix)),
                "`{prefix}` was staged out of a real OTP tree"
            );
        }
    }

    let kernel = staged
        .root
        .boot_refs()
        .iter()
        .find(|dir| dir.starts_with("kernel-"))
        .expect("the boot file names a kernel version")
        .clone();
    assert!(
        staged
            .root()
            .join("lib")
            .join(&kernel)
            .join("ebin")
            .is_dir(),
        "the boot file requires lib/{kernel}/ebin and the staged tree must hold exactly it"
    );
}

#[test]
fn running_the_staged_root_does_not_change_a_byte_of_it() {
    let Some(tools) = require_tools(&["gleam", "erl"]) else {
        return;
    };
    let staged = stage_hello_ffi(&tools);

    let before: Vec<Vec<u8>> = staged
        .root
        .files()
        .iter()
        .map(|file| std::fs::read(staged.root().join(&file.path)).expect("a staged file"))
        .collect();

    let first = run_staged(staged.root(), APP, &["0"], &staged.home("run-one"));
    let second = run_staged(staged.root(), APP, &["0"], &staged.home("run-two"));

    assert_eq!(first.status.code(), Some(0));
    assert_eq!(second.status.code(), Some(0));

    let after: Vec<Vec<u8>> = staged
        .root
        .files()
        .iter()
        .map(|file| std::fs::read(staged.root().join(&file.path)).expect("a staged file"))
        .collect();
    assert_eq!(
        before, after,
        "a cache entry is immutable; running out of it must not write to it"
    );
}

#[test]
fn the_staged_hello_ffi_has_bytes_in_every_category_an_artifact_needs() {
    let Some(tools) = require_tools(&["gleam", "erl"]) else {
        return;
    };
    let staged = stage_hello_ffi(&tools);
    let totals = staged.root.bytes_by_category();

    for category in [
        Category::ErtsBinary,
        Category::Boot,
        Category::OtpBeam,
        Category::GleamBeam,
        Category::AppResource,
        Category::Priv,
    ] {
        let (bytes, files) = totals
            .get(&category)
            .copied()
            .unwrap_or_else(|| panic!("nothing was staged as {category}"));
        assert!(files > 0 && bytes > 0, "{category} is empty");
    }

    // The first real size number the project has. `docs/dev/log/A1c.md`
    // records it; printing it here is how it is kept honest.
    eprintln!("{}", staged.root.explain());
}
