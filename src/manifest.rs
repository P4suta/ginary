// SPDX-License-Identifier: MIT OR Apache-2.0
//! `ginary.json` and `ginary.index.json`, the two files that describe an
//! artifact from the inside.
//!
//! [`Manifest`] is the first entry of the payload and the only thing the
//! launcher needs in order to start the application: what the runtime is,
//! which applications are on the code path, and the exact argument vector.
//! [`Index`] is the second entry and describes every other one, so that
//! `ginary verify` can check an artifact without extracting it.
//!
//! Three rules shape the module.
//!
//! **Every path is relative to the extracted root and uses `/`.** The launcher
//! joins them with the native separator, and [`LaunchSpec::validate`] refuses
//! an absolute path, a `..` component and a backslash before one is ever
//! written into a manifest.
//!
//! **Unknown keys survive.** A key this build does not know lands in
//! [`Manifest::extra`] and is written back out, so a newer manifest that has
//! not changed its `format_version` still round-trips through an older
//! launcher.
//!
//! **Nothing here reads a clock.** [`created_at`] is a pure function of an
//! environment snapshot and a second count, so a caller that honours
//! `SOURCE_DATE_EPOCH` and a test that pins a date use the same code path.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::assemble::{Category, StagedFile};
use crate::target::Target;

/// The manifest format version this build writes and reads.
pub const FORMAT_VERSION: u32 = 1;

/// The name of the manifest, the payload's first entry.
pub const MANIFEST_NAME: &str = "ginary.json";

/// The name of the index, the payload's second entry.
pub const INDEX_NAME: &str = "ginary.index.json";

/// The variables manifest creation reads.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EnvSnapshot {
    /// Value of `SOURCE_DATE_EPOCH`, the reproducible-builds clock override.
    pub source_date_epoch: Option<OsString>,
}

impl EnvSnapshot {
    /// Reads the relevant variables from the current process environment.
    pub fn from_env() -> Self {
        Self {
            source_date_epoch: std::env::var_os("SOURCE_DATE_EPOCH"),
        }
    }
}

/// An application in the artifact, with the version its `.app` file gives.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppRef {
    /// The application name.
    pub name: String,
    /// The version from its `.app` file.
    pub vsn: String,
}

/// What kind of object file a native artifact is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeKind {
    /// An ELF object, the Linux and BSD shape.
    Elf,
    /// A Mach-O object, the macOS shape.
    Macho,
    /// A PE object, the Windows shape.
    Pe,
}

/// A native object in the artifact, for `ginary verify` to re-inspect.
///
/// The four fields after `kind` are what C4 added, and every one of them is
/// what the build *ended up* shipping rather than what the shipment held: an
/// artifact replaced by an override or a build hook records the machine of the
/// file that replaced it, so `ginary verify` can hold the manifest to the
/// bytes in the payload. They carry serde defaults, so an artifact built
/// before C4 still reads back at `format_version` 1.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeRef {
    /// The path relative to the extracted root, `/`-separated.
    pub path: String,
    /// What kind of object file it is.
    pub kind: NativeKind,
    /// The machine, as [`crate::elf::ElfInfo::machine`] spells it.
    ///
    /// [`None`] for an object whose header would not parse, which is listed
    /// rather than dropped: an artifact carrying one is one `ginary verify`
    /// has something to say about.
    #[serde(default)]
    pub machine: Option<String>,
    /// The target the object names, when its header names a whole one.
    #[serde(default)]
    pub target: Option<Target>,
    /// Whether the build replaced the shipment's own file.
    #[serde(default)]
    pub replaced: bool,
    /// What replaced it: `override`, `hook`, or [`None`] for neither.
    #[serde(default)]
    pub source: Option<String>,
}

/// The C library a runtime needs, and the lowest release it runs against.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibcRequirement {
    /// `gnu` or `musl`.
    pub kind: String,
    /// The lowest release the emulator runs against, `2.31` rather than
    /// `GLIBC_2.31`. [`None`] for musl, which carries no symbol versions.
    #[serde(default)]
    pub min: Option<String>,
}

