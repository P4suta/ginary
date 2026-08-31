// SPDX-License-Identifier: MIT OR Apache-2.0
//! The two ERTS sources C3 adds: a catalogue entry and a user-supplied
//! tarball.
//!
//! Both fill the same cache and both end at the same place — `inspect_root`
//! plus the emulator's own header — which is the point of the file. A
//! catalogue is an *index*: it says a runtime is a static musl x86-64 build
//! that loads no NIFs, and nothing downstream is allowed to believe that
//! without reading the `beam.smp` it describes. Where the two disagree the
//! build stops and the message names both sides.
//!
//! `tests/erts_source.rs` covers the grammar and the `host` and `dir:` arms.
//! This file is the catalogue half, driven through
//! [`erts_source::resolve_in_with`] so that a fixture runtime — whose emulator
//! is a shell script — can still exercise everything above the ELF reader.
// The command line half of the suite.
#![cfg(feature = "cli")]

mod common;

use std::path::{Path, PathBuf};

use ginary::catalog::{self, CatalogError, CatalogPaths, OtpReq};
use ginary::diag::Diag;
use ginary::download::{DownloadError, Net};
use ginary::erts_source::{self, ElfFacts, ErtsError, ErtsSourceSpec, ResolvedErts, SourceContext};
use ginary::target::{Linkage, Target};

use crate::common::catalog::{
    CatalogBuilder, ERTS_VSN, RELEASE, VERSION, plant_cached_otp, runtime_tarball, static_variant,
};
use crate::common::fake_otp::FakeOtp;
use crate::common::payload::sha256_hex;

/// The target every test in this file builds for.
const MUSL: &str = "linux-x86_64-musl";

/// A temporary directory the test owns.
fn tempdir() -> tempfile::TempDir {
    tempfile::tempdir().expect("a temporary directory")
}

/// The named target.
fn target(name: &str) -> Target {
    name.parse()
        .unwrap_or_else(|error| panic!("`{name}` must be a target: {error}"))
}

/// The facts a fully static x86-64 emulator reads back as.
fn static_facts(machine: &str) -> ElfFacts {
    ElfFacts {
        machine: machine.to_owned(),
        interp: None,
        needed: Vec::new(),
        glibc_max: None,
    }
}

/// The facts a dynamically linked musl emulator reads back as.
fn dynamic_musl_facts(machine: &str) -> ElfFacts {
    ElfFacts {
        machine: machine.to_owned(),
        interp: Some(format!("/lib/ld-musl-{machine}.so.1")),
        needed: vec![format!("libc.musl-{machine}.so.1")],
        glibc_max: None,
    }
}

/// A context over `cache`, reading `catalog`, offline unless told otherwise.
fn context<'a>(
    catalog: &'a CatalogPaths,
    cache: &'a Path,
    net: &'a Net,
    diag: &'a Diag,
    req: &'a OtpReq,
    variant: Option<&'a str>,
) -> SourceContext<'a> {
    SourceContext {
        catalog_paths: catalog,
        cache_root: cache,
        net,
        host_release: RELEASE,
        otp_version: req,
        variant,
        diag,
    }
}

/// Resolves the catalogue source with every emulator reading back as `facts`.
fn resolve_catalog(
    ctx: &SourceContext<'_>,
    requested: &Target,
    facts: ElfFacts,
) -> Result<ResolvedErts, ErtsError> {
    erts_source::resolve_in_with(&ErtsSourceSpec::Catalog, requested, ctx, |path| {
        assert!(
            path.ends_with("beam.smp"),
            "the emulator is the file that is read, not {}",
            path.display()
        );
        Ok(facts.clone())
    })
}

// ------------------------------------------------------ the catalog --

#[test]
fn a_catalog_source_resolves_through_the_cached_runtime_it_names() {
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
    let catalog_path = CatalogBuilder::new()
        .entry(VERSION, RELEASE, ERTS_VSN, MUSL, "static", entry)
        .write_in(&dir.path().join("dist/otp"));
    let paths = CatalogPaths {
        explicit: Some(catalog_path),
        cache: None,
    };
    let net = Net::offline();
    let diag = Diag::disabled();
    let req = OtpReq::Host(RELEASE);

    let resolved = resolve_catalog(
        &context(&paths, &cache, &net, &diag, &req, None),
        &target(MUSL),
        static_facts("x86_64"),
    )
    .expect("the runtime is already in the cache");

    assert_eq!(resolved.otp.root, planted);
    assert_eq!(resolved.otp.erts_vsn, ERTS_VSN);
    assert_eq!(resolved.target, target(MUSL));
    assert_eq!(resolved.linkage, Linkage::Static);
    assert!(
        !resolved.nif_loading,
        "the static build is the one that dlopens nothing"
    );
    assert_eq!(
        resolved.provenance,
        erts_source::catalog_provenance(VERSION, MUSL, "static")
    );
    assert_eq!(
        resolved.provenance, "catalog:29.0.5/linux-x86_64-musl/static",
        "the manifest records which entry the runtime came from, not just `catalog`"
    );
}

