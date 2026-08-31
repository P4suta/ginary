// SPDX-License-Identifier: MIT OR Apache-2.0
//! The prebuilt-OTP catalogue, the cache it fills, and the local pipeline that
//! produces it.
//!
//! A cross-target build needs a BEAM runtime for a machine it is not running
//! on. The catalogue is the index of those runtimes: one JSON document naming,
//! per OTP version, per target, per variant, a `.tar.zst` with its SHA-256, its
//! length and everything ginary would otherwise have to guess — the linkage,
//! whether a NIF can be loaded into it, the libc floor, and where upstream it
//! came from.
//!
//! **The catalogue is local first.** No hosted `ginary-otp` repository exists,
//! nothing is published, and [`EMBEDDED`] is deliberately an empty document.
//! [`repack`] is the pipeline itself, run on a developer's machine: it fetches
//! an upstream release asset, verifies it against the digest the release API
//! reported, prunes it, dereferences every symlink, checks the emulator really
//! is for the target claimed, re-packs it deterministically and appends the
//! entry to a `catalog.json` beside the tarballs. See
//! `docs/adr/0013-local-first-otp-catalog.md` for what flips when a hosted
//! catalogue appears.
//!
//! **A catalogue is an index, never evidence.** Every claim in it is checked:
//! the tarball against its digest by [`crate::download`], and the extracted
//! runtime against its own `beam.smp` by [`crate::erts_source`], which is the
//! build's single trust anchor. An entry that says `linux-aarch64-musl` and
//! unpacks to an x86-64 emulator is a hard error naming both.
//!
//! A URL without a scheme is resolved against the directory the catalogue was
//! read from, which is what makes `dist/otp/catalog.json` usable from a
//! checkout with the tarballs beside it and nothing hosted anywhere.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::diag::Diag;
use crate::download::{self, DownloadError, Expect, Net};
use crate::elf::{ElfError, ElfInfo};
use crate::process::{shell_quote, shell_quote_path};
use crate::target::{Libc, Linkage, Target};

/// The only catalogue schema this ginary reads.
pub const SCHEMA_VERSION: u32 = 1;

/// The variable naming a catalogue, equivalent to `--catalog`.
pub const CATALOG_ENV_VAR: &str = "GINARY_CATALOG";

/// The catalogue file's name, wherever it is found.
pub const CATALOG_FILE: &str = "catalog.json";

/// The directory of the cache this module owns, under the ginary cache root.
pub const CACHE_SUBDIR: &str = "otp";

/// The file whose presence means an extracted runtime is complete.
pub const META_FILE: &str = ".meta.json";

/// Where the fill locks live, under the cache root.
///
/// Beside the entries rather than inside them: an entry directory does not
/// exist until the rename that finishes its extraction, so a lock kept inside
/// one would not exist until the work it guards was already done. Dotted, so
/// it sorts with the bookkeeping rather than looking like a runtime.
pub const LOCK_SUBDIR: &str = ".locks";

/// The variant a musl target uses when the configuration names none.
pub const DEFAULT_MUSL_VARIANT: &str = "static";

/// The variant name an entry with one unnamed variant carries.
pub const DEFAULT_VARIANT: &str = "default";

/// What a `.meta.json` records where a user-supplied tarball has no catalogue
/// entry to copy.
///
/// A tarball names no version, target or variant of its own: those are the
/// emulator's to answer, and [`crate::erts_source`] reads them out of it a
/// moment later. The marker says so rather than inventing a value.
pub const TARBALL_ORIGIN: &str = "tarball";

/// How many OTP releases ahead of the host's a catalogue entry may be before
/// it is worth a warning.
pub const RELEASE_WARN_AHEAD: u32 = 2;

/// The catalogue compiled into this binary.
///
/// Empty on purpose. There is no hosted catalogue to snapshot, so a document
/// with entries in it would be a claim about files nobody can fetch; a build
/// that needs a runtime says so and names `--catalog` and `ginary otp repack`
/// instead. See `docs/adr/0013-local-first-otp-catalog.md`.
pub const EMBEDDED: &str =
    "{\"schema_version\":1,\"generated_at\":\"1970-01-01T00:00:00Z\",\"otp\":{}}";

/// The repository the Linux runtimes are repackaged from.
pub const UPSTREAM_REPO: &str = "gleam-community/erlang-linux-builds";

/// Directories that never travel in a repacked runtime.
///
/// Sorted, because the list is printed. `include` is not here and must not be:
/// a NIF built against a packaged runtime needs `erts-*/include`.
pub const PRUNE_DIRS: [&str; 7] = ["c_src", "doc", "emacs", "examples", "man", "misc", "src"];

/// Directories the prune list must never touch, asserted rather than assumed.
pub const KEPT_DIRS: [&str; 3] = ["ebin", "include", "priv"];

/// File suffixes that never travel in a repacked runtime.
pub const PRUNE_SUFFIXES: [&str; 1] = [".pdb"];

/// The zstd level a repacked runtime is written at.
pub const REPACK_LEVEL: i32 = 19;

/// The buffer archives are copied through.
const COPY_BUFFER: usize = 64 * 1024;

// ------------------------------------------------------ the schema --

/// One catalogue document, schema [`SCHEMA_VERSION`].
///
/// Unknown keys are kept in `extra` rather than refused: a catalogue written by
/// a newer ginary is still readable by this one, and a key it does not
/// understand survives a round trip instead of being silently dropped.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct Catalog {
    /// Always [`SCHEMA_VERSION`] for a document this ginary accepts.
    pub schema_version: u32,
    /// When the document was generated, RFC 3339 in UTC.
    pub generated_at: String,
    /// OTP version to what is available for it.
    pub otp: BTreeMap<String, OtpVersionEntry>,
    /// Every key at this level this ginary does not know.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

/// One OTP version, and the targets it is available for.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct OtpVersionEntry {
    /// The ERTS version inside, `17.0.5` for OTP 29.0.5.
    pub erts_vsn: String,
    /// The OTP release, the number a compiled module is compatible with.
    pub otp_release: u32,
    /// Target name to what is available for it.
    pub targets: BTreeMap<String, TargetEntry>,
    /// Every key at this level this ginary does not know.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

/// One target of one OTP version, and the variants built for it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct TargetEntry {
    /// Variant name to the runtime built for it.
    pub variants: BTreeMap<String, Variant>,
    /// Every key at this level this ginary does not know.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

/// One runtime: a tarball and everything known about what is in it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct Variant {
    /// Where the tarball is. A URL with no scheme is resolved against the
    /// directory the catalogue itself was read from; see [`resolve_url`].
    pub url: String,
    /// The tarball's SHA-256, 64 lower-case hexadecimal digits.
    pub sha256: String,
    /// The tarball's length in bytes.
    pub size: u64,
    /// How the emulator is linked.
    pub linkage: String,
    /// Whether a NIF can be loaded into this runtime.
    pub nif_loading: bool,
    /// The C library the runtime needs.
    pub libc: LibcSpec,
    /// The OpenSSL the crypto application was built against.
    ///
    /// Empty when the repack could not read one out of the tree: a statically
    /// linked `crypto` carries the `OpenSSL x.y.z` banner in its own bytes and
    /// a dynamically linked one resolves it on the target machine, where this
    /// build cannot see it.
    pub openssl: String,
    /// Whether the emulator carries the JIT.
    pub jit: bool,
    /// Applications the repack left out entirely.
    pub excluded_apps: Vec<String>,
    /// Where the bytes came from before ginary touched them.
    pub upstream: Upstream,
    /// When the tarball was built, RFC 3339 in UTC.
    pub built_at: String,
    /// Every key at this level this ginary does not know.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

impl Variant {
    /// What a user-supplied tarball records where a catalogue entry would be.
    ///
    /// Only the three things a file can state about itself are filled in — its
    /// path, its digest and its length. Every other field is a claim the
    /// catalogue would have made and a tarball does not, and the emulator is
    /// read for all of them a moment later, so nothing here is a fact anything
    /// downstream reads.
    pub fn of_tarball(archive: &Path, sha256: &str, size: u64) -> Self {
        Self {
            url: archive.display().to_string(),
            sha256: sha256.to_owned(),
            size,
            linkage: String::new(),
            nif_loading: false,
            libc: LibcSpec::default(),
            openssl: String::new(),
            jit: false,
            excluded_apps: Vec::new(),
            upstream: Upstream {
                repo: String::new(),
                tag: String::new(),
                file: archive
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                sha256: sha256.to_owned(),
            },
            built_at: String::new(),
            extra: BTreeMap::new(),
        }
    }
}

/// The C library a runtime needs.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct LibcSpec {
    /// `gnu`, `musl`, or `none` for a static runtime.
    pub kind: String,
    /// The exact version a dynamic musl runtime was built against.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// The lowest glibc a dynamic gnu runtime will load against.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<String>,
}

/// Where a repacked runtime came from.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct Upstream {
    /// The repository, `owner/name`.
    pub repo: String,
    /// The release tag.
    pub tag: String,
    /// The asset file name.
    pub file: String,
    /// The asset's SHA-256, as the release API reported it.
    pub sha256: String,
}

/// What an OTP version was asked for by.
///
/// Both spellings carry the release the *host* compiles with, because the
/// version rule applies to both: `Host` selects by it, and `Exact` is still
/// held to it. A runtime older than the compiler that made the modules cannot
/// load them, whichever way the version was written down.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OtpReq {
    /// `otp_version = "host"`: the newest entry whose release is the host's.
    Host(u32),
    /// One version key and no other, held to the host rule all the same.
    ///
    /// Nothing in a `gleam.toml` builds one today: `[tools.ginary.target.*]`
    /// has four keys and `otp_version` is not among them, and
    /// [`crate::bundle`] always asks with [`OtpReq::Host`]. This is the shape a
    /// pin will take, and the shape the release guard's forward warning is
    /// reachable through — the host rule selects entries of the host's own
    /// release and can never choose one that is ahead of it.
    Exact {
        /// The version key the configuration named.
        version: String,
        /// The release the shipment was compiled by.
        host_release: u32,
    },
}

impl OtpReq {
    /// The spelling this request was written as.
    pub fn label(&self) -> String {
        match self {
            Self::Host(release) => format!("host (release {release})"),
            Self::Exact { version, .. } => version.clone(),
        }
    }

    /// The release the shipment was compiled by.
    pub const fn host_release(&self) -> u32 {
        match self {
            Self::Host(release) => *release,
            Self::Exact { host_release, .. } => *host_release,
        }
    }
}

/// Where a catalogue may be read from, in the order they are tried.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CatalogPaths {
    /// `--catalog PATH`, or [`CATALOG_ENV_VAR`].
    pub explicit: Option<PathBuf>,
    /// `<cache>/otp/catalog.json`, what `ginary otp update` writes.
    pub cache: Option<PathBuf>,
}

