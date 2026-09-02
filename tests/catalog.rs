// SPDX-License-Identifier: MIT OR Apache-2.0
//! The prebuilt-OTP catalogue: reading one, choosing out of one, and filling
//! the cache from one.
//!
//! Three groups of claim. The **schema** is text, and the one property that
//! matters beyond the field list is forward compatibility: a key this ginary
//! does not know survives a read rather than being refused or dropped.
//!
//! The **selection** is the rule a `gleam.toml` states and never spells out —
//! `otp_version = "host"` means the newest patch of the release that compiled
//! the shipment, a musl target means the static variant unless somebody says
//! otherwise, and a runtime older than the compiler is refused rather than
//! shipped.
//!
//! The **cache** is where the two meet: a complete extraction is one with a
//! `.meta.json` beside it, an incomplete one is fetched again, and a catalogue
//! URL with no scheme is a file beside the catalogue itself, which is what
//! makes a committed `dist/otp/catalog.json` work in a checkout with no hosted
//! catalogue anywhere.
// The command line half of the suite: a launcher-only build has no catalogue.
#![cfg(feature = "cli")]

mod common;

use std::cmp::Ordering;
use std::path::{Path, PathBuf};

use ginary::catalog::{
    self, Catalog, CatalogError, CatalogOrigin, CatalogPaths, DEFAULT_MUSL_VARIANT,
    DEFAULT_VARIANT, EMBEDDED, EnsureContext, META_FILE, Meta, OtpReq, RELEASE_WARN_AHEAD,
    SCHEMA_VERSION, SourceUrl,
};
use ginary::diag::Diag;
use ginary::download::{DownloadError, Net};

use crate::common::catalog::{
    CatalogBuilder, ERTS_VSN, RELEASE, VERSION, gnu_variant, plant_cached_otp, runtime_tarball,
    static_variant, write_catalog_text,
};
use crate::common::fake_otp::FakeOtp;
use crate::common::http::{Reply, TestServer};
use crate::common::payload::{
    RawEntry, RawTar, TYPE_CHAR_DEVICE, TYPE_SYMLINK, sha256_hex, tree_listing,
};

/// The musl target every selection test asks for.
const MUSL: &str = "linux-x86_64-musl";

/// The glibc target beside it.
const GNU: &str = "linux-x86_64-gnu";

/// A temporary directory the test owns.
fn tempdir() -> tempfile::TempDir {
    tempfile::tempdir().expect("a temporary directory")
}

/// A catalogue with one static musl runtime and one dynamic glibc runtime.
fn two_target_catalog() -> CatalogBuilder {
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
            DEFAULT_VARIANT,
            gnu_variant(
                "otp-29.0.5-linux-x86_64-gnu.tar.zst",
                &"b".repeat(64),
                40_000_000,
            ),
        )
}

// -------------------------------------------------------- the schema --

#[test]
fn a_schema_v1_document_reads_back_every_field_of_an_entry() {
    let text = two_target_catalog().json();

    let catalog = Catalog::parse(&text, "the fixture").expect("the fixture is schema 1");

    assert_eq!(catalog.schema_version, SCHEMA_VERSION);
    assert_eq!(catalog.generated_at, "2026-08-31T00:00:00Z");
    let version = catalog.otp.get(VERSION).expect("OTP 29.0.5 is in it");
    assert_eq!(version.erts_vsn, ERTS_VSN);
    assert_eq!(version.otp_release, RELEASE);
    assert_eq!(
        version.targets.keys().cloned().collect::<Vec<_>>(),
        vec![GNU.to_owned(), MUSL.to_owned()],
        "targets are sorted, so a catalog reads the same on every machine"
    );

    let entry = version.targets[MUSL].variants["static"].clone();
    assert_eq!(entry.url, "otp-29.0.5-linux-x86_64-musl-static.tar.zst");
    assert_eq!(entry.sha256, "a".repeat(64));
    assert_eq!(entry.size, 41_943_040);
    assert_eq!(entry.linkage, "static");
    assert!(!entry.nif_loading, "a fully static runtime dlopens nothing");
    assert_eq!(entry.libc.kind, "none");
    assert_eq!(entry.libc.min, None);
    assert_eq!(entry.openssl, "3.5.4");
    assert!(entry.jit);
    assert_eq!(entry.excluded_apps, Vec::<String>::new());
    assert_eq!(entry.upstream.repo, catalog::UPSTREAM_REPO);
    assert_eq!(entry.upstream.tag, "OTP-29.0.5");
    assert_eq!(entry.upstream.file, "erlang-29.0.5-x64.tar.gz");
    assert_eq!(entry.built_at, "2026-08-31T00:00:00Z");
}

#[test]
fn a_key_this_ginary_does_not_know_is_kept_rather_than_refused_or_dropped() {
    let mut value: serde_json::Value =
        serde_json::from_str(&two_target_catalog().json()).expect("the fixture is JSON");
    value["signature"] = serde_json::Value::from("cosign:later");
    value["otp"][VERSION]["targets"][MUSL]["variants"]["static"]["wx"] =
        serde_json::Value::from(false);
    let text = serde_json::to_string_pretty(&value).expect("re-serialise");

    let catalog = Catalog::parse(&text, "the fixture").expect("a newer catalog still reads");

    assert_eq!(
        catalog.extra.get("signature"),
        Some(&serde_json::Value::from("cosign:later")),
        "a catalog written by a newer ginary is readable by this one"
    );
    assert_eq!(
        catalog.otp[VERSION].targets[MUSL].variants["static"]
            .extra
            .get("wx"),
        Some(&serde_json::Value::from(false)),
        "and the key it does not understand survives to be written back"
    );
}

