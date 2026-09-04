// SPDX-License-Identifier: MIT OR Apache-2.0
//! A Windows runtime that arrived through the cache was handed to the ELF
//! reader and refused as "not an ELF runtime".
//!
//! **What went wrong.** The same defect
//! `e16_a_cached_macos_runtime_was_read_by_the_elf_reader` records, left
//! standing on the third platform. `resolve_with` — the `host` and `dir:` arm
//! — makes *two* dispatches before it reaches the ELF seam: a tree
//! [`ginary::assemble::is_windows_erts_bin`] recognises goes to the PE reader,
//! and an emulator whose first bytes are a Mach-O goes to the Mach-O reader.
//! `read_emulator` — the `catalog` and `tarball:` arm — made only the second.
//! So a Windows runtime out of the cache reached `emulator_path`, which names
//! `erts-<vsn>/bin/beam.smp`, a file a Windows tree does not have (its emulator
//! is `beam.smp.dll`), and the absent file was reported as the ELF reader's
//! failure to read one:
//!
//! ```text
//! `...\erts-17.0.5\bin\beam.smp` is not an ELF runtime: Io { kind: NotFound }
//! ```
//!
//! That is the very sentence D2 records as the old Windows failure, arriving
//! again through the two sources that fill a cache.
//!
//! It was not unreachable. `crate::otp::inspect_root` accepts a Windows tree —
//! `check_erts_binaries` dispatches on the same flavour test, and
//! `d2_a_windows_runtime_root_could_not_be_resolved.rs` asserts exactly that —
//! so nothing above `read_emulator` refuses one first.
//!
//! **The input.** Any `catalog` or `tarball:` source whose `erts-<vsn>/bin`
//! holds `erl.exe` and `beam.smp.dll` — that is, every Windows runtime that is
//! not already on the machine.
//!
//! **The correct behaviour.** Which reader reads an emulator is the tree's and
//! the emulator's own to say, on every path that reads one, and in the order
//! `resolve_with` decides it: the flavour of the tree first, the emulator's
//! magic second. A Windows runtime from the cache resolves exactly as one from
//! `dir:` does — the machine comes off the PE's own COFF header, the linkage is
//! dynamic, there is no glibc floor, and the provenance is the source's own
//! label — and the ELF seam is not consulted at all.

use std::path::Path;

use ginary::catalog::{CatalogPaths, OtpReq};
use ginary::diag::Diag;
use ginary::download::Net;
use ginary::elf::ElfError;
use ginary::erts_source::{self, ElfFacts, ErtsError, ErtsSourceSpec, SourceContext};
use ginary::target::{Linkage, Target};

use crate::common::catalog::{
    CatalogBuilder, ERTS_VSN, RELEASE, VERSION, plant_cached_otp_of, runtime_tarball,
    static_variant,
};
use crate::common::fake_otp::{FakeOtp, PE_MACHINE_AMD64};

/// The target every runtime in this file is for.
const WINDOWS: &str = "windows-x86_64";

/// The named target.
fn target(name: &str) -> Target {
    name.parse()
        .unwrap_or_else(|error| panic!("`{name}` must be a target: {error}"))
}

/// An ELF seam that refuses everything it is shown.
///
/// The whole claim is that this is never called: a Windows runtime's emulator
/// is a PE read by `object`, and an ELF reader that is handed one — or handed
/// the unix name a Windows tree does not carry — has already lost.
fn elf_reader_refuses(_: &Path) -> Result<ElfFacts, ElfError> {
    Err(ElfError::Parse {
        message: "the ELF reader was handed a Windows runtime".to_owned(),
    })
}

/// A context over `cache` with the given catalogue, offline.
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

/// What a Windows catalogue entry claims: dynamically linked against the
/// system's own C runtime, and NIFs load.
fn windows_variant() -> ginary::catalog::Variant {
    let mut entry = static_variant("otp.tar.zst", &"c".repeat(64), 1);
    entry.linkage = "dynamic".to_owned();
    entry.nif_loading = true;
    entry
}