/// Which of the three sources a catalogue was read from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CatalogOrigin {
    /// `--catalog`, or [`CATALOG_ENV_VAR`].
    Explicit(PathBuf),
    /// The catalogue `ginary otp update` wrote into the cache.
    Cache(PathBuf),
    /// [`EMBEDDED`], which is empty.
    Embedded,
}

impl CatalogOrigin {
    /// How the origin is named in a message.
    pub fn label(&self) -> String {
        match self {
            Self::Explicit(path) | Self::Cache(path) => path.display().to_string(),
            Self::Embedded => "the embedded catalog".to_owned(),
        }
    }

    /// The directory a scheme-less URL resolves against, or [`None`] for the
    /// embedded catalogue, which names no files.
    pub fn dir(&self) -> Option<&Path> {
        match self {
            Self::Explicit(path) | Self::Cache(path) => path.parent(),
            Self::Embedded => None,
        }
    }

    /// The path a `--catalog` flag would have to name to read this again.
    ///
    /// [`None`] for the two origins a command finds by itself: the cached
    /// catalogue and the embedded one both need no flag, and printing one for
    /// them would put a path into a message that adds nothing.
    pub fn flag_path(&self) -> Option<&Path> {
        match self {
            Self::Explicit(path) => Some(path),
            Self::Cache(_) | Self::Embedded => None,
        }
    }
}

/// A catalogue and where it was read from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadedCatalog {
    /// The document.
    pub catalog: Catalog,
    /// Which source it came from.
    pub origin: CatalogOrigin,
}

/// One entry, chosen out of a catalogue.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selected<'a> {
    /// The OTP version key.
    pub version: &'a str,
    /// The target name.
    pub target: &'a str,
    /// The variant name.
    pub variant: &'a str,
    /// The runtime itself.
    pub entry: &'a Variant,
    /// The version's own fields, `erts_vsn` and `otp_release`.
    pub otp: &'a OtpVersionEntry,
    /// Anything a reader should know that is not an error, such as a release
    /// further ahead of the host's than [`RELEASE_WARN_AHEAD`].
    pub warnings: Vec<String>,
}

impl Selected<'_> {
    /// The cache directory this entry extracts into, relative to the root.
    pub fn dir_name(&self) -> String {
        entry_dir_name(self.version, self.target, self.variant)
    }
}

/// Where a scheme-less catalogue URL points.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceUrl {
    /// An absolute URL, fetched by [`crate::download::fetch`].
    Remote(String),
    /// A file on this machine, copied rather than fetched.
    File(PathBuf),
}

// ------------------------------------------------------ the errors --

/// Why a catalogue could not be read, or an entry could not be chosen.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CatalogError {
    /// The document is not JSON, or not this schema's shape.
    #[error("{origin} is not a ginary catalog: {message}")]
    Parse {
        /// Where the document came from.
        origin: String,
        /// What the reader said, with its line and column.
        message: String,
    },
    /// The document is a catalogue of another schema.
    #[error("{origin} is catalog schema {found} and this ginary reads schema {supported}")]
    SchemaVersion {
        /// Where the document came from.
        origin: String,
        /// The schema it declares.
        found: u32,
        /// The schema this ginary reads.
        supported: u32,
    },
    /// No entry for the version that was asked for.
    #[error("{origin} has no OTP {req} entry; it has {available}")]
    NoSuchVersion {
        /// Where the catalogue came from.
        origin: String,
        /// What was asked for.
        req: String,
        /// What it does have, comma separated, or `nothing at all`.
        available: String,
    },
    /// No entry for the target that was asked for.
    #[error("{origin} has no {target} entry for OTP {version}; it has {available}")]
    NoSuchTarget {
        /// Where the catalogue came from.
        origin: String,
        /// The version that was chosen.
        version: String,
        /// The target that was asked for.
        target: String,
        /// The targets it does have.
        available: String,
    },
    /// No such variant of that target.
    #[error("OTP {version} for {target} has no `{variant}` variant; it has {available}")]
    NoSuchVariant {
        /// The version that was chosen.
        version: String,
        /// The target that was chosen.
        target: String,
        /// The variant that was asked for.
        variant: String,
        /// The variants it does have.
        available: String,
    },
    /// Several variants and nothing to choose between them.
    #[error(
        "OTP {version} for {target} has {available} and no default; name one with `otp_variant`"
    )]
    AmbiguousVariant {
        /// The version that was chosen.
        version: String,
        /// The target that was chosen.
        target: String,
        /// The variants it has.
        available: String,
    },
    /// The runtime is older than the compiler that made the modules.
    #[error(
        "the catalog's OTP {version} is release {entry_release} and this machine compiles with \
         release {host_release}; a module compiled by OTP {host_release} does not load on OTP \
         {entry_release}"
    )]
    OtpTooOld {
        /// The version that was chosen.
        version: String,
        /// Its release.
        entry_release: u32,
        /// The host's release.
        host_release: u32,
    },
    /// The runtime is not in the cache and nothing was asked to fetch it.
    ///
    /// `command` is built by [`fetch_command`] out of the flags the run that
    /// failed was given, so the remedy is a command that works rather than one
    /// that reads a different catalogue.
    #[error("{dir} is not cached; run `{command}`")]
    NotCached {
        /// Where it would be.
        dir: PathBuf,
        /// The version it is for.
        version: String,
        /// The target it is for.
        target: String,
        /// The `ginary otp fetch` line that would fill it.
        command: String,
    },
    /// An entry's `sha256` is not a digest, so nothing could be verified.
    #[error(
        "the catalog entry for OTP {version} on {target} has sha256 `{value}`, which is not 64 \
         lower-case hexadecimal digits"
    )]
    BadDigest {
        /// The version the entry is for.
        version: String,
        /// The target the entry is for.
        target: String,
        /// What the entry said.
        value: String,
    },
    /// The tarball could not be fetched.
    #[error("cannot fetch the runtime")]
    Download(#[from] DownloadError),
    /// A file could not be read or written.
    #[error("cannot use {path}: {message}")]
    Io {
        /// The file.
        path: PathBuf,
        /// What the operating system said.
        message: String,
    },
    /// The tarball is not an archive of a runtime root.
    #[error("{path} is not a runtime tarball: {message}")]
    Extract {
        /// The archive.
        path: PathBuf,
        /// What was wrong with it.
        message: String,
    },
}

/// Why a runtime could not be repacked.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RepackError {
    /// The target and variant map to no upstream asset.
    #[error("{upstream} has no asset for {target}:{variant}")]
    NoUpstreamAsset {
        /// The repository that was consulted.
        upstream: &'static str,
        /// The target that was asked for.
        target: String,
        /// The variant that was asked for.
        variant: String,
    },
    /// A `target:variant` selector could not be read.
    #[error("`{value}` is not a `<target>[:<variant>]` selector: {reason}")]
    BadSelector {
        /// The value that was refused.
        value: String,
        /// Why.
        reason: String,
    },
    /// A symlink points at nothing.
    #[error("{path} is a symlink to {target}, which is not there")]
    DanglingSymlink {
        /// The link.
        path: PathBuf,
        /// What it pointed at.
        target: PathBuf,
    },
    /// A symlink points outside the runtime root.
    #[error("{path} is a symlink to {target}, which is outside the runtime root")]
    UnsafeSymlink {
        /// The link.
        path: PathBuf,
        /// What it pointed at.
        target: PathBuf,
    },
    /// A symlink survived dereferencing, so the strict extractor would refuse
    /// the tarball this repack is about to write.
    #[error("{path} is still a symlink after dereferencing")]
    SymlinkRemains {
        /// The link.
        path: PathBuf,
    },
    /// A file could not be read or written.
    #[error("cannot use {path}: {message}")]
    Io {
        /// The file.
        path: PathBuf,
        /// What the operating system said.
        message: String,
    },
    /// The upstream asset is not for the target it was requested as.
    #[error(
        "{file} was requested as {target} and its emulator is for {actual}; either upstream \
         mislabelled the asset or the mapping in `upstream_asset` is wrong"
    )]
    UpstreamMismatch {
        /// The upstream asset.
        file: String,
        /// The target it was requested as.
        target: String,
        /// The target its emulator is really for.
        actual: String,
    },
    /// The upstream asset is not linked the way the variant name claims.
    #[error(
        "{file} was requested as the `{variant}` variant, which is {claimed}, and its emulator \
         is {actual}"
    )]
    UpstreamLinkage {
        /// The upstream asset.
        file: String,
        /// The variant it was requested as.
        variant: String,
        /// The linkage that variant claims.
        claimed: &'static str,
        /// The linkage its emulator really has.
        actual: &'static str,
    },
    /// The upstream tag does not name an OTP version.
    #[error("`{tag}` is not an `OTP-<version>` release tag, so nothing could be keyed by it")]
    BadTag {
        /// The tag that was given.
        tag: String,
    },
    /// The release API answered something that is not a release.
    #[error("{url} is not a release description: {message}")]
    Api {
        /// The URL that was read.
        url: String,
        /// What was wrong with the answer.
        message: String,
    },
    /// The upstream asset could not be fetched.
    ///
    /// The fetch's own message is rendered inline rather than left as a
    /// `source`: what a reader needs is the URL and what to do about it, and
    /// the pipeline's callers print one line.
    #[error("cannot fetch the upstream asset: {reason}")]
    Download {
        /// What the fetch said.
        reason: DownloadError,
    },
}

impl From<DownloadError> for RepackError {
    /// A failed fetch, carried up with its own message intact.
    fn from(reason: DownloadError) -> Self {
        Self::Download { reason }
    }
}

// ------------------------------------------------------ reading one --

