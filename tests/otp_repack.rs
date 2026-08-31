// SPDX-License-Identifier: MIT OR Apache-2.0
//! `ginary otp repack`: the local repackaging pipeline, held to what it maps,
//! what it strips, what it dereferences and what it writes.
//!
//! This is the whole of what would otherwise be a hosted `ginary-otp`
//! repository, run on a developer's machine. Nothing here publishes anything:
//! what comes out is a directory of `.tar.zst` files and a `catalog.json`
//! beside them whose URLs are file names relative to itself.
//!
//! The pipeline's own trust check reads a `beam.smp`, and a fixture upstream
//! tree carries a shell script there, so the tests drive
//! [`catalog::repack_with`] with the inspection injected — the same seam
//! `erts_source::resolve_with` has, for the same reason. What that leaves
//! covered is everything the pipeline actually decides: the upstream asset
//! name, the prune, the dereference, the mismatch refusal and the catalogue
//! entry.
// The command line half of the suite: a launcher-only build repacks nothing.
#![cfg(feature = "cli")]

mod common;

use std::path::{Path, PathBuf};

use ginary::catalog::{
    self, CatalogError, DEFAULT_VARIANT, KEPT_DIRS, PRUNE_DIRS, RepackError, RepackOptions,
    RepackSelector, SourceUrl, UPSTREAM_REPO,
};
use ginary::diag::Diag;
use ginary::download::{DownloadError, GITHUB_API_BASE, Net};
use ginary::elf::{ElfError, ElfInfo, ElfKind};
use ginary::target::Linkage;

use crate::common::catalog::{ERTS_VSN, FakeUpstream, UPSTREAM_TAG, VERSION};
use crate::common::http::{Reply, TestServer};

/// A temporary directory the test owns.
fn tempdir() -> tempfile::TempDir {
    tempfile::tempdir().expect("a temporary directory")
}

/// What a fully static x86-64 emulator reads back as.
fn static_x86_64() -> ElfInfo {
    ElfInfo {
        class: 64,
        kind: ElfKind::Executable,
        machine: "x86_64".to_owned(),
        interp: None,
        needed: Vec::new(),
        glibc_max: None,
        is_pie: false,
        stripped: true,
    }
}

/// What a dynamically linked aarch64 musl emulator reads back as.
fn dynamic_aarch64() -> ElfInfo {
    ElfInfo {
        machine: "aarch64".to_owned(),
        interp: Some("/lib/ld-musl-aarch64.so.1".to_owned()),
        needed: vec!["libc.musl-aarch64.so.1".to_owned()],
        ..static_x86_64()
    }
}

/// Repack options for one selector, reading a pre-downloaded asset.
fn options(out: &Path, upstream_dir: &Path, selector: &str) -> RepackOptions {
    RepackOptions {
        upstream_tag: UPSTREAM_TAG.to_owned(),
        selectors: vec![RepackSelector::parse(selector).unwrap_or_else(|_| {
            RepackSelector {
                // A selector this test wrote itself: the parser is under test
                // elsewhere, and a RED parser must not take this file with it.
                target: selector
                    .split_once(':')
                    .map_or(selector, |(target, _)| target)
                    .to_owned(),
                variant: selector
                    .split_once(':')
                    .map_or(DEFAULT_VARIANT, |(_, variant)| variant)
                    .to_owned(),
            }
        })],
        out: out.to_path_buf(),
        upstream_dir: Some(upstream_dir.to_path_buf()),
        source_date_epoch: Some(1_756_598_400),
    }
}

// ---------------------------------------------------- the asset table --

#[test]
fn an_upstream_asset_is_named_by_the_architecture_and_the_variant() {
    let table = [
        ("linux-x86_64-musl", "static", "erlang-29.0.5-x64.tar.gz"),
        ("linux-aarch64-musl", "static", "erlang-29.0.5-arm64.tar.gz"),
        (
            "linux-x86_64-gnu",
            DEFAULT_VARIANT,
            "erlang-29.0.5-x64-glibc.tar.gz",
        ),
        (
            "linux-aarch64-gnu",
            DEFAULT_VARIANT,
            "erlang-29.0.5-arm64-glibc.tar.gz",
        ),
        (
            "linux-x86_64-musl",
            "dynamic",
            "erlang-29.0.5-x64-musl.tar.gz",
        ),
        (
            "linux-aarch64-musl",
            "dynamic",
            "erlang-29.0.5-arm64-musl.tar.gz",
        ),
    ];

    for (target, variant, expected) in table {
        assert_eq!(
            catalog::upstream_asset(VERSION, target, variant),
            Ok(expected.to_owned()),
            "{target}:{variant} is upstream's {expected}"
        );
    }
}