#[test]
fn a_windows_runtime_unpacked_from_a_tarball_is_read_as_a_pe() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let source = dir.path().join("source");
    FakeOtp::new()
        .erts_vsn(ERTS_VSN)
        .release(RELEASE)
        .otp_version(VERSION)
        .windows()
        .pe_machine(PE_MACHINE_AMD64)
        .build_in(&source);
    let archive = dir.path().join("otp-29.0.5-windows-x86_64.tar.zst");
    std::fs::write(&archive, runtime_tarball(&source)).expect("the archive");
    let cache = dir.path().join("cache/otp");

    let paths = CatalogPaths::default();
    let net = Net::offline();
    let diag = Diag::disabled();
    let req = OtpReq::Host(RELEASE);
    let ctx = context(&paths, &cache, &net, &diag, &req);

    let resolved = erts_source::resolve_in_with(
        &ErtsSourceSpec::Tarball(archive.clone()),
        &target(WINDOWS),
        &ctx,
        elf_reader_refuses,
    )
    .expect("a tarball whose `erts-<vsn>/bin` is a Windows one is a Windows runtime");

    assert_eq!(
        resolved.target,
        target(WINDOWS),
        "the machine comes off the emulator's own COFF header, exactly as it does through `dir:`"
    );
    assert_eq!(
        resolved.linkage,
        Linkage::Dynamic,
        "a Windows runtime is a set of DLLs `erl.exe` loads"
    );
    assert!(resolved.nif_loading, "and therefore loads NIFs");
    assert_eq!(
        resolved.libc_min, None,
        "Windows has one system C runtime and no symbol-version floor to record"
    );
    assert_eq!(
        resolved.provenance,
        format!("tarball:{}", archive.display()),
        "the provenance is the spelling the configuration used"
    );
}

#[test]
fn a_windows_runtime_already_in_the_cache_is_read_as_a_pe() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let cache = dir.path().join("cache/otp");
    let entry = windows_variant();
    plant_cached_otp_of(
        &cache,
        // `catalog::entry_dir_name` leaves the unnamed `default` variant out of
        // the directory name, so a planted entry that carried it would be a
        // cache miss and the test would measure a download instead.
        "29.0.5-windows-x86_64",
        VERSION,
        WINDOWS,
        "default",
        &entry,
        FakeOtp::new().windows().pe_machine(PE_MACHINE_AMD64),
    );

    let catalog_path = CatalogBuilder::new()
        .entry(VERSION, RELEASE, ERTS_VSN, WINDOWS, "default", entry)
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
        &target(WINDOWS),
        &ctx,
        elf_reader_refuses,
    )
    .expect("a catalogue entry whose tree is a Windows one is a Windows runtime");

    assert_eq!(resolved.target, target(WINDOWS));
    assert_eq!(resolved.linkage, Linkage::Dynamic);
    assert_eq!(resolved.libc_min, None);
    assert_eq!(
        resolved.provenance,
        erts_source::catalog_provenance(VERSION, WINDOWS, "default"),
        "the provenance names the entry, and the claim check passed against the PE"
    );
}

#[test]
fn a_catalogue_entry_that_claims_glibc_for_a_windows_runtime_is_refused() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let cache = dir.path().join("cache/otp");
    let mut entry = windows_variant();
    // The lie: `none` is the only honest C library for a Windows runtime, and
    // this entry claims the one a Linux gnu build would name. Without it,
    // nothing in the suite could tell `libc_kind: Some("none")` from a plain
    // `None` — `resolve_catalog` spells both as `none` — so the field would be
    // pinned by no assertion at all.
    entry.libc.kind = "gnu".to_owned();
    plant_cached_otp_of(
        &cache,
        "29.0.5-windows-x86_64",
        VERSION,
        WINDOWS,
        "default",
        &entry,
        FakeOtp::new().windows().pe_machine(PE_MACHINE_AMD64),
    );

    let catalog_path = CatalogBuilder::new()
        .entry(VERSION, RELEASE, ERTS_VSN, WINDOWS, "default", entry)
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
        &target(WINDOWS),
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
                "the emulator is the evidence, and a Windows one names the system C runtime"
            );
        }
        other => panic!(
            "a catalogue that claims glibc for a Windows runtime must be refused with both sides \
             named, and it answered: {other:?}"
        ),
    }
}