impl Catalog {
    /// An empty catalogue of this schema.
    pub fn empty(generated_at: &str) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            generated_at: generated_at.to_owned(),
            otp: BTreeMap::new(),
            extra: BTreeMap::new(),
        }
    }

    /// Reads one catalogue document.
    ///
    /// The schema version is read before the rest of the document, so a
    /// catalogue of another schema is reported as one rather than as a list of
    /// fields this ginary happens not to find.
    ///
    /// # Errors
    ///
    /// [`CatalogError::Parse`] with the reader's line and column, or
    /// [`CatalogError::SchemaVersion`] for a document of another schema.
    pub fn parse(text: &str, origin: &str) -> Result<Self, CatalogError> {
        let value: serde_json::Value =
            serde_json::from_str(text).map_err(|error| CatalogError::Parse {
                origin: origin.to_owned(),
                message: error.to_string(),
            })?;
        let found = value
            .get("schema_version")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| CatalogError::Parse {
                origin: origin.to_owned(),
                message: "no `schema_version` at the top level".to_owned(),
            })?;
        let found = u32::try_from(found).unwrap_or(u32::MAX);
        if found != SCHEMA_VERSION {
            return Err(CatalogError::SchemaVersion {
                origin: origin.to_owned(),
                found,
                supported: SCHEMA_VERSION,
            });
        }
        serde_json::from_value(value).map_err(|error| CatalogError::Parse {
            origin: origin.to_owned(),
            message: error.to_string(),
        })
    }

    /// The first catalogue that is there, whole.
    ///
    /// `--catalog`, then the cache, then [`EMBEDDED`]. First found wins the
    /// *whole file*: there is no per-entry merge, because two catalogues
    /// disagreeing about one digest is a question with no safe answer, and a
    /// user who names a catalogue means that catalogue.
    ///
    /// # Errors
    ///
    /// As [`Catalog::parse`], plus [`CatalogError::Io`] for a `--catalog` that
    /// is not readable. A *cache* catalogue that is not there is not an error;
    /// one that is there and unreadable is.
    pub fn load(paths: &CatalogPaths) -> Result<LoadedCatalog, CatalogError> {
        if let Some(path) = &paths.explicit {
            let text = std::fs::read_to_string(path).map_err(|error| CatalogError::Io {
                path: path.clone(),
                message: error.to_string(),
            })?;
            return Ok(LoadedCatalog {
                catalog: Self::parse(&text, &path.display().to_string())?,
                origin: CatalogOrigin::Explicit(path.clone()),
            });
        }

        if let Some(path) = &paths.cache {
            match std::fs::read_to_string(path) {
                Ok(text) => {
                    return Ok(LoadedCatalog {
                        catalog: Self::parse(&text, &path.display().to_string())?,
                        origin: CatalogOrigin::Cache(path.clone()),
                    });
                }
                // A cache that has never been filled is the normal state of a
                // fresh machine, and is not a failure; anything else is.
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(CatalogError::Io {
                        path: path.clone(),
                        message: error.to_string(),
                    });
                }
            }
        }

        Ok(LoadedCatalog {
            catalog: Self::parse(EMBEDDED, &CatalogOrigin::Embedded.label())?,
            origin: CatalogOrigin::Embedded,
        })
    }

    /// Chooses one runtime, and holds it to the version rule.
    ///
    /// `Host` takes the newest patch whose `otp_release` is the host's;
    /// `Exact` takes the version key it names. Both are then checked against
    /// the host release by [`check_release`], which is why the request carries
    /// it: an entry older than the compiler is refused here, where the
    /// alternative is an artifact that cannot load its own modules.
    ///
    /// # Errors
    ///
    /// [`CatalogError`], naming what the catalogue does have at whichever
    /// level the lookup failed, and [`CatalogError::OtpTooOld`] when the
    /// chosen release is older than the host's.
    pub fn select(
        &self,
        req: &OtpReq,
        target: &str,
        variant: Option<&str>,
        origin: &str,
    ) -> Result<Selected<'_>, CatalogError> {
        let version = match req {
            OtpReq::Host(release) => self
                .otp
                .iter()
                .filter(|(_, entry)| entry.otp_release == *release)
                .max_by(|(left, _), (right, _)| compare_versions(left, right))
                .map(|(version, _)| version.as_str()),
            OtpReq::Exact { version, .. } => self
                .otp
                .get_key_value(version.as_str())
                .map(|(version, _): (&String, &OtpVersionEntry)| version.as_str()),
        };
        let Some(version) = version else {
            return Err(CatalogError::NoSuchVersion {
                origin: origin.to_owned(),
                req: req.label(),
                available: listed(self.otp.keys()),
            });
        };

        let mut selected = self.lookup(version, target, variant, origin)?;
        if let Some(warning) = check_release(version, selected.otp.otp_release, req.host_release())?
        {
            selected.warnings.push(warning);
        }
        Ok(selected)
    }

    /// The entry one version, target and variant name, without the version
    /// rule.
    ///
    /// What `ginary otp list`, `fetch` and `path` read: those commands answer
    /// questions about a catalogue rather than build anything, and the release
    /// guard is the *build's* — refusing to tell somebody where a runtime is
    /// because their own Erlang is newer would be an answer to a question
    /// nobody asked.
    ///
    /// # Errors
    ///
    /// [`CatalogError::NoSuchVersion`], [`CatalogError::NoSuchTarget`],
    /// [`CatalogError::NoSuchVariant`] or [`CatalogError::AmbiguousVariant`],
    /// each naming what the catalogue does hold at that level.
    pub fn lookup(
        &self,
        version: &str,
        target: &str,
        variant: Option<&str>,
        origin: &str,
    ) -> Result<Selected<'_>, CatalogError> {
        let Some((version, otp)) = self.otp.get_key_value(version) else {
            return Err(CatalogError::NoSuchVersion {
                origin: origin.to_owned(),
                req: version.to_owned(),
                available: listed(self.otp.keys()),
            });
        };
        let Some((target, entry)) = otp.targets.get_key_value(target) else {
            return Err(CatalogError::NoSuchTarget {
                origin: origin.to_owned(),
                version: version.clone(),
                target: target.to_owned(),
                available: listed(otp.targets.keys()),
            });
        };

        let name = match variant {
            Some(name) => name.to_owned(),
            None => default_variant(version, target, entry)?,
        };
        let Some((name, chosen)) = entry.variants.get_key_value(&name) else {
            return Err(CatalogError::NoSuchVariant {
                version: version.clone(),
                target: target.clone(),
                variant: name,
                available: listed(entry.variants.keys()),
            });
        };

        Ok(Selected {
            version,
            target,
            variant: name,
            entry: chosen,
            otp,
            warnings: Vec::new(),
        })
    }

    /// Adds one runtime, creating the version and target levels as needed.
    pub fn insert(
        &mut self,
        version: &str,
        release: u32,
        erts_vsn: &str,
        target: &str,
        variant: &str,
        entry: Variant,
    ) {
        let version_entry = self
            .otp
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
    }

    /// The document as it is written to disk: two-space indent, sorted keys,
    /// one trailing newline.
    ///
    /// The empty string for a document that will not serialise, which is a
    /// shape these types cannot take; the repack's writer refuses to write an
    /// empty one rather than truncating a catalogue somebody was relying on.
    pub fn to_json(&self) -> String {
        match serde_json::to_string_pretty(self) {
            Ok(mut text) => {
                text.push('\n');
                text
            }
            Err(_) => String::new(),
        }
    }
}

/// The variant a target with no named one resolves to.
fn default_variant(
    version: &str,
    target: &str,
    entry: &TargetEntry,
) -> Result<String, CatalogError> {
    // A musl target has a documented default, because the static build runs on
    // any Linux and is the one a cross build wants unless somebody says
    // otherwise.
    if target.ends_with("-musl") && entry.variants.contains_key(DEFAULT_MUSL_VARIANT) {
        return Ok(DEFAULT_MUSL_VARIANT.to_owned());
    }
    let mut names = entry.variants.keys();
    match (names.next(), names.next()) {
        (Some(only), None) => Ok(only.clone()),
        _ => Err(CatalogError::AmbiguousVariant {
            version: version.to_owned(),
            target: target.to_owned(),
            available: listed(entry.variants.keys()),
        }),
    }
}

/// Names, comma separated, or `nothing at all` for an empty list.
fn listed<'a>(names: impl Iterator<Item = &'a String>) -> String {
    let listed: Vec<&str> = names.map(String::as_str).collect();
    if listed.is_empty() {
        "nothing at all".to_owned()
    } else {
        listed.join(", ")
    }
}

/// Compares two dotted numeric versions component by component.
///
/// String order puts `29.0.10` below `29.0.9`, which would make "newest patch"
/// answer the wrong entry exactly once per ten releases.
pub fn compare_versions(left: &str, right: &str) -> std::cmp::Ordering {
    let mut lefts = left.split('.');
    let mut rights = right.split('.');
    loop {
        match (lefts.next(), rights.next()) {
            (None, None) => return std::cmp::Ordering::Equal,
            // A version with fewer components is below one that carries them:
            // `29.0` is before `29.0.0`.
            (None, Some(_)) => return std::cmp::Ordering::Less,
            (Some(_), None) => return std::cmp::Ordering::Greater,
            (Some(left), Some(right)) => {
                let order = match (left.parse::<u64>(), right.parse::<u64>()) {
                    (Ok(left), Ok(right)) => left.cmp(&right),
                    // A component that is not a number is compared as text, so
                    // a pre-release suffix orders deterministically rather
                    // than not at all.
                    _ => left.cmp(right),
                };
                if order != std::cmp::Ordering::Equal {
                    return order;
                }
            }
        }
    }
}

/// The version guard, restated from C1 against a catalogue entry.
///
/// Older than the host is [`CatalogError::OtpTooOld`]; more than
/// [`RELEASE_WARN_AHEAD`] ahead is a warning; anything between is silence.
///
/// # Errors
///
/// [`CatalogError::OtpTooOld`].
pub fn check_release(
    version: &str,
    entry_release: u32,
    host_release: u32,
) -> Result<Option<String>, CatalogError> {
    if entry_release < host_release {
        return Err(CatalogError::OtpTooOld {
            version: version.to_owned(),
            entry_release,
            host_release,
        });
    }
    if entry_release > host_release.saturating_add(RELEASE_WARN_AHEAD) {
        return Ok(Some(format!(
            "the catalog's OTP {version} is release {entry_release} and this machine compiles \
             with release {host_release}; that is further ahead than ginary has tested"
        )));
    }
    Ok(None)
}

// ------------------------------------------------------- the cache --

/// The cache directory one entry extracts into, relative to the cache root.
///
/// `<version>-<target>` for the single unnamed variant,
/// `<version>-<target>-<variant>` for a named one, so a `static` and a
/// `dynamic` musl runtime of one version cannot land on top of one another.
pub fn entry_dir_name(version: &str, target: &str, variant: &str) -> String {
    if variant == DEFAULT_VARIANT {
        format!("{version}-{target}")
    } else {
        format!("{version}-{target}-{variant}")
    }
}

/// The `ginary otp fetch` line that fills one entry's cache directory.
///
/// Carries the flags the run that asked was given, because a remedy that
/// dropped them is a remedy that fails: without `--catalog` the fetch reads
/// the embedded catalogue, which is empty, and without `--variant` it fetches
/// whichever runtime the default rule picks, which may not be the one that was
/// asked about.
///
/// Every value is rendered through [`crate::process::shell_quote`], so a
/// catalogue under `~/My Documents` is one argument when the line is pasted
/// rather than two. A remedy is a command, and a command is quoted.
pub fn fetch_command(
    version: &str,
    target: &str,
    variant: Option<&str>,
    catalog: Option<&Path>,
) -> String {
    let version = shell_quote(version);
    let target = shell_quote(target);
    let mut line = format!("ginary otp fetch --version {version} --target {target}");
    if let Some(variant) = variant {
        line.push_str(&format!(" --variant {}", shell_quote(variant)));
    }
    if let Some(catalog) = catalog {
        line.push_str(&format!(" --catalog {}", shell_quote_path(catalog)));
    }
    line
}

/// The root of this module's cache: `<cache>/otp`.
pub fn cache_root(cache_dir: &Path) -> PathBuf {
    cache_dir.join(CACHE_SUBDIR)
}