#[test]
fn a_document_of_another_schema_names_both_versions() {
    let text = r#"{"schema_version":2,"generated_at":"2026-08-31T00:00:00Z","otp":{}}"#;

    let error = Catalog::parse(text, "/tmp/catalog.json").expect_err("schema 2 is not schema 1");

    assert_eq!(
        error,
        CatalogError::SchemaVersion {
            origin: "/tmp/catalog.json".to_owned(),
            found: 2,
            supported: SCHEMA_VERSION,
        }
    );
    assert_eq!(
        error.to_string(),
        "/tmp/catalog.json is catalog schema 2 and this ginary reads schema 1"
    );
}

#[test]
fn a_document_that_is_not_a_catalog_says_where_the_reader_stopped() {
    let error = Catalog::parse("{\n  \"schema_version\": ,\n}\n", "/tmp/catalog.json")
        .expect_err("that is not JSON");

    match error {
        CatalogError::Parse { origin, message } => {
            assert_eq!(origin, "/tmp/catalog.json");
            assert!(
                message.contains("line 2") && message.contains("column"),
                "the reader's position travels: {message}"
            );
        }
        other => panic!("a malformed document is a parse error, not {other:?}"),
    }
}

#[test]
fn the_embedded_catalog_is_a_valid_document_with_nothing_in_it() {
    let catalog = Catalog::parse(EMBEDDED, "the embedded catalog").expect("it parses");

    assert_eq!(catalog.schema_version, SCHEMA_VERSION);
    assert!(
        catalog.otp.is_empty(),
        "there is no hosted catalog to snapshot, so it holds no entries at all"
    );
}

// -------------------------------------------------------- the loading --

#[test]
fn a_named_catalog_wins_over_the_cached_one() {
    let dir = tempdir();
    let named = two_target_catalog().write_in(&dir.path().join("named"));
    let cached = CatalogBuilder::new()
        .entry(
            "29.0.4",
            RELEASE,
            ERTS_VSN,
            MUSL,
            "static",
            static_variant("otp-29.0.4.tar.zst", &"c".repeat(64), 1),
        )
        .write_in(&dir.path().join("cache"));

    let loaded = Catalog::load(&CatalogPaths {
        explicit: Some(named.clone()),
        cache: Some(cached),
    })
    .expect("the named catalog is there");

    assert_eq!(loaded.origin, CatalogOrigin::Explicit(named.clone()));
    assert_eq!(
        loaded.catalog.otp.keys().cloned().collect::<Vec<_>>(),
        vec![VERSION.to_owned()],
        "first found wins the whole file: there is no per-entry merge"
    );
    assert_eq!(loaded.origin.dir(), named.parent());
}

#[test]
fn the_cached_catalog_is_used_when_none_was_named() {
    let dir = tempdir();
    let cached = two_target_catalog().write_in(&dir.path().join("cache"));

    let loaded = Catalog::load(&CatalogPaths {
        explicit: None,
        cache: Some(cached.clone()),
    })
    .expect("the cached catalog is there");

    assert_eq!(loaded.origin, CatalogOrigin::Cache(cached));
    assert_eq!(loaded.catalog.otp.len(), 1);
}

#[test]
fn nothing_on_disk_falls_back_to_the_embedded_catalog() {
    let dir = tempdir();

    let loaded = Catalog::load(&CatalogPaths {
        explicit: None,
        cache: Some(dir.path().join("cache/catalog.json")),
    })
    .expect("an absent cache catalog is not an error");

    assert_eq!(loaded.origin, CatalogOrigin::Embedded);
    assert!(loaded.catalog.otp.is_empty());
    assert_eq!(
        loaded.origin.dir(),
        None,
        "the embedded catalog names no files"
    );
    assert_eq!(loaded.origin.label(), "the embedded catalog");
}

#[test]
fn a_catalog_that_was_named_and_is_not_there_is_an_error_rather_than_a_fallback() {
    let dir = tempdir();
    let missing = dir.path().join("nowhere/catalog.json");

    let error = Catalog::load(&CatalogPaths {
        explicit: Some(missing.clone()),
        cache: None,
    })
    .expect_err("a catalog somebody named has to be there");

    match error {
        CatalogError::Io { path, .. } => assert_eq!(path, missing),
        other => panic!("a named catalog that is absent is an io error, not {other:?}"),
    }
}

#[test]
fn a_cached_catalog_that_is_there_and_unreadable_is_an_error() {
    let dir = tempdir();
    let cached = write_catalog_text(&dir.path().join("cache"), "not a catalog at all\n");

    let error = Catalog::load(&CatalogPaths {
        explicit: None,
        cache: Some(cached),
    })
    .expect_err("a broken cached catalog is not silently skipped");

    assert!(
        matches!(error, CatalogError::Parse { .. }),
        "an unreadable catalog is reported, not fallen back from: {error:?}"
    );
}

// ------------------------------------------------------ the selection --

