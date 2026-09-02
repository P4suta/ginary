// SPDX-License-Identifier: MIT OR Apache-2.0
//! `ginary otp path` told the user to run a command that could not work.
//!
//! `CatalogError::NotCached` was written as a format string over two fields:
//!
//! ```rust
//! #[error("{dir} is not cached; run `ginary otp fetch --version {version} --target {target}`")]
//! ```
//!
//! so the `--catalog` the run was given and the `--variant` it resolved were
//! both dropped. A developer working off a local `dist/otp/catalog.json` was
//! told to run a `ginary otp fetch` that reads the *embedded* catalogue, which
//! is empty:
//!
//! ```text
//! error: .../otp/29.0.5-linux-x86_64-gnu is not cached;
//!        run `ginary otp fetch --version 29.0.5 --target linux-x86_64-gnu`
//! error: the embedded catalog has no OTP 29.0.5 entry; it has nothing at all
//! ```
//!
//! and a target with a named variant was told to fetch a different runtime.
//! The right behaviour: the command in the message is the command that works,
//! which this test proves by running it.
#![cfg(feature = "cli")]

use std::path::{Path, PathBuf};

use assert_cmd::Command;

use crate::common::catalog::{
    CatalogBuilder, ERTS_VSN, RELEASE, VERSION, static_variant, write_catalog_text,
};
use crate::common::fake_otp::FakeOtp;
use crate::common::payload::sha256_hex;

/// The target the fixture catalogue holds two variants of.
const MUSL: &str = "linux-x86_64-musl";

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

/// A catalogue whose two musl variants are real tarballs beside it.
///
/// Two variants, because one of the two flags the message dropped is
/// `--variant`, and a target with a single variant would never need it.
fn catalog_with_two_variants(dir: &Path) -> PathBuf {
    let source = dir.join("source");
    FakeOtp::new()
        .erts_vsn(ERTS_VSN)
        .release(RELEASE)
        .otp_version(VERSION)
        .build_in(&source);
    let tarball = crate::common::catalog::runtime_tarball(&source);
    let digest = sha256_hex(&tarball);

    let out = dir.join("dist/otp");
    std::fs::create_dir_all(&out).expect("the catalog directory");
    for name in ["otp-static.tar.zst", "otp-dynamic.tar.zst"] {
        std::fs::write(out.join(name), &tarball).expect("a runtime tarball beside the catalog");
    }

    let text = CatalogBuilder::new()
        .entry(
            VERSION,
            RELEASE,
            ERTS_VSN,
            MUSL,
            "static",
            static_variant("otp-static.tar.zst", &digest, tarball.len() as u64),
        )
        .entry(
            VERSION,
            RELEASE,
            ERTS_VSN,
            MUSL,
            "dynamic",
            static_variant("otp-dynamic.tar.zst", &digest, tarball.len() as u64),
        )
        .json();
    write_catalog_text(&out, &text)
}

/// The command inside the backticks of `message`.
fn suggested_command(message: &str) -> Vec<String> {
    let (_, rest) = message
        .split_once("run `")
        .unwrap_or_else(|| panic!("the message names a command to run: {message}"));
    let (command, _) = rest
        .split_once('`')
        .unwrap_or_else(|| panic!("the command is closed: {message}"));
    command.split_whitespace().map(str::to_owned).collect()
}

#[test]
fn the_command_ginary_otp_path_prints_is_a_command_that_works() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let catalog = catalog_with_two_variants(dir.path());
    let cache = dir.path().join("cache");

    let assert = ginary_with_cache(&cache)
        .args([
            "otp",
            "path",
            "--version",
            VERSION,
            "--target",
            MUSL,
            "--variant",
            "dynamic",
            "--catalog",
        ])
        .arg(&catalog)
        .assert()
        .failure();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf-8");

    let command = suggested_command(&stderr);
    assert_eq!(
        command.first().map(String::as_str),
        Some("ginary"),
        "the message names ginary itself: {stderr}"
    );
    assert!(
        command.iter().any(|token| token == "--catalog"),
        "the remedy carries the catalogue this run was given, or it reads the empty embedded \
         one: {stderr}"
    );
    assert!(
        command.iter().any(|token| token == "dynamic"),
        "and the variant this run resolved, or it fetches a different runtime: {stderr}"
    );

    // The assertion that cannot drift: run it.
    let fetched = ginary_with_cache(&cache)
        .args(&command[1..])
        .assert()
        .success();
    let printed = String::from_utf8(fetched.get_output().stdout.clone()).expect("utf-8");
    assert!(
        printed.contains(&format!("{VERSION}-{MUSL}-dynamic")),
        "the fetch filled the directory `path` was asked about: {printed}"
    );

    // And `path` now answers rather than refusing.
    let after = ginary_with_cache(&cache)
        .args([
            "otp",
            "path",
            "--version",
            VERSION,
            "--target",
            MUSL,
            "--variant",
            "dynamic",
            "--catalog",
        ])
        .arg(&catalog)
        .assert()
        .success();
    assert_eq!(
        String::from_utf8(after.get_output().stdout.clone()).expect("utf-8"),
        printed,
        "`path` and `fetch` answer about one directory"
    );
}
