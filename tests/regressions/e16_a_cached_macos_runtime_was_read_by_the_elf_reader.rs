// SPDX-License-Identifier: MIT OR Apache-2.0
//! A macOS runtime that arrived through the cache was handed to the ELF reader
//! and refused as "not an ELF runtime".
//!
//! **What went wrong.** `erts_source` has two entry points and only one of
//! them chooses a reader. `resolve_with` — the `host` and `dir:` arm — reads
//! the emulator's own first bytes and sends a Mach-O to `resolve_macos`,
//! which is the fix D3 made after both macOS jobs of the first live run died
//! inside the ELF parser. `resolve_in_with` — the `catalog` and `tarball`
//! arm — does not. It calls `read_emulator`, and `read_emulator` calls the
//! ELF seam directly and maps whatever it says to
//! [`ErtsError::NotAnElfRuntime`]:
//!
//! ```text
//! `.../erts-17.0.5/bin/beam.smp` is not an ELF runtime: not an ELF file
//! ```
//!
//! So the one reader that knows a Mach-O is reached from one of the two
//! callers, and a macOS runtime downloaded from the catalogue or unpacked
//! from a tarball is refused for being what it is. `ginary build
//! --erts catalog` on macOS is the shortest path to it, and it is the path a
//! user with no local OTP takes.
//!
//! **The input.** Any `catalog` or `tarball:` source whose `beam.smp` is a
//! Mach-O — that is, every macOS runtime that is not already on the machine.
//!
//! **The correct behaviour.** Which reader reads an emulator is the
//! emulator's own to say, on every path that reads one. A Mach-O from the
//! cache resolves exactly as a Mach-O from `dir:` does: the target comes off
//! the `cputype`, the linkage is dynamic, there is no glibc floor, and the
//! provenance is the source's own label. The ELF seam is not consulted at all,
//! so `NotAnElfRuntime` cannot be what a macOS user is told.

use std::path::Path;

use ginary::catalog::{CatalogPaths, OtpReq};
use ginary::diag::Diag;
use ginary::download::Net;
use ginary::elf::ElfError;
use ginary::erts_source::{self, ElfFacts, ErtsError, ErtsSourceSpec, SourceContext};
use ginary::target::{Linkage, Target};

use crate::common::catalog::{
    CatalogBuilder, ERTS_VSN, RELEASE, VERSION, plant_cached_otp, runtime_tarball, static_variant,
};
use crate::common::fake_otp::FakeOtp;
use crate::common::macho::{CPU_TYPE_ARM64, MH_EXECUTE, thin_header};

/// The target every runtime in this file is for.
const MACOS: &str = "macos-aarch64";

/// The named target.
fn target(name: &str) -> Target {
    name.parse()
        .unwrap_or_else(|error| panic!("`{name}` must be a target: {error}"))
}

/// An ELF seam that refuses everything it is shown.
///
/// The whole claim is that this is never called: a Mach-O emulator is read by
/// `crate::macho`, and an ELF reader that is handed one has already lost. The
/// refusal is spelled out rather than a `panic!` so the failure arrives as the
/// error a user would actually see, through `resolve_in_with`'s own mapping.
fn elf_reader_refuses(_: &Path) -> Result<ElfFacts, ElfError> {
    Err(ElfError::Parse {
        message: "the ELF reader was handed a macOS emulator".to_owned(),
    })
}

/// A context over `cache` with no catalogue, offline.
fn context<'a>(
    catalog: &'a CatalogPaths,
    cache: &'a Path,
    net: &'a Net,
    diag: &'a Diag,
    req: &'a OtpReq,
) -> SourceContext<'a> {
    SourceContext {
        catalog_paths: catalog,
        cache_root: cache,
        net,
        host_release: RELEASE,
        otp_version: req,
        variant: None,
        diag,
    }
}

#[test]
fn a_macos_runtime_unpacked_from_a_tarball_is_read_as_a_macho() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let source = dir.path().join("source");
    FakeOtp::new()
        .erts_vsn(ERTS_VSN)
        .release(RELEASE)
        .otp_version(VERSION)
        .macos()
        .macho_cpu_type(CPU_TYPE_ARM64)
        .build_in(&source);
    let archive = dir.path().join("otp-29.0.5-macos-aarch64.tar.zst");
    std::fs::write(&archive, runtime_tarball(&source)).expect("the archive");
    let cache = dir.path().join("cache/otp");

    let paths = CatalogPaths::default();
    let net = Net::offline();
    let diag = Diag::disabled();
    let req = OtpReq::Host(RELEASE);
    let ctx = context(&paths, &cache, &net, &diag, &req);

    let resolved = erts_source::resolve_in_with(
        &ErtsSourceSpec::Tarball(archive.clone()),
        &target(MACOS),
        &ctx,
        elf_reader_refuses,
    )
    .expect("a tarball whose emulator is a Mach-O is a macOS runtime, not a broken ELF one");

    assert_eq!(
        resolved.target,
        target(MACOS),
        "the target comes off the emulator's own `cputype`, exactly as it does through `dir:`"
    );
    assert_eq!(
        resolved.linkage,
        Linkage::Dynamic,
        "a macOS emulator resolves libSystem at load time"
    );
    assert!(resolved.nif_loading, "and therefore loads NIFs");
    assert_eq!(
        resolved.libc_min, None,
        "macOS has one system C library and no symbol-version floor to record"
    );
    assert_eq!(
        resolved.provenance,
        format!("tarball:{}", archive.display()),
        "the provenance is the spelling the configuration used"
    );
}