#[test]
fn the_host_rule_takes_the_newest_patch_of_the_hosts_release() {
    let catalog = CatalogBuilder::new()
        .entry(
            "29.0.9",
            29,
            ERTS_VSN,
            MUSL,
            "static",
            static_variant("a.tar.zst", &"a".repeat(64), 1),
        )
        .entry(
            "29.0.10",
            29,
            ERTS_VSN,
            MUSL,
            "static",
            static_variant("b.tar.zst", &"b".repeat(64), 2),
        )
        .entry(
            "30.0.1",
            30,
            ERTS_VSN,
            MUSL,
            "static",
            static_variant("c.tar.zst", &"c".repeat(64), 3),
        )
        .build();

    let selected = catalog
        .select(&OtpReq::Host(29), MUSL, None, "the fixture")
        .expect("release 29 is in the catalog");

    assert_eq!(
        selected.version, "29.0.10",
        "the newest patch, compared component by component rather than as strings"
    );
    assert_eq!(selected.entry.url, "b.tar.zst");
    assert_eq!(selected.warnings, Vec::<String>::new());
}

#[test]
fn an_exact_version_takes_that_entry_and_no_other() {
    let catalog = CatalogBuilder::new()
        .entry(
            "29.0.9",
            29,
            ERTS_VSN,
            MUSL,
            "static",
            static_variant("a.tar.zst", &"a".repeat(64), 1),
        )
        .entry(
            "29.0.10",
            29,
            ERTS_VSN,
            MUSL,
            "static",
            static_variant("b.tar.zst", &"b".repeat(64), 2),
        )
        .build();

    let selected = catalog
        .select(
            &OtpReq::Exact {
                version: "29.0.9".to_owned(),
                host_release: RELEASE,
            },
            MUSL,
            None,
            "the fixture",
        )
        .expect("29.0.9 is in the catalog");

    assert_eq!(selected.version, "29.0.9");
    assert_eq!(selected.entry.url, "a.tar.zst");
}

#[test]
fn a_version_that_is_not_in_the_catalog_names_the_ones_that_are() {
    let catalog = two_target_catalog().build();

    let error = catalog
        .select(
            &OtpReq::Exact {
                version: "28.3.1".to_owned(),
                host_release: RELEASE,
            },
            MUSL,
            None,
            "dist/otp/catalog.json",
        )
        .expect_err("28.3.1 is not in it");

    assert_eq!(
        error,
        CatalogError::NoSuchVersion {
            origin: "dist/otp/catalog.json".to_owned(),
            req: "28.3.1".to_owned(),
            available: "29.0.5".to_owned(),
        }
    );
    assert_eq!(
        error.to_string(),
        "dist/otp/catalog.json has no OTP 28.3.1 entry; it has 29.0.5"
    );
}

#[test]
fn a_host_release_that_is_not_in_the_catalog_says_so_as_the_host_rule() {
    let catalog = two_target_catalog().build();

    let error = catalog
        .select(&OtpReq::Host(27), MUSL, None, "dist/otp/catalog.json")
        .expect_err("there is no release 27 entry");

    assert_eq!(
        error,
        CatalogError::NoSuchVersion {
            origin: "dist/otp/catalog.json".to_owned(),
            req: "host (release 27)".to_owned(),
            available: "29.0.5".to_owned(),
        }
    );
}

#[test]
fn a_target_that_is_not_built_names_the_targets_that_are() {
    let catalog = two_target_catalog().build();

    let error = catalog
        .select(
            &OtpReq::Exact {
                version: VERSION.to_owned(),
                host_release: RELEASE,
            },
            "linux-aarch64-musl",
            None,
            "dist/otp/catalog.json",
        )
        .expect_err("aarch64 is not in this fixture");

    assert_eq!(
        error,
        CatalogError::NoSuchTarget {
            origin: "dist/otp/catalog.json".to_owned(),
            version: VERSION.to_owned(),
            target: "linux-aarch64-musl".to_owned(),
            available: "linux-x86_64-gnu, linux-x86_64-musl".to_owned(),
        }
    );
}

#[test]
fn a_musl_target_defaults_to_the_static_variant() {
    let catalog = CatalogBuilder::new()
        .entry(
            VERSION,
            RELEASE,
            ERTS_VSN,
            MUSL,
            "static",
            static_variant("static.tar.zst", &"a".repeat(64), 1),
        )
        .entry(
            VERSION,
            RELEASE,
            ERTS_VSN,
            MUSL,
            "dynamic",
            gnu_variant("dynamic.tar.zst", &"b".repeat(64), 2),
        )
        .build();

    let selected = catalog
        .select(&OtpReq::Host(RELEASE), MUSL, None, "the fixture")
        .expect("a musl target has a default");

    assert_eq!(
        selected.variant, DEFAULT_MUSL_VARIANT,
        "the static build runs on any Linux, which is why it is the default"
    );
    assert_eq!(selected.entry.url, "static.tar.zst");
}

#[test]
fn a_named_variant_overrides_the_musl_default() {
    let catalog = CatalogBuilder::new()
        .entry(
            VERSION,
            RELEASE,
            ERTS_VSN,
            MUSL,
            "static",
            static_variant("static.tar.zst", &"a".repeat(64), 1),
        )
        .entry(
            VERSION,
            RELEASE,
            ERTS_VSN,
            MUSL,
            "dynamic",
            gnu_variant("dynamic.tar.zst", &"b".repeat(64), 2),
        )
        .build();

    let selected = catalog
        .select(&OtpReq::Host(RELEASE), MUSL, Some("dynamic"), "the fixture")
        .expect("dynamic is in the catalog");

    assert_eq!(selected.variant, "dynamic");
    assert!(
        selected.entry.nif_loading,
        "the dynamic musl build is the one a NIF can be loaded into"
    );
}