#[test]
fn a_combination_upstream_does_not_build_names_the_repository_that_was_asked() {
    for (target, variant) in [
        ("linux-x86_64-gnu", "static"),
        ("macos-aarch64", DEFAULT_VARIANT),
        ("windows-x86_64", DEFAULT_VARIANT),
        ("linux-x86_64-musl", "slim"),
    ] {
        assert_eq!(
            catalog::upstream_asset(VERSION, target, variant),
            Err(RepackError::NoUpstreamAsset {
                upstream: UPSTREAM_REPO,
                target: target.to_owned(),
                variant: variant.to_owned(),
            }),
            "{target}:{variant} has no upstream asset"
        );
    }
}

#[test]
fn a_selector_is_a_target_and_an_optional_variant() {
    assert_eq!(
        RepackSelector::parse("linux-x86_64-musl:static"),
        Ok(RepackSelector {
            target: "linux-x86_64-musl".to_owned(),
            variant: "static".to_owned(),
        })
    );
    assert_eq!(
        RepackSelector::parse("linux-x86_64-gnu"),
        Ok(RepackSelector {
            target: "linux-x86_64-gnu".to_owned(),
            variant: DEFAULT_VARIANT.to_owned(),
        }),
        "a bare target is its default variant"
    );
}

#[test]
fn a_selector_with_an_empty_half_is_refused_rather_than_guessed() {
    for value in ["linux-x86_64-musl:", ":static", "", "a:b:c"] {
        let error =
            RepackSelector::parse(value).expect_err(&format!("`{value}` is not a selector"));
        match &error {
            RepackError::BadSelector { value: named, .. } => assert_eq!(named, value),
            other => panic!("`{value}` is a bad selector, not {other:?}"),
        }
    }
}

#[test]
fn an_upstream_tag_names_the_version_the_catalog_is_keyed_by() {
    assert_eq!(
        catalog::version_from_tag("OTP-29.0.5"),
        Some("29.0.5".to_owned())
    );
    assert_eq!(catalog::version_from_tag("29.0.5"), None);
    assert_eq!(catalog::version_from_tag("OTP-"), None);
    assert_eq!(catalog::version_from_tag("v29.0.5"), None);
}

// --------------------------------------------------------- the prune --

#[test]
fn the_prune_list_strips_the_fat_and_keeps_the_include_directory() {
    let pruned = [
        "lib/stdlib-8.0.3/src/lists.erl",
        "lib/crypto-5.9.2/c_src/crypto.c",
        "lib/stdlib-8.0.3/doc/html/index.html",
        "erts-17.0.5/man/man1/erl.1",
        "lib/tools-4.1/emacs/erlang.el",
        "lib/ssl-11.7.4/examples/server.erl",
        "lib/otp_mibs-1.3/misc/notes",
        "erts-17.0.5/bin/beam.pdb",
    ];
    for relative in pruned {
        assert!(
            catalog::is_pruned(Path::new(relative)),
            "{relative} is fat a packaged runtime never reads"
        );
    }

    let kept = [
        "erts-17.0.5/include/erl_nif.h",
        "lib/erl_interface-5.6/include/ei.h",
        "lib/stdlib-8.0.3/ebin/lists.beam",
        "lib/crypto-5.9.2/priv/lib/crypto.so",
        "bin/no_dot_erlang.boot",
        "releases/29/OTP_VERSION",
        "lib/stdlib-8.0.3/ebin/source.erl",
    ];
    for relative in kept {
        assert!(
            !catalog::is_pruned(Path::new(relative)),
            "{relative} is what the runtime is made of"
        );
    }
}

