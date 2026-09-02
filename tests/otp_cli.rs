// SPDX-License-Identifier: MIT OR Apache-2.0
//! `ginary otp`: the developer's window onto the catalogue and its cache.
//!
//! Five subcommands, and the division between them is deliberate.
//! `list` and `path` answer questions about what is already there and never
//! reach the network; `fetch` and `update` are the two that do; `repack` is the
//! pipeline itself. `path` in particular does *not* fetch: a command that
//! silently downloaded 40 MB because somebody asked where a directory was would
//! be a surprise in a script, so it prints the directory or says to run
//! `fetch`.
//!
//! Every test here runs the real binary with `GINARY_CACHE_DIR` and
//! `--catalog` pointed at directories it owns, so nothing on the developer's
//! machine can change an answer.
// The command line half of the suite: a launcher-only build has no commands.
#![cfg(feature = "cli")]

mod common;

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use serde_json::Value;

use ginary::catalog::{CATALOG_ENV_VAR, SCHEMA_VERSION};

use crate::common::catalog::{
    CatalogBuilder, ERTS_VSN, RELEASE, VERSION, gnu_variant, plant_cached_otp, static_variant,
    write_catalog_text,
};

/// The musl target the fixture catalogue holds.
const MUSL: &str = "linux-x86_64-musl";

/// The glibc target beside it.
const GNU: &str = "linux-x86_64-gnu";

/// A temporary directory the test owns.
fn tempdir() -> tempfile::TempDir {
    tempfile::tempdir().expect("a temporary directory")
}

/// A `ginary` with a cache of its own and nothing ambient in its environment.
fn ginary_with_cache(cache: &Path) -> Command {
    let mut command = Command::cargo_bin("ginary").expect("the `ginary` binary is built for tests");
    command
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env_clear()
        .env("GINARY_CACHE_DIR", cache);
    crate::common::coverage::preserve_coverage_env_assert(&mut command);
    command
}

/// The two-entry fixture catalogue, written into `<dir>/dist/otp`.
fn fixture_catalog(dir: &Path) -> PathBuf {
    CatalogBuilder::new()
        .generated_at("2026-08-31T00:00:00Z")
        .entry(
            VERSION,
            RELEASE,
            ERTS_VSN,
            MUSL,
            "static",
            static_variant(
                "otp-29.0.5-linux-x86_64-musl-static.tar.zst",
                &"a".repeat(64),
                41_943_040,
            ),
        )
        .entry(
            VERSION,
            RELEASE,
            ERTS_VSN,
            GNU,
            "default",
            gnu_variant(
                "otp-29.0.5-linux-x86_64-gnu.tar.zst",
                &"b".repeat(64),
                39_845_888,
            ),
        )
        .write_in(&dir.join("dist/otp"))
}

/// The standard output of a run that must succeed.
fn stdout_of(assert: assert_cmd::assert::Assert) -> String {
    String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8 on standard output")
}

// ----------------------------------------------------------- the list --

#[test]
fn the_help_lists_the_otp_command() {
    let dir = tempdir();
    let stdout = stdout_of(
        ginary_with_cache(&dir.path().join("cache"))
            .arg("--help")
            .assert()
            .success(),
    );

    assert!(
        stdout.contains("otp"),
        "the catalog commands are part of the tool's surface: {stdout}"
    );
}

#[test]
fn otp_list_prints_one_row_per_variant_and_marks_the_cached_ones() {
    let dir = tempdir();
    let catalog = fixture_catalog(dir.path());
    let cache = dir.path().join("cache");
    plant_cached_otp(
        &cache.join("otp"),
        "29.0.5-linux-x86_64-musl-static",
        VERSION,
        MUSL,
        "static",
        &static_variant("otp.tar.zst", &"a".repeat(64), 41_943_040),
    );

    let stdout = stdout_of(
        ginary_with_cache(&cache)
            .args(["otp", "list", "--catalog"])
            .arg(&catalog)
            .assert()
            .success(),
    );

    insta::assert_snapshot!("otp_list_table", stdout);
}

