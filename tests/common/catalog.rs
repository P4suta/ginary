// SPDX-License-Identifier: MIT OR Apache-2.0
//! The three fixtures the catalogue half needs: a catalogue document, an
//! upstream release asset, and a runtime already in the cache.
//!
//! None of them goes through the code under test. A catalogue is built out of
//! the schema types and serialised with `serde_json`, never with
//! `Catalog::to_json`, because a test that wrote its fixture with the writer it
//! is checking would pass whatever the writer did. The same rule the payload
//! tests follow when they build tar headers by hand.
//!
//! [`FakeUpstream`] is what `ginary otp repack` reads: a `FakeOtp` tree wrapped
//! in a top-level directory and gzipped, which is the shape
//! `gleam-community/erlang-linux-builds` publishes. It is a *fake* upstream in
//! the same sense `FakeOtp` is a fake runtime — the structure is real and the
//! emulator is a shell script — so everything above the ELF reader is
//! reachable without a 42 MB download.
//!
//! [`plant_cached_otp`] is the other end: an extraction that already happened,
//! with the `.meta.json` completion marker `catalog::is_complete` looks for, so
//! a test can assert that a warm cache is used rather than re-fetched.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use ginary::catalog::{
    Catalog, LibcSpec, META_FILE, Meta, OtpVersionEntry, SCHEMA_VERSION, Upstream, Variant,
};

use crate::common::fake_otp::{FakeOtp, FakeOtpRoot};

/// The OTP version every fixture here uses unless it is told another.
pub const VERSION: &str = "29.0.5";

/// The OTP release [`VERSION`] belongs to.
pub const RELEASE: u32 = 29;

/// The ERTS version inside [`VERSION`].
pub const ERTS_VSN: &str = "17.0.5";

/// The upstream tag [`VERSION`] is published under.
pub const UPSTREAM_TAG: &str = "OTP-29.0.5";

/// A catalogue entry with everything filled in, for a fully static musl
/// runtime.
///
/// The digest and the length are the caller's, because they have to be the
/// tarball's real ones for anything downstream to verify.
pub fn static_variant(url: &str, sha256: &str, size: u64) -> Variant {
    Variant {
        url: url.to_owned(),
        sha256: sha256.to_owned(),
        size,
        linkage: "static".to_owned(),
        nif_loading: false,
        libc: LibcSpec {
            kind: "none".to_owned(),
            version: None,
            min: None,
        },
        openssl: "3.5.4".to_owned(),
        jit: true,
        excluded_apps: Vec::new(),
        upstream: Upstream {
            repo: "gleam-community/erlang-linux-builds".to_owned(),
            tag: UPSTREAM_TAG.to_owned(),
            file: "erlang-29.0.5-x64.tar.gz".to_owned(),
            sha256: "0".repeat(64),
        },
        built_at: "2026-08-31T00:00:00Z".to_owned(),
        extra: BTreeMap::new(),
    }
}

/// A catalogue entry for a dynamically linked glibc runtime.
pub fn gnu_variant(url: &str, sha256: &str, size: u64) -> Variant {
    Variant {
        linkage: "dynamic".to_owned(),
        nif_loading: true,
        libc: LibcSpec {
            kind: "gnu".to_owned(),
            version: None,
            min: Some("2.31".to_owned()),
        },
        upstream: Upstream {
            repo: "gleam-community/erlang-linux-builds".to_owned(),
            tag: UPSTREAM_TAG.to_owned(),
            file: "erlang-29.0.5-x64-glibc.tar.gz".to_owned(),
            sha256: "1".repeat(64),
        },
        ..static_variant(url, sha256, size)
    }
}

/// A catalogue document assembled out of the schema types.
#[derive(Clone, Debug, Default)]
pub struct CatalogBuilder {
    generated_at: String,
    versions: BTreeMap<String, OtpVersionEntry>,
}

impl CatalogBuilder {
    /// A builder whose document is generated at the epoch.
    pub fn new() -> Self {
        Self {
            generated_at: "1970-01-01T00:00:00Z".to_owned(),
            versions: BTreeMap::new(),
        }
    }