#[test]
fn the_prune_list_matches_whole_components_and_never_a_substring() {
    assert!(
        !catalog::is_pruned(Path::new("lib/mydoc-1.0/ebin/mydoc.beam")),
        "an application whose name contains `doc` is not documentation"
    );
    assert!(
        !catalog::is_pruned(Path::new("lib/x-1.0/priv/srcfile")),
        "a file called `srcfile` is not a `src` directory"
    );
    assert!(
        catalog::is_pruned(Path::new("lib/mydoc-1.0/doc/mydoc.html")),
        "and the `doc` directory of that same application still goes"
    );
    assert_eq!(
        PRUNE_DIRS,
        ["c_src", "doc", "emacs", "examples", "man", "misc", "src"],
        "the list is sorted, because it is printed"
    );
    assert_eq!(KEPT_DIRS, ["ebin", "include", "priv"]);
}

#[test]
fn a_prune_reports_every_path_it_removed_and_what_they_cost() {
    let upstream = FakeUpstream::build(
        "erlang-29.0.5",
        &[
            "lib/stdlib-8.0.3/src/lists.erl",
            "lib/stdlib-8.0.3/doc/index.html",
            "erts-17.0.5/include/erl_nif.h",
        ],
    );
    let root = upstream.root();

    let summary = catalog::prune_tree(&root).expect("the tree is prunable");

    assert_eq!(
        summary.paths,
        vec![
            "lib/stdlib-8.0.3/doc/index.html".to_owned(),
            "lib/stdlib-8.0.3/src/lists.erl".to_owned(),
        ],
        "sorted, relative to the root, and nothing else was touched"
    );
    assert_eq!(summary.removed_files, 2);
    assert_eq!(summary.removed_bytes, 2 * b"upstream fat\n".len() as u64);
    assert!(
        root.join("erts-17.0.5/include/erl_nif.h").is_file(),
        "a NIF built against a packaged runtime needs the headers"
    );
    assert!(!root.join("lib/stdlib-8.0.3/src").exists());
}

// ---------------------------------------------------- the dereference --

#[test]
fn every_symlink_is_replaced_by_a_copy_of_what_it_points_at() {
    let upstream = FakeUpstream::build("erlang-29.0.5", &["lib/stdlib-8.0.3/priv/real.txt"]);
    let root = upstream.root();
    std::os::unix::fs::symlink("real.txt", root.join("lib/stdlib-8.0.3/priv/link.txt"))
        .expect("a relative link inside the application");

    let summary = catalog::dereference_symlinks(&root).expect("the links resolve");

    assert_eq!(
        summary.paths,
        vec!["lib/stdlib-8.0.3/priv/link.txt".to_owned()]
    );
    assert_eq!(summary.bytes_added, b"upstream fat\n".len() as u64);
    assert!(
        !root
            .join("lib/stdlib-8.0.3/priv/link.txt")
            .symlink_metadata()
            .expect("the former link")
            .file_type()
            .is_symlink(),
        "what is there now is a file, not a link"
    );
    assert_eq!(
        std::fs::read(root.join("lib/stdlib-8.0.3/priv/link.txt")).expect("the copy"),
        b"upstream fat\n",
        "and it holds what the link pointed at"
    );
    assert_eq!(catalog::assert_no_symlinks(&root), Ok(()));
}

#[test]
fn a_symlink_to_nothing_is_refused_rather_than_quietly_dropped() {
    let upstream = FakeUpstream::build("erlang-29.0.5", &[]);
    let root = upstream.root();
    let link = root.join("bin/dangling");
    std::os::unix::fs::symlink("nowhere", &link).expect("a dangling link");

    let error = catalog::dereference_symlinks(&root).expect_err("it points at nothing");

    assert_eq!(
        error,
        RepackError::DanglingSymlink {
            path: link,
            target: PathBuf::from("nowhere"),
        }
    );
}

#[test]
fn a_symlink_out_of_the_runtime_root_is_refused() {
    let upstream = FakeUpstream::build("erlang-29.0.5", &[]);
    let root = upstream.root();
    let outside = upstream.root().parent().expect("a parent").join("outside");
    std::fs::write(&outside, b"not part of the runtime\n").expect("a file outside the root");
    let link = root.join("bin/escape");
    std::os::unix::fs::symlink(&outside, &link).expect("an escaping link");

    let error = catalog::dereference_symlinks(&root).expect_err("it leaves the root");

    assert_eq!(
        error,
        RepackError::UnsafeSymlink {
            path: link,
            target: outside,
        }
    );
}

