// SPDX-License-Identifier: MIT OR Apache-2.0
//! The version guard's "further ahead than ginary has tested" warning was
//! computed and then thrown away.
//!
//! `Catalog::select` puts the sentence on `Selected::warnings`, and
//! `erts_source::resolve_catalog` did the only thing with it:
//!
//! ```rust
//! for warning in &selected.warnings {
//!     ctx.diag.kv("catalog", &[("warning", warning)]);
//! }
//! ```
//!
//! `Diag::record` returns immediately when no sink is armed, so without
//! `GINARY_DEBUG` or `GINARY_TRACE` the line went nowhere. `ResolvedErts` had
//! no room for a warning either, so nothing could carry it up to
//! `BuildReport::warnings`, which is the channel a user reads. A build against
//! a catalogue three releases ahead of the host's own Erlang was therefore
//! silent — the outcome `CLAUDE.md` forbids: "Skipping is a reported decision
//! or an error, never a default."
//!
//! The right behaviour: the runtime that was resolved carries its own
//! warnings, so the build can print them. The trace copy stays.
#![cfg(feature = "cli")]

use ginary::catalog::{CatalogPaths, OtpReq, RELEASE_WARN_AHEAD};
use ginary::diag::Diag;
use ginary::download::Net;
use ginary::erts_source::{self, ElfFacts, ErtsSourceSpec, SourceContext};
use ginary::target::Target;

use crate::common::catalog::{CatalogBuilder, ERTS_VSN, plant_cached_otp, static_variant};

/// The target the fixture runtime is for.
const MUSL: &str = "linux-x86_64-musl";

/// The release this machine is pretending to compile with.
const HOST_RELEASE: u32 = 29;

/// A release far enough ahead of the host's to earn the warning.
const AHEAD_RELEASE: u32 = HOST_RELEASE + RELEASE_WARN_AHEAD + 1;

#[test]
fn a_catalog_entry_further_ahead_than_ginary_has_tested_warns_the_build_itself() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let cache = dir.path().join("cache/otp");
    let version = format!("{AHEAD_RELEASE}.0.1");
    let entry = static_variant("otp.tar.zst", &"a".repeat(64), 1);
    plant_cached_otp(
        &cache,
        &format!("{version}-{MUSL}-static"),
        &version,
        MUSL,
        "static",
        &entry,
    );
    let catalog_path = CatalogBuilder::new()
        .entry(&version, AHEAD_RELEASE, ERTS_VSN, MUSL, "static", entry)
        .write_in(&dir.path().join("dist/otp"));

    let paths = CatalogPaths {
        explicit: Some(catalog_path),
        cache: None,
    };
    let net = Net::offline();
    // Deliberately no sink: this is the ordinary build, the one the warning
    // was invisible in.
    let diag = Diag::disabled();
    // Exact, because the host rule selects entries of the host's *own*
    // release and could never reach a runtime that is ahead of it; the guard
    // exists for a version somebody pinned.
    let req = OtpReq::Exact {
        version: version.clone(),
        host_release: HOST_RELEASE,
    };
    let ctx = SourceContext {
        catalog_paths: &paths,
        cache_root: &cache,
        net: &net,
        host_release: HOST_RELEASE,
        otp_version: &req,
        variant: None,
        diag: &diag,
    };

    let target: Target = MUSL.parse().expect("a supported target");
    let resolved = erts_source::resolve_in_with(&ErtsSourceSpec::Catalog, &target, &ctx, |_| {
        Ok(ElfFacts {
            machine: "x86_64".to_owned(),
            interp: None,
            needed: Vec::new(),
            glibc_max: None,
        })
    })
    .expect("a newer runtime still resolves; it is a warning, not a refusal");

    assert_eq!(
        resolved.warnings.len(),
        1,
        "the guard raised exactly one thing worth saying: {:?}",
        resolved.warnings
    );
    let warning = &resolved.warnings[0];
    assert!(
        warning.contains("further ahead than ginary has tested"),
        "and it is the version guard's own sentence: {warning}"
    );
    assert!(
        warning.contains(&AHEAD_RELEASE.to_string()) && warning.contains(&HOST_RELEASE.to_string()),
        "naming both releases, so a reader can judge it: {warning}"
    );
}

#[test]
fn a_catalog_entry_of_the_hosts_own_release_carries_no_warning_at_all() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let cache = dir.path().join("cache/otp");
    let version = format!("{HOST_RELEASE}.0.5");
    let entry = static_variant("otp.tar.zst", &"a".repeat(64), 1);
    plant_cached_otp(
        &cache,
        &format!("{version}-{MUSL}-static"),
        &version,
        MUSL,
        "static",
        &entry,
    );
    let catalog_path = CatalogBuilder::new()
        .entry(&version, HOST_RELEASE, ERTS_VSN, MUSL, "static", entry)
        .write_in(&dir.path().join("dist/otp"));

    let paths = CatalogPaths {
        explicit: Some(catalog_path),
        cache: None,
    };
    let net = Net::offline();
    let diag = Diag::disabled();
    let req = OtpReq::Host(HOST_RELEASE);
    let ctx = SourceContext {
        catalog_paths: &paths,
        cache_root: &cache,
        net: &net,
        host_release: HOST_RELEASE,
        otp_version: &req,
        variant: None,
        diag: &diag,
    };

    let target: Target = MUSL.parse().expect("a supported target");
    let resolved = erts_source::resolve_in_with(&ErtsSourceSpec::Catalog, &target, &ctx, |_| {
        Ok(ElfFacts {
            machine: "x86_64".to_owned(),
            interp: None,
            needed: Vec::new(),
            glibc_max: None,
        })
    })
    .expect("the host's own release resolves");

    assert!(
        resolved.warnings.is_empty(),
        "an ordinary build says nothing: {:?}",
        resolved.warnings
    );
}