#[test]
fn a_target_with_one_variant_needs_no_name_at_all() {
    let catalog = two_target_catalog().build();

    let selected = catalog
        .select(&OtpReq::Host(RELEASE), GNU, None, "the fixture")
        .expect("one variant is its own default");

    assert_eq!(selected.variant, DEFAULT_VARIANT);
    assert_eq!(selected.dir_name(), "29.0.5-linux-x86_64-gnu");
}

#[test]
fn several_variants_and_no_default_lists_them_and_says_how_to_choose() {
    let catalog = CatalogBuilder::new()
        .entry(
            VERSION,
            RELEASE,
            ERTS_VSN,
            GNU,
            "slim",
            static_variant("slim.tar.zst", &"a".repeat(64), 1),
        )
        .entry(
            VERSION,
            RELEASE,
            ERTS_VSN,
            GNU,
            "full",
            static_variant("full.tar.zst", &"b".repeat(64), 2),
        )
        .build();

    let error = catalog
        .select(&OtpReq::Host(RELEASE), GNU, None, "the fixture")
        .expect_err("there is nothing to choose between them");

    assert_eq!(
        error,
        CatalogError::AmbiguousVariant {
            version: VERSION.to_owned(),
            target: GNU.to_owned(),
            available: "full, slim".to_owned(),
        }
    );
    assert!(
        error.to_string().contains("otp_variant"),
        "the message names the setting that answers it: {error}"
    );
}

#[test]
fn a_variant_that_is_not_there_names_the_ones_that_are() {
    let catalog = two_target_catalog().build();

    let error = catalog
        .select(&OtpReq::Host(RELEASE), MUSL, Some("dynamic"), "the fixture")
        .expect_err("this fixture has only the static musl build");

    assert_eq!(
        error,
        CatalogError::NoSuchVariant {
            version: VERSION.to_owned(),
            target: MUSL.to_owned(),
            variant: "dynamic".to_owned(),
            available: "static".to_owned(),
        }
    );
}

// --------------------------------------------------- the version rule --

#[test]
fn a_runtime_older_than_the_compiler_is_refused_with_both_releases() {
    let error = catalog::check_release("28.3.1", 28, 29).expect_err("28 cannot run 29's modules");

    assert_eq!(
        error,
        CatalogError::OtpTooOld {
            version: "28.3.1".to_owned(),
            entry_release: 28,
            host_release: 29,
        }
    );
    assert_eq!(
        error.to_string(),
        "the catalog's OTP 28.3.1 is release 28 and this machine compiles with release 29; a \
         module compiled by OTP 29 does not load on OTP 28"
    );
}

#[test]
fn a_runtime_of_the_hosts_own_release_says_nothing() {
    assert_eq!(catalog::check_release(VERSION, 29, 29), Ok(None));
}

#[test]
fn a_runtime_within_two_releases_ahead_says_nothing_either() {
    assert_eq!(catalog::check_release("31.0.1", 31, 29), Ok(None));
    assert_eq!(RELEASE_WARN_AHEAD, 2);
}

#[test]
fn a_runtime_further_ahead_than_that_is_a_warning_rather_than_an_error() {
    let warning = catalog::check_release("32.0.1", 32, 29)
        .expect("a newer runtime is allowed")
        .expect("and is worth saying out loud");

    assert!(
        warning.contains("release 32") && warning.contains("release 29"),
        "the warning names both releases: {warning}"
    );
}

#[test]
fn an_entry_older_than_the_host_is_refused_by_the_selection_itself() {
    let catalog = CatalogBuilder::new()
        .entry(
            "28.3.1",
            28,
            "16.0.2",
            MUSL,
            "static",
            static_variant("old.tar.zst", &"a".repeat(64), 1),
        )
        .build();

    let error = catalog
        .select(
            &OtpReq::Exact {
                version: "28.3.1".to_owned(),
                host_release: RELEASE,
            },
            MUSL,
            None,
            "the fixture",
        )
        .expect_err("an exact version is still held to the version rule");

    assert!(
        matches!(error, CatalogError::OtpTooOld { .. }),
        "the guard is part of choosing, not of a later check: {error:?}"
    );
}

#[test]
fn versions_are_compared_component_by_component_rather_than_as_strings() {
    assert_eq!(
        catalog::compare_versions("29.0.10", "29.0.9"),
        Ordering::Greater
    );
    assert_eq!(
        catalog::compare_versions("29.0.9", "29.0.10"),
        Ordering::Less
    );
    assert_eq!(
        catalog::compare_versions("29.0.5", "29.0.5"),
        Ordering::Equal
    );
    assert_eq!(
        catalog::compare_versions("29.1", "29.0.9"),
        Ordering::Greater
    );
    assert_eq!(catalog::compare_versions("29.0", "29.0.0"), Ordering::Less);
}

// ----------------------------------------------------------- the URLs --

#[test]
fn a_url_with_no_scheme_resolves_against_the_catalogs_own_directory() {
    let resolved = catalog::resolve_url(
        "otp-29.0.5-linux-x86_64-musl-static.tar.zst",
        Some(Path::new("/repo/dist/otp")),
    );

    assert_eq!(
        resolved,
        SourceUrl::File(PathBuf::from(
            "/repo/dist/otp/otp-29.0.5-linux-x86_64-musl-static.tar.zst"
        )),
        "a committed catalog works in a checkout with the tarballs beside it"
    );
}