#[test]
fn a_link_left_behind_fails_the_assertion_that_guards_the_strict_extractor() {
    let upstream = FakeUpstream::build("erlang-29.0.5", &[]);
    let root = upstream.root();
    let link = root.join("bin/still-a-link");
    std::os::unix::fs::symlink("no_dot_erlang.boot", &link).expect("a link");

    assert_eq!(
        catalog::assert_no_symlinks(&root),
        Err(RepackError::SymlinkRemains { path: link }),
        "the tarball the cache extracts must hold no link at all"
    );
}

// ---------------------------------------------------- the pipeline --

#[test]
fn a_repack_reads_a_pre_downloaded_asset_and_writes_a_tarball_and_an_entry() {
    let dir = tempdir();
    let upstream = FakeUpstream::build(
        "erlang-29.0.5",
        &["lib/stdlib-8.0.3/src/lists.erl", "erts-17.0.5/doc/notes.md"],
    );
    let upstream_dir = dir.path().join("upstream");
    upstream.write_in(&upstream_dir, "erlang-29.0.5-x64.tar.gz");
    let out = dir.path().join("dist/otp");

    let report = catalog::repack_with(
        &options(&out, &upstream_dir, "linux-x86_64-musl:static"),
        &Net::offline(),
        &Diag::disabled(),
        |_| Ok(static_x86_64()),
    )
    .expect("a pre-downloaded asset needs no network");

    assert_eq!(report.outcomes.len(), 1);
    let outcome = &report.outcomes[0];
    assert_eq!(outcome.target, "linux-x86_64-musl");
    assert_eq!(outcome.variant, "static");
    assert_eq!(outcome.upstream_file, "erlang-29.0.5-x64.tar.gz");
    assert_eq!(
        outcome.tarball,
        out.join("otp-29.0.5-linux-x86_64-musl-static.tar.zst")
    );
    assert!(outcome.tarball.is_file(), "the tarball is where it says");
    assert_eq!(
        outcome.tarball_bytes,
        std::fs::metadata(&outcome.tarball)
            .expect("the tarball")
            .len()
    );
    assert_eq!(
        outcome.prune.paths,
        vec![
            "erts-17.0.5/doc/notes.md".to_owned(),
            "lib/stdlib-8.0.3/src/lists.erl".to_owned(),
        ],
        "the prune is reported per repack, not summed away"
    );

    assert_eq!(outcome.entry.linkage, "static");
    assert!(!outcome.entry.nif_loading);
    assert_eq!(outcome.entry.libc.kind, "none");
    assert_eq!(outcome.entry.size, outcome.tarball_bytes);
    assert_eq!(outcome.entry.upstream.repo, UPSTREAM_REPO);
    assert_eq!(outcome.entry.upstream.tag, UPSTREAM_TAG);
    assert_eq!(outcome.entry.upstream.file, "erlang-29.0.5-x64.tar.gz");
    assert_eq!(
        outcome.entry.upstream.sha256,
        upstream.sha256_hex(),
        "the provenance carries the digest of the bytes upstream actually served"
    );
    assert_eq!(
        outcome.entry.built_at, "2025-08-31T00:00:00Z",
        "SOURCE_DATE_EPOCH decides the timestamp, so two repacks agree"
    );
}

#[test]
fn the_catalog_a_repack_writes_names_its_tarballs_relative_to_itself() {
    let dir = tempdir();
    let upstream = FakeUpstream::build("erlang-29.0.5", &[]);
    let upstream_dir = dir.path().join("upstream");
    upstream.write_in(&upstream_dir, "erlang-29.0.5-x64.tar.gz");
    let out = dir.path().join("dist/otp");

    let report = catalog::repack_with(
        &options(&out, &upstream_dir, "linux-x86_64-musl:static"),
        &Net::offline(),
        &Diag::disabled(),
        |_| Ok(static_x86_64()),
    )
    .expect("the repack runs");

    assert_eq!(report.catalog, out.join("catalog.json"));
    let text = std::fs::read_to_string(&report.catalog).expect("the catalog");
    let written = ginary::catalog::Catalog::parse(&text, "the repacked catalog")
        .expect("what it wrote, it can read");
    let entry = &written.otp[VERSION].targets["linux-x86_64-musl"].variants["static"];

    assert_eq!(
        entry.url, "otp-29.0.5-linux-x86_64-musl-static.tar.zst",
        "a file name, so a checkout with the tarballs beside it resolves it"
    );
    assert_eq!(
        catalog::resolve_url(&entry.url, report.catalog.parent()),
        SourceUrl::File(out.join("otp-29.0.5-linux-x86_64-musl-static.tar.zst"))
    );
    assert_eq!(written.otp[VERSION].erts_vsn, ERTS_VSN);
    assert_eq!(written.generated_at, "2025-08-31T00:00:00Z");
}

