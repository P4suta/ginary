// SPDX-License-Identifier: MIT OR Apache-2.0
//! The release gate read `Cargo.toml` with no existence check, so a tree
//! missing it failed with `sed`'s own message — in the runner's locale — and
//! said nothing about which of the three records was gone.
//!
//! **What went wrong.** `scripts/ci/version-consistency.sh` guards the manifest
//! with an explicit `-f` test and an actionable sentence, and reads `Cargo.toml`
//! with none:
//!
//! ```text
//! $ GINARY_VERSION_ROOT=$PWD/nocargo bash scripts/ci/version-consistency.sh v1.2.3
//! sed: /tmp/.../nocargo/Cargo.toml を読み込めません: そのようなファイルやディレクトリはありません
//! exit=2
//! ```
//!
//! The exit code is right and the message is somebody else's. The script's own
//! header promises exit 2 means "a record cannot be read", and the whole value
//! of that promise is that the script names the record.
//!
//! **The input.** Any tree the check is pointed at that has no `Cargo.toml`:
//! a checkout that failed half way, a release job whose working directory is
//! not what it thinks, a fixture. `set -euo pipefail` makes `sed`'s failure the
//! script's exit status, so nothing after it runs and nothing else is printed.
//!
//! **The correct behaviour.** The symmetric guard: the script says which record
//! it could not read and what that record is for, in its own voice, before it
//! hands the path to a tool with a locale.

// A unix file: it spawns `scripts/ci/version-consistency.sh`.
#![cfg(unix)]

use std::process::Command;

use crate::common::repo::root;
use crate::common::version::{ROOT_VAR, VersionRoot};

/// The version the fixture tree is built around.
const FIXTURE_VERSION: &str = "1.2.3";

#[test]
fn a_tree_with_no_cargo_toml_is_named_by_the_script_and_not_by_sed() {
    let tree = VersionRoot::released(FIXTURE_VERSION).without("Cargo.toml");
    let output = Command::new(root().join("scripts/ci/version-consistency.sh"))
        .arg(format!("v{FIXTURE_VERSION}"))
        .current_dir(root())
        .env_remove("GITHUB_REF_NAME")
        .env(ROOT_VAR, tree.path())
        .output()
        .expect("spawn version-consistency.sh");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

    assert_eq!(
        output.status.code(),
        Some(2),
        "a record that cannot be read is exit 2, which the script's header promises: {stderr}"
    );
    assert!(
        stderr.starts_with("version-consistency:"),
        "the failure has to be the script's own sentence. `sed`'s message is written in the \
         runner's locale and names neither which of the three records is missing nor what it is \
         for: {stderr}"
    );
    assert!(
        stderr.contains("Cargo.toml"),
        "the failure names the record that is missing: {stderr}"
    );
}