#[test]
fn a_url_with_a_scheme_is_fetched_and_never_resolved_against_a_directory() {
    assert_eq!(
        catalog::resolve_url(
            "https://example.test/otp.tar.zst",
            Some(Path::new("/repo/dist/otp"))
        ),
        SourceUrl::Remote("https://example.test/otp.tar.zst".to_owned())
    );
}

#[test]
fn an_absolute_path_url_is_taken_exactly_as_written() {
    assert_eq!(
        catalog::resolve_url("/srv/otp/otp.tar.zst", Some(Path::new("/repo/dist/otp"))),
        SourceUrl::File(PathBuf::from("/srv/otp/otp.tar.zst"))
    );
}

// ---------------------------------------------------------- the cache --

#[test]
fn an_entry_directory_is_named_after_the_version_the_target_and_the_variant() {
    assert_eq!(
        catalog::entry_dir_name(VERSION, MUSL, "static"),
        "29.0.5-linux-x86_64-musl-static",
        "a static and a dynamic musl runtime of one version may not share a directory"
    );
    assert_eq!(
        catalog::entry_dir_name(VERSION, GNU, DEFAULT_VARIANT),
        "29.0.5-linux-x86_64-gnu",
        "the single unnamed variant adds nothing to the name"
    );
}

#[test]
fn a_runtime_is_complete_exactly_when_its_marker_is_there() {
    let dir = tempdir();
    let entry = static_variant("otp.tar.zst", &"a".repeat(64), 1);
    let (complete, _) = plant_cached_otp(
        dir.path(),
        "29.0.5-linux-x86_64-musl-static",
        VERSION,
        MUSL,
        "static",
        &entry,
    );
    let incomplete = dir.path().join("29.0.5-linux-x86_64-gnu");
    FakeOtp::new().build_in(&incomplete);

    assert!(catalog::is_complete(&complete));
    assert!(
        !catalog::is_complete(&incomplete),
        "a tree with no marker is a crashed extraction, not a runtime"
    );
    assert!(!catalog::is_complete(&dir.path().join("nothing-here")));
}

#[test]
fn a_cached_runtime_is_used_rather_than_fetched_again() {
    let dir = tempdir();
    let cache = dir.path().join("cache/otp");
    let entry = static_variant("otp.tar.zst", &"a".repeat(64), 1);
    let (planted, _) = plant_cached_otp(
        &cache,
        "29.0.5-linux-x86_64-musl-static",
        VERSION,
        MUSL,
        "static",
        &entry,
    );

    let catalog = two_target_catalog().build();
    let selected = catalog
        .select(&OtpReq::Host(RELEASE), MUSL, None, "the fixture")
        .expect("the fixture holds it");
    let diag = Diag::disabled();
    let net = Net::offline();

    let resolved = catalog::ensure_otp(
        &selected,
        &EnsureContext {
            cache_root: &cache,
            catalog_dir: Some(dir.path()),
            net: &net,
            diag: &diag,
        },
    )
    .expect("a complete extraction needs no network at all");

    assert_eq!(resolved, planted);
}

#[test]
fn a_missing_runtime_is_fetched_extracted_and_marked_complete() {
    let dir = tempdir();
    let source = dir.path().join("source");
    FakeOtp::new()
        .erts_vsn(ERTS_VSN)
        .release(RELEASE)
        .otp_version(VERSION)
        .build_in(&source);
    let tarball = runtime_tarball(&source);
    let digest = sha256_hex(&tarball);
    let server = TestServer::one("/otp.tar.zst", Reply::ok(&tarball));

    let catalog = CatalogBuilder::new()
        .entry(
            VERSION,
            RELEASE,
            ERTS_VSN,
            MUSL,
            "static",
            static_variant(&server.url("/otp.tar.zst"), &digest, tarball.len() as u64),
        )
        .build();
    let selected = catalog
        .select(&OtpReq::Host(RELEASE), MUSL, None, "the fixture")
        .expect("the fixture holds it");

    let cache = dir.path().join("cache/otp");
    let diag = Diag::disabled();
    let net = Net::online();
    let resolved = catalog::ensure_otp(
        &selected,
        &EnsureContext {
            cache_root: &cache,
            catalog_dir: None,
            net: &net,
            diag: &diag,
        },
    )
    .expect("the tarball verified and extracted");

    assert_eq!(resolved, cache.join("29.0.5-linux-x86_64-musl-static"));
    assert!(
        resolved
            .join(format!("erts-{ERTS_VSN}/bin/beam.smp"))
            .is_file(),
        "what came out is a runtime root: {:?}",
        tree_listing(&resolved)
    );

    let meta: Meta = serde_json::from_slice(
        &std::fs::read(resolved.join(META_FILE)).expect("the completion marker"),
    )
    .expect("the marker is the meta record");
    assert_eq!(meta.version, VERSION);
    assert_eq!(meta.target, MUSL);
    assert_eq!(meta.variant, "static");
    assert_eq!(meta.entry.sha256, digest);
    assert_eq!(server.hits("/otp.tar.zst"), 1);
}