    /// The `generated_at` the document carries.
    pub fn generated_at(mut self, stamp: &str) -> Self {
        self.generated_at = stamp.to_owned();
        self
    }

    /// Adds one runtime, creating the version and target levels as needed.
    pub fn entry(
        mut self,
        version: &str,
        release: u32,
        erts_vsn: &str,
        target: &str,
        variant: &str,
        entry: Variant,
    ) -> Self {
        let version_entry =
            self.versions
                .entry(version.to_owned())
                .or_insert_with(|| OtpVersionEntry {
                    erts_vsn: erts_vsn.to_owned(),
                    otp_release: release,
                    targets: BTreeMap::new(),
                    extra: BTreeMap::new(),
                });
        version_entry
            .targets
            .entry(target.to_owned())
            .or_default()
            .variants
            .insert(variant.to_owned(), entry);
        self
    }

    /// The document as a value.
    pub fn build(&self) -> Catalog {
        Catalog {
            schema_version: SCHEMA_VERSION,
            generated_at: self.generated_at.clone(),
            otp: self.versions.clone(),
            extra: BTreeMap::new(),
        }
    }

    /// The document as JSON, serialised without the writer under test.
    ///
    /// # Panics
    ///
    /// If the schema types do not serialise, which would be a defect in the
    /// schema rather than in a test.
    pub fn json(&self) -> String {
        let mut text = serde_json::to_string_pretty(&self.build()).expect("the catalog serialises");
        text.push('\n');
        text
    }

    /// Writes the document as `<dir>/catalog.json` and returns its path.
    ///
    /// # Panics
    ///
    /// If the directory cannot be created or the file cannot be written.
    pub fn write_in(&self, dir: &Path) -> PathBuf {
        write_catalog_text(dir, &self.json())
    }
}

/// Writes `text` as `<dir>/catalog.json`, whatever it is.
///
/// The malformed-document tests need a file that is not a catalogue at all, so
/// the writer takes text rather than a value.
///
/// # Panics
///
/// If the directory cannot be created or the file cannot be written.
pub fn write_catalog_text(dir: &Path, text: &str) -> PathBuf {
    std::fs::create_dir_all(dir).expect("the catalog directory");
    let path = dir.join("catalog.json");
    std::fs::write(&path, text).expect("write the catalog");
    path
}

/// A runtime root packed roughly the way the repack pipeline writes one.
///
/// One zstd stream over a deterministic tar, which is all a cache test needs:
/// what it is a fixture *for* is the extraction, and the pipeline's own
/// packing — path order, `mtime` 0, `uid`/`gid` 0 — is asserted against
/// `catalog::pack_runtime` itself in
/// `tests/regressions/c3_a_repacked_runtime_carried_a_non_zero_mtime.rs`
/// rather than reproduced here.
///
/// # Panics
///
/// If the tree cannot be read or the archive cannot be built.
pub fn runtime_tarball(root: &Path) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    builder.mode(tar::HeaderMode::Deterministic);
    builder
        .append_dir_all(".", root)
        .expect("pack the runtime root");
    let tar_bytes = builder.into_inner().expect("finish the archive");
    crate::common::payload::zstd_bytes(&tar_bytes, 19)
}

/// One upstream release asset: a runtime under a top-level directory, gzipped.
pub struct FakeUpstream {
    dir: tempfile::TempDir,
    top: String,
    bytes: Vec<u8>,
}

impl FakeUpstream {
    /// Builds `<top>/{bin,erts-*,lib,releases}` and gzips it.
    ///
    /// `extras` are paths relative to the top-level directory that are written
    /// as small files, which is how a test plants the fat a prune is supposed
    /// to strip.
    ///
    /// # Panics
    ///
    /// If the tree cannot be built or the archive cannot be written.
    pub fn build(top: &str, extras: &[&str]) -> Self {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let root = dir.path().join(top);
        FakeOtp::new()
            .erts_vsn(ERTS_VSN)
            .release(RELEASE)
            .otp_version(VERSION)
            .build_in(&root);
        for relative in extras {
            let path = root.join(relative);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("an extra directory");
            }
            std::fs::write(&path, b"upstream fat\n").expect("an extra file");
        }