/// Whether an extracted runtime is complete, which is `.meta.json` being there.
///
/// The same completion-marker discipline `src/cache.rs` uses: extraction
/// happens in a temporary sibling and is renamed into place, and the marker is
/// written last.
pub fn is_complete(dir: &Path) -> bool {
    dir.join(META_FILE).is_file()
}

/// What `.meta.json` records about an extracted runtime.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Meta {
    /// The OTP version.
    pub version: String,
    /// The target.
    pub target: String,
    /// The variant.
    pub variant: String,
    /// The catalogue entry this was extracted from, copied verbatim.
    pub entry: Variant,
    /// When it was extracted, RFC 3339 in UTC.
    pub extracted_at: String,
}

/// Installs a catalogue document at `destination`, atomically.
///
/// Written to `<destination>.tmp-<pid>` and renamed, the discipline
/// [`crate::download::fetch`] and [`ensure_otp`] already follow: a reader
/// either sees the whole document or sees the one that was there before, and
/// an install that fails partway through leaves neither a truncated file nor a
/// cache with no catalogue in it.
///
/// The bytes are written as they were given, never re-serialised: the digests
/// in a catalogue are what a build verifies against, and a round trip through
/// this ginary's writer would silently drop whatever a newer one had added.
///
/// # Errors
///
/// [`CatalogError::Io`] naming the file that could not be written.
pub fn install(text: &str, destination: &Path) -> Result<(), CatalogError> {
    let io = |path: &Path, error: std::io::Error| CatalogError::Io {
        path: path.to_path_buf(),
        message: error.to_string(),
    };
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent).map_err(|error| io(parent, error))?;
    }
    let mut name = destination.as_os_str().to_os_string();
    name.push(format!(".tmp-{}", std::process::id()));
    let tmp = PathBuf::from(name);
    std::fs::write(&tmp, text).map_err(|error| io(&tmp, error))?;
    std::fs::rename(&tmp, destination).map_err(|error| {
        let _ = std::fs::remove_file(&tmp);
        io(destination, error)
    })
}

/// Where a catalogue URL points.
///
/// A URL with a scheme is remote. Anything else is a path: absolute as
/// written, relative resolved against the directory the catalogue was read
/// from, which is what makes a committed `dist/otp/catalog.json` work from a
/// checkout with the tarballs beside it.
pub fn resolve_url(url: &str, catalog_dir: Option<&Path>) -> SourceUrl {
    if has_scheme(url) {
        return SourceUrl::Remote(url.to_owned());
    }
    let path = Path::new(url);
    match catalog_dir {
        Some(dir) if path.is_relative() => SourceUrl::File(dir.join(path)),
        _ => SourceUrl::File(path.to_path_buf()),
    }
}

/// Whether a URL begins with a scheme, `<alpha>[<alnum>+-.]*://`.
///
/// The single spelling of the question. `ginary otp update` asks it too, of a
/// `--catalog` value that is either a path or a URL, and a second spelling
/// there — `value.contains("://")` — made a path holding those three
/// characters into a network request.
pub fn has_scheme(url: &str) -> bool {
    let Some(end) = url.find("://") else {
        return false;
    };
    let scheme = &url[..end];
    !scheme.is_empty()
        && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

/// What [`ensure_otp`] needs besides the entry.
#[derive(Clone, Copy, Debug)]
pub struct EnsureContext<'a> {
    /// The root of this module's cache, [`cache_root`].
    pub cache_root: &'a Path,
    /// The directory a scheme-less URL resolves against.
    pub catalog_dir: Option<&'a Path>,
    /// Whether this build may fetch, and where the bases point.
    pub net: &'a Net,
    /// Where the phases are reported.
    pub diag: &'a Diag,
}

/// The extracted runtime for one entry, fetching and unpacking it if needed.
///
/// Complete means `.meta.json` is there. Otherwise the tarball is fetched into
/// the cache, verified against the entry's digest and length, extracted into a
/// temporary sibling with the payload extractor's rules — no absolute path, no
/// escaping path, no symlink at all, which the repack guarantees by
/// dereferencing them — and renamed into place with the marker written last.
///
/// The fill is held under an exclusive `flock` on
/// `<cache>/otp/.locks/<entry>/.lock`, so two builds racing for one runtime
/// produce one download and one extraction: the second waits, looks again and
/// finds the entry complete. The completion check is made twice for that
/// reason, once before the lock and once after it.
///
/// # Errors
///
/// [`CatalogError`]. Offline with nothing cached is
/// [`DownloadError::Offline`], carried through, so the message names the URL
/// and where the file would have gone.
pub fn ensure_otp(
    selected: &Selected<'_>,
    ctx: &EnsureContext<'_>,
) -> Result<PathBuf, CatalogError> {
    let dir = ctx.cache_root.join(selected.dir_name());
    if is_complete(&dir) {
        ctx.diag
            .kv("otp-cache", &[("hit", &dir.display().to_string())]);
        return Ok(dir);
    }
    let _phase = ctx.diag.phase("otp-ensure");

    // One filler at a time. Everything below removes and renames the entry
    // directory, so two processes that both saw it incomplete would delete one
    // another's work; the lock makes the second one wait and then find the
    // entry complete.
    let _fill = fill_lock(ctx.cache_root, &selected.dir_name(), ctx.diag)?;
    if is_complete(&dir) {
        ctx.diag.kv(
            "otp-cache",
            &[("filled-by-another", &dir.display().to_string())],
        );
        return Ok(dir);
    }

    let expected =
        download::parse_sha256(&selected.entry.sha256).ok_or_else(|| CatalogError::BadDigest {
            version: selected.version.to_owned(),
            target: selected.target.to_owned(),
            value: selected.entry.sha256.clone(),
        })?;
    let expect = Expect::exactly(expected, selected.entry.size);

    let source = resolve_url(&selected.entry.url, ctx.catalog_dir);
    // A fetched archive is a temporary: it is verified, unpacked and removed,
    // because the extracted tree is what every later build reads and thirteen
    // megabytes of tarball beside it would be a cache nobody empties. A local
    // one is read where it lies and never touched.
    let (archive, fetched) = match &source {
        SourceUrl::Remote(url) => {
            let into = ctx
                .cache_root
                .join(format!("{}.tar.zst", selected.dir_name()));
            ctx.diag.kv("otp-fetch", &[("url", url)]);
            download::fetch(url, &into, &expect, ctx.net)?;
            (into, true)
        }
        SourceUrl::File(path) => {
            verify_file(path, &expect)?;
            (path.clone(), false)
        }
    };

    let meta = Meta {
        version: selected.version.to_owned(),
        target: selected.target.to_owned(),
        variant: selected.variant.to_owned(),
        entry: selected.entry.clone(),
        extracted_at: timestamp(now_epoch()),
    };
    let extracted = extract_into_cache(&archive, &dir, &meta);
    if fetched {
        let _ = std::fs::remove_file(&archive);
    }
    extracted?;
    Ok(dir)
}

/// The extracted runtime for a user-supplied tarball, keyed by its SHA-256.
///
/// The same cache as [`ensure_otp`], the same rules and the same fill lock,
/// under `tarball-<sha256>`: two builds naming the same archive share one
/// extraction, and two different archives with the same file name do not.
///
/// # Errors
///
/// [`CatalogError`].
pub fn ensure_tarball(
    archive: &Path,
    cache_root: &Path,
    diag: &Diag,
) -> Result<PathBuf, CatalogError> {
    let (digest, size) = digest_file(archive)?;
    let name = tarball_dir_name(&digest);
    let dir = cache_root.join(&name);
    if is_complete(&dir) {
        diag.kv("otp-cache", &[("hit", &dir.display().to_string())]);
        return Ok(dir);
    }
    let _phase = diag.phase("otp-tarball");

    let _fill = fill_lock(cache_root, &name, diag)?;
    if is_complete(&dir) {
        diag.kv(
            "otp-cache",
            &[("filled-by-another", &dir.display().to_string())],
        );
        return Ok(dir);
    }

    let meta = Meta {
        version: TARBALL_ORIGIN.to_owned(),
        target: TARBALL_ORIGIN.to_owned(),
        variant: TARBALL_ORIGIN.to_owned(),
        entry: Variant::of_tarball(archive, &digest, size),
        extracted_at: timestamp(now_epoch()),
    };
    extract_into_cache(archive, &dir, &meta)?;
    Ok(dir)
}

/// The cache directory a tarball extracts into: `tarball-<sha256>`.
pub fn tarball_dir_name(sha256_hex: &str) -> String {
    format!("tarball-{sha256_hex}")
}

/// Takes the fill lock for one cache entry, waiting for whoever holds it.
///
/// `<cache_root>/.locks/<entry>/.lock`, so the lock outlives the removals and
/// renames the fill itself makes. The same `flock` discipline `src/cache.rs`
/// takes around the payload cache — an advisory lock the kernel releases when
/// the holder exits, however it exits — and the waiting form of it, because
/// what holds this one is another build doing the work this one was about to
/// do.
///
/// # Errors
///
/// [`CatalogError::Io`] naming the lock, which is what a caller sees when the
/// budget ran out with somebody else still filling the entry.
fn fill_lock(
    cache_root: &Path,
    entry: &str,
    diag: &Diag,
) -> Result<crate::cache_lock::ExclusiveLock, CatalogError> {
    let dir = cache_root.join(LOCK_SUBDIR).join(entry);
    std::fs::create_dir_all(&dir).map_err(|error| CatalogError::Io {
        path: dir.clone(),
        message: error.to_string(),
    })?;
    crate::cache_lock::wait_exclusive(&dir, crate::cache_lock::FILL_LOCK_BUDGET).map_err(|error| {
        diag.kv("otp-cache", &[("blocked", &dir.display().to_string())]);
        CatalogError::Io {
            path: crate::cache_lock::lock_path(&dir),
            message: error.to_string(),
        }
    })
}

/// Extracts `archive` into `dir`, marker last, through a temporary sibling.
///
/// An incomplete `dir` — a crashed extraction — is removed rather than merged
/// with: half of one runtime and half of another is a tree nothing could
/// explain.
fn extract_into_cache(archive: &Path, dir: &Path, meta: &Meta) -> Result<(), CatalogError> {
    if dir.exists() {
        std::fs::remove_dir_all(dir).map_err(|error| CatalogError::Io {
            path: dir.to_path_buf(),
            message: error.to_string(),
        })?;
    }
    let parent = dir.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent).map_err(|error| CatalogError::Io {
        path: parent.to_path_buf(),
        message: error.to_string(),
    })?;

    let mut name = dir.as_os_str().to_os_string();
    name.push(format!(".tmp-{}", std::process::id()));
    let tmp = PathBuf::from(name);
    if tmp.exists() {
        std::fs::remove_dir_all(&tmp).map_err(|error| CatalogError::Io {
            path: tmp.clone(),
            message: error.to_string(),
        })?;
    }

    let extracted = extract_runtime(archive, &tmp);
    if let Err(error) = extracted {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(error);
    }

    // The marker goes in before the rename, so the directory is complete the
    // instant it appears under its own name and no reader can see the halfway
    // state.
    let text = serde_json::to_string_pretty(meta).map_err(|error| CatalogError::Io {
        path: tmp.join(META_FILE),
        message: error.to_string(),
    })?;
    std::fs::write(tmp.join(META_FILE), text).map_err(|error| CatalogError::Io {
        path: tmp.join(META_FILE),
        message: error.to_string(),
    })?;

    std::fs::rename(&tmp, dir).map_err(|error| {
        let _ = std::fs::remove_dir_all(&tmp);
        CatalogError::Io {
            path: dir.to_path_buf(),
            message: error.to_string(),
        }
    })
}

