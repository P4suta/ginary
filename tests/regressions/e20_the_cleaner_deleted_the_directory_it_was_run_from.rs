// SPDX-License-Identifier: MIT OR Apache-2.0
//! `mise run clean:cache` fell back to `$PWD` when `MISE_PROJECT_ROOT` was
//! unset and then ran seven `rm -rf` on relative paths, so run outside `mise`
//! from any directory it deleted *that* directory's `target/debug`,
//! `target/release`, `fuzz/target` and `mutants.out`.
//!
//! **What went wrong.** The task opened with
//!
//! ```sh
//! set -eu
//! cd "${MISE_PROJECT_ROOT:-$PWD}"
//! ```
//!
//! `MISE_PROJECT_ROOT` is set by `mise` and by nothing else. Extracted and run
//! by hand — which is how a disk-space chore actually gets done at the point
//! the disk is full, and how `cargo mutants`, a shell script or a CI step
//! would run it — the `cd` is a no-op and the removals apply to the caller's
//! own tree:
//!
//! ```text
//! $ mkdir -p fake/sub/target/debug
//! $ (cd fake/sub && env -u MISE_PROJECT_ROOT bash clean-cache.sh)
//! clean:cache: reclaimed 0 MiB
//! $ ls fake/sub/target
//! (empty)
//! ```
//!
//! The test that pinned the task named this exact failure in its own message —
//! "otherwise `mise run clean:cache` from a subdirectory deletes nothing, or
//! something else" — and then asserted only that *some* command started with
//! `cd `.
//!
//! **The input.** Any invocation with `MISE_PROJECT_ROOT` unset from a
//! directory that is not this project. The other `${MISE_PROJECT_ROOT:-$PWD}`
//! uses in `mise.toml` are builds and reads, where guessing wrong costs a
//! failed command; this is the only destructive one, where guessing wrong
//! costs somebody else's build output.
//!
//! **The correct behaviour.** Refuse rather than guess. The task establishes
//! the root, checks it is this project's, and only then changes into it and
//! removes anything. A tree that is not ginary's is a non-zero exit with a
//! sentence naming the directory it declined to clean — and what it must keep,
//! `target/stubs` and `dist/otp`, is still there afterwards when it does run.

// A unix file: it runs the task's run block under `bash`.
#![cfg(unix)]

use std::path::Path;
use std::process::Command;

use tempfile::TempDir;

/// The task under test.
const CACHE_TASK: &str = "clean:cache";

/// The run block of `[tasks."clean:cache"]`, as committed.
fn run_block() -> String {
    let Some(task) = crate::common::mise::task(CACHE_TASK) else {
        panic!("mise.toml declares no [tasks.\"{CACHE_TASK}\"]");
    };
    task.run.join("\n")
}

/// A tree with something in every directory the cleaner touches.
///
/// `ginary` is `true` when the tree is meant to look like this project's root,
/// which is what the cleaner has to satisfy itself of before it removes
/// anything.
fn fixture(ginary: bool) -> TempDir {
    let dir = TempDir::new().expect("temporary directory for a cleaner fixture");
    for tree in [
        "target/debug",
        "target/release",
        "target/stubs",
        "fuzz/target",
        "mutants.out",
        "dist/otp",
    ] {
        let path = dir.path().join(tree);
        std::fs::create_dir_all(&path).expect("create a fixture tree");
        std::fs::write(path.join("marker"), b"marker\n").expect("write a fixture marker");
    }
    if ginary {
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"ginary\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .expect("write the fixture Cargo.toml");
        std::fs::write(
            dir.path().join("mise.toml"),
            "[tasks.noop]\nrun = \"true\"\n",
        )
        .expect("write the fixture mise.toml");
    }
    dir
}

/// Runs the task's run block with `cwd` as the working directory and
/// `MISE_PROJECT_ROOT` set to `root`, or unset when it is `None`.
fn run_cleaner(cwd: &Path, root: Option<&Path>) -> (i32, String) {
    let mut command = Command::new("bash");
    command.arg("-c").arg(run_block()).current_dir(cwd);
    match root {
        Some(root) => command.env("MISE_PROJECT_ROOT", root),
        None => command.env_remove("MISE_PROJECT_ROOT"),
    };
    let output = command.output().expect("run the clean:cache run block");
    let mut said = String::from_utf8_lossy(&output.stdout).into_owned();
    said.push_str(&String::from_utf8_lossy(&output.stderr));
    (output.status.code().unwrap_or(-1), said)
}

#[test]
fn a_cleaner_run_outside_mise_does_not_clean_whatever_directory_it_is_in() {
    let tree = fixture(false);
    let (code, said) = run_cleaner(tree.path(), None);
    assert!(
        tree.path().join("target/debug/marker").is_file(),
        "with `MISE_PROJECT_ROOT` unset the task fell back to `$PWD` and removed this tree's \
         `target/debug`. It is not ginary's tree, and a maintenance task that guesses which \
         project it is cleaning is one `cd` away from deleting somebody's build output: {said}"
    );
    assert_ne!(
        code, 0,
        "declining to clean a tree is a refusal, not a success: a caller that cannot tell the \
         difference will believe the disk was reclaimed: {said}"
    );
    assert!(
        said.contains(CACHE_TASK),
        "the refusal says which task declined and why: {said}"
    );
}

#[test]
fn a_project_root_that_is_not_ginarys_is_refused_too() {
    let tree = fixture(false);
    let (code, said) = run_cleaner(tree.path(), Some(tree.path()));
    assert_ne!(
        code, 0,
        "`MISE_PROJECT_ROOT` pointing at a tree that is not this project is the same mistake with \
         the variable set — a `mise.toml` in a parent directory, or an exported variable left \
         over from another checkout: {said}"
    );
    assert!(
        tree.path().join("target/debug/marker").is_file(),
        "nothing is removed from a tree the task declined to clean: {said}"
    );
}

#[test]
fn the_cleaner_removes_the_regenerable_trees_and_keeps_the_expensive_ones() {
    // The positive control: the refusals above have to be refusals of the wrong
    // tree, not of every tree.
    let tree = fixture(true);
    let (code, said) = run_cleaner(tree.path(), Some(tree.path()));
    assert_eq!(
        code, 0,
        "a tree that is this project's root is one the cleaner runs in: {said}"
    );
    for gone in [
        "target/debug",
        "target/release",
        "fuzz/target",
        "mutants.out",
    ] {
        assert!(
            !tree.path().join(gone).exists(),
            "`{gone}` is regenerable by a plain `cargo` or `cargo mutants` run and is what the \
             task exists to reclaim; it is still there: {said}"
        );
    }
    for kept in ["target/stubs", "dist/otp"] {
        assert!(
            tree.path().join(kept).join("marker").is_file(),
            "`{kept}` costs `cross`, a docker daemon and minutes per target, or 130 MB of \
             downloads and a repack; the cleaner took it: {said}"
        );
    }
}