/// What the bundled runtime is, and where it came from.
///
/// Recorded by the build from [`crate::erts_source::ResolvedErts`], which
/// derives every field from the emulator itself. It is additive: an artifact
/// built before C1 carries no `otp` block and deserialises to
/// [`OtpProvenance::default`], which says `unknown` rather than inventing a
/// linkage nobody read.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OtpProvenance {
    /// `dynamic`, `static`, or [`UNKNOWN_PROVENANCE`] for an older artifact.
    pub linkage: String,
    /// The C library, absent on a platform that has only one.
    #[serde(default)]
    pub libc: Option<LibcRequirement>,
    /// Whether a NIF can be loaded into the bundled runtime.
    pub nif_loading: bool,
    /// The source, as `[tools.ginary.target.<name>] erts` spelled it, with the
    /// root appended: `host:/usr/lib/erlang`, `dir:/opt/otp`.
    pub source: String,
}

/// What an artifact that recorded no provenance is reported as.
pub const UNKNOWN_PROVENANCE: &str = "unknown";

impl Default for OtpProvenance {
    /// What an artifact built before the block existed is read as.
    ///
    /// `nif_loading` is `true` because every artifact that predates the block
    /// bundled the host's own dynamically linked runtime, which loads them;
    /// the two strings say `unknown` because nothing read them.
    fn default() -> Self {
        Self {
            linkage: UNKNOWN_PROVENANCE.to_owned(),
            libc: None,
            nif_loading: true,
            source: UNKNOWN_PROVENANCE.to_owned(),
        }
    }
}

/// Everything the launcher needs to build its argument vector.
///
/// [`LaunchSpec::program`] is a program *name* and [`LaunchSpec::bindir`] the
/// directory it lives in, because the launcher needs the directory anyway: it
/// is what `BINDIR` is set to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaunchSpec {
    /// The program to exec, a bare name inside [`LaunchSpec::bindir`].
    pub program: String,
    /// The runtime's `bin` directory, relative to the extracted root.
    pub bindir: String,
    /// The boot script, relative to the extracted root and without the
    /// `.boot` suffix, as `-boot` wants it.
    pub boot: String,
    /// The code path, one root-relative directory per `-pa`.
    pub pa: Vec<String>,
    /// The Erlang expression `-eval` is given.
    pub eval: String,
    /// Extra flags to place before `-extra`.
    pub erl_flags: Vec<String>,
    /// The args file `-args_file` is given, relative to the extracted root.
    ///
    /// [`None`] when the project named no `vm_args`. It is inserted *before*
    /// ginary's own fixed flags, because `erl` takes the last value of a
    /// repeated flag: putting the user's file first is what makes ginary's
    /// flags win over it rather than the other way round.
    #[serde(default)]
    pub args_file: Option<String>,
    /// The `-config` argument, root-relative and without the `.config` suffix.
    #[serde(default)]
    pub config: Option<String>,
    /// Whether the artifact starts the runtime distributed.
    ///
    /// `true` means `epmd` is in the bundle and `-start_epmd false` is *not*
    /// in the argument vector.
    #[serde(default)]
    pub distribution: bool,
    /// One of [`crate::config::FILENAME_ENCODINGS`], mapped to `+fnu`, `+fnl`
    /// or `+fna`.
    #[serde(default = "default_filename_encoding")]
    pub filename_encoding: String,
    /// Whether `heart` is bundled, `-heart` passed and `HEART_COMMAND` set.
    #[serde(default)]
    pub heart: bool,
    /// Variables the launcher sets, each only when the caller has not.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

/// The filename encoding a manifest that names none carries.
///
/// A serde default rather than a constant expression because
/// `#[serde(default = "...")]` takes a function; the value is
/// [`crate::config::DEFAULT_FILENAME_ENCODING`].
fn default_filename_encoding() -> String {
    crate::config::DEFAULT_FILENAME_ENCODING.to_owned()
}

