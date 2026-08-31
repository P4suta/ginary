// SPDX-License-Identifier: MIT OR Apache-2.0
//! `ginary otp update` had two spellings of one question and one unsafe write.
//!
//! The command decided remote-versus-path with `source.contains("://")` while
//! `catalog::resolve_url` used the stricter `has_scheme` — a scheme is an
//! ASCII letter followed by alphanumerics, `+`, `-` or `.` — so a *path* that
//! happened to hold `://` was posted to the network by one half of the module
//! and read off the disk by the other.
//!
//! And the write itself was `std::fs::write(&destination, &text)`, which
//! truncates before it writes. The comment above it promised that "an update
//! that damaged the catalog already installed would take the cache down with
//! the document it refused" — true of a document that fails to parse, and not
//! true of one that fails halfway through the write, or of any reader that
//! opens the file while it is being replaced. The whole rest of the crate uses
//! temp-then-rename for exactly this: `download::fetch`, `extract_into_cache`,
//! and the artifact writer.
//!
//! The right behaviour: one spelling of "is this a URL", and an install that
//! a reader either sees entirely or does not see at all.
#![cfg(feature = "cli")]

use std::path::Path;

use assert_cmd::Command;

use ginary::catalog::{self, CATALOG_FILE, Catalog};

use crate::common::catalog::{CatalogBuilder, ERTS_VSN, RELEASE, VERSION, static_variant};

/// How many installs the reader has to catch in flight.
const ROUNDS: usize = 60;

/// A catalogue big enough that a truncating write is caught mid-flight.
fn big_catalog(entries: usize) -> String {
    let mut builder = CatalogBuilder::new();
    for index in 0..entries {
        builder = builder.entry(
            &format!("29.0.{index}"),
            RELEASE,
            ERTS_VSN,
            "linux-x86_64-musl",
            "static",
            static_variant(&format!("otp-{index}.tar.zst"), &"a".repeat(64), 41_943_040),
        );
    }
    builder.json()
}

#[test]
fn a_reader_never_sees_half_an_installed_catalog() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let destination = dir.path().join(CATALOG_FILE);
    let text = big_catalog(400);
    assert!(
        text.len() > 256 * 1024,
        "the fixture has to be larger than one write buffer: {} bytes",
        text.len()
    );

    catalog::install(&text, &destination).expect("the first install");

    let stop = std::sync::atomic::AtomicBool::new(false);
    let seen = std::thread::scope(|scope| {
        let writer = scope.spawn(|| {
            for _ in 0..ROUNDS {
                catalog::install(&text, &destination).expect("an install");
            }
            stop.store(true, std::sync::atomic::Ordering::SeqCst);
        });
        let reader = scope.spawn(|| {
            let mut reads = 0_u64;
            while !stop.load(std::sync::atomic::Ordering::SeqCst) {
                let Ok(read) = std::fs::read_to_string(&destination) else {
                    // The file may be renamed out from under the open; that is
                    // the atomic case, not a torn read.
                    continue;
                };
                reads += 1;
                Catalog::parse(&read, "the installed catalog").unwrap_or_else(|error| {
                    panic!(
                        "a reader saw {} bytes of a {} byte document: {error}",
                        read.len(),
                        text.len()
                    )
                });
            }
            reads
        });
        writer.join().expect("the writer finished");
        reader.join().expect("the reader finished")
    });

    assert!(seen > 0, "the reader looked at the file at least once");
    assert_eq!(
        std::fs::read_to_string(&destination).expect("the installed catalog"),
        text,
        "and what is left is the whole document"
    );
    let residue: Vec<String> = std::fs::read_dir(dir.path())
        .expect("the directory")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name != CATALOG_FILE)
        .collect();
    assert!(
        residue.is_empty(),
        "no staging file is left behind: {residue:?}"
    );
}

#[test]
fn a_path_that_holds_a_double_slash_is_a_path_and_not_a_url() {
    // `contains("://")` says this is remote; `has_scheme` says the scheme
    // would have to start with a letter, and `/tmp/.../x` does not.
    let dir = tempfile::tempdir().expect("a temporary directory");
    let odd = dir.path().join("x:");
    std::fs::create_dir_all(&odd).expect("a directory whose name ends in a colon");
    let source = odd.join("catalog.json");
    std::fs::write(
        &source,
        CatalogBuilder::new()
            .entry(
                VERSION,
                RELEASE,
                ERTS_VSN,
                "linux-x86_64-musl",
                "static",
                static_variant("otp.tar.zst", &"a".repeat(64), 1),
            )
            .json(),
    )
    .expect("a catalog at an awkward path");

    let spelled = format!("{}//catalog.json", odd.display());
    assert!(
        spelled.contains("://"),
        "the fixture is the shape the two spellings disagree about: {spelled}"
    );
    assert!(
        !catalog::has_scheme(&spelled),
        "and it is a path, because a scheme starts with a letter: {spelled}"
    );

    let cache = dir.path().join("cache");
    let mut command = Command::cargo_bin("ginary").expect("the `ginary` binary is built for tests");
    command
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env_clear()
        .env("GINARY_CACHE_DIR", &cache);
    let assert = command
        .args(["otp", "update", "--catalog"])
        .arg(&spelled)
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8");
    assert!(
        stdout.contains("installed 1 entries"),
        "the document was read off the disk rather than posted at a network: {stdout}"
    );
    assert!(
        installed(&cache).contains(VERSION),
        "and it is the document that was installed"
    );
}

/// The catalogue text `ginary otp update` left in `cache`.
fn installed(cache: &Path) -> String {
    std::fs::read_to_string(cache.join("otp").join(CATALOG_FILE))
        .expect("the installed catalog is where `otp update` puts it")
}