/// Unpacks one runtime archive under `dest`, strictly.
///
/// The payload extractor's rules: every entry is a regular file or a
/// directory, every path stays under `dest`, and a symlink is refused outright.
/// The repack is what makes the last rule possible — it dereferences every
/// link — and refusing one here is what keeps a runtime tarball as safe to
/// unpack as an artifact's payload.
fn extract_runtime(archive: &Path, dest: &Path) -> Result<(), CatalogError> {
    let file = std::fs::File::open(archive).map_err(|error| CatalogError::Io {
        path: archive.to_path_buf(),
        message: error.to_string(),
    })?;
    let reader = std::io::BufReader::new(file);
    let reader = decompressed(archive, reader)?;

    std::fs::create_dir_all(dest).map_err(|error| CatalogError::Io {
        path: dest.to_path_buf(),
        message: error.to_string(),
    })?;

    let mut archive_reader = tar::Archive::new(reader);
    archive_reader.set_preserve_permissions(true);
    archive_reader.set_preserve_mtime(false);
    archive_reader.set_unpack_xattrs(false);
    archive_reader.set_overwrite(false);

    let bad = |message: String| CatalogError::Extract {
        path: archive.to_path_buf(),
        message,
    };

    let entries = archive_reader
        .entries()
        .map_err(|error| bad(error.to_string()))?;
    for entry in entries {
        let mut entry = entry.map_err(|error| bad(error.to_string()))?;
        let path = entry
            .path()
            .map_err(|error| bad(error.to_string()))?
            .into_owned();
        let name = path.display().to_string();

        match classify(&path) {
            Entry::Root => continue,
            Entry::Unsafe => return Err(bad(format!("`{name}` does not stay under the root"))),
            Entry::Normal => {}
        }
        let kind = entry.header().entry_type();
        if !(kind.is_file() || kind.is_dir()) {
            return Err(bad(format!(
                "`{name}` is a {kind:?} entry; a runtime tarball holds regular files and \
                 directories only"
            )));
        }
        if !entry
            .unpack_in(dest)
            .map_err(|error| bad(error.to_string()))?
        {
            return Err(bad(format!("`{name}` was refused by the extractor")));
        }
    }
    Ok(())
}

/// What one archive entry's path is.
enum Entry {
    /// The archive's own root, `.`, which names no file.
    Root,
    /// A path that leaves the destination.
    Unsafe,
    /// A path that stays under it.
    Normal,
}

/// Reads an entry path against the extraction rules.
fn classify(path: &Path) -> Entry {
    let mut normal = false;
    for component in path.components() {
        match component {
            Component::Normal(_) => normal = true,
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Entry::Unsafe;
            }
        }
    }
    if normal { Entry::Normal } else { Entry::Root }
}

/// Wraps `reader` in the decompressor the archive's magic bytes name.
fn decompressed<'a, R: Read + 'a>(
    archive: &Path,
    mut reader: R,
) -> Result<Box<dyn Read + 'a>, CatalogError> {
    let mut magic = [0_u8; 4];
    reader
        .read_exact(&mut magic)
        .map_err(|error| CatalogError::Extract {
            path: archive.to_path_buf(),
            message: format!("cannot read its first four bytes: {error}"),
        })?;
    let joined = std::io::Read::chain(std::io::Cursor::new(magic.to_vec()), reader);

    if magic == [0x28, 0xb5, 0x2f, 0xfd] {
        let decoder =
            zstd::stream::read::Decoder::new(joined).map_err(|error| CatalogError::Extract {
                path: archive.to_path_buf(),
                message: error.to_string(),
            })?;
        return Ok(Box::new(decoder));
    }
    if magic[..2] == [0x1f, 0x8b] {
        return Ok(Box::new(flate2::read::GzDecoder::new(joined)));
    }
    Err(CatalogError::Extract {
        path: archive.to_path_buf(),
        message: "it is neither a zstd nor a gzip stream".to_owned(),
    })
}

/// The SHA-256 and the length of a file on this machine.
fn digest_file(path: &Path) -> Result<(String, u64), CatalogError> {
    let file = std::fs::File::open(path).map_err(|error| CatalogError::Io {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    let mut reader = std::io::BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; COPY_BUFFER];
    let mut size = 0_u64;
    loop {
        let read = reader.read(&mut buffer).map_err(|error| CatalogError::Io {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        size = size.saturating_add(read as u64);
    }
    Ok((hex::encode(hasher.finalize()), size))
}

/// Holds a file already on this machine to the same digest and length a
/// fetched one is held to.
///
/// A catalogue whose URLs are file names beside it is not a weaker promise than
/// one whose URLs are hosted: the bytes are checked either way.
fn verify_file(path: &Path, expect: &Expect) -> Result<(), CatalogError> {
    let (digest, size) = digest_file(path)?;
    let url = path.display().to_string();
    if let Some(wanted) = expect.sha256 {
        let wanted = hex::encode(wanted);
        if digest != wanted {
            return Err(CatalogError::Download(DownloadError::ChecksumMismatch {
                url,
                expected: wanted,
                actual: digest,
            }));
        }
    }
    if let Some(wanted) = expect.size
        && size != wanted
    {
        return Err(CatalogError::Download(DownloadError::SizeMismatch {
            url,
            expected: wanted,
            actual: size,
        }));
    }
    Ok(())
}

// ------------------------------------------------------ the repack --

/// One `<target>[:<variant>]` the repack was asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepackSelector {
    /// The target name.
    pub target: String,
    /// The variant name, [`DEFAULT_VARIANT`] when none was written.
    pub variant: String,
}

impl RepackSelector {
    /// Reads `linux-x86_64-musl:static`, or `linux-x86_64-gnu` for the default
    /// variant.
    ///
    /// # Errors
    ///
    /// [`RepackError::BadSelector`] naming the value and what was wrong.
    pub fn parse(value: &str) -> Result<Self, RepackError> {
        let bad = |reason: &str| RepackError::BadSelector {
            value: value.to_owned(),
            reason: reason.to_owned(),
        };
        let mut halves = value.split(':');
        let target = halves.next().unwrap_or_default();
        let variant = halves.next();
        if halves.next().is_some() {
            return Err(bad("it holds more than one colon"));
        }
        if target.is_empty() {
            return Err(bad("it names no target"));
        }
        let variant = match variant {
            Some("") => return Err(bad("the colon names no variant")),
            Some(name) => name.to_owned(),
            None => DEFAULT_VARIANT.to_owned(),
        };
        Ok(Self {
            target: target.to_owned(),
            variant,
        })
    }

    /// `<target>:<variant>`, as it was written.
    pub fn label(&self) -> String {
        format!("{}:{}", self.target, self.variant)
    }
}

/// The upstream asset a target and variant are built from.
///
/// The mapping is the whole of what ginary knows about the upstream naming
/// scheme: `x64`/`arm64` for the architecture, no suffix for the fully static
/// musl build, `-glibc` for the dynamic glibc one and `-musl` for the dynamic
/// musl one.
///
/// # Errors
///
/// [`RepackError::NoUpstreamAsset`] for a combination upstream does not build.
pub fn upstream_asset(version: &str, target: &str, variant: &str) -> Result<String, RepackError> {
    let no_asset = || RepackError::NoUpstreamAsset {
        upstream: UPSTREAM_REPO,
        target: target.to_owned(),
        variant: variant.to_owned(),
    };
    let arch = match target {
        "linux-x86_64-musl" | "linux-x86_64-gnu" => "x64",
        "linux-aarch64-musl" | "linux-aarch64-gnu" => "arm64",
        _ => return Err(no_asset()),
    };
    let suffix = match (target.ends_with("-musl"), variant) {
        // The fully static musl build is the one upstream publishes without a
        // suffix; it is the only `static` variant there is.
        (true, "static") => "",
        (true, "dynamic") => "-musl",
        (false, DEFAULT_VARIANT) => "-glibc",
        _ => return Err(no_asset()),
    };
    Ok(format!("erlang-{version}-{arch}{suffix}.tar.gz"))
}

/// The URL a release asset is published at, without asking the API.
///
/// What an offline repack names when it cannot look one up, so that the
/// message says which file to go and fetch by hand.
pub fn asset_url(repo: &str, tag: &str, file: &str) -> String {
    format!("https://github.com/{repo}/releases/download/{tag}/{file}")
}

/// The release API's description of one tag.
fn release_api_url(repo: &str, tag: &str) -> String {
    format!(
        "{}/repos/{repo}/releases/tags/{tag}",
        download::GITHUB_API_BASE
    )
}

/// Whether a path inside a runtime root is pruned.
///
/// A path component in [`PRUNE_DIRS`], or a file name ending in one of
/// [`PRUNE_SUFFIXES`]. Components, not substrings: a file called `source.erl`
/// is not a `src` directory.
pub fn is_pruned(relative: &Path) -> bool {
    for component in relative.components() {
        if let Component::Normal(part) = component {
            let part = part.to_string_lossy();
            if PRUNE_DIRS.contains(&part.as_ref()) {
                return true;
            }
        }
    }
    relative
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .is_some_and(|name| PRUNE_SUFFIXES.iter().any(|suffix| name.ends_with(suffix)))
}

/// What a prune took off a runtime root.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PruneSummary {
    /// How many files were removed.
    pub removed_files: u64,
    /// How many bytes they were.
    pub removed_bytes: u64,
    /// Every removed path, relative to the root, sorted.
    pub paths: Vec<String>,
}

/// Removes everything [`is_pruned`] answers `true` for.
///
/// # Errors
///
/// [`RepackError::Io`] naming the file that could not be removed.
pub fn prune_tree(root: &Path) -> Result<PruneSummary, RepackError> {
    let mut summary = PruneSummary::default();
    let mut directories: Vec<PathBuf> = Vec::new();

    for entry in walk(root)? {
        let relative = entry.relative.clone();
        if !is_pruned(&relative) {
            continue;
        }
        if entry.is_dir {
            directories.push(entry.path);
            continue;
        }
        let bytes = std::fs::symlink_metadata(&entry.path)
            .map(|metadata| metadata.len())
            .unwrap_or_default();
        std::fs::remove_file(&entry.path).map_err(|error| RepackError::Io {
            path: entry.path.clone(),
            message: error.to_string(),
        })?;
        summary.removed_files = summary.removed_files.saturating_add(1);
        summary.removed_bytes = summary.removed_bytes.saturating_add(bytes);
        summary.paths.push(slash_path(&relative));
    }

    // The directories last and deepest first, so that a pruned directory is
    // empty by the time it is removed and one that holds something unexpected
    // fails loudly rather than taking it with it.
    directories.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    for path in directories {
        if path.exists() {
            std::fs::remove_dir_all(&path).map_err(|error| RepackError::Io {
                path: path.clone(),
                message: error.to_string(),
            })?;
        }
    }

    summary.paths.sort();
    Ok(summary)
}

/// What a dereference did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DerefSummary {
    /// Every link that was replaced by a copy, relative to the root, sorted.
    pub paths: Vec<String>,
    /// How many bytes the copies added.
    pub bytes_added: u64,
}