impl LaunchSpec {
    /// Checks every path in the spec.
    ///
    /// # Errors
    ///
    /// [`ManifestError::UnsafePath`] naming the field and the value when a
    /// path is absolute, holds a `..` component, holds a backslash or is
    /// empty.
    pub fn validate(&self) -> Result<(), ManifestError> {
        check_name("launch.program", &self.program)?;
        check_path("launch.bindir", &self.bindir)?;
        check_path("launch.boot", &self.boot)?;
        for (position, entry) in self.pa.iter().enumerate() {
            check_path(&format!("launch.pa[{position}]"), entry)?;
        }
        // Additive, and checked exactly like the fields that came before them:
        // an older manifest is not a way past this function, and both values
        // are joined onto the extracted root by [`crate::launch::plan`].
        if let Some(args_file) = &self.args_file {
            check_path("launch.args_file", args_file)?;
        }
        if let Some(config) = &self.config {
            check_path("launch.config", config)?;
        }
        Ok(())
    }
}

/// The contents of `ginary.json`.
///
/// The field order is the serialised order; `docs/format.md` prints the same
/// object.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    /// The manifest format version, [`FORMAT_VERSION`] for this build.
    pub format_version: u32,
    /// The packaged application's name.
    pub app: String,
    /// The packaged application's version.
    pub app_version: String,
    /// The Gleam compiler that built the shipment, when it said so.
    pub gleam_version: Option<String>,
    /// The OTP release, as `erlang:system_info(otp_release)` reports it.
    pub otp_release: u32,
    /// The full OTP version, as `erlang:system_info(version)` reports it.
    pub otp_version: String,
    /// The ERTS version, and with it the `erts-<vsn>` directory name.
    pub erts_version: String,
    /// What the bundled runtime is and where it came from.
    #[serde(default)]
    pub otp: OtpProvenance,
    /// The target the artifact was built for, by its canonical name.
    pub target: Target,
    /// The applications that came from the OTP library.
    pub otp_applications: Vec<AppRef>,
    /// The applications that came from the shipment.
    pub gleam_applications: Vec<String>,
    /// How to start the application.
    pub launch: LaunchSpec,
    /// The native objects in the artifact.
    pub native: Vec<NativeRef>,
    /// When the artifact was built, RFC 3339 in UTC.
    pub created_at: String,
    /// The version of ginary that built it.
    pub ginary_version: String,
    /// Keys this build does not know, preserved across a round trip.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

impl Manifest {
    /// Checks that this build can act on the manifest.
    ///
    /// Parsing is deliberately permissive so that the error names the version
    /// rather than a serde field; this is the check the launcher makes before
    /// it trusts anything else in the object.
    ///
    /// # Errors
    ///
    /// [`ManifestError::UnsupportedVersion`] when
    /// [`Manifest::format_version`] is not [`FORMAT_VERSION`].
    pub fn check_version(&self) -> Result<(), ManifestError> {
        if self.format_version == FORMAT_VERSION {
            Ok(())
        } else {
            Err(ManifestError::UnsupportedVersion {
                found: self.format_version,
                supported: FORMAT_VERSION,
            })
        }
    }

    /// Checks every field the launcher interpolates into a path.
    ///
    /// This is the whole of it: [`Manifest::app`], which is the `<app>`
    /// component of every cache path, and [`Manifest::launch`], which is the
    /// program the launcher execs and the directories it puts on the code
    /// path. Both are strings an *artifact* chose, and both are joined onto a
    /// directory the launcher created, so both are checked at the moment the
    /// launcher first trusts them rather than at the moment they were written.
    ///
    /// # Errors
    ///
    /// [`ManifestError::UnsafePath`] naming the field and the value.
    pub fn validate(&self) -> Result<(), ManifestError> {
        check_name("app", &self.app)?;
        self.launch.validate()
    }
}

/// One file in `ginary.index.json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexFile {
    /// The path relative to the extracted root, `/`-separated.
    pub path: String,
    /// The exact size in bytes.
    pub size: u64,
    /// The Unix permission bits, `st_mode & 0o7777`.
    pub mode: u32,
    /// The file's SHA-256, in lower-case hexadecimal.
    pub sha256: String,
    /// What the file is, as staging categorised it.
    pub category: Category,
}

/// The contents of `ginary.index.json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Index {
    /// Every file in the payload but the manifest and the index themselves,
    /// sorted by [`IndexFile::path`].
    pub files: Vec<IndexFile>,
}