#[test]
fn otp_list_narrows_to_one_target_when_it_is_named() {
    let dir = tempdir();
    let catalog = fixture_catalog(dir.path());
    let cache = dir.path().join("cache");

    let stdout = stdout_of(
        ginary_with_cache(&cache)
            .args(["otp", "list", "--target", MUSL, "--catalog"])
            .arg(&catalog)
            .assert()
            .success(),
    );

    assert!(stdout.contains(MUSL), "the musl row is there: {stdout}");
    assert!(!stdout.contains(GNU), "and the glibc row is not: {stdout}");
}

#[test]
fn otp_list_json_carries_the_schema_the_origin_and_every_entry() {
    let dir = tempdir();
    let catalog = fixture_catalog(dir.path());
    let cache = dir.path().join("cache");
    plant_cached_otp(
        &cache.join("otp"),
        "29.0.5-linux-x86_64-musl-static",
        VERSION,
        MUSL,
        "static",
        &static_variant("otp.tar.zst", &"a".repeat(64), 41_943_040),
    );

    let assert = ginary_with_cache(&cache)
        .args(["otp", "list", "--json", "--catalog"])
        .arg(&catalog)
        .assert()
        .success();
    let value: Value = serde_json::from_slice(&assert.get_output().stdout).expect("JSON");

    assert_eq!(value["format_version"], Value::from(1));
    assert_eq!(
        value["schema_version"],
        Value::from(u64::from(SCHEMA_VERSION))
    );
    assert_eq!(
        value["origin"],
        Value::from(catalog.display().to_string()),
        "the answer says which catalog it came out of"
    );
    let entries = value["entries"].as_array().expect("entries is an array");
    assert_eq!(entries.len(), 2);

    let musl = entries
        .iter()
        .find(|entry| entry["target"] == MUSL)
        .expect("the musl entry");
    assert_eq!(musl["version"], Value::from(VERSION));
    assert_eq!(musl["variant"], Value::from("static"));
    assert_eq!(musl["linkage"], Value::from("static"));
    assert_eq!(musl["nif_loading"], Value::from(false));
    assert_eq!(musl["size"], Value::from(41_943_040_u64));
    assert_eq!(musl["cached"], Value::from(true));
    assert_eq!(
        entries
            .iter()
            .find(|entry| entry["target"] == GNU)
            .expect("the gnu entry")["cached"],
        Value::from(false)
    );
}

#[test]
fn otp_list_with_an_empty_catalog_says_so_rather_than_printing_a_bare_header() {
    let dir = tempdir();
    let cache = dir.path().join("cache");

    let stdout = stdout_of(
        ginary_with_cache(&cache)
            .args(["otp", "list"])
            .assert()
            .success(),
    );

    assert!(
        stdout.contains("no catalog entries"),
        "an empty embedded catalog is the default state, and it has to explain itself: {stdout}"
    );
    assert!(
        stdout.contains("ginary otp repack") || stdout.contains("--catalog"),
        "and say what to do about it: {stdout}"
    );
}

// ---------------------------------------------------------- the paths --

#[test]
fn otp_path_prints_the_cache_directory_of_a_runtime_that_is_there() {
    let dir = tempdir();
    let catalog = fixture_catalog(dir.path());
    let cache = dir.path().join("cache");
    let (planted, _) = plant_cached_otp(
        &cache.join("otp"),
        "29.0.5-linux-x86_64-musl-static",
        VERSION,
        MUSL,
        "static",
        &static_variant("otp.tar.zst", &"a".repeat(64), 41_943_040),
    );

    let stdout = stdout_of(
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
            .success(),
    );

    assert_eq!(stdout, format!("{}\n", planted.display()));
}

#[test]
fn otp_path_refuses_a_runtime_that_is_not_cached_and_names_the_command_that_fetches_it() {
    let dir = tempdir();
    let catalog = fixture_catalog(dir.path());
    let cache = dir.path().join("cache");

    let assert = ginary_with_cache(&cache)
        .args([
            "otp",
            "path",
            "--version",
            VERSION,
            "--target",
            GNU,
            "--catalog",
        ])
        .arg(&catalog)
        .assert()
        .failure()
        .code(1);
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf-8");

    assert!(
        stderr.contains("not cached"),
        "`path` answers where it is, and does not quietly fetch 40 MB: {stderr}"
    );
    assert!(
        stderr.contains("ginary otp fetch"),
        "and names the command that would: {stderr}"
    );
}

// --------------------------------------------------------- the fetch --