#[test]
fn a_macos_runtime_already_in_the_cache_is_read_as_a_macho() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let cache = dir.path().join("cache/otp");
    let mut entry = static_variant("otp.tar.zst", &"b".repeat(64), 1);
    // What a macOS catalogue entry claims: it is dynamically linked, it loads
    // NIFs, and its C library is the system's own.
    entry.linkage = "dynamic".to_owned();
    entry.nif_loading = true;
    let (_, planted) = plant_cached_otp(
        &cache,
        // `catalog::entry_dir_name` leaves the unnamed `default` variant out
        // of the directory name, so a planted entry that carried it would be a
        // cache miss and the test would measure a download instead.
        "29.0.5-macos-aarch64",
        VERSION,
        MACOS,
        "default",
        &entry,
    );
    // `plant_cached_otp` writes the unix tree every other catalogue test wants,
    // whose emulator is a shell script. Only the emulator differs on macOS, so
    // only the emulator is replaced.
    std::fs::write(
        planted.erts_bin().join("beam.smp"),
        thin_header(CPU_TYPE_ARM64, MH_EXECUTE),
    )
    .expect("replace the emulator with a thin Mach-O");

    let catalog_path = CatalogBuilder::new()
        .entry(VERSION, RELEASE, ERTS_VSN, MACOS, "default", entry)
        .write_in(&dir.path().join("dist/otp"));
    let paths = CatalogPaths {
        explicit: Some(catalog_path),
        cache: None,
    };
    let net = Net::offline();
    let diag = Diag::disabled();
    let req = OtpReq::Host(RELEASE);
    let ctx = context(&paths, &cache, &net, &diag, &req);

    let resolved = erts_source::resolve_in_with(
        &ErtsSourceSpec::Catalog,
        &target(MACOS),
        &ctx,
        elf_reader_refuses,
    )
    .expect("a catalogue entry whose emulator is a Mach-O is a macOS runtime");

    assert_eq!(resolved.target, target(MACOS));
    assert_eq!(resolved.linkage, Linkage::Dynamic);
    assert_eq!(resolved.libc_min, None);
    assert_eq!(
        resolved.provenance,
        erts_source::catalog_provenance(VERSION, MACOS, "default"),
        "the provenance names the entry, and the claim check passed against the Mach-O"
    );
}

#[test]
fn a_catalogue_entry_that_claims_glibc_for_a_macos_runtime_is_refused() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let cache = dir.path().join("cache/otp");
    let mut entry = static_variant("otp.tar.zst", &"b".repeat(64), 1);
    entry.linkage = "dynamic".to_owned();
    entry.nif_loading = true;
    // The lie. Without it nothing in the suite could tell `read_macho_emulator`'s
    // `libc_kind: Some("none")` from a plain `None`: `resolve_catalog` spells
    // both as `none`, so the passing entry above proves only that the two agree
    // and not which of them the emulator actually reported. This is the
    // assertion that makes the field load-bearing.
    entry.libc.kind = "gnu".to_owned();
    let (_, planted) = plant_cached_otp(
        &cache,
        "29.0.5-macos-aarch64",
        VERSION,
        MACOS,
        "default",
        &entry,
    );
    std::fs::write(
        planted.erts_bin().join("beam.smp"),
        thin_header(CPU_TYPE_ARM64, MH_EXECUTE),
    )
    .expect("replace the emulator with a thin Mach-O");

    let catalog_path = CatalogBuilder::new()
        .entry(VERSION, RELEASE, ERTS_VSN, MACOS, "default", entry)
        .write_in(&dir.path().join("dist/otp"));
    let paths = CatalogPaths {
        explicit: Some(catalog_path),
        cache: None,
    };
    let net = Net::offline();
    let diag = Diag::disabled();
    let req = OtpReq::Host(RELEASE);
    let ctx = context(&paths, &cache, &net, &diag, &req);

    match erts_source::resolve_in_with(
        &ErtsSourceSpec::Catalog,
        &target(MACOS),
        &ctx,
        elf_reader_refuses,
    ) {
        Err(ErtsError::CatalogClaim {
            field,
            claimed,
            actual,
            ..
        }) => {
            assert_eq!(field, "libc");
            assert_eq!(claimed, "gnu");
            assert_eq!(
                actual, "none",
                "macOS has one system C library, and the emulator is the evidence"
            );
        }
        other => panic!(
            "a catalogue that claims glibc for a Mach-O runtime must be refused with both sides \
             named, and it answered: {other:?}"
        ),
    }
}