impl Index {
    /// Hashes every file the staging listing names.
    ///
    /// The categories come from the listing rather than being derived again,
    /// so the index and `ginary.stage.json` cannot disagree about what a file
    /// is. `ginary.stage.json` itself is not in the listing, is not packed and
    /// is therefore not indexed: the index supersedes it.
    ///
    /// # Errors
    ///
    /// [`IndexError::Io`] naming the file that could not be read.
    pub fn from_staged(root: &Path, listing: &[StagedFile]) -> Result<Self, IndexError> {
        let mut files = Vec::with_capacity(listing.len());
        for staged in listing {
            let path = root.join(&staged.path);
            let (size, sha256) = hash_file(&path).map_err(|source| IndexError::Io {
                path: path.clone(),
                source,
            })?;
            let mode = mode_of(&path).map_err(|source| IndexError::Io {
                path: path.clone(),
                source,
            })?;
            files.push(IndexFile {
                path: staged.path.clone(),
                size,
                mode,
                sha256,
                category: staged.category,
            });
        }
        files.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(Self { files })
    }
}

/// Why a manifest is not usable.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ManifestError {
    /// The manifest was written by a newer ginary.
    #[error(
        "this artifact carries manifest format version {found}, and this ginary reads version \
         {supported}"
    )]
    UnsupportedVersion {
        /// The version the manifest carries.
        found: u32,
        /// The version this build understands.
        supported: u32,
    },
    /// A path in the manifest is not root-relative.
    #[error("`{field}` must be a relative `/`-separated path, and it is `{value}`")]
    UnsafePath {
        /// The field the path came from, such as `launch.pa[0]`.
        field: String,
        /// The offending value.
        value: String,
    },
    /// `SOURCE_DATE_EPOCH` is set to something that is not a second count.
    #[error("SOURCE_DATE_EPOCH is `{value}`, which is not a number of seconds since the epoch")]
    InvalidSourceDateEpoch {
        /// The value the environment held, lossily converted for the message.
        value: String,
    },
}

/// Why an index could not be built.
///
/// `#[non_exhaustive]` because reading a staged tree is the only way this can
/// fail today and will not be the only way for long: a caller outside the
/// crate — `tests/manifest.rs` is one — must keep a wildcard arm rather than
/// have a new variant turn into a silent behaviour change.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum IndexError {
    /// A file the staging listing names could not be read.
    #[error("reading `{path}` to hash it for the artifact index failed")]
    Io {
        /// The file that could not be read.
        path: PathBuf,
        /// What the read failed with.
        #[source]
        source: std::io::Error,
    },
}

/// The `created_at` value for a manifest, RFC 3339 in UTC.
///
/// `now_secs` is seconds since the Unix epoch and is passed in rather than
/// read, because nothing in the format path may reach for a wall clock: a
/// function that did could not be tested and could not be reproducible.
/// `SOURCE_DATE_EPOCH` overrides it.
///
/// # Errors
///
/// [`ManifestError::InvalidSourceDateEpoch`] when the variable is set to
/// something that is not a second count. It is not ignored: a build that was
/// asked to be reproducible and silently was not is worse than one that stops.
pub fn created_at(env: &EnvSnapshot, now_secs: u64) -> Result<String, ManifestError> {
    let seconds = match env.source_date_epoch.as_deref() {
        // An empty variable is an unset one, the rule `cache_dir::resolve`
        // follows for every path it reads: a caller that exported nothing did
        // not ask for a fixed timestamp.
        None => now_secs,
        Some(value) if value.is_empty() => now_secs,
        Some(value) => value
            .to_str()
            .and_then(|text| text.parse::<u64>().ok())
            .ok_or_else(|| ManifestError::InvalidSourceDateEpoch {
                value: value.to_string_lossy().into_owned(),
            })?,
    };
    Ok(format_rfc3339(seconds))
}

