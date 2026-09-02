// SPDX-License-Identifier: MIT OR Apache-2.0
//! The remedy `ginary otp path` prints stopped being a command at the first
//! space.
//!
//! `catalog::fetch_command` interpolated the `--catalog` it was given with
//! `Path::display`, so a catalogue under a directory whose name holds a space
//! rendered as
//!
//! ```text
//! run `ginary otp fetch --version 29.0.5 --target linux-x86_64-musl \
//!      --catalog /tmp/my catalogs/dist/otp/catalog.json`
//! ```
//!
//! Pasted into a shell that is three arguments, not two: `ginary otp fetch`
//! reads `/tmp/my` as the catalogue, fails, and the user is left debugging the
//! remedy rather than the problem. A space is the ordinary case — `My
//! Documents`, `Application Support` — and a `$(...)` or a `;` in the path
//! would be worse than a broken command.
//!
//! The right behaviour: every argument of a suggested command is rendered
//! shell-safe, so the line can be pasted as it stands. This test proves it by
//! pasting it: the command inside the backticks is handed to `/bin/sh -c` and
//! has to fill the cache the run was refused for.
#![cfg(feature = "cli")]

use std::path::{Path, PathBuf};

use assert_cmd::Command;

use crate::common::catalog::{
    CatalogBuilder, ERTS_VSN, RELEASE, VERSION, static_variant, write_catalog_text,
};
use crate::common::fake_otp::FakeOtp;
use crate::common::payload::sha256_hex;

/// The target the fixture catalogue holds.
const MUSL: &str = "linux-x86_64-musl";

/// The directory name that broke the remedy.
const AWKWARD: &str = "my catalogs";

/// A `ginary` with a cache of its own and nothing ambient around it.
fn ginary_with_cache(cache: &Path) -> Command {
    let mut command = Command::cargo_bin("ginary").expect("the `ginary` binary is built for tests");
    command
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env_clear()
        .env("GINARY_CACHE_DIR", cache);
    crate::common::coverage::preserve_coverage_env_assert(&mut command);
    command
}

/// A catalogue with one real tarball beside it, under a path holding a space.
fn catalog_under_a_path_with_a_space(dir: &Path) -> PathBuf {
    let source = dir.join("source");
    FakeOtp::new()
        .erts_vsn(ERTS_VSN)
        .release(RELEASE)
        .otp_version(VERSION)
        .build_in(&source);
    let tarball = crate::common::catalog::runtime_tarball(&source);
    let digest = sha256_hex(&tarball);

    let out = dir.join(AWKWARD).join("dist/otp");
    std::fs::create_dir_all(&out).expect("the catalog directory");
    std::fs::write(out.join("otp-static.tar.zst"), &tarball).expect("a runtime tarball");

    let text = CatalogBuilder::new()
        .entry(
            VERSION,
            RELEASE,
            ERTS_VSN,
            MUSL,
            "static",
            static_variant("otp-static.tar.zst", &digest, tarball.len() as u64),
        )
        .json();
    write_catalog_text(&out, &text)
}

/// The command inside the backticks of `message`.
fn suggested_command(message: &str) -> String {
    let (_, rest) = message
        .split_once("run `")
        .unwrap_or_else(|| panic!("the message names a command to run: {message}"));
    let (command, _) = rest
        .split_once('`')
        .unwrap_or_else(|| panic!("the command is closed: {message}"));
    command.to_owned()
}

/// The refusal `ginary otp path` prints for an entry that is not cached.
fn not_cached_message(catalog: &Path, cache: &Path) -> String {
    let assert = ginary_with_cache(cache)
        .args([
            "otp",
            "path",
            "--version",
            VERSION,
            "--target",
            MUSL,
            "--catalog",
        ])
        .arg(catalog)
        .assert()
        .failure();
    String::from_utf8(assert.get_output().stderr.clone()).expect("utf-8")
}

#[test]
fn the_remedy_quotes_a_catalog_path_that_holds_a_space() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let catalog = catalog_under_a_path_with_a_space(dir.path());
    let stderr = not_cached_message(&catalog, &dir.path().join("cache"));

    let command = suggested_command(&stderr);
    let rendered = catalog.display().to_string();
    assert!(
        !command.contains(&format!("--catalog {rendered}")),
        "the path is not pasted bare, or the command ends at the space: {command}"
    );
    assert!(
        command.contains(&format!("--catalog '{rendered}'")),
        "the path is quoted as one argument: {command}"
    );
}

#[cfg(unix)]
#[test]
fn the_remedy_a_space_in_the_path_earns_still_runs_in_a_shell() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let catalog = catalog_under_a_path_with_a_space(dir.path());
    let cache = dir.path().join("cache");
    let stderr = not_cached_message(&catalog, &cache);
    let command = suggested_command(&stderr);

    // `ginary` under a name a shell finds, so the line runs as it was printed.
    let bin = dir.path().join("bin");
    std::fs::create_dir_all(&bin).expect("a bin directory");
    std::os::unix::fs::symlink(assert_cmd::cargo::cargo_bin("ginary"), bin.join("ginary"))
        .expect("a link to this test run's own binary");

    let mut shell = Command::new("/bin/sh");
    shell
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env_clear()
        .env("GINARY_CACHE_DIR", &cache)
        .env("PATH", &bin);
    // The shell is not instrumented, but the `ginary` it execs is: the profile
    // file threaded here is inherited across the `exec`, so the launched
    // binary's coverage merges into the run.
    crate::common::coverage::preserve_coverage_env_assert(&mut shell);
    let run = shell.arg("-c").arg(&command).assert().success();
    let printed = String::from_utf8(run.get_output().stdout.clone()).expect("utf-8");
    assert!(
        printed.contains(&format!("{VERSION}-{MUSL}")),
        "the pasted remedy filled the directory `path` was refused for: {printed}"
    );

    // And the question it was the answer to now has one.
    ginary_with_cache(&cache)
        .args([
            "otp",
            "path",
            "--version",
            VERSION,
            "--target",
            MUSL,
            "--catalog",
        ])
        .arg(&catalog)
        .assert()
        .success();
}