#[test]
fn an_asset_whose_emulator_is_for_another_target_is_refused_before_it_is_shipped() {
    let dir = tempdir();
    let upstream = FakeUpstream::build("erlang-29.0.5", &[]);
    let upstream_dir = dir.path().join("upstream");
    upstream.write_in(&upstream_dir, "erlang-29.0.5-x64.tar.gz");
    let out = dir.path().join("dist/otp");

    let error = catalog::repack_with(
        &options(&out, &upstream_dir, "linux-x86_64-musl:static"),
        &Net::offline(),
        &Diag::disabled(),
        |_| Ok(dynamic_aarch64()),
    )
    .expect_err("the emulator is not the machine the asset name claims");

    assert_eq!(
        error,
        RepackError::UpstreamMismatch {
            file: "erlang-29.0.5-x64.tar.gz".to_owned(),
            target: "linux-x86_64-musl".to_owned(),
            actual: "linux-aarch64-musl".to_owned(),
        },
        "a mislabelled upstream asset must not become a mislabelled catalog entry"
    );
    assert!(
        !out.join("catalog.json").exists(),
        "and nothing is written for a target that failed its check"
    );
}

#[test]
fn a_repack_with_no_local_asset_and_no_network_says_which_asset_is_missing() {
    let dir = tempdir();
    let upstream_dir = dir.path().join("empty");
    std::fs::create_dir_all(&upstream_dir).expect("an empty upstream directory");
    let out = dir.path().join("dist/otp");

    let error = catalog::repack_with(
        &options(&out, &upstream_dir, "linux-x86_64-musl:static"),
        &Net::offline(),
        &Diag::disabled(),
        |_| Ok(static_x86_64()),
    )
    .expect_err("there is nothing to repack");

    let rendered = error.to_string();
    assert!(
        rendered.contains("erlang-29.0.5-x64.tar.gz"),
        "the message names the asset that would have been fetched: {rendered}"
    );
}

#[test]
fn the_emulator_reader_is_a_seam_and_its_error_travels() {
    let dir = tempdir();
    let upstream = FakeUpstream::build("erlang-29.0.5", &[]);
    let upstream_dir = dir.path().join("upstream");
    upstream.write_in(&upstream_dir, "erlang-29.0.5-x64.tar.gz");
    let out = dir.path().join("dist/otp");

    let error = catalog::repack_with(
        &options(&out, &upstream_dir, "linux-x86_64-musl:static"),
        &Net::offline(),
        &Diag::disabled(),
        |_| Err(ElfError::NotElf),
    )
    .expect_err("an unreadable emulator is not a runtime");

    assert!(
        error.to_string().contains("not an ELF file"),
        "what the reader said travels rather than being swallowed: {error}"
    );
}

// -------------------------------------------------- the release API --

/// The path the release API is asked for, once the base is redirected.
fn release_path() -> String {
    format!("/repos/{UPSTREAM_REPO}/releases/tags/{UPSTREAM_TAG}")
}

/// The path the asset is served from.
const ASSET_PATH: &str = "/assets/erlang-29.0.5-x64.tar.gz";

/// The asset name the mapping produces for `linux-x86_64-musl:static`.
const ASSET_NAME: &str = "erlang-29.0.5-x64.tar.gz";

