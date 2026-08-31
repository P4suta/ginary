// SPDX-License-Identifier: MIT OR Apache-2.0
//! A suffixed build rewrote an output path that was not UTF-8.
//!
//! **What went wrong.** `BuildOptions::artifact_path` built the suffixed file
//! name by rendering the `--out` file name through `to_string_lossy`, so every
//! byte that is not UTF-8 became U+FFFD before `-<target>` was appended. An
//! unsuffixed build returns `--out` unchanged and is byte-exact, so adding
//! `--target host` to a build silently moved the artifact to a *different*
//! path — one the user never named and a wrapper quoting the original cannot
//! find. `manifest_copy_path` derived its name from the same lossy string.
//!
//! **The input.** `--out` naming a file whose name holds a `0xff` byte, which
//! on Linux is an ordinary file name, together with a `--target` that makes
//! the build suffixed.
//!
//! **The correct behaviour.** The suffix is appended to the file name's own
//! bytes, so the artifact and its manifest copy are the `--out` path with
//! `-<target>` after it and nothing else changed. It is the rule
//! `a4_a_non_utf8_output_path_failed_the_json_report` states from the other
//! side: a path the build handled is not a path the build may rewrite.

use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::{OsStrExt as _, OsStringExt as _};
use std::path::{Path, PathBuf};

use ginary::config::{BuildFlags, BuildOptions, ProjectConfig};
use ginary::target::Target;

use crate::common::project::config_fixture;

/// The project the fixture describes.
const ROOT: &str = "/w/plain_app";

/// `--out` naming a file whose name is not UTF-8.
fn out_path() -> PathBuf {
    PathBuf::from(OsString::from_vec(b"/w/plain_app/out/he\xffllo".to_vec()))
}

/// The options a build with `--out` and an explicit host target resolves to.
fn options() -> BuildOptions {
    let root = Path::new(ROOT);
    let config =
        ProjectConfig::from_toml(&config_fixture("defaults.toml"), &root.join("gleam.toml"))
            .expect("the fixture parses");
    let flags = BuildFlags {
        start: root.to_path_buf(),
        out: Some(out_path()),
        targets: vec![Target::host().name()],
        ..BuildFlags::default()
    };
    BuildOptions::merge(root, &config, &flags).expect("the merge succeeds")
}

#[test]
fn the_suffix_is_appended_to_the_bytes_the_user_typed() {
    let options = options();
    assert_eq!(options.out, out_path(), "the merge keeps `--out` as it was");

    let artifact = options.artifact_path(Target::host());

    let mut expected = OsString::from_vec(b"/w/plain_app/out/he\xffllo".to_vec());
    expected.push(format!("-{}", Target::host().name()));
    assert_eq!(
        artifact,
        PathBuf::from(&expected),
        "the artifact is the path that was asked for with the target after it, and every byte \
         of it is the user's"
    );
    assert!(
        artifact
            .file_name()
            .is_some_and(|name| name.as_bytes().contains(&0xff)),
        "the byte that is not UTF-8 survives: {}",
        artifact.display()
    );
}

#[test]
fn the_manifest_copy_is_named_after_the_same_bytes() {
    let copy = options()
        .manifest_copy_path(Target::host())
        .expect("a suffixed build writes a manifest copy");

    let mut expected = OsString::from_vec(b"/w/plain_app/out/he\xffllo".to_vec());
    expected.push(format!("-{}.json", Target::host().name()));
    assert_eq!(copy, PathBuf::from(&expected));
    assert_eq!(
        copy.extension(),
        Some(OsStr::new("json")),
        "and it is still a JSON document: {}",
        copy.display()
    );
}