/// Replaces every symlink under `root` with a copy of what it points at.
///
/// The repacked tarball holds no symlink at all, which is what lets the
/// extractor keep the payload's strict rules — a runtime tarball is unpacked by
/// the same code that unpacks an artifact, and that code refuses a link.
///
/// # Errors
///
/// [`RepackError::DanglingSymlink`] for a link to nothing and
/// [`RepackError::UnsafeSymlink`] for one pointing outside `root`.
pub fn dereference_symlinks(root: &Path) -> Result<DerefSummary, RepackError> {
    let mut summary = DerefSummary::default();
    let anchor = root.canonicalize().map_err(|error| RepackError::Io {
        path: root.to_path_buf(),
        message: error.to_string(),
    })?;

    // Repeatedly, because a link may point at a link: each pass replaces every
    // link that resolves to a real file, and a pass that changes nothing while
    // links remain is a cycle, which `assert_no_symlinks` then reports.
    loop {
        let mut changed = false;
        for entry in walk(root)? {
            if !entry.is_symlink {
                continue;
            }
            let target = std::fs::read_link(&entry.path).map_err(|error| RepackError::Io {
                path: entry.path.clone(),
                message: error.to_string(),
            })?;
            let resolved = match entry.path.canonicalize() {
                Ok(resolved) => resolved,
                Err(_) => {
                    return Err(RepackError::DanglingSymlink {
                        path: entry.path.clone(),
                        target,
                    });
                }
            };
            if !resolved.starts_with(&anchor) {
                return Err(RepackError::UnsafeSymlink {
                    path: entry.path.clone(),
                    target: resolved,
                });
            }

            let added = copy_over_link(&entry.path, &resolved)?;
            summary.bytes_added = summary.bytes_added.saturating_add(added);
            summary.paths.push(slash_path(&entry.relative));
            changed = true;
        }
        if !changed {
            break;
        }
    }

    summary.paths.sort();
    summary.paths.dedup();
    Ok(summary)
}

/// Replaces the link at `link` with a copy of `resolved`, and answers what the
/// copy cost.
fn copy_over_link(link: &Path, resolved: &Path) -> Result<u64, RepackError> {
    std::fs::remove_file(link).map_err(|error| RepackError::Io {
        path: link.to_path_buf(),
        message: error.to_string(),
    })?;
    let metadata = std::fs::metadata(resolved).map_err(|error| RepackError::Io {
        path: resolved.to_path_buf(),
        message: error.to_string(),
    })?;
    if metadata.is_dir() {
        copy_tree(resolved, link)
    } else {
        std::fs::copy(resolved, link).map_err(|error| RepackError::Io {
            path: link.to_path_buf(),
            message: error.to_string(),
        })
    }
}

/// Copies a whole directory, which is what a link to one dereferences to.
fn copy_tree(from: &Path, to: &Path) -> Result<u64, RepackError> {
    std::fs::create_dir_all(to).map_err(|error| RepackError::Io {
        path: to.to_path_buf(),
        message: error.to_string(),
    })?;
    let mut bytes = 0_u64;
    for entry in walk(from)? {
        let destination = to.join(&entry.relative);
        if entry.is_dir {
            std::fs::create_dir_all(&destination).map_err(|error| RepackError::Io {
                path: destination.clone(),
                message: error.to_string(),
            })?;
        } else {
            if let Some(parent) = destination.parent() {
                std::fs::create_dir_all(parent).map_err(|error| RepackError::Io {
                    path: parent.to_path_buf(),
                    message: error.to_string(),
                })?;
            }
            bytes = bytes.saturating_add(std::fs::copy(&entry.path, &destination).map_err(
                |error| RepackError::Io {
                    path: destination.clone(),
                    message: error.to_string(),
                },
            )?);
        }
    }
    Ok(bytes)
}

/// Proves the dereference left nothing behind.
///
/// # Errors
///
/// [`RepackError::SymlinkRemains`] naming the first link found, in sorted
/// order, so the message is the same on every run.
pub fn assert_no_symlinks(root: &Path) -> Result<(), RepackError> {
    let mut links: Vec<PathBuf> = walk(root)?
        .into_iter()
        .filter(|entry| entry.is_symlink)
        .map(|entry| entry.path)
        .collect();
    links.sort();
    match links.into_iter().next() {
        Some(path) => Err(RepackError::SymlinkRemains { path }),
        None => Ok(()),
    }
}

/// One file, directory or link under a root.
struct Walked {
    /// Its absolute path.
    path: PathBuf,
    /// Its path relative to the root that was walked.
    relative: PathBuf,
    /// Whether it is a directory, following no links to decide.
    is_dir: bool,
    /// Whether it is a symlink.
    is_symlink: bool,
}

/// Every entry under `root`, sorted, following no links.
fn walk(root: &Path) -> Result<Vec<Walked>, RepackError> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir).map_err(|error| RepackError::Io {
            path: dir.clone(),
            message: error.to_string(),
        })?;
        for entry in entries {
            let entry = entry.map_err(|error| RepackError::Io {
                path: dir.clone(),
                message: error.to_string(),
            })?;
            let path = entry.path();
            let metadata = std::fs::symlink_metadata(&path).map_err(|error| RepackError::Io {
                path: path.clone(),
                message: error.to_string(),
            })?;
            let is_symlink = metadata.file_type().is_symlink();
            let is_dir = metadata.is_dir();
            let relative = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
            found.push(Walked {
                path: path.clone(),
                relative,
                is_dir,
                is_symlink,
            });
            if is_dir && !is_symlink {
                stack.push(path);
            }
        }
    }
    found.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(found)
}

/// A relative path with `/` separators, which is what a report prints.
fn slash_path(relative: &Path) -> String {
    relative
        .components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// An RFC 3339 UTC timestamp, from `SOURCE_DATE_EPOCH` when it is set.
///
/// Two repacks of one upstream asset under one `SOURCE_DATE_EPOCH` produce the
/// same catalogue bytes, which is the same reproducibility rule the payload
/// follows.
pub fn timestamp(epoch_seconds: u64) -> String {
    let days = i64::try_from(epoch_seconds / 86_400).unwrap_or(i64::MAX);
    let seconds = epoch_seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        seconds / 3600,
        (seconds / 60) % 60,
        seconds % 60
    )
}

/// The calendar date `days` after 1970-01-01, by Howard Hinnant's algorithm.
fn civil_from_days(days: i64) -> (i64, u64, u64) {
    let shifted = days + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = (shifted - era * 146_097) as u64;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era as i64 + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    };
    (if month <= 2 { year + 1 } else { year }, month, day)
}

/// The current time in seconds since the epoch.
fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default()
}

/// What one `ginary otp repack` was asked to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepackOptions {
    /// The upstream release tag, `OTP-29.0.5`.
    pub upstream_tag: String,
    /// The targets and variants to build.
    pub selectors: Vec<RepackSelector>,
    /// Where the tarballs and `catalog.json` go.
    pub out: PathBuf,
    /// A directory of already-downloaded upstream assets, for an offline run.
    pub upstream_dir: Option<PathBuf>,
    /// `SOURCE_DATE_EPOCH`, or the current time when absent.
    pub source_date_epoch: Option<u64>,
}

/// What the module strip did to one repacked runtime.
///
/// Upstream ships its `.beam` files with their `Dbgi` chunk, which is about
/// four fifths of what a module weighs and is of no use to a packaged
/// application: `ginary build` strips the modules it stages anyway, so the
/// only thing carrying them through a catalog achieves is a larger download
/// for everybody. They are removed here instead, once, by the *host's* Erlang
/// — a `.beam` is portable and an emulator is not, so the host can strip a
/// tree built for another machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BeamStrip {
    /// No module carried debug information; nothing was run.
    NothingToStrip,
    /// The modules were stripped.
    Stripped {
        /// How many modules were rewritten.
        files: u64,
        /// What they weighed before.
        before: u64,
        /// What they weigh now.
        after: u64,
    },
    /// It did not happen, and this is why.
    ///
    /// A reported decision, never a silent one: a catalog entry that carries
    /// debug information is bigger than it says it needs to be, and the person
    /// who runs the pipeline is the one who can do something about it.
    Skipped {
        /// One line saying why.
        reason: String,
    },
}

impl BeamStrip {
    /// One line for the pipeline's report.
    pub fn describe(&self) -> String {
        match self {
            Self::NothingToStrip => "no module carried debug information".to_owned(),
            Self::Stripped {
                files,
                before,
                after,
            } => format!("stripped {files} modules, {before} bytes to {after}",),
            Self::Skipped { reason } => format!("modules not stripped: {reason}"),
        }
    }
}

/// What one repacked runtime cost and produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepackOutcome {
    /// The target.
    pub target: String,
    /// The variant.
    pub variant: String,
    /// The upstream asset it came from.
    pub upstream_file: String,
    /// The bytes the upstream asset unpacked to.
    pub unpacked_bytes: u64,
    /// What the prune took off.
    pub prune: PruneSummary,
    /// What the dereference did.
    pub deref: DerefSummary,
    /// What stripping the modules did, or why it did not happen.
    pub beam_strip: BeamStrip,
    /// The tarball that was written.
    pub tarball: PathBuf,
    /// Its length.
    pub tarball_bytes: u64,
    /// The OTP release the repacked tree itself reports.
    pub entry_release: u32,
    /// The ERTS version the repacked tree itself reports.
    pub erts_vsn: String,
    /// The catalogue entry that was appended.
    pub entry: Variant,
}

/// Everything one run of the pipeline produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepackReport {
    /// One per requested target and variant, in the order they were asked for.
    pub outcomes: Vec<RepackOutcome>,
    /// The catalogue that was written.
    pub catalog: PathBuf,
}

/// The local repackaging pipeline: upstream asset to `.tar.zst` plus a
/// catalogue entry.
///
/// Nothing is published and nothing is pushed. What comes out is a directory of
/// tarballs and a `catalog.json` beside them whose URLs are file names relative
/// to itself, which [`resolve_url`] resolves against the catalogue's own
/// directory.
///
/// # Errors
///
/// [`RepackError`]. A target whose emulator disagrees with the target it was
/// requested as is refused here rather than shipped, because a mislabelled
/// upstream asset would otherwise become a mislabelled catalogue entry.
pub fn repack(
    options: &RepackOptions,
    net: &Net,
    diag: &Diag,
) -> Result<RepackReport, RepackError> {
    repack_with(options, net, diag, crate::elf::inspect)
}