/// A release document naming one asset called `name`.
///
/// `browser_download_url` is written under [`GITHUB_API_BASE`] so that the
/// same base override that redirects the API redirects the download; upstream
/// serves it from another host, and what is under test here is that the
/// pipeline follows the URL the API gave it.
fn release_json(name: &str, digest: Option<&str>, size: Option<u64>) -> Vec<u8> {
    let mut asset = serde_json::Map::new();
    asset.insert("name".to_owned(), serde_json::json!(name));
    asset.insert(
        "browser_download_url".to_owned(),
        serde_json::json!(format!("{GITHUB_API_BASE}{ASSET_PATH}")),
    );
    if let Some(digest) = digest {
        asset.insert("digest".to_owned(), serde_json::json!(digest));
    }
    if let Some(size) = size {
        asset.insert("size".to_owned(), serde_json::json!(size));
    }
    serde_json::to_vec(&serde_json::json!({
        "tag_name": UPSTREAM_TAG,
        "assets": [serde_json::Value::Object(asset)],
    }))
    .expect("the fake release document serialises")
}

/// A server answering the release API and the asset it names.
fn release_server(document: Vec<u8>, asset: &[u8]) -> TestServer {
    TestServer::start(std::collections::BTreeMap::from([
        (release_path(), vec![Reply::ok(&document)]),
        (ASSET_PATH.to_owned(), vec![Reply::ok(asset)]),
    ]))
}

/// A network whose GitHub base is `server`.
fn net_through(server: &TestServer) -> Net {
    Net {
        offline: false,
        base_overrides: std::collections::BTreeMap::from([(
            GITHUB_API_BASE.to_owned(),
            server.base(),
        )]),
    }
}

/// Runs one repack that has no local asset, so the API is consulted.
fn repack_through(
    server: &TestServer,
    out: &Path,
    upstream_dir: &Path,
) -> Result<catalog::RepackReport, RepackError> {
    catalog::repack_with(
        &options(out, upstream_dir, "linux-x86_64-musl:static"),
        &net_through(server),
        &Diag::disabled(),
        |_| Ok(static_x86_64()),
    )
}

#[test]
fn a_repack_with_no_local_asset_pins_the_digest_the_release_api_reported() {
    let dir = tempdir();
    let upstream = FakeUpstream::build("erlang-29.0.5", &[]);
    let digest = upstream.sha256_hex();
    let server = release_server(
        release_json(
            ASSET_NAME,
            Some(&format!("sha256:{digest}")),
            Some(upstream.bytes().len() as u64),
        ),
        upstream.bytes(),
    );

    let out = dir.path().join("dist/otp");
    let upstream_dir = dir.path().join("upstream");
    let report = repack_through(&server, &out, &upstream_dir)
        .expect("the release API named an asset and the bytes verified");

    let outcome = &report.outcomes[0];
    assert_eq!(
        outcome.entry.upstream.sha256, digest,
        "the entry pins the digest the API reported, which is the digest of the bytes"
    );
    assert_eq!(outcome.entry.upstream.tag, UPSTREAM_TAG);
    assert_eq!(outcome.entry.upstream.file, ASSET_NAME);
    assert!(outcome.tarball.is_file(), "and a tarball came out");
    assert_eq!(server.hits(&release_path()), 1, "the API is asked once");
    assert_eq!(server.hits(ASSET_PATH), 1, "and the asset fetched once");
    assert!(
        upstream_dir.join(ASSET_NAME).is_file(),
        "the asset lands in --upstream-dir, so a second run costs nothing"
    );
}

#[test]
fn an_asset_whose_bytes_do_not_match_the_reported_digest_is_refused() {
    let dir = tempdir();
    let upstream = FakeUpstream::build("erlang-29.0.5", &[]);
    let lie = "b".repeat(64);
    let server = release_server(
        release_json(
            ASSET_NAME,
            Some(&format!("sha256:{lie}")),
            Some(upstream.bytes().len() as u64),
        ),
        upstream.bytes(),
    );

    let error = repack_through(
        &server,
        &dir.path().join("dist/otp"),
        &dir.path().join("upstream"),
    )
    .expect_err("the API's digest is what the bytes are held to");

    match &error {
        RepackError::Download {
            reason:
                DownloadError::ChecksumMismatch {
                    expected, actual, ..
                },
        } => {
            assert_eq!(*expected, lie, "the message names what was promised");
            assert_eq!(*actual, upstream.sha256_hex(), "and what actually arrived");
        }
        other => panic!("expected a checksum mismatch, got {other:?}"),
    }
    assert!(
        !dir.path().join("dist/otp").join("catalog.json").exists(),
        "and nothing was written into the output directory"
    );
}