#[test]
fn an_extraction_with_no_marker_is_thrown_away_and_fetched_again() {
    let dir = tempdir();
    let source = dir.path().join("source");
    FakeOtp::new()
        .erts_vsn(ERTS_VSN)
        .release(RELEASE)
        .otp_version(VERSION)
        .build_in(&source);
    let tarball = runtime_tarball(&source);
    let digest = sha256_hex(&tarball);
    let server = TestServer::one("/otp.tar.zst", Reply::ok(&tarball));

    let cache = dir.path().join("cache/otp");
    let half = cache.join("29.0.5-linux-x86_64-musl-static");
    std::fs::create_dir_all(&half).expect("a half-extracted entry");
    std::fs::write(half.join("leftover"), b"from a crashed extraction").expect("a leftover file");

    let catalog = CatalogBuilder::new()
        .entry(
            VERSION,
            RELEASE,
            ERTS_VSN,
            MUSL,
            "static",
            static_variant(&server.url("/otp.tar.zst"), &digest, tarball.len() as u64),
        )
        .build();
    let selected = catalog
        .select(&OtpReq::Host(RELEASE), MUSL, None, "the fixture")
        .expect("the fixture holds it");
    let diag = Diag::disabled();
    let net = Net::online();

    let resolved = catalog::ensure_otp(
        &selected,
        &EnsureContext {
            cache_root: &cache,
            catalog_dir: None,
            net: &net,
            diag: &diag,
        },
    )
    .expect("an incomplete entry is replaced, not reused");

    assert!(
        !resolved.join("leftover").exists(),
        "the crashed extraction is gone rather than merged with the new one"
    );
    assert!(catalog::is_complete(&resolved));
    assert_eq!(server.hits("/otp.tar.zst"), 1);
}

#[test]
fn an_offline_build_with_nothing_cached_names_the_url_and_the_cache_directory() {
    let dir = tempdir();
    let cache = dir.path().join("cache/otp");
    let catalog = CatalogBuilder::new()
        .entry(
            VERSION,
            RELEASE,
            ERTS_VSN,
            MUSL,
            "static",
            static_variant("https://example.test/otp.tar.zst", &"a".repeat(64), 1),
        )
        .build();
    let selected = catalog
        .select(&OtpReq::Host(RELEASE), MUSL, None, "the fixture")
        .expect("the fixture holds it");
    let diag = Diag::disabled();
    let net = Net::offline();

    let error = catalog::ensure_otp(
        &selected,
        &EnsureContext {
            cache_root: &cache,
            catalog_dir: None,
            net: &net,
            diag: &diag,
        },
    )
    .expect_err("offline with an empty cache cannot produce a runtime");

    match error {
        CatalogError::Download(DownloadError::Offline { url, dest_hint }) => {
            assert_eq!(url, "https://example.test/otp.tar.zst");
            assert!(
                dest_hint.starts_with(&cache),
                "the file would have gone into the cache: {}",
                dest_hint.display()
            );
        }
        other => panic!("offline travels through the catalog unchanged, not as {other:?}"),
    }
}

#[test]
fn a_user_supplied_tarball_is_cached_under_its_own_digest() {
    let dir = tempdir();
    let source = dir.path().join("source");
    FakeOtp::new()
        .erts_vsn(ERTS_VSN)
        .release(RELEASE)
        .otp_version(VERSION)
        .build_in(&source);
    let tarball = runtime_tarball(&source);
    let digest = sha256_hex(&tarball);
    let archive = dir.path().join("some-name.tar.zst");
    std::fs::write(&archive, &tarball).expect("the archive");

    let cache = dir.path().join("cache/otp");
    let diag = Diag::disabled();

    let resolved =
        catalog::ensure_tarball(&archive, &cache, &diag).expect("the archive is a runtime root");

    assert_eq!(
        resolved,
        cache.join(catalog::tarball_dir_name(&digest)),
        "two builds naming one archive share an extraction; two archives with one name do not"
    );
    assert_eq!(
        catalog::tarball_dir_name(&digest),
        format!("tarball-{digest}")
    );
    assert!(catalog::is_complete(&resolved));
    assert!(
        resolved
            .join(format!("erts-{ERTS_VSN}/bin/beam.smp"))
            .is_file()
    );
}

// ------------------------------------------- the extractor's refusals --

/// Extracts a hand-built archive as a user-supplied tarball.
///
/// `ensure_tarball` is the shortest path to the strict extractor: it takes any
/// file, so a test reaches the three refusals without arranging a catalogue
/// entry and a digest first. The archives are written block by block through
/// `RawTar`, for the reason `tests/payload.rs` writes its malicious ones that
/// way — a packer that plays by the rules cannot lay down a `..` path, an
/// absolute one or a device node, and those are the entries under test.
fn extract_hostile(archive: &RawTar) -> (tempfile::TempDir, Result<PathBuf, CatalogError>) {
    let dir = tempdir();
    let path = dir.path().join("hostile.tar.zst");
    std::fs::write(&path, archive.build_zstd(19)).expect("the archive");
    let cache = dir.path().join("cache/otp");
    let diag = Diag::disabled();
    let result = catalog::ensure_tarball(&path, &cache, &diag);
    (dir, result)
}

/// The `Extract` message, or a failure naming what came back instead.
fn extract_refusal(result: Result<PathBuf, CatalogError>) -> String {
    match result {
        Ok(path) => panic!("the archive was unpacked into {}", path.display()),
        Err(CatalogError::Extract { message, .. }) => message,
        Err(other) => panic!("expected CatalogError::Extract, got {other:?}"),
    }
}