/// The RFC 3339 spelling, in UTC, of a count of seconds since the Unix epoch.
///
/// The calendar arithmetic is Howard Hinnant's `civil_from_days`, which is
/// exact for every proleptic Gregorian date and needs neither a table nor a
/// leap-year special case. Every operation is on `u64` values bounded by the
/// day number, so none of them can overflow whatever `secs` is.
fn format_rfc3339(secs: u64) -> String {
    const SECONDS_PER_DAY: u64 = 86_400;

    let days = secs / SECONDS_PER_DAY;
    let time_of_day = secs % SECONDS_PER_DAY;
    let (hour, minute, second) = (
        time_of_day / 3_600,
        (time_of_day % 3_600) / 60,
        time_of_day % 60,
    );

    // Shift the epoch to 0000-03-01, which puts the leap day at the end of the
    // year and makes the month lengths a repeating pattern.
    let shifted = days + 719_468;
    let era = shifted / 146_097;
    let day_of_era = shifted % 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + u64::from(month <= 2);

    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Checks a bare name: one path component, and nothing that could climb out of
/// the directory it is joined onto.
///
/// Two fields are names rather than paths, and both are interpolated into a
/// filesystem path by the launcher: `launch.program`, which is looked up in
/// the bindir, and `app`, which is the `<app>` component of
/// `<cache>/<app>/<key>`. `pub(crate)` because [`crate::cache`] checks the
/// second one again at the point it creates that directory.
pub(crate) fn check_name(field: &str, value: &str) -> Result<(), ManifestError> {
    check_path(field, value)?;
    if value.contains('/') {
        return Err(ManifestError::UnsafePath {
            field: field.to_owned(),
            value: value.to_owned(),
        });
    }
    Ok(())
}

/// Checks one root-relative, `/`-separated path.
///
/// Empty, absolute, backslash-separated, and anything holding a `.`, a `..` or
/// an empty component are all refused. The last three matter because the
/// launcher joins these strings onto a cache directory it created: a value
/// that walked out of that directory would be a path the artifact chose and
/// the launcher followed.
fn check_path(field: &str, value: &str) -> Result<(), ManifestError> {
    let unsafe_path = || ManifestError::UnsafePath {
        field: field.to_owned(),
        value: value.to_owned(),
    };

    if value.is_empty() || value.starts_with('/') || value.contains('\\') {
        return Err(unsafe_path());
    }
    if value
        .split('/')
        .any(|component| matches!(component, "" | "." | ".."))
    {
        return Err(unsafe_path());
    }
    Ok(())
}

/// The length and the lower-case hexadecimal SHA-256 of one file.
///
/// The file is streamed through a fixed buffer rather than read into memory:
/// the largest file in a staged tree is `beam.smp`, and an index is built over
/// every one of them.
fn hash_file(path: &Path) -> Result<(u64, String), std::io::Error> {
    use sha2::{Digest, Sha256};

    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut size = 0u64;
    loop {
        let read = std::io::Read::read(&mut file, &mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        size = size.saturating_add(read as u64);
    }
    Ok((size, hex::encode(hasher.finalize())))
}

/// A file's permission bits, `st_mode & 0o7777`.
#[cfg(unix)]
fn mode_of(path: &Path) -> Result<u32, std::io::Error> {
    use std::os::unix::fs::PermissionsExt as _;

    Ok(std::fs::symlink_metadata(path)?.permissions().mode() & 0o7777)
}

/// The permission bits a Windows build records for a file.
///
/// Windows has no mode word, so there is nothing to read; what is recorded is
/// what the `tar` crate itself writes into the archive header on this platform,
/// 0o755 for a directory and 0o644 for everything else. Recording the same
/// value on both sides is what keeps `ginary verify` — which compares the index
/// against the header — from reporting a mismatch that means nothing, and the
/// column is informational on a Windows artifact rather than a permission
/// anything enforces.
///
/// **A unix artifact cross-built on Windows loses execute bits outside the
/// bindir.** There is no mode to read, so an executable under
/// `lib/<app>-<vsn>/priv/bin` is recorded 0o644 and extracted 0o644;
/// [`crate::cache::ensure_extracted`] repairs the bindir and nothing else,
/// because the bindir is the only place it knows every file has to be
/// runnable. It is a stated limitation rather than a silent one: it is in the
/// README's `## Windows` section and in
/// `docs/adr/0015-windows-launcher-stays-resident.md`, and the honest fix —
/// carrying the source tree's modes through a Windows build — needs a mode
/// column that does not come from the filesystem, which no milestone has asked
/// for yet.
#[cfg(windows)]
fn mode_of(path: &Path) -> Result<u32, std::io::Error> {
    let metadata = std::fs::symlink_metadata(path)?;
    Ok(if metadata.is_dir() { 0o755 } else { 0o644 })
}