#[test]
fn an_asset_the_release_api_reports_no_digest_for_is_refused_rather_than_pinned_to_nothing() {
    let dir = tempdir();
    let upstream = FakeUpstream::build("erlang-29.0.5", &[]);
    let server = release_server(
        release_json(ASSET_NAME, None, Some(upstream.bytes().len() as u64)),
        upstream.bytes(),
    );

    let error = repack_through(
        &server,
        &dir.path().join("dist/otp"),
        &dir.path().join("upstream"),
    )
    .expect_err("a repack pins what it fetched and cannot pin nothing");

    match &error {
        RepackError::Api { url, message } => {
            assert!(url.contains(UPSTREAM_TAG), "the URL that was read: {url}");
            assert!(
                message.contains("carries no digest"),
                "and what was missing: {message}"
            );
        }
        other => panic!("expected RepackError::Api, got {other:?}"),
    }
    assert_eq!(server.hits(ASSET_PATH), 0, "nothing was fetched");
}

#[test]
fn a_release_that_does_not_hold_the_asset_names_the_one_that_was_looked_for() {
    let dir = tempdir();
    let upstream = FakeUpstream::build("erlang-29.0.5", &[]);
    let server = release_server(
        release_json("erlang-29.0.5-arm64.tar.gz", Some(&"0".repeat(64)), None),
        upstream.bytes(),
    );

    let error = repack_through(
        &server,
        &dir.path().join("dist/otp"),
        &dir.path().join("upstream"),
    )
    .expect_err("the release holds another architecture's asset and not this one");

    match &error {
        RepackError::Api { message, .. } => assert!(
            message.contains(ASSET_NAME) && message.contains(UPSTREAM_TAG),
            "the message names the tag and the file: {message}"
        ),
        other => panic!("expected RepackError::Api, got {other:?}"),
    }
}

#[test]
fn a_release_document_that_is_not_one_says_so_rather_than_guessing() {
    let dir = tempdir();
    let server = TestServer::one(&release_path(), Reply::ok(b"{\"tag_name\":\"OTP-29.0.5\"}"));

    let error = repack_through(
        &server,
        &dir.path().join("dist/otp"),
        &dir.path().join("upstream"),
    )
    .expect_err("a document with no assets array is not a release description");

    match &error {
        RepackError::Api { message, .. } => assert!(
            message.contains("names no assets"),
            "the message says what was missing: {message}"
        ),
        other => panic!("expected RepackError::Api, got {other:?}"),
    }
}

// -------------------------------------------------------- the timestamps --

#[test]
fn a_timestamp_is_rfc_3339_in_utc() {
    assert_eq!(catalog::timestamp(0), "1970-01-01T00:00:00Z");
    assert_eq!(catalog::timestamp(1_756_598_400), "2025-08-31T00:00:00Z");
    assert_eq!(catalog::timestamp(1_756_598_401), "2025-08-31T00:00:01Z");
}

#[test]
fn the_variant_name_decides_the_linkage_the_entry_claims() {
    assert_eq!(catalog::claimed_linkage("static"), Linkage::Static);
    assert_eq!(catalog::claimed_linkage("dynamic"), Linkage::Dynamic);
    assert_eq!(catalog::claimed_linkage(DEFAULT_VARIANT), Linkage::Dynamic);
}

#[test]
fn a_catalog_error_is_not_a_repack_error_and_the_two_do_not_share_a_variant() {
    // The repack writes a catalogue and the build reads one; a reader that
    // conflated the two error types would print "cannot fetch the runtime"
    // when the pipeline could not write a file.
    let repack: RepackError = RepackError::Io {
        path: PathBuf::from("/dist/otp/catalog.json"),
        message: "permission denied".to_owned(),
    };
    let read: CatalogError = CatalogError::Io {
        path: PathBuf::from("/dist/otp/catalog.json"),
        message: "permission denied".to_owned(),
    };

    assert_eq!(
        repack.to_string(),
        "cannot use /dist/otp/catalog.json: permission denied"
    );
    assert_eq!(read.to_string(), repack.to_string());
    assert_eq!(
        catalog::version_from_tag(UPSTREAM_TAG),
        Some(VERSION.to_owned())
    );
}