/// The runtime directories under `cache`, which a refusal must leave none of.
///
/// The lock directory is not one: `<cache>/.locks/<entry>/.lock` is
/// bookkeeping the fill takes and releases, and it outliving a refusal is the
/// design rather than residue.
fn cached_runtimes(cache: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(cache) else {
        return Vec::new();
    };
    let mut found: Vec<String> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name != catalog::LOCK_SUBDIR)
        .collect();
    found.sort();
    found
}

#[test]
fn a_runtime_tarball_holding_a_symlink_is_refused_and_names_it() {
    // The strict rule the repack's dereference exists to make possible. A
    // symlink here is either an escape or a runtime the repack did not
    // produce, and either way it is not unpacked.
    let (dir, result) = extract_hostile(
        &RawTar::new()
            .push(RawEntry::file("bin/erl", b"#!/bin/sh\n"))
            .push(RawEntry::special(
                "bin/erl.link",
                TYPE_SYMLINK,
                "/etc/passwd",
            )),
    );
    let message = extract_refusal(result);

    assert!(
        message.contains("bin/erl.link"),
        "the entry that was refused is named: {message}"
    );
    assert!(
        message.contains("regular files and directories only"),
        "and the rule is stated: {message}"
    );
    assert_eq!(
        cached_runtimes(&dir.path().join("cache/otp")),
        Vec::<String>::new(),
        "a refused archive leaves no runtime behind, complete or otherwise"
    );
}

#[test]
fn a_runtime_tarball_whose_path_climbs_out_of_the_root_is_refused() {
    let (dir, result) =
        extract_hostile(&RawTar::new().push(RawEntry::file("../escaped.txt", b"owned")));
    let message = extract_refusal(result);

    assert!(
        message.contains("../escaped.txt") && message.contains("does not stay under the root"),
        "an escaping entry is named and refused: {message}"
    );
    assert!(
        !dir.path().join("escaped.txt").exists() && !dir.path().join("cache/escaped.txt").exists(),
        "and nothing was written beside the destination"
    );
}

#[test]
fn a_runtime_tarball_naming_an_absolute_path_is_refused() {
    let victim = "/tmp/ginary-must-not-be-written-by-a-runtime-tarball";
    let (_dir, result) = extract_hostile(&RawTar::new().push(RawEntry::file(victim, b"owned")));
    let message = extract_refusal(result);

    assert!(
        message.contains(victim) && message.contains("does not stay under the root"),
        "an absolute entry is named and refused: {message}"
    );
    assert!(
        !Path::new(victim).exists(),
        "and the path it named was never written"
    );
}

#[test]
fn a_runtime_tarball_holding_a_device_node_is_refused() {
    let (_dir, result) =
        extract_hostile(&RawTar::new().push(RawEntry::special("dev/null", TYPE_CHAR_DEVICE, "")));
    let message = extract_refusal(result);

    assert!(
        message.contains("dev/null") && message.contains("regular files and directories only"),
        "a runtime is files and directories, and nothing else: {message}"
    );
}

// ------------------------------------------------------- the writing --

#[test]
fn a_catalog_is_written_as_two_space_json_with_one_trailing_newline() {
    let mut catalog = Catalog::empty("2026-08-31T00:00:00Z");
    catalog.insert(
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
    );
    catalog.insert(
        VERSION,
        RELEASE,
        ERTS_VSN,
        GNU,
        DEFAULT_VARIANT,
        gnu_variant(
            "otp-29.0.5-linux-x86_64-gnu.tar.zst",
            &"b".repeat(64),
            40_000_000,
        ),
    );

    let text = catalog.to_json();

    assert_eq!(
        text,
        two_target_catalog().json(),
        "what `otp repack` commits is exactly what the schema serialises to"
    );
    assert!(text.ends_with("}\n"), "one trailing newline, no more");
    insta::assert_snapshot!("catalog_json", text);
}

#[test]
fn inserting_two_variants_of_one_target_keeps_both() {
    let mut catalog = Catalog::empty("2026-08-31T00:00:00Z");
    catalog.insert(
        VERSION,
        RELEASE,
        ERTS_VSN,
        MUSL,
        "static",
        static_variant("s.tar.zst", &"a".repeat(64), 1),
    );
    catalog.insert(
        VERSION,
        RELEASE,
        ERTS_VSN,
        MUSL,
        "dynamic",
        gnu_variant("d.tar.zst", &"b".repeat(64), 2),
    );

    let variants: Vec<String> = catalog.otp[VERSION].targets[MUSL]
        .variants
        .keys()
        .cloned()
        .collect();

    assert_eq!(variants, vec!["dynamic".to_owned(), "static".to_owned()]);
    assert_eq!(catalog.otp[VERSION].erts_vsn, ERTS_VSN);
    assert_eq!(catalog.otp[VERSION].otp_release, RELEASE);
}

#[test]
fn a_selection_carries_the_erts_version_the_assembly_will_look_for() {
    let catalog = two_target_catalog().build();

    let selected = catalog
        .select(&OtpReq::Host(RELEASE), MUSL, None, "the fixture")
        .expect("the fixture holds it");

    assert_eq!(selected.otp.erts_vsn, ERTS_VSN);
    assert_eq!(selected.otp.otp_release, RELEASE);
    assert_eq!(selected.target, MUSL);
    assert_eq!(selected.dir_name(), "29.0.5-linux-x86_64-musl-static");
    assert_eq!(
        catalog::cache_root(Path::new("/home/u/.cache/ginary")),
        PathBuf::from("/home/u/.cache/ginary/otp"),
        "and the cache it lands in is the `otp` directory of the ginary cache"
    );
}