#[test]
fn a_catalog_entry_whose_emulator_is_for_another_machine_names_both_sides() {
    let dir = tempdir();
    let cache = dir.path().join("cache/otp");
    let entry = static_variant("otp.tar.zst", &"a".repeat(64), 1);
    plant_cached_otp(
        &cache,
        "29.0.5-linux-x86_64-musl-static",
        VERSION,
        MUSL,
        "static",
        &entry,
    );
    let catalog_path = CatalogBuilder::new()
        .entry(VERSION, RELEASE, ERTS_VSN, MUSL, "static", entry)
        .write_in(&dir.path().join("dist/otp"));
    let paths = CatalogPaths {
        explicit: Some(catalog_path),
        cache: None,
    };
    let net = Net::offline();
    let diag = Diag::disabled();
    let req = OtpReq::Host(RELEASE);

    let error = resolve_catalog(
        &context(&paths, &cache, &net, &diag, &req, None),
        &target(MUSL),
        static_facts("aarch64"),
    )
    .expect_err("the catalog said x86_64 and the emulator is aarch64");

    match error {
        ErtsError::CatalogClaim {
            entry,
            field,
            claimed,
            actual,
            ..
        } => {
            assert_eq!(entry, "catalog:29.0.5/linux-x86_64-musl/static");
            assert_eq!(field, "target");
            assert_eq!(claimed, "linux-x86_64-musl");
            assert_eq!(actual, "linux-aarch64-musl");
        }
        other => panic!("a catalog that lies about its target is a claim error, not {other:?}"),
    }
}

#[test]
fn a_catalog_entry_that_claims_a_linkage_the_emulator_denies_is_refused() {
    let dir = tempdir();
    let cache = dir.path().join("cache/otp");
    let entry = static_variant("otp.tar.zst", &"a".repeat(64), 1);
    plant_cached_otp(
        &cache,
        "29.0.5-linux-x86_64-musl-static",
        VERSION,
        MUSL,
        "static",
        &entry,
    );
    let catalog_path = CatalogBuilder::new()
        .entry(VERSION, RELEASE, ERTS_VSN, MUSL, "static", entry)
        .write_in(&dir.path().join("dist/otp"));
    let paths = CatalogPaths {
        explicit: Some(catalog_path),
        cache: None,
    };
    let net = Net::offline();
    let diag = Diag::disabled();
    let req = OtpReq::Host(RELEASE);

    let error = resolve_catalog(
        &context(&paths, &cache, &net, &diag, &req, None),
        &target(MUSL),
        dynamic_musl_facts("x86_64"),
    )
    .expect_err("`static` and a dynamically linked emulator cannot both be true");

    match error {
        ErtsError::CatalogClaim {
            field,
            claimed,
            actual,
            ..
        } => {
            assert_eq!(field, "linkage");
            assert_eq!(claimed, "static");
            assert_eq!(actual, "dynamic");
        }
        other => panic!(
            "an entry claiming `nif_loading: false` for a runtime that loads them is a claim \
             error, not {other:?}"
        ),
    }
}

#[test]
fn a_catalog_source_offline_with_an_empty_cache_carries_the_offline_error() {
    let dir = tempdir();
    let cache = dir.path().join("cache/otp");
    let catalog_path = CatalogBuilder::new()
        .entry(
            VERSION,
            RELEASE,
            ERTS_VSN,
            MUSL,
            "static",
            static_variant("https://example.test/otp.tar.zst", &"a".repeat(64), 1),
        )
        .write_in(&dir.path().join("dist/otp"));
    let paths = CatalogPaths {
        explicit: Some(catalog_path),
        cache: None,
    };
    let net = Net::offline();
    let diag = Diag::disabled();
    let req = OtpReq::Host(RELEASE);

    let error = resolve_catalog(
        &context(&paths, &cache, &net, &diag, &req, None),
        &target(MUSL),
        static_facts("x86_64"),
    )
    .expect_err("offline with nothing cached cannot produce a runtime");

    match error {
        ErtsError::Catalog(CatalogError::Download(DownloadError::Offline { url, .. })) => {
            assert_eq!(url, "https://example.test/otp.tar.zst");
        }
        other => panic!("the offline error travels through both layers, not {other:?}"),
    }
}

