// SPDX-License-Identifier: MIT OR Apache-2.0
//! `ginary cache clean --app` deleted any directory the caller could name.
//!
//! **What went wrong.** `cache::clean` joined the `--app` value onto the cache
//! root with `Path::join` and then called `remove_dir_all` on the result. An
//! absolute value replaces the whole path, and a `..` component walks out of
//! the root, so `--app /home/u/work` removed `/home/u/work` and `--app ..`
//! emptied the cache root's parent. Nothing between clap and the removal
//! looked at the value: the CLI passed `Option<String>` straight through, and
//! the only test used `--app hello`.
//!
//! **The input.** `--app` values that are not a single path component:
//! `<somewhere else>`, `..`, `a/b` and the empty string. The same values reach
//! `cache::clean` directly from the library.
//!
//! **The correct behaviour.** An application name is one path component or it
//! is not an application name. Each of those values is refused, nothing is
//! removed, and the directory the value pointed at is still there afterwards.

use std::path::Path;

use ginary::cache;

/// Builds `<root>/hello/<key>/ginary.json`, a cache with one entry in it.
fn cache_with_one_entry(root: &Path) {
    let entry = root.join("hello").join("0123456789abcdef");
    std::fs::create_dir_all(&entry).expect("create the entry");
    std::fs::write(entry.join("ginary.json"), b"{}").expect("write the manifest");
}

/// Every `--app` value that is not a single path component.
///
/// The absolute one is built from `outside`, which is the directory the test
/// then proves is still there.
fn hostile_values(outside: &Path) -> Vec<String> {
    vec![
        outside.display().to_string(),
        "..".to_owned(),
        "../..".to_owned(),
        "a/b".to_owned(),
        "hello/../..".to_owned(),
        ".".to_owned(),
        String::new(),
    ]
}

#[test]
fn clean_refuses_an_application_name_that_is_not_one_component() {
    let dir = tempfile::tempdir().expect("tempdir");
    let outside = dir.path().join("precious");
    std::fs::create_dir(&outside).expect("create the directory that must survive");
    std::fs::write(
        outside.join("work.txt"),
        b"a file nobody asked ginary to remove",
    )
    .expect("write the file that must survive");

    for value in hostile_values(&outside) {
        let root = dir.path().join("cache");
        cache_with_one_entry(&root);

        let error = cache::clean(&root, Some(&value)).err().unwrap_or_else(|| {
            panic!("`--app {value}` must be refused rather than removing a directory")
        });
        assert_eq!(
            error.exit_code(),
            124,
            "a cache refusal is a cache error, and `--app {value}` gave `{error}`"
        );
        assert!(
            root.join("hello").is_dir(),
            "`--app {value}` must leave the cache alone"
        );
        assert!(
            outside.join("work.txt").is_file(),
            "`--app {value}` removed a directory outside the cache root"
        );
        std::fs::remove_dir_all(&root).expect("reset the cache between values");
    }
}

#[test]
fn clean_still_empties_the_application_it_was_named() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("cache");
    cache_with_one_entry(&root);

    let report = cache::clean(&root, Some("hello")).expect("a name that is one component");
    assert_eq!(report.removed, vec![root.join("hello")]);
    assert!(!root.join("hello").exists());
    assert!(root.is_dir(), "the cache root itself stays");
}

#[test]
fn the_command_line_refuses_it_too_and_leaves_the_directory() {
    let dir = tempfile::tempdir().expect("tempdir");
    let outside = dir.path().join("precious");
    std::fs::create_dir(&outside).expect("create the directory that must survive");
    let root = dir.path().join("cache");
    cache_with_one_entry(&root);

    let mut command =
        assert_cmd::Command::cargo_bin("ginary").expect("the `ginary` binary is built for tests");
    let assert = command
        .env_clear()
        .env("GINARY_CACHE_DIR", &root)
        .args(["cache", "clean", "--app"])
        .arg(&outside)
        .assert()
        .failure();

    assert!(
        outside.is_dir(),
        "`ginary cache clean --app <path>` removed a directory outside the cache root"
    );
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
    assert!(
        stderr.contains("error:"),
        "the refusal must be reported, and it said `{stderr}`"
    );
}