// --------------------------------------------------------- macOS (D3) ----

#[test]
fn erlef_upstream_asset_names_the_apple_darwin_files_pinned_from_otp_29_0_5() {
    assert_eq!(
        catalog::erlef_upstream_asset("29.0.5", ginary::target::Arch::X86_64),
        "otp-x86_64-apple-darwin.tar.gz"
    );
    assert_eq!(
        catalog::erlef_upstream_asset("29.0.5", ginary::target::Arch::Aarch64),
        "otp-aarch64-apple-darwin.tar.gz"
    );
}

#[test]
fn macos_catalog_admissible_is_true_only_when_the_entry_release_matches_the_host_exactly() {
    assert!(
        catalog::macos_catalog_admissible(29, 29),
        "the release that was actually found (OTP-29.0.5) matches this repository's host \
         release (29) and should be committable"
    );
    assert!(
        !catalog::macos_catalog_admissible(28, 29),
        "an entry older than the host release is not committed to dist/otp/catalog.json, even \
         though a build may still accept it via check_release"
    );
    assert!(
        !catalog::macos_catalog_admissible(30, 29),
        "an entry newer than the host release is not committed either"
    );
}

// ------------------------------------------------- further edge cases --

/// A document that is a JSON object but never names `schema_version` is refused
/// at that first gate, before any entry is read.
#[test]
fn a_catalog_missing_its_schema_version_says_which_field_is_absent() {
    let text = r#"{"generated_at":"2026-08-31T00:00:00Z","otp":{}}"#;

    let error = Catalog::parse(text, "/tmp/catalog.json").expect_err("no schema_version at all");

    match error {
        CatalogError::Parse { origin, message } => {
            assert_eq!(origin, "/tmp/catalog.json");
            assert!(
                message.contains("schema_version"),
                "the message names the missing field: {message}"
            );
        }
        other => panic!("a missing schema_version is a Parse error, not {other:?}"),
    }
}

/// `lookup` answers questions about a catalogue without the version rule, so
/// its own version miss must name what the catalogue does hold rather than
/// deferring to `select`.
#[test]
fn lookup_of_a_version_that_is_not_there_names_the_ones_that_are() {
    let catalog = two_target_catalog().build();

    let error = catalog
        .lookup("28.3.1", MUSL, None, "dist/otp/catalog.json")
        .expect_err("28.3.1 is not in the fixture");

    assert_eq!(
        error,
        CatalogError::NoSuchVersion {
            origin: "dist/otp/catalog.json".to_owned(),
            req: "28.3.1".to_owned(),
            available: "29.0.5".to_owned(),
        }
    );
}

/// A version component that is not a number is ordered as text, so a
/// pre-release suffix compares deterministically rather than not at all.
#[test]
fn a_non_numeric_component_is_compared_as_text() {
    assert_eq!(
        catalog::compare_versions("29.0.0-rc1", "29.0.0-rc2"),
        Ordering::Less,
        "rc1 sorts before rc2 by text when the component is not a number"
    );
    assert_eq!(
        catalog::compare_versions("29.0.0-rc2", "29.0.0-rc1"),
        Ordering::Greater
    );
}

/// The origin an explicit `--catalog` records carries its path back for a
/// message; the cache and embedded origins carry none, because a path there
/// would add nothing.
#[test]
fn only_an_explicit_origin_reports_a_flag_path() {
    let named = PathBuf::from("/tmp/my-catalog.json");
    assert_eq!(
        CatalogOrigin::Explicit(named.clone()).flag_path(),
        Some(named.as_path()),
        "an explicit catalogue names its path"
    );
    assert_eq!(
        CatalogOrigin::Embedded.flag_path(),
        None,
        "the embedded catalogue has no flag path"
    );
}

/// A document that is schema 1 and valid JSON but whose entry does not fit the
/// catalogue shape is a parse failure at the point the reader tries to build
/// the typed value, not a schema error.
#[test]
fn a_schema_one_document_whose_entry_is_malformed_is_a_parse_error() {
    // `otp_release` is a number in the shape; a string there is well-formed
    // JSON that `serde_json::from_value` refuses.
    let text = r#"{"schema_version":1,"generated_at":"2026-08-31T00:00:00Z",
        "otp":{"29.0.5":{"otp_release":"twenty-nine","erts_version":"17.0.5","targets":{}}}}"#;

    let error = Catalog::parse(text, "/tmp/catalog.json").expect_err("otp_release is not a number");

    match error {
        CatalogError::Parse { origin, .. } => assert_eq!(origin, "/tmp/catalog.json"),
        other => panic!("a malformed entry is a Parse error, not {other:?}"),
    }
}

/// An empty catalogue names `nothing at all` where it would otherwise list what
/// it holds, rather than an empty string that reads as a dropped line.
#[test]
fn a_miss_against_an_empty_catalog_says_nothing_at_all() {
    let catalog = CatalogBuilder::new().build();

    let error = catalog
        .select(
            &OtpReq::Exact {
                version: VERSION.to_owned(),
                host_release: RELEASE,
            },
            MUSL,
            None,
            "the empty fixture",
        )
        .expect_err("an empty catalogue holds no version");

    assert_eq!(
        error,
        CatalogError::NoSuchVersion {
            origin: "the empty fixture".to_owned(),
            req: VERSION.to_owned(),
            available: "nothing at all".to_owned(),
        }
    );
}