#[test]
fn a_catalog_with_no_entry_for_the_target_is_reported_as_a_catalog_error() {
    let dir = tempdir();
    let cache = dir.path().join("cache/otp");
    let catalog_path = CatalogBuilder::new()
        .entry(
            VERSION,
            RELEASE,
            ERTS_VSN,
            "linux-x86_64-gnu",
            "default",
            static_variant("otp.tar.zst", &"a".repeat(64), 1),
        )
        .write_in(&dir.path().join("dist/otp"));
    let paths = CatalogPaths {
        explicit: Some(catalog_path),
        cache: None,
    };
    let net = Net::offline();
    let diag = Diag::disabled();
    let req = OtpReq::Host(RELEASE);

    let error = resolve_catalog(
        &context(&paths, &cache, &net, &diag, &req, None),
        &target(MUSL),
        static_facts("x86_64"),
    )
    .expect_err("there is no musl entry");

    assert!(
        matches!(error, ErtsError::Catalog(CatalogError::NoSuchTarget { .. })),
        "the selection's own error is what a user needs to read: {error:?}"
    );
}

// ------------------------------------------------------ the tarball --

#[test]
fn a_tarball_source_extracts_into_the_cache_and_is_inspected_there() {
    let dir = tempdir();
    let source = dir.path().join("source");
    FakeOtp::new()
        .erts_vsn(ERTS_VSN)
        .release(RELEASE)
        .otp_version(VERSION)
        .build_in(&source);
    let bytes = runtime_tarball(&source);
    let archive = dir.path().join("otp-29.0.5-musl.tar.zst");
    std::fs::write(&archive, &bytes).expect("the archive");
    let cache = dir.path().join("cache/otp");

    let paths = CatalogPaths::default();
    let net = Net::offline();
    let diag = Diag::disabled();
    let req = OtpReq::Host(RELEASE);
    let ctx = context(&paths, &cache, &net, &diag, &req, None);

    let resolved = erts_source::resolve_in_with(
        &ErtsSourceSpec::Tarball(archive.clone()),
        &target(MUSL),
        &ctx,
        |_| Ok(static_facts("x86_64")),
    )
    .expect("the tarball is a runtime root");

    assert_eq!(
        resolved.otp.root,
        cache.join(catalog::tarball_dir_name(&sha256_hex(&bytes))),
        "a tarball is cached under its own digest, not under its file name"
    );
    assert_eq!(resolved.otp.otp_version, VERSION);
    assert_eq!(resolved.linkage, Linkage::Static);
    assert_eq!(
        resolved.provenance,
        format!("tarball:{}", archive.display()),
        "the provenance is the spelling the configuration used"
    );
}

#[test]
fn a_tarball_that_is_not_a_runtime_root_says_so_rather_than_resolving() {
    let dir = tempdir();
    let archive = dir.path().join("not-a-runtime.tar.zst");
    std::fs::write(&archive, b"not an archive at all\n").expect("the archive");
    let cache = dir.path().join("cache/otp");

    let paths = CatalogPaths::default();
    let net = Net::offline();
    let diag = Diag::disabled();
    let req = OtpReq::Host(RELEASE);
    let ctx = context(&paths, &cache, &net, &diag, &req, None);

    let error = erts_source::resolve_in_with(
        &ErtsSourceSpec::Tarball(archive.clone()),
        &target(MUSL),
        &ctx,
        |_| Ok(static_facts("x86_64")),
    )
    .expect_err("those bytes are not an archive");

    match error {
        ErtsError::Catalog(CatalogError::Extract { path, .. }) => assert_eq!(path, archive),
        other => panic!("a broken archive is an extract error naming it, not {other:?}"),
    }
}

#[test]
fn the_provenance_of_a_catalog_entry_is_its_version_target_and_variant() {
    assert_eq!(
        erts_source::catalog_provenance(VERSION, MUSL, "static"),
        "catalog:29.0.5/linux-x86_64-musl/static"
    );
    assert_eq!(
        erts_source::catalog_provenance(VERSION, "linux-x86_64-gnu", "default"),
        "catalog:29.0.5/linux-x86_64-gnu/default",
        "the variant is always spelled out, so two entries never read alike"
    );
    assert_eq!(
        erts_source::catalog_provenance(VERSION, MUSL, "static")
            .parse::<PathBuf>()
            .map(|path| path.is_absolute()),
        Ok(false),
        "a provenance is a label, never a path something might open"
    );
}