/// The OTP version an upstream tag names: `OTP-29.0.5` is `29.0.5`.
///
/// Returns [`None`] for a tag that is not one of those, because a repack that
/// guessed would write a catalogue keyed by a version nothing else agrees with.
pub fn version_from_tag(tag: &str) -> Option<String> {
    let version = tag.strip_prefix("OTP-")?;
    if version.is_empty() || !version.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    Some(version.to_owned())
}

/// [`repack`], with the emulator inspection injected.
///
/// The same seam [`crate::erts_source::resolve_with`] has, and for the same
/// reason: the pipeline's own check that an upstream asset really is for the
/// target it was requested as reads a `beam.smp`, and a fixture upstream tree
/// carries a shell script there. Everything above the ELF reader — the asset
/// mapping, the prune, the dereference, the packing, the catalogue entry — is
/// reachable without a 42 MB download.
///
/// # Errors
///
/// As [`repack`].
pub fn repack_with(
    options: &RepackOptions,
    net: &Net,
    diag: &Diag,
    inspect: impl Fn(&Path) -> Result<ElfInfo, ElfError>,
) -> Result<RepackReport, RepackError> {
    let version = version_from_tag(&options.upstream_tag).ok_or_else(|| RepackError::BadTag {
        tag: options.upstream_tag.clone(),
    })?;
    let stamp = timestamp(options.source_date_epoch.unwrap_or_else(now_epoch));

    let work = tempfile::tempdir().map_err(|error| RepackError::Io {
        path: std::env::temp_dir(),
        message: error.to_string(),
    })?;

    let mut outcomes = Vec::with_capacity(options.selectors.len());
    for selector in &options.selectors {
        let _phase = diag.phase("repack");
        diag.kv("repack", &[("target", &selector.label())]);
        outcomes.push(repack_one(
            options,
            selector,
            &version,
            &stamp,
            work.path(),
            net,
            diag,
            &inspect,
        )?);
    }

    let catalog_path = options.out.join(CATALOG_FILE);
    let mut catalog = read_or_new(&catalog_path, &stamp)?;
    catalog.generated_at = stamp.clone();
    for outcome in &outcomes {
        catalog.insert(
            &version,
            outcome.entry_release,
            &outcome.erts_vsn,
            &outcome.target,
            &outcome.variant,
            outcome.entry.clone(),
        );
    }
    write_catalog(&catalog_path, &catalog)?;

    Ok(RepackReport {
        outcomes,
        catalog: catalog_path,
    })
}

/// One target and variant, from the upstream asset to the catalogue entry.
///
/// The order is the point: the asset is verified, unpacked, pruned and
/// dereferenced, and only then is its emulator read and held to the target it
/// was requested as. A mismatch stops the target before a single byte of it is
/// written into the output directory.
#[allow(clippy::too_many_arguments)]
fn repack_one(
    options: &RepackOptions,
    selector: &RepackSelector,
    version: &str,
    stamp: &str,
    work: &Path,
    net: &Net,
    diag: &Diag,
    inspect: &impl Fn(&Path) -> Result<ElfInfo, ElfError>,
) -> Result<RepackOutcome, RepackError> {
    let file = upstream_asset(version, &selector.target, &selector.variant)?;
    let (asset, upstream_sha) = upstream_bytes(options, &file, net, diag)?;

    let unpacked = work.join(format!("{}-{}", selector.target, selector.variant));
    if unpacked.exists() {
        std::fs::remove_dir_all(&unpacked).map_err(|error| RepackError::Io {
            path: unpacked.clone(),
            message: error.to_string(),
        })?;
    }
    unpack_upstream(&asset, &unpacked)?;
    let root = single_root(&unpacked)?;
    let unpacked_bytes = tree_bytes(&root)?;

    let prune = prune_tree(&root)?;
    let deref = dereference_symlinks(&root)?;
    assert_no_symlinks(&root)?;

    let otp = crate::otp::inspect_root(&root).map_err(|error| RepackError::Io {
        path: root.clone(),
        message: error.to_string(),
    })?;
    let emulator = otp.erts_bin.join(crate::erts_source::EMULATOR);
    let info = inspect(&emulator).map_err(|error| RepackError::Io {
        path: emulator.clone(),
        message: error.to_string(),
    })?;

    let requested: Target =
        selector
            .target
            .parse()
            .map_err(
                |error: crate::target::ParseTargetError| RepackError::BadSelector {
                    value: selector.label(),
                    reason: error.to_string(),
                },
            )?;
    let Some(elf) = Target::from_elf(&info.machine, info.interp.as_deref()) else {
        return Err(RepackError::UpstreamMismatch {
            file: file.clone(),
            target: selector.target.clone(),
            actual: format!("machine `{}`, which ginary has no target for", info.machine),
        });
    };
    let actual = elf.resolve(requested.libc);
    if actual != requested {
        return Err(RepackError::UpstreamMismatch {
            file,
            target: selector.target.clone(),
            actual: actual.name(),
        });
    }
    let linkage = if elf.linkage() == Linkage::Static && !info.needed.is_empty() {
        Linkage::Dynamic
    } else {
        elf.linkage()
    };
    let claimed = claimed_linkage(&selector.variant);
    if linkage != claimed {
        return Err(RepackError::UpstreamLinkage {
            file,
            variant: selector.variant.clone(),
            claimed: claimed.as_str(),
            actual: linkage.as_str(),
        });
    }

    let beam_strip = strip_beams(&root, diag)?;
    let name = format!(
        "otp-{version}-{}-{}.tar.zst",
        selector.target, selector.variant
    );
    let tarball = options.out.join(&name);
    let tarball_bytes = pack_runtime(&root, &tarball)?;
    let (sha256, _) = digest_file(&tarball).map_err(|error| RepackError::Io {
        path: tarball.clone(),
        message: error.to_string(),
    })?;

    let entry = Variant {
        url: name,
        sha256,
        size: tarball_bytes,
        linkage: linkage.as_str().to_owned(),
        nif_loading: linkage.loads_nifs(),
        libc: libc_spec(linkage, elf.target().map(|target| target.libc), &info),
        openssl: openssl_version(&root, &emulator),
        jit: has_jit(&emulator),
        excluded_apps: Vec::new(),
        upstream: Upstream {
            repo: UPSTREAM_REPO.to_owned(),
            tag: options.upstream_tag.clone(),
            file: file.clone(),
            sha256: upstream_sha,
        },
        built_at: stamp.to_owned(),
        extra: BTreeMap::new(),
    };

    Ok(RepackOutcome {
        target: selector.target.clone(),
        variant: selector.variant.clone(),
        upstream_file: file,
        unpacked_bytes,
        prune,
        deref,
        beam_strip,
        tarball,
        tarball_bytes,
        entry_release: otp.release,
        erts_vsn: otp.erts_vsn,
        entry,
    })
}

/// The upstream asset on this machine, and the digest of the bytes it holds.
///
/// A file in `--upstream-dir` is used as it is, and its digest is computed from
/// the bytes rather than taken from anybody's word. Otherwise the release API
/// is asked for the asset's `digest` and download URL, and the fetch is held to
/// both.
fn upstream_bytes(
    options: &RepackOptions,
    file: &str,
    net: &Net,
    diag: &Diag,
) -> Result<(PathBuf, String), RepackError> {
    let local_dir = options
        .upstream_dir
        .clone()
        .unwrap_or_else(|| options.out.join(".upstream"));
    let local = local_dir.join(file);
    if local.is_file() {
        let (sha256, _) = digest_file(&local).map_err(|error| RepackError::Io {
            path: local.clone(),
            message: error.to_string(),
        })?;
        diag.kv("upstream", &[("local", &local.display().to_string())]);
        return Ok((local, sha256));
    }

    if net.offline {
        return Err(RepackError::Download {
            reason: DownloadError::Offline {
                url: asset_url(UPSTREAM_REPO, &options.upstream_tag, file),
                dest_hint: local,
            },
        });
    }

    let asset = release_asset(&options.upstream_tag, file, net)?;
    let expect = Expect {
        sha256: download::parse_sha256(&asset.sha256),
        size: asset.size,
    };
    if expect.sha256.is_none() {
        return Err(RepackError::Api {
            url: release_api_url(UPSTREAM_REPO, &options.upstream_tag),
            message: format!(
                "asset `{file}` has digest `{}`, which is not a sha256",
                asset.sha256
            ),
        });
    }
    diag.kv("upstream", &[("fetch", &asset.url)]);
    download::fetch(&asset.url, &local, &expect, net)?;
    Ok((local, asset.sha256))
}

/// What the release API says about one asset.
struct ReleaseAsset {
    /// Where to fetch it.
    url: String,
    /// Its SHA-256, without the `sha256:` prefix the API writes.
    sha256: String,
    /// Its length, when the API reported one.
    size: Option<u64>,
}

/// Asks the release API for one asset of one tag.
fn release_asset(tag: &str, file: &str, net: &Net) -> Result<ReleaseAsset, RepackError> {
    let url = release_api_url(UPSTREAM_REPO, tag);
    let body = download::get_text(&url, net)?;
    let value: serde_json::Value =
        serde_json::from_str(&body).map_err(|error| RepackError::Api {
            url: url.clone(),
            message: error.to_string(),
        })?;
    let assets = value
        .get("assets")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| RepackError::Api {
            url: url.clone(),
            message: "it names no assets".to_owned(),
        })?;
    let asset = assets
        .iter()
        .find(|asset| asset.get("name").and_then(serde_json::Value::as_str) == Some(file))
        .ok_or_else(|| RepackError::Api {
            url: url.clone(),
            message: format!("release `{tag}` has no asset called `{file}`"),
        })?;

    let download_url = asset
        .get("browser_download_url")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| RepackError::Api {
            url: url.clone(),
            message: format!("asset `{file}` has no download URL"),
        })?;
    let digest = asset
        .get("digest")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| RepackError::Api {
            url: url.clone(),
            message: format!(
                "asset `{file}` carries no digest; a repack pins what it fetched and cannot \
                 pin nothing"
            ),
        })?;

    Ok(ReleaseAsset {
        url: download_url.to_owned(),
        sha256: digest.strip_prefix("sha256:").unwrap_or(digest).to_owned(),
        size: asset.get("size").and_then(serde_json::Value::as_u64),
    })
}