        let mut builder = tar::Builder::new(Vec::new());
        builder.mode(tar::HeaderMode::Deterministic);
        builder
            .append_dir_all(top, &root)
            .expect("pack the upstream tree");
        let tar_bytes = builder.into_inner().expect("finish the archive");

        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&tar_bytes).expect("gzip the archive");
        let bytes = encoder.finish().expect("finish the gzip");

        Self {
            dir,
            top: top.to_owned(),
            bytes,
        }
    }

    /// The unpacked tree, which is what a prune and a dereference read.
    pub fn root(&self) -> PathBuf {
        self.dir.path().join(&self.top)
    }

    /// The gzipped archive.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// The archive's SHA-256, in lower-case hexadecimal.
    pub fn sha256_hex(&self) -> String {
        crate::common::payload::sha256_hex(&self.bytes)
    }

    /// Writes the archive into `dir` under `name` and returns its path.
    ///
    /// # Panics
    ///
    /// If the file cannot be written.
    pub fn write_in(&self, dir: &Path, name: &str) -> PathBuf {
        std::fs::create_dir_all(dir).expect("the upstream directory");
        let path = dir.join(name);
        std::fs::write(&path, &self.bytes).expect("write the upstream asset");
        path
    }
}

/// An extraction that already happened, marker and all.
///
/// Returns the entry directory. The runtime inside is a `FakeOtp`, so
/// `otp::inspect_root` accepts it and the emulator is a shell script — which is
/// exactly the shape that makes the *unseamed* inspection refuse it, and is
/// what the catalogue-claim tests inject around.
///
/// # Panics
///
/// If the tree or the marker cannot be written.
pub fn plant_cached_otp(
    cache_root: &Path,
    dir_name: &str,
    version: &str,
    target: &str,
    variant: &str,
    entry: &Variant,
) -> (PathBuf, FakeOtpRoot) {
    plant_cached_otp_of(
        cache_root,
        dir_name,
        version,
        target,
        variant,
        entry,
        FakeOtp::new(),
    )
}

/// [`plant_cached_otp`], over a runtime flavour the caller chose.
///
/// The cache holds whatever the catalogue served, and what it served is not
/// always a unix tree: a macOS entry extracts a Mach-O emulator and a Windows
/// entry extracts `beam.smp.dll` beside `erl.exe`. The builder is taken rather
/// than derived from the target name, because the target a *catalogue entry*
/// claims and the flavour of the tree that came out of it are the two sides
/// `resolve_catalog` exists to compare — see
/// `tests/regressions/e16_a_cached_windows_runtime_was_read_by_the_elf_reader.rs`.
///
/// The version, release and ERTS version are set here, after the caller's
/// builder, so a planted entry always agrees with the marker written beside it.
///
/// # Panics
///
/// If the tree or the marker cannot be written.
pub fn plant_cached_otp_of(
    cache_root: &Path,
    dir_name: &str,
    version: &str,
    target: &str,
    variant: &str,
    entry: &Variant,
    builder: FakeOtp,
) -> (PathBuf, FakeOtpRoot) {
    let dir = cache_root.join(dir_name);
    let otp = builder
        .erts_vsn(ERTS_VSN)
        .release(RELEASE)
        .otp_version(version)
        .build_in(&dir);

    let meta = Meta {
        version: version.to_owned(),
        target: target.to_owned(),
        variant: variant.to_owned(),
        entry: entry.clone(),
        extracted_at: "2026-08-31T00:00:00Z".to_owned(),
    };
    let text = serde_json::to_string_pretty(&meta).expect("the marker serialises");
    std::fs::write(dir.join(META_FILE), text).expect("write the completion marker");

    (dir, otp)
}
