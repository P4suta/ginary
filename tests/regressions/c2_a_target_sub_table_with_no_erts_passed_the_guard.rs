// SPDX-License-Identifier: MIT OR Apache-2.0
//! A cross target whose sub-table named no runtime slipped the pre-export guard.
//!
//! **What went wrong.** `check_cross_erts` asked whether
//! `[tools.ginary.target.<name>]` *existed*, not whether it said where the
//! runtime comes from. Three of that table's four keys are recorded rather
//! than acted on today — `otp_variant`, `native` and `codesign` — so a table
//! holding only one of them satisfied a guard whose whole subject is `erts`.
//! `TargetConfig::erts_spec` then answered `ErtsSourceSpec::Host` for it, the
//! build carried on past the export, and the failure arrived minutes later as
//! C1's target-mismatch sentence: the host's runtime is for the host, and this
//! build is not. That is the wrong sentence in the wrong place — the guard
//! exists because a fault in `gleam.toml` must be reported before `gleam
//! export` spends the minutes.
//!
//! **The input.** A project whose manifest carries
//! `[tools.ginary.target."<foreign>"]` with `otp_variant = "dynamic"` and no
//! `erts`, and a stub for that target so that the build reaches the guard at
//! all.
//!
//! **The correct behaviour.** `BundleError::CrossErtsNotConfigured`, before
//! the export, quoting the table to write — the same answer a target with no
//! sub-table at all earns, because a table that names no runtime names no
//! runtime.

#![cfg(unix)]

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use ginary::target::Target;

use crate::common::project::TempProject;
use crate::common::repack::{foreign_machine, foreign_target, patch_elf_machine};
use crate::common::stubfile::{self, Marker};

/// A stub for `target` that passes every gate in `stub::verify`.
///
/// This test run's own `ginary`, with `e_machine` rewritten to the other
/// architecture and the identity marker rewritten to match. The interpreter
/// stays this machine's glibc loader, which is what makes the file a *dynamic
/// gnu* binary for the foreign machine rather than a static one — exactly the
/// target the sub-table below names. Fabricated rather than cross-built so
/// that the guard is asserted on every machine, with or without `cross`.
fn fabricated_stub(dir: &Path, target: &Target) -> PathBuf {
    let bytes = std::fs::read(stubfile::ginary_bin()).expect("the ginary binary is readable");
    let mut bytes = patch_elf_machine(&bytes, foreign_machine());
    let marker = Marker::for_target(target).bytes();
    match stubfile::offsets(&bytes).as_slice() {
        [] => bytes.extend_from_slice(&marker),
        [offset] => bytes[*offset..*offset + stubfile::MARKER_LEN].copy_from_slice(&marker),
        many => panic!("the ginary binary carries {} markers", many.len()),
    }
    stubfile::write_executable(dir, "cross-stub", &bytes)
}

/// A project whose only ginary configuration is `table`.
fn project_with(table: &str) -> TempProject {
    TempProject::new(&format!("name = \"hello\"\nversion = \"1.0.0\"\n\n{table}"))
}

/// Runs `ginary build` in `project` with a stub directory and a cache that are
/// both empty, so the only stub in the search is the one `--stub` names.
fn build_in(project: &Path, empty: &Path, args: &[&str]) -> assert_cmd::assert::Assert {
    let stubs = empty.join("stubs");
    let cache = empty.join("cache");
    std::fs::create_dir_all(&stubs).expect("an empty stub directory");
    std::fs::create_dir_all(&cache).expect("an empty cache");
    let mut command = Command::cargo_bin("ginary").expect("the `ginary` binary is built for tests");
    command
        .current_dir(project)
        .env(ginary::stub::STUB_DIR_VAR, &stubs)
        .env("GINARY_CACHE_DIR", &cache)
        .args(args)
        .assert()
}

#[test]
fn a_sub_table_that_names_no_runtime_is_refused_before_the_export() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let target = foreign_target();
    let stub = fabricated_stub(dir.path(), &target);
    let project = project_with(&format!(
        "[tools.ginary.target.\"{}\"]\notp_variant = \"dynamic\"\n",
        target.name()
    ));

    let assert = build_in(
        project.root(),
        dir.path(),
        &[
            "build",
            "--skip-export",
            "--target",
            &target.name(),
            "--stub",
            &stub.display().to_string(),
        ],
    )
    .failure();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf-8");

    assert!(
        stderr.contains(&format!("[tools.ginary.target.\"{}\"]", target.name()))
            && stderr.contains("erts = \"dir:"),
        "a table with no `erts` earns the same dictation an absent table does: {stderr}"
    );
    assert!(
        !stderr.contains("cannot obtain the Gleam shipment"),
        "the guard answers before the export is even looked for, which is the whole reason it \
         runs where it does: {stderr}"
    );
}