/// Unpacks an upstream release asset, links and all.
///
/// Deliberately not [`extract_runtime`]: upstream ships symlinks, and this is
/// the tree the dereference is about to remove them from. Paths are still held
/// to the destination by the tar reader's own `unpack_in`.
fn unpack_upstream(archive: &Path, dest: &Path) -> Result<(), RepackError> {
    let file = std::fs::File::open(archive).map_err(|error| RepackError::Io {
        path: archive.to_path_buf(),
        message: error.to_string(),
    })?;
    let reader = flate2::read::GzDecoder::new(std::io::BufReader::new(file));
    std::fs::create_dir_all(dest).map_err(|error| RepackError::Io {
        path: dest.to_path_buf(),
        message: error.to_string(),
    })?;
    let mut tar = tar::Archive::new(reader);
    tar.set_preserve_permissions(true);
    tar.set_preserve_mtime(false);
    tar.set_unpack_xattrs(false);
    tar.unpack(dest).map_err(|error| RepackError::Io {
        path: archive.to_path_buf(),
        message: error.to_string(),
    })
}

/// The runtime root inside an unpacked upstream asset.
///
/// Upstream wraps the tree in one directory; a future asset that does not is
/// handled by answering the extraction directory itself, so the shape is read
/// rather than assumed.
fn single_root(unpacked: &Path) -> Result<PathBuf, RepackError> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(unpacked)
        .map_err(|error| RepackError::Io {
            path: unpacked.to_path_buf(),
            message: error.to_string(),
        })?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect();
    entries.sort();
    match entries.as_slice() {
        [only] if only.is_dir() => Ok(only.clone()),
        _ => Ok(unpacked.to_path_buf()),
    }
}

/// How many bytes a tree holds, following no links.
fn tree_bytes(root: &Path) -> Result<u64, RepackError> {
    let mut bytes = 0_u64;
    for entry in walk(root)? {
        if !entry.is_dir && !entry.is_symlink {
            let len = std::fs::symlink_metadata(&entry.path)
                .map(|metadata| metadata.len())
                .unwrap_or_default();
            bytes = bytes.saturating_add(len);
        }
    }
    Ok(bytes)
}

/// How many `.beam` modules under `root` still carry debug information.
fn beams_with_debug_info(root: &Path) -> Result<u64, RepackError> {
    let mut found = 0_u64;
    for entry in walk(root)? {
        if entry.is_dir || entry.path.extension().is_none_or(|ext| ext != "beam") {
            continue;
        }
        let Ok(bytes) = std::fs::read(&entry.path) else {
            continue;
        };
        if let Ok(chunks) = crate::beam::chunks(&bytes)
            && chunks
                .iter()
                .any(|chunk| chunk.id == crate::beam::DEBUG_INFO_CHUNK)
        {
            found = found.saturating_add(1);
        }
    }
    Ok(found)
}

/// Removes the debug information from every module under `root`.
///
/// Through the same code a build strips a staged tree with, so the modules are
/// verified afterwards by the same rules — no `Dbgi`, no `Docs`, and a `Code`
/// chunk still there. The Erlang that does it is the *host's*: a `.beam` is
/// portable, so an aarch64 tree's modules are stripped here as readily as an
/// x86-64 tree's, and the emulator inside the tree is never run.
///
/// # Errors
///
/// Never for a missing Erlang, which is a reported [`BeamStrip::Skipped`]; the
/// strip itself failing is [`RepackError::Io`] naming the tree, because a tree
/// whose modules were half-rewritten is not one to publish.
fn strip_beams(root: &Path, diag: &Diag) -> Result<BeamStrip, RepackError> {
    let carrying = beams_with_debug_info(root)?;
    if carrying == 0 {
        return Ok(BeamStrip::NothingToStrip);
    }
    let host = match crate::otp::discover(None) {
        Ok(host) => host,
        Err(error) => {
            return Ok(BeamStrip::Skipped {
                reason: format!(
                    "{carrying} modules carry debug information and this machine has no usable \
                     Erlang to remove it with: {error}"
                ),
            });
        }
    };
    diag.kv(
        "repack-strip",
        &[
            ("modules", &carrying.to_string()),
            ("erl", &host.root.display().to_string()),
        ],
    );

    let report = crate::strip::strip(
        root,
        &host,
        &crate::strip::StripOptions {
            elf: false,
            beams: true,
        },
    )
    .map_err(|error| RepackError::Io {
        path: root.to_path_buf(),
        message: error.to_string(),
    })?;

    Ok(match report.beams {
        crate::strip::BeamOutcome::Stripped {
            files,
            before,
            after,
        } => BeamStrip::Stripped {
            files: files as u64,
            before,
            after,
        },
        crate::strip::BeamOutcome::Skipped { reason } => BeamStrip::Skipped { reason },
        crate::strip::BeamOutcome::Disabled => BeamStrip::Skipped {
            reason: "the strip was not asked for".to_owned(),
        },
    })
}

/// The libc a repacked runtime needs, as the emulator describes it.
fn libc_spec(linkage: Linkage, libc: Option<Libc>, info: &ElfInfo) -> LibcSpec {
    match (linkage, libc) {
        // A fully static emulator resolves nothing at load time, whatever it
        // was built against.
        (Linkage::Static, _) | (_, None) => LibcSpec {
            kind: "none".to_owned(),
            version: None,
            min: None,
        },
        (_, Some(Libc::Gnu)) => LibcSpec {
            kind: "gnu".to_owned(),
            version: None,
            min: info.glibc_max.clone(),
        },
        (_, Some(Libc::Musl)) => LibcSpec {
            kind: "musl".to_owned(),
            version: None,
            min: None,
        },
        (_, Some(Libc::None)) => LibcSpec {
            kind: "none".to_owned(),
            version: None,
            min: None,
        },
    }
}

/// The OpenSSL banner a statically linked runtime carries in its own bytes.
///
/// Read rather than assumed, and empty when there is nothing to read: a
/// dynamically linked `crypto` resolves its OpenSSL on the machine that runs
/// the artifact, where this build cannot see it.
fn openssl_version(root: &Path, emulator: &Path) -> String {
    let mut candidates = vec![emulator.to_path_buf()];
    if let Ok(entries) = walk(&root.join("lib")) {
        candidates.extend(
            entries
                .into_iter()
                .filter(|entry| {
                    entry
                        .path
                        .file_name()
                        .is_some_and(|name| name == "crypto.so")
                })
                .map(|entry| entry.path),
        );
    }
    for candidate in candidates {
        let Ok(bytes) = std::fs::read(&candidate) else {
            continue;
        };
        if let Some(version) = openssl_banner(&bytes) {
            return version;
        }
    }
    String::new()
}

/// The version out of an `OpenSSL <version> <date>` banner in `bytes`.
fn openssl_banner(bytes: &[u8]) -> Option<String> {
    let needle = b"OpenSSL ";
    let mut from = 0_usize;
    while let Some(at) = find(&bytes[from..], needle) {
        let start = from + at + needle.len();
        let version: String = bytes[start..]
            .iter()
            .take_while(|byte| byte.is_ascii_digit() || **byte == b'.')
            .map(|byte| *byte as char)
            .collect();
        if version.split('.').count() >= 3 {
            return Some(version);
        }
        from = start;
    }
    None
}

/// Whether the emulator carries the just-in-time compiler.
///
/// BeamAsm names itself in the emulator's own bytes; an interpreter-only build
/// does not. Read rather than assumed, because `jit` is a claim the catalogue
/// makes and a claim nobody checked is a claim nobody should print.
fn has_jit(emulator: &Path) -> bool {
    std::fs::read(emulator).is_ok_and(|bytes| find(&bytes, b"beamasm").is_some())
}

/// The first offset of `needle` in `haystack`.
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// Packs a runtime root deterministically and answers the tarball's length.
///
/// Sorted paths, zeroed mtime, uid and gid, and one zstd stream, which is what
/// makes two repacks of one upstream asset byte-identical.
fn pack_runtime(root: &Path, tarball: &Path) -> Result<u64, RepackError> {
    if let Some(parent) = tarball.parent() {
        std::fs::create_dir_all(parent).map_err(|error| RepackError::Io {
            path: parent.to_path_buf(),
            message: error.to_string(),
        })?;
    }
    let file = std::fs::File::create(tarball).map_err(|error| RepackError::Io {
        path: tarball.to_path_buf(),
        message: error.to_string(),
    })?;
    let io = |error: std::io::Error| RepackError::Io {
        path: tarball.to_path_buf(),
        message: error.to_string(),
    };

    let encoder = zstd::stream::write::Encoder::new(file, REPACK_LEVEL).map_err(io)?;
    let mut builder = tar::Builder::new(encoder);
    for entry in walk(root)? {
        let name = slash_path(&entry.relative);
        let metadata = std::fs::symlink_metadata(&entry.path).map_err(|error| RepackError::Io {
            path: entry.path.clone(),
            message: error.to_string(),
        })?;
        // `HeaderMode::Deterministic` zeroes the owner and reduces the mode,
        // and it does *not* zero the mtime: tar-rs writes a fixed non-zero
        // timestamp there to work around tools that mishandle a zero one. The
        // payload writes headers the same way and for the same reason; see
        // `src/payload.rs`.
        let mut header = tar::Header::new_gnu();
        header.set_metadata_in_mode(&metadata, tar::HeaderMode::Deterministic);
        header.set_mtime(0);
        if entry.is_dir {
            header.set_size(0);
            builder
                .append_data(&mut header, Path::new(&name), std::io::empty())
                .map_err(io)?;
        } else {
            let file = std::fs::File::open(&entry.path).map_err(|error| RepackError::Io {
                path: entry.path.clone(),
                message: error.to_string(),
            })?;
            builder
                .append_data(&mut header, Path::new(&name), file)
                .map_err(io)?;
        }
    }
    builder
        .into_inner()
        .map_err(io)?
        .finish()
        .map_err(io)?
        .sync_all()
        .map_err(io)?;

    std::fs::metadata(tarball)
        .map(|metadata| metadata.len())
        .map_err(io)
}

/// The catalogue already in the output directory, or a new one.
fn read_or_new(path: &Path, stamp: &str) -> Result<Catalog, RepackError> {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            Catalog::parse(&text, &path.display().to_string()).map_err(|error| RepackError::Io {
                path: path.to_path_buf(),
                message: error.to_string(),
            })
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Catalog::empty(stamp)),
        Err(error) => Err(RepackError::Io {
            path: path.to_path_buf(),
            message: error.to_string(),
        }),
    }
}

/// Writes a catalogue, refusing to write one that would not serialise.
fn write_catalog(path: &Path, catalog: &Catalog) -> Result<(), RepackError> {
    let text = catalog.to_json();
    if text.is_empty() {
        return Err(RepackError::Io {
            path: path.to_path_buf(),
            message: "the catalog did not serialise".to_owned(),
        });
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| RepackError::Io {
            path: parent.to_path_buf(),
            message: error.to_string(),
        })?;
    }
    std::fs::write(path, text).map_err(|error| RepackError::Io {
        path: path.to_path_buf(),
        message: error.to_string(),
    })
}

/// The linkage a variant name implies, checked against the emulator later.
///
/// `static` is a fully static build that cannot `dlopen` a NIF; every other
/// variant is dynamic.
pub fn claimed_linkage(variant: &str) -> Linkage {
    if variant == "static" {
        Linkage::Static
    } else {
        Linkage::Dynamic
    }
}