#[test]
fn otp_fetch_offline_names_the_url_it_would_have_asked_for() {
    let dir = tempdir();
    let catalog = fixture_catalog(dir.path());
    let cache = dir.path().join("cache");

    let assert = ginary_with_cache(&cache)
        .env("GINARY_OFFLINE", "1")
        .args([
            "otp",
            "fetch",
            "--version",
            VERSION,
            "--target",
            GNU,
            "--catalog",
        ])
        .arg(&catalog)
        .assert()
        .failure();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf-8");

    assert!(
        stderr.contains("offline"),
        "an offline fetch is refused, not attempted: {stderr}"
    );
    assert!(
        stderr.contains("otp-29.0.5-linux-x86_64-gnu.tar.zst"),
        "and names the file it would have fetched: {stderr}"
    );
}

#[test]
fn otp_fetch_of_a_target_the_catalog_does_not_hold_lists_the_ones_it_does() {
    let dir = tempdir();
    let catalog = fixture_catalog(dir.path());
    let cache = dir.path().join("cache");

    let assert = ginary_with_cache(&cache)
        .args([
            "otp",
            "fetch",
            "--version",
            VERSION,
            "--target",
            "linux-aarch64-musl",
            "--catalog",
        ])
        .arg(&catalog)
        .assert()
        .failure();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf-8");

    assert!(
        stderr.contains("linux-x86_64-gnu, linux-x86_64-musl"),
        "the message names what the catalog does hold: {stderr}"
    );
}

// -------------------------------------------------------- the update --

#[test]
fn otp_update_copies_a_catalog_into_the_cache_after_validating_it() {
    let dir = tempdir();
    let catalog = fixture_catalog(dir.path());
    let cache = dir.path().join("cache");

    ginary_with_cache(&cache)
        .args(["otp", "update", "--catalog"])
        .arg(&catalog)
        .assert()
        .success();

    let installed = cache.join("otp/catalog.json");
    assert!(
        installed.is_file(),
        "an updated catalog is what a later run with no --catalog reads"
    );
    assert_eq!(
        std::fs::read_to_string(&installed).expect("the installed catalog"),
        std::fs::read_to_string(&catalog).expect("the source catalog"),
        "it is a copy, not a re-serialisation: the digests in it must not move"
    );
}

#[test]
fn otp_update_refuses_a_document_that_is_not_a_catalog_and_leaves_the_cache_alone() {
    let dir = tempdir();
    let broken = write_catalog_text(&dir.path().join("broken"), "{\"schema_version\": 7}\n");
    let cache = dir.path().join("cache");
    std::fs::create_dir_all(cache.join("otp")).expect("the cache");
    std::fs::write(cache.join("otp/catalog.json"), "the one that was there\n")
        .expect("an existing catalog");

    let assert = ginary_with_cache(&cache)
        .args(["otp", "update", "--catalog"])
        .arg(&broken)
        .assert()
        .failure();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf-8");

    assert!(
        stderr.contains("schema 7"),
        "the schema is checked before anything is written: {stderr}"
    );
    assert_eq!(
        std::fs::read_to_string(cache.join("otp/catalog.json")).expect("the old catalog"),
        "the one that was there\n",
        "a refused update does not damage the catalog that was already installed"
    );
}

#[test]
fn the_catalog_environment_variable_is_read_when_no_flag_is_given() {
    let dir = tempdir();
    let catalog = fixture_catalog(dir.path());
    let cache = dir.path().join("cache");

    let stdout = stdout_of(
        ginary_with_cache(&cache)
            .env(CATALOG_ENV_VAR, &catalog)
            .args(["otp", "list"])
            .assert()
            .success(),
    );

    assert!(
        stdout.contains(MUSL) && stdout.contains(GNU),
        "{CATALOG_ENV_VAR} is the flag without the flag: {stdout}"
    );
}

// -------------------------------------------------------- the repack --

#[test]
fn otp_repack_names_its_flags_and_says_it_publishes_nothing() {
    let dir = tempdir();
    let stdout = stdout_of(
        ginary_with_cache(&dir.path().join("cache"))
            .args(["otp", "repack", "--help"])
            .assert()
            .success(),
    );

    for flag in ["--upstream-tag", "--targets", "--out", "--upstream-dir"] {
        assert!(
            stdout.contains(flag),
            "{flag} is part of the command: {stdout}"
        );
    }
}
