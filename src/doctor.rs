// SPDX-License-Identifier: MIT OR Apache-2.0
//! Environment diagnosis for `ginary doctor`.
//!
//! `doctor` answers one question: can this machine build a ginary artifact, and
//! if not, what is missing? It probes the external programs ginary shells out
//! to, reports the host target and the cache root, and states explicitly that a
//! Rust toolchain is *not* part of the answer — neither ginary nor the
//! executables it produces need `rustc` or `cargo` at run time.
//!
//! Probing never fails the command. A missing or broken tool is data, not an
//! error, so `doctor` always exits 0 and the caller reads the report.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime};

use serde::{Deserialize, Serialize};

use crate::cache_dir::{self, EnvSnapshot};
use crate::config::{ProjectConfig, TargetConfig};
use crate::elf::{self, ElfKind};
use crate::erts_source::{ErtsError, ErtsSourceSpec, ResolvedErts};
use crate::otp;
use crate::process::{NULL_DEVICE, run_with_timeout};
use crate::target::Target;

/// Searching `PATH` for a program, re-exported from [`crate::process`].
///
/// `doctor` is where the search is visible to a user of the crate — it is what
/// the `gleam:`, `erl:`, `strip:` and `docker:` lines report — while the rule
/// itself is shared with [`crate::otp`].
pub use crate::process::find_in_path;

/// Version of the `doctor --json` schema.
pub const FORMAT_VERSION: u32 = 1;

/// How long a single tool probe may run before it is killed.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// One external program `doctor` knows how to probe.
struct Probe {
    /// Program name, looked up on `PATH`.
    name: &'static str,
    /// Arguments that make the program print its version and exit.
    args: &'static [&'static str],
    /// Turns the program's standard output into a human-readable version.
    parse: fn(&str) -> Option<String>,
}

/// The programs `doctor` probes, in report order.
const PROBES: [Probe; 4] = [
    Probe {
        name: "gleam",
        args: &["--version"],
        parse: parse_gleam_version,
    },
    Probe {
        name: "erl",
        args: &[
            "-noshell",
            // A broken OTP install can dump core on start-up. Without this the
            // probe would leave an `erl_crash.dump` in the user's working
            // directory, which is one of the UX problems ginary exists to fix.
            "-env",
            "ERL_CRASH_DUMP",
            NULL_DEVICE,
            "-eval",
            "io:format(\"~ts ~ts\",[erlang:system_info(otp_release),erlang:system_info(version)]),halt(0).",
        ],
        parse: parse_erl_version,
    },
    Probe {
        name: "strip",
        args: &["--version"],
        parse: parse_strip_version,
    },
    Probe {
        name: "docker",
        args: &["version", "--format", "{{.Server.Version}}"],
        parse: parse_docker_version,
    },
];

/// The state of one probed program.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ToolReport {
    /// Program name as spelled on `PATH`.
    pub name: String,
    /// Whether an executable of that name was found on `PATH`.
    pub found: bool,
    /// Version string, or `None` when the program is absent or did not answer.
    pub version: Option<String>,
    /// Absolute path of the executable, or `None` when it was not found.
    pub path: Option<PathBuf>,
}

impl ToolReport {
    /// Renders the one-line human form, for example `gleam: 1.18.1 (/usr/bin/gleam)`.
    fn render(&self) -> String {
        match (&self.version, &self.path) {
            (Some(version), Some(path)) => {
                format!("{}: {version} ({})", self.name, path.display())
            }
            (None, Some(path)) => {
                format!("{}: found, version unknown ({})", self.name, path.display())
            }
            (_, None) => format!("{}: not found", self.name),
        }
    }
}

/// What `doctor` says about the OTP installation it found.
///
/// A summary rather than the whole [`crate::otp::OtpInfo`]: `doctor` reports
/// what a person needs in order to recognise the installation, and the derived
/// paths are reconstructible from the root.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OtpReport {
    /// The code root the installation lives in.
    pub root: PathBuf,
    /// The major release, for example `29`.
    pub release: u32,
    /// The ERTS version, for example `17.0.5`.
    pub erts_vsn: String,
    /// The full version, for example `29.0.5`.
    pub otp_version: String,
    /// What `crypto`'s NIF needs, when the installation has one.
    pub crypto: Option<CryptoReport>,
}

impl OtpReport {
    /// Summarises a discovered installation.
    pub fn of(info: &otp::OtpInfo) -> Self {
        Self {
            root: info.root.clone(),
            release: info.release,
            erts_vsn: info.erts_vsn.clone(),
            otp_version: info.otp_version.clone(),
            crypto: crypto_report(&info.root),
        }
    }

    /// Renders the two `otp` lines of the human-readable report.
    fn render(&self) -> String {
        format!(
            "otp: {} (release {}, erts {})\notp root: {}",
            self.otp_version,
            self.release,
            self.erts_vsn,
            self.root.display()
        )
    }
}

/// The hint a cache directory that cannot be used earns.
pub const CACHE_DIR_HINT: &str = "set GINARY_CACHE_DIR to a directory this user can write to on a filesystem that is not \
     mounted `noexec`";

/// What a cache directory turned out to allow.
///
/// A packaged application does two things with its cache and both can be
/// forbidden separately: it writes an extracted runtime there, and it execs
/// programs out of it. A `noexec` mount, a read-only home and a full disk are
/// three different diagnoses, and a `doctor` that only reported the resolved
/// path would name none of them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CacheProbe {
    /// Whether a file could be created in the directory.
    pub writable: bool,
    /// Whether a file created there could then be executed.
    pub executable: bool,
    /// What the operating system said, when either half failed.
    pub detail: Option<String>,
}

impl CacheProbe {
    /// Renders the two `cache` lines, and the hint when one of them failed.
    ///
    /// ```text
    /// cache writable: yes
    /// cache executable: no (mounted noexec?)
    /// hint: set GINARY_CACHE_DIR to a directory this user can write to ...
    /// ```
    pub fn render(&self) -> String {
        let mut text = format!("cache writable: {}\n", yes_no(self.writable));
        text.push_str(&format!(
            "cache executable: {}\n",
            match (self.writable, self.executable) {
                (_, true) => "yes",
                (true, false) => "no (mounted noexec?)",
                // Nothing could be written, so nothing was run: saying
                // `noexec` here would send a reader to the mount table for a
                // problem that is in the directory's permissions.
                (false, false) => "no (nothing could be written to run)",
            }
        ));
        if let Some(detail) = &self.detail {
            text.push_str(&format!("cache detail: {detail}\n"));
        }
        if !self.writable || !self.executable {
            text.push_str(&format!("hint: {CACHE_DIR_HINT}\n"));
        }
        text
    }
}

/// The probe program: the smallest thing a kernel will exec.
const PROBE_PROGRAM: &[u8] = b"#!/bin/sh\nexit 0\n";

/// The mode the probe file is given, which is the mode the cache gives every
/// program under an extracted bindir.
const PROBE_MODE: u32 = 0o755;

/// How long a probe is retried while the kernel answers `ETXTBSY`.
///
/// The probe writes a program and immediately execs it, which is the race
/// `src/process.rs` documents for its own test helper: a `fork` on another
/// thread inherits the write descriptor this one is holding, and every exec of
/// that inode fails with `ETXTBSY` until the forked child execs. The window is
/// microseconds long, and it is not a `noexec` mount — reporting it as one
/// would send a reader to the mount table for a problem that is not there.
const PROBE_BUSY_BUDGET: Duration = Duration::from_secs(2);

/// How long the probe waits between attempts while the kernel says `ETXTBSY`.
const PROBE_BUSY_POLL: Duration = Duration::from_millis(2);

/// How many probes this process has run, so that two cannot share a name.
static PROBE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// The name of the file the probe creates.
///
/// A dot-file, so a cache directory a user looks at is not littered by a
/// `doctor` that was killed between creating the file and removing it. The pid
/// keeps two `doctor` *processes* over one directory from removing each
/// other's, and `sequence` keeps two probes of one process apart: `doctor`
/// probes once, but its tests run in threads of one binary and a pid alone is
/// not unique between them.
fn probe_file_name(pid: u32, sequence: u64) -> String {
    format!(".ginary-doctor-probe-{pid}-{sequence}")
}

/// Creates a file in `dir`, makes it executable and tries to run it.
///
/// The probe is the only honest answer: `access(2)` reports the permission
/// bits and says nothing about the mount, and `noexec` is the failure that
/// actually reaches users. Whatever it creates is removed again.
///
/// The directory itself is created when it is not there, because that is what
/// a launch would do: a `doctor` that reported "not writable" for a cache
/// nobody has used yet would be reporting its absence rather than a problem.
pub fn probe_cache_dir(dir: &Path) -> CacheProbe {
    let refused = |detail: std::io::Error| CacheProbe {
        writable: false,
        executable: false,
        detail: Some(detail.to_string()),
    };

    if let Err(error) = std::fs::create_dir_all(dir) {
        return refused(error);
    }
    let path = dir.join(probe_file_name(
        std::process::id(),
        PROBE_SEQUENCE.fetch_add(1, Ordering::Relaxed),
    ));
    if let Err(error) = write_probe(&path) {
        let _ = std::fs::remove_file(&path);
        return refused(error);
    }

    let outcome = run_probe(&path);
    // Removed before the answer is returned, whatever the answer is: a probe
    // that left an executable behind in a cache directory would be a probe
    // nobody should run twice.
    let _ = std::fs::remove_file(&path);

    match outcome {
        Ok(()) => CacheProbe {
            writable: true,
            executable: true,
            detail: None,
        },
        Err(error) => CacheProbe {
            writable: true,
            executable: false,
            detail: Some(error.to_string()),
        },
    }
}

/// Writes the probe program and makes it executable.
fn write_probe(path: &Path) -> std::io::Result<()> {
    std::fs::write(path, PROBE_PROGRAM)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(PROBE_MODE))?;
    }
    Ok(())
}

/// Runs the probe program and reports whether the kernel would start it.
///
/// A program that starts and exits non-zero still proves the mount allows
/// execution, which is the whole question; only a failure to *start* it is an
/// answer of `no`.
///
/// `ETXTBSY` is not such a failure. It says some process still holds a write
/// descriptor on the file this function just wrote, which is a race against
/// `doctor` itself rather than anything about the directory, so it is retried
/// for [`PROBE_BUSY_BUDGET`] and only then reported.
fn run_probe(path: &Path) -> std::io::Result<()> {
    let deadline = Instant::now() + PROBE_BUSY_BUDGET;
    loop {
        let outcome = std::process::Command::new(path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        match outcome {
            Ok(_) => return Ok(()),
            Err(error)
                if error.kind() == std::io::ErrorKind::ExecutableFileBusy
                    && Instant::now() < deadline =>
            {
                std::thread::sleep(PROBE_BUSY_POLL);
            }
            Err(error) => return Err(error),
        }
    }
}

/// `yes` or `no`.
fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

/// What `crypto`'s NIF needs from the machine.
///
/// The one library whose linkage decides whether a packaged application is
/// portable: an OTP built against a *static* OpenSSL leaves a `crypto.so` that
/// needs nothing but libc, and that is the guarantee ginary's artifacts rest
/// on. One that needs a `libssl.so.3` is an artifact that will not start on a
/// machine without that exact soname.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CryptoReport {
    /// The NIF, under `<root>/lib/crypto-<vsn>/priv/lib/`.
    pub path: PathBuf,
    /// Its `DT_NEEDED` entries, in the order the dynamic section lists them.
    pub needed: Vec<String>,
    /// Whether it needs nothing beyond a C runtime.
    pub statically_linked_openssl: bool,
}

impl CryptoReport {
    /// Renders the `crypto:` line and, when it is static, why that matters.
    pub fn render(&self) -> String {
        let mut text = format!("crypto: {}\n", self.path.display());
        text.push_str(&format!(
            "crypto needs: {}\n",
            if self.needed.is_empty() {
                "-".to_owned()
            } else {
                self.needed.join(", ")
            }
        ));
        text.push_str(if self.statically_linked_openssl {
            "crypto note: nothing beyond a C runtime, so this OTP's OpenSSL is linked in \
             statically; that is what lets an artifact built here start on a machine with no \
             libssl of its own\n"
        } else {
            "crypto note: a machine running an artifact built from this OTP must already carry \
             the libraries above, which is the portability floor of everything built with it\n"
        });
        text
    }
}

/// The libraries every glibc program links against, whatever it does.
///
/// A `crypto.so` that needs only these is one whose OpenSSL was linked in
/// statically. Anything else — a `libcrypto.so.3`, a `libssl.so.3` — is a file
/// the target machine has to supply.
const C_RUNTIME_LIBRARIES: [&str; 6] = [
    "libc.so.6",
    "libm.so.6",
    "libdl.so.2",
    "libpthread.so.0",
    "librt.so.1",
    "libgcc_s.so.1",
];

/// The prefix of the application directory `crypto`'s NIF lives under.
const CRYPTO_APP_PREFIX: &str = "crypto-";

/// The NIF, relative to the `crypto` application directory.
const CRYPTO_NIF: &str = "priv/lib/crypto.so";

/// Finds `crypto.so` under an OTP root and reads what it needs.
///
/// `None` when the installation carries no `crypto` application, which a
/// runtime assembled from ERTS binaries alone legitimately does not, and also
/// when the file is there and is not an ELF this build can read: `doctor`
/// never fails, and a NIF nothing can parse is named by `ginary verify` on the
/// artifact that carries it rather than guessed at here.
pub fn crypto_report(otp_root: &Path) -> Option<CryptoReport> {
    let path = crypto_nif(otp_root)?;
    let info = elf::inspect(&path).ok()?;
    let statically_linked_openssl = info
        .needed
        .iter()
        .all(|needed| C_RUNTIME_LIBRARIES.contains(&needed.as_str()));
    Some(CryptoReport {
        path,
        needed: info.needed,
        statically_linked_openssl,
    })
}

/// `<root>/lib/crypto-<vsn>/priv/lib/crypto.so`, found by prefix.
///
/// The version is not known here and is not worth discovering separately: the
/// directory is the only `crypto-*` an installation has, and reading
/// `OTP_VERSION` to learn a number that is already in the path would be a
/// second source of truth. The highest name wins if an installation somehow
/// holds two, so the answer does not depend on directory order.
fn crypto_nif(otp_root: &Path) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = std::fs::read_dir(otp_root.join("lib"))
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(OsStr::to_str)
                .is_some_and(|name| name.starts_with(CRYPTO_APP_PREFIX))
        })
        .map(|path| path.join(CRYPTO_NIF))
        .filter(|path| path.is_file())
        .collect();
    candidates.sort();
    candidates.pop()
}

/// The targets the project `start` is in builds for, and their sub-tables.
///
/// [`None`] when there is no project at or above `start`, when its manifest
/// cannot be read, or when what it says about targets is not usable — a
/// `doctor` that refused to print because a `gleam.toml` is wrong would be
/// withholding the report the user ran it to get. The rest of the report says
/// what is wrong with the table; this half falls back to the host.
fn project_targets(start: &Path) -> Option<(Vec<Target>, BTreeMap<String, TargetConfig>)> {
    let project = crate::gleam::find_project(start).ok()?;
    let config = ProjectConfig::read(&project.manifest()).ok()?;
    let targets = crate::target::resolve_targets(&[], &config.tools.targets).ok()?;
    Some((targets, config.tools.target))
}

/// What `[tools.ginary]` said, or why it could not be read.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ConfigStatus {
    /// The project declares no `[tools.ginary]` table.
    Absent,
    /// The table parsed.
    Ok,
    /// The table did not parse, and this is what the parser said.
    Error {
        /// The message, verbatim: an unknown key is named by serde and a
        /// paraphrase would lose the key.
        message: String,
    },
}

/// The shipment a project has already exported.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ShipmentReport {
    /// `<project>/build/erlang-shipment`.
    pub path: PathBuf,
    /// How old it is, in seconds.
    pub age_secs: u64,
}

/// One native object found under a shipment's `priv`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct NativeObject {
    /// The path, relative to the shipment and `/`-separated.
    pub path: String,
    /// The machine, as [`crate::elf::ElfInfo::machine`] spells it.
    pub machine: String,
    /// What kind of object the file is.
    pub kind: crate::elf::ElfKind,
    /// Its `DT_NEEDED` entries.
    pub needed: Vec<String>,
    /// Whether its machine is the one this host runs.
    pub matches_host: bool,
}

/// What `doctor` says when it is run inside a Gleam project.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProjectReport {
    /// The directory holding `gleam.toml`.
    pub root: PathBuf,
    /// The project name.
    pub name: String,
    /// The project version, when the manifest declares one.
    pub version: Option<String>,
    /// The exported shipment, when there is one.
    pub shipment: Option<ShipmentReport>,
    /// What `[tools.ginary]` said.
    pub config: ConfigStatus,
    /// Every ELF under the shipment's `priv`, in path order.
    pub native: Vec<NativeObject>,
}

impl ProjectReport {
    /// Renders the project block: name, shipment, configuration, and the
    /// native table when there is native code.
    ///
    /// ```text
    /// project: notify 3.1.4 (/w/notify)
    /// shipment: /w/notify/build/erlang-shipment (3600 seconds old)
    /// [tools.ginary]: unknown field `not_a_key`
    ///
    /// path                    machine  kind           host  needed
    /// notify/priv/lib/nif.so  aarch64  shared object  no    libc.so.6
    /// ```
    pub fn render(&self) -> String {
        let mut text = format!(
            "project: {} {} ({})\n",
            self.name,
            self.version.as_deref().unwrap_or(DASH),
            self.root.display()
        );
        text.push_str(&match &self.shipment {
            Some(shipment) => format!(
                "shipment: {} ({} seconds old)\n",
                shipment.path.display(),
                shipment.age_secs
            ),
            None => "shipment: none exported yet\n".to_owned(),
        });
        text.push_str(&match &self.config {
            ConfigStatus::Absent => "[tools.ginary]: absent\n".to_owned(),
            ConfigStatus::Ok => "[tools.ginary]: read\n".to_owned(),
            // Verbatim: serde names the key that is wrong, and a paraphrase
            // would lose the one word the reader has to search their manifest
            // for.
            ConfigStatus::Error { message } => format!("[tools.ginary]: {message}\n"),
        });

        if self.native.is_empty() {
            return text;
        }
        let rows: Vec<[String; 5]> = self
            .native
            .iter()
            .map(|object| {
                [
                    object.path.clone(),
                    object.machine.clone(),
                    kind_label(object.kind),
                    yes_no(object.matches_host).to_owned(),
                    if object.needed.is_empty() {
                        DASH.to_owned()
                    } else {
                        object.needed.join(", ")
                    },
                ]
            })
            .collect();
        text.push('\n');
        text.push_str(&crate::closure::render_table(
            ["path", "machine", "kind", "host", "needed"],
            &rows,
        ));
        text
    }
}

/// What a missing value prints as.
const DASH: &str = "-";

/// The word one [`ElfKind`] prints as.
fn kind_label(kind: ElfKind) -> String {
    match kind {
        ElfKind::Relocatable => "relocatable".to_owned(),
        ElfKind::Executable => "executable".to_owned(),
        ElfKind::SharedObject => "shared object".to_owned(),
        ElfKind::Core => "core".to_owned(),
        ElfKind::Other(value) => format!("e_type {value}"),
    }
}

/// The directory component under which an application keeps its native code.
const PRIV_DIR: &str = "priv";

/// The largest file under a `priv` directory that is read to be inspected.
///
/// The same bound [`crate::verify`] applies, for the same reason: an ELF
/// header says nothing about how large the file behind it is, and `doctor` may
/// not be the command that runs a machine out of memory.
const MAX_NATIVE_BYTES: u64 = crate::verify::MAX_OBJECT_BYTES;

/// How deep the shipment walk goes.
///
/// A shipment is `<app>/<ebin|priv>/…`, so five levels is already more than a
/// real one uses and twelve is generous rather than exact: a NIF under
/// `priv/lib/<arch>/<abi>/` is still found, and nothing a Gleam project
/// produces goes deeper. The bound is against a cycle somebody put in a build
/// directory, not against a deep project.
const MAX_SHIPMENT_DEPTH: usize = 12;

/// A `gleam.toml` as `doctor` reads it, before any rule is applied.
///
/// [`ProjectConfig`] refuses a manifest with an unknown `[tools.ginary]` key,
/// which is exactly the case this report exists to *describe*: the identity of
/// the project has to survive a `[tools.ginary]` that does not parse, so the
/// name and the version are read here and the table's status is read
/// separately.
#[derive(Debug, Default, Deserialize)]
struct RawManifest {
    /// The project name.
    #[serde(default)]
    name: Option<String>,
    /// The project version.
    #[serde(default)]
    version: Option<String>,
    /// The `[tools]` table, when there is one.
    #[serde(default)]
    tools: Option<RawTools>,
}

/// The `[tools]` table of a `gleam.toml`, as far as `doctor` reads it.
#[derive(Debug, Default, Deserialize)]
struct RawTools {
    /// Whether `[tools.ginary]` is there at all, whatever is in it.
    #[serde(default)]
    ginary: Option<serde::de::IgnoredAny>,
}

/// Reads the project `start` is in, when it is in one.
///
/// `None` when no `gleam.toml` is found at or above `start`, which is the
/// ordinary case for `ginary doctor` run anywhere else. `now` is passed in
/// rather than read so that the shipment's age is a pure function.
pub fn project_context(start: &Path, now: SystemTime) -> Option<ProjectReport> {
    let project = crate::gleam::find_project(start).ok()?;
    let manifest = project.manifest();
    let text = std::fs::read_to_string(&manifest).ok()?;
    let raw: RawManifest = toml::from_str(&text).unwrap_or_default();

    let name = raw.name.unwrap_or_else(|| {
        project.root().file_name().map_or_else(
            || DASH.to_owned(),
            |name| name.to_string_lossy().into_owned(),
        )
    });

    let shipment_dir = project.shipment();
    let shipment = shipment_report(&shipment_dir, now);
    let native = if shipment.is_some() {
        native_objects(&shipment_dir)
    } else {
        Vec::new()
    };

    Some(ProjectReport {
        root: project.root().to_path_buf(),
        name,
        version: raw.version,
        shipment,
        config: config_status(&text, &manifest, raw.tools.and_then(|tools| tools.ginary)),
        native,
    })
}

/// What `[tools.ginary]` said, or why it could not be read.
///
/// The table's *presence* and its *validity* are two questions, and only the
/// raw read answers the first: [`ProjectConfig`] fills in the defaults for a
/// project that declares no table at all, so a manifest with no `[tools.ginary]`
/// and one with an empty `[tools.ginary]` parse identically.
fn config_status(
    text: &str,
    manifest: &Path,
    declared: Option<serde::de::IgnoredAny>,
) -> ConfigStatus {
    match ProjectConfig::from_toml(text, manifest) {
        Ok(_) if declared.is_some() => ConfigStatus::Ok,
        Ok(_) => ConfigStatus::Absent,
        // The whole chain, for the reason `a1a_doctor_dropped_the_otp_error`
        // gives: `ConfigError::Target` is a headline — "cannot resolve the
        // targets to build" — whose cause is the only half that names the
        // entry nothing can build.
        Err(error) => ConfigStatus::Error {
            message: error_chain(&error),
        },
    }
}

/// The exported shipment and its age, when a project has one.
fn shipment_report(shipment: &Path, now: SystemTime) -> Option<ShipmentReport> {
    let modified = std::fs::metadata(shipment)
        .ok()
        .filter(|metadata| metadata.is_dir())?;
    let age_secs = modified
        .modified()
        .ok()
        .and_then(|when| now.duration_since(when).ok())
        .map_or(0, |age| age.as_secs());
    Some(ShipmentReport {
        path: shipment.to_path_buf(),
        age_secs,
    })
}

/// Every ELF under the shipment's `priv` directories, in path order.
///
/// The magic decides and never the extension: a `.so` that is really a shell
/// wrapper is not native code, and a NIF under `priv/lib` may be called
/// anything. A file whose first bytes *are* the magic and which does not parse
/// as an ELF is not listed — there is nothing this table could say about it —
/// and `ginary verify` names it on the artifact that carries it.
fn native_objects(shipment: &Path) -> Vec<NativeObject> {
    let host = Target::host().arch.as_str();
    let mut found = Vec::new();
    walk_shipment(shipment, shipment, 0, &mut |relative, path| {
        if !relative.split('/').any(|component| component == PRIV_DIR) {
            return;
        }
        let Ok(metadata) = std::fs::metadata(path) else {
            return;
        };
        if !metadata.is_file() || metadata.len() > MAX_NATIVE_BYTES {
            return;
        }
        // The magic first and the whole file only after it: a `priv` directory
        // holds assets as well as objects, and a ninety-megabyte one that is
        // not an ELF must not be read into memory to learn that it is not.
        if !begins_with_elf_magic(path) {
            return;
        }
        let Ok(info) = elf::inspect(path) else {
            return;
        };
        found.push(NativeObject {
            path: relative.to_owned(),
            machine: info.machine.clone(),
            kind: info.kind,
            needed: info.needed,
            matches_host: info.machine == host,
        });
    });
    found.sort_by(|left, right| left.path.cmp(&right.path));
    found
}

/// Whether the first bytes of `path` are [`elf::ELF_MAGIC`].
///
/// Four bytes, and nothing else is read. `false` for a file that cannot be
/// opened or that is shorter than the magic, both of which are files this
/// table has nothing to say about.
fn begins_with_elf_magic(path: &Path) -> bool {
    use std::io::Read as _;

    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut head = [0u8; elf::ELF_MAGIC.len()];
    let mut filled = 0usize;
    while filled < head.len() {
        match file.read(&mut head[filled..]) {
            Ok(0) => return false,
            Ok(read) => filled = filled.saturating_add(read),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return false,
        }
    }
    elf::is_elf(&head)
}

/// Calls `visit` for every regular file under `dir`, with its path relative to
/// `root`.
///
/// Depth-bounded by [`MAX_SHIPMENT_DEPTH`], because a `build/` directory is a
/// place other tools write and a loop in it must not become a `doctor` that
/// does not return. A directory symlink is therefore never descended into: a
/// symlink is followed only when it resolves to a regular *file*, which is how
/// a NIF installed as a link reaches the table, and a symlink to a directory,
/// to nothing, or to a device is passed over.
fn walk_shipment(root: &Path, dir: &Path, depth: usize, visit: &mut impl FnMut(&str, &Path)) {
    if depth >= MAX_SHIPMENT_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.filter_map(Result::ok).map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        let Ok(kind) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if kind.is_dir() {
            walk_shipment(root, &path, depth + 1, visit);
            continue;
        }
        // `metadata` rather than `symlink_metadata`: this is where a symlink
        // is followed, and only a symlink whose target is a regular file gets
        // past it.
        if !std::fs::metadata(&path).is_ok_and(|target| target.is_file()) {
            continue;
        }
        let Ok(relative) = path.strip_prefix(root) else {
            continue;
        };
        visit(&relative.to_string_lossy().replace('\\', "/"), &path);
    }
}

/// One row of the targets table `doctor` prints.
///
/// A build asks two questions about a target before it starts, and this is
/// where a user asks them first: where would the runtime come from, and can
/// this machine get it today. The two facts below the answer — the linkage and
/// the minimum glibc — are the ones that decide whether a NIF will load and
/// which machines the artifact will start on, and they are read out of the
/// emulator rather than out of the configuration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TargetProbe {
    /// The canonical target name.
    pub name: String,
    /// The ERTS source, as `[tools.ginary.target.<name>] erts` spelled it.
    pub erts: String,
    /// Whether this ginary can resolve that source, on this machine, today.
    ///
    /// One question with one meaning: `false` is a build of this target
    /// failing here and now, whether the source arrives with a later
    /// milestone, points at the wrong machine's runtime, or is a runtime that
    /// is here and could not be read. [`TargetProbe::detail`] says which.
    pub resolvable: bool,
    /// What was resolved, or why it could not be.
    pub detail: Option<String>,
    /// How the runtime is linked, when it was resolved.
    pub linkage: Option<String>,
    /// The lowest glibc the runtime runs against, when there is one.
    pub libc_min: Option<String>,
}

impl TargetProbe {
    /// The row's cells, in the order [`render_targets`] prints them.
    ///
    /// The two facts that are only known for a runtime somebody actually read
    /// — the linkage and the minimum libc — print `-` for a target nothing
    /// looked at, which is not the same claim as `static` or `none`.
    pub fn cells(&self) -> [String; 6] {
        [
            self.name.clone(),
            self.erts.clone(),
            if self.resolvable { "yes" } else { "not yet" }.to_owned(),
            self.linkage.clone().unwrap_or_else(|| DASH.to_owned()),
            self.libc_min.clone().unwrap_or_else(|| DASH.to_owned()),
            self.detail.clone().unwrap_or_else(|| DASH.to_owned()),
        ]
    }
}

/// Probes every target a project or a command line named.
///
/// `config` is `[tools.ginary.target]`, so a target with no sub-table is
/// probed as `host`, which is what a build of it would use. Only the host's
/// own runtime is inspected: a `dir:` source is reported as resolvable without
/// being read, because `doctor` describes the machine rather than performing
/// half of a build.
///
/// Two rows answer `not yet` without a milestone behind them: `host` on a
/// target this machine is not — the runtime that spelling resolves to is for
/// the host, and a build would refuse it with a target mismatch — and the
/// host's own row when this machine's installation cannot be read. Both are a
/// build failing here today, which is what the column asks.
pub fn probe_targets(
    targets: &[Target],
    config: &BTreeMap<String, TargetConfig>,
) -> Vec<TargetProbe> {
    probe_targets_with(targets, config, crate::erts_source::resolve)
}

/// [`probe_targets`], with the host runtime's resolution injected.
///
/// The one row that reads a runtime is the host's own, and reading it needs an
/// Erlang on the machine. Without this seam every assertion about that row
/// would be toolchain-gated, and the rule the column states — the host's
/// runtime resolves, and a machine whose installation cannot be read answers
/// `not yet` with the reason beside it — would go untested exactly where it
/// matters most, on a machine that has no Erlang.
///
/// `resolve` is [`crate::erts_source::resolve`] in the wrapper above. It is
/// called for the host's own row and for nothing else, so a probe of any other
/// target never reaches it.
pub fn probe_targets_with(
    targets: &[Target],
    config: &BTreeMap<String, TargetConfig>,
    resolve: impl Fn(&ErtsSourceSpec, &Target) -> Result<ResolvedErts, ErtsError>,
) -> Vec<TargetProbe> {
    targets
        .iter()
        .map(|target| probe_target(*target, config.get(&target.name()), &resolve))
        .collect()
}

/// One row: what the target's `erts` says, and what can be done about it.
fn probe_target(
    target: Target,
    config: Option<&TargetConfig>,
    resolve: &impl Fn(&ErtsSourceSpec, &Target) -> Result<ResolvedErts, ErtsError>,
) -> TargetProbe {
    let name = target.name();
    let spec = match config.map_or_else(|| Ok(ErtsSourceSpec::Host), TargetConfig::erts_spec) {
        Ok(spec) => spec,
        // A value `[tools.ginary.target.<name>] erts` cannot be read as a
        // source. `ProjectConfig::read` refuses it, so `doctor` only sees one
        // when it was handed a configuration nothing validated; the row says
        // what the value was and why it is not a source.
        Err(error) => {
            return TargetProbe {
                name,
                erts: config
                    .and_then(|config| config.erts.clone())
                    .unwrap_or_else(|| ErtsSourceSpec::Host.label()),
                resolvable: false,
                detail: Some(error.to_string()),
                linkage: None,
                libc_min: None,
            };
        }
    };
    let erts = spec.label();

    // A catalogue entry and a tarball resolve, and both are *described* here
    // rather than performed, for the reason a `dir:` is: consulting a catalogue
    // means a cache and possibly a fetch, and `doctor` describes the machine
    // rather than performing half of a build.
    if let ErtsSourceSpec::Catalog | ErtsSourceSpec::Tarball(_) = spec {
        return TargetProbe {
            name,
            detail: Some(match &spec {
                ErtsSourceSpec::Tarball(path) => {
                    format!("read from {} at build time", path.display())
                }
                _ => "read from the OTP catalog at build time; `ginary otp list` says what it \
                      holds"
                    .to_owned(),
            }),
            erts,
            resolvable: true,
            linkage: None,
            libc_min: None,
        };
    }

    if let Some(milestone) = spec.milestone() {
        return TargetProbe {
            name,
            erts,
            resolvable: false,
            detail: Some(format!("arrives with the {milestone} milestone")),
            linkage: None,
            libc_min: None,
        };
    }

    // The one runtime `doctor` reads is the one it is standing on. A `dir:`
    // that is on this machine is reported as resolvable without being opened:
    // `doctor` describes the machine, and inspecting a runtime root is half of
    // a build.
    if spec == ErtsSourceSpec::Host && target == Target::host() {
        return match resolve(&spec, &target) {
            Ok(erts) => TargetProbe {
                name,
                erts: spec.label(),
                resolvable: true,
                detail: Some(erts.provenance),
                linkage: Some(erts.linkage.as_str().to_owned()),
                libc_min: erts.libc_min,
            },
            // The source is one this ginary resolves and this machine's own
            // installation is what could not be read, which is a `no` to the
            // question the column asks: a build of this target would fail
            // here, today, for the reason in the detail. Answering `yes` and
            // printing the reason beside it would contradict the `otp:
            // unusable` line three rows above.
            Err(error) => TargetProbe {
                name,
                erts: spec.label(),
                resolvable: false,
                detail: Some(error_chain(&error)),
                linkage: None,
                libc_min: None,
            },
        };
    }

    // `host` on a target this machine is not. The branch above took the host's
    // own row, so the runtime this spelling resolves to is for another target
    // and `erts_source::resolve` would refuse it with a mismatch; a row that
    // said `yes` would send a user to run the build to find that out.
    if spec == ErtsSourceSpec::Host {
        let host = Target::host();
        return TargetProbe {
            detail: Some(format!(
                "this machine's runtime is for {host}; name a {name} runtime with \
                 `[tools.ginary.target.{name}] erts = \"dir:PATH\"`"
            )),
            name,
            erts,
            resolvable: false,
            linkage: None,
            libc_min: None,
        };
    }

    TargetProbe {
        name,
        erts,
        resolvable: true,
        detail: None,
        linkage: None,
        libc_min: None,
    }
}

/// One error and every cause under it, as one sentence.
///
/// `ErtsError::Otp` says "cannot use the runtime" and the cause says which
/// directory has no `erts-*` in it, so a row that printed only the headline
/// would name a fault and not a file.
fn error_chain(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        text.push_str(&format!(": {cause}"));
        source = cause.source();
    }
    text
}

/// The targets table, one row per probe.
pub fn render_targets(probes: &[TargetProbe]) -> String {
    let rows: Vec<[String; 6]> = probes.iter().map(TargetProbe::cells).collect();
    crate::closure::render_table(
        ["target", "erts", "resolves", "linkage", "libc", "detail"],
        &rows,
    )
}

/// The full `doctor` result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Report {
    /// Version of this schema; see [`FORMAT_VERSION`].
    pub format_version: u32,
    /// The target ginary itself runs on.
    pub host_target: Target,
    /// Always `false`: no Rust toolchain is needed to run ginary or its output.
    pub rustc_required: bool,
    /// The resolved cache root, or `None` when no variable located one.
    pub cache_dir: Option<PathBuf>,
    /// The environment variable the cache root came from.
    pub cache_dir_source: Option<&'static str>,
    /// Why the cache root could not be resolved, when it could not.
    pub cache_dir_error: Option<String>,
    /// What the cache root turned out to allow, when there was one to probe.
    pub cache_probe: Option<CacheProbe>,
    /// One entry per probed program: `gleam`, `erl`, `strip`, `docker`, in that
    /// order.
    pub tools: Vec<ToolReport>,
    /// The OTP installation [`crate::otp::discover`] found, or `None` when
    /// there is none to report.
    pub otp: Option<OtpReport>,
    /// Why there is none, when there is none.
    ///
    /// A machine with no Erlang and a machine whose Erlang cannot be packaged
    /// are both `otp: null`, and only this field tells them apart. Discovery
    /// failing is a reported decision, never a silent one, so it is `None`
    /// exactly when [`Report::otp`] is `Some`.
    pub otp_error: Option<String>,
    /// The Gleam project `doctor` was run inside, when it was run in one.
    pub project: Option<ProjectReport>,
    /// One row per target the project or the host asks for.
    pub targets: Vec<TargetProbe>,
}

impl Report {
    /// Probes the current environment.
    ///
    /// Every probe is bounded by [`PROBE_TIMEOUT`]; a program that hangs is
    /// killed and reported as present but without a version. The bound covers
    /// reading the program's output as well as waiting for it, so a probe that
    /// leaves a background process holding the pipes cannot stall the report
    /// either — see [`crate::process::run_with_timeout`].
    pub fn gather() -> Self {
        Self::gather_from(
            &PROBES,
            std::env::var_os("PATH").as_deref(),
            &EnvSnapshot::from_env(),
            otp::discover(None)
                .map(|info| OtpReport::of(&info))
                .map_err(|error| error.to_string()),
            std::env::current_dir()
                .ok()
                .and_then(|cwd| project_context(&cwd, SystemTime::now())),
            {
                let (targets, config) = std::env::current_dir()
                    .ok()
                    .and_then(|cwd| project_targets(&cwd))
                    .unwrap_or_else(|| (vec![Target::host()], BTreeMap::new()));
                probe_targets(&targets, &config)
            },
        )
    }

    /// Builds a report from an explicit environment.
    ///
    /// This is the half that is unit-tested: it reads neither `PATH` nor the
    /// process environment, so a test can hand it a temporary directory of fake
    /// programs and a fixed [`EnvSnapshot`]. [`Report::gather`] is the thin
    /// wrapper that captures the real ones.
    fn gather_from(
        probes: &[Probe],
        path_var: Option<&OsStr>,
        env: &EnvSnapshot,
        otp: Result<OtpReport, String>,
        project: Option<ProjectReport>,
        targets: Vec<TargetProbe>,
    ) -> Self {
        let (otp, otp_error) = match otp {
            Ok(report) => (Some(report), None),
            Err(reason) => (None, Some(reason)),
        };
        let tools = probes
            .iter()
            .map(|probe| probe_tool(probe, path_var))
            .collect();

        let (cache_dir, cache_dir_source, cache_dir_error) = match cache_dir::resolve(env) {
            Ok(resolved) => (Some(resolved.path), Some(resolved.source.variable()), None),
            Err(error) => (None, None, Some(error.to_string())),
        };
        let cache_probe = cache_dir.as_deref().map(probe_cache_dir);

        Self {
            format_version: FORMAT_VERSION,
            host_target: Target::host(),
            rustc_required: false,
            cache_dir,
            cache_dir_source,
            cache_dir_error,
            cache_probe,
            tools,
            otp,
            otp_error,
            project,
            targets,
        }
    }

    /// Renders the human-readable report, one subject per line.
    pub fn render_text(&self) -> String {
        let mut lines = vec![
            format!("host target: {}", self.host_target),
            "rustc/cargo: not required (neither ginary nor its artifacts need a Rust toolchain)"
                .to_owned(),
            match (&self.cache_dir, self.cache_dir_source) {
                (Some(path), Some(source)) => {
                    format!("cache dir: {} (from {source})", path.display())
                }
                _ => format!(
                    "cache dir: unresolved ({})",
                    self.cache_dir_error.as_deref().unwrap_or("unknown reason")
                ),
            },
        ];
        if let Some(probe) = &self.cache_probe {
            lines.extend(block(&probe.render()));
        }
        lines.extend(self.tools.iter().map(ToolReport::render));
        lines.push(match (&self.otp, &self.otp_error) {
            (Some(otp), _) => otp.render(),
            (None, Some(reason)) => format!("otp: unusable ({reason})"),
            (None, None) => "otp: not found".to_owned(),
        });
        if let Some(crypto) = self.otp.as_ref().and_then(|otp| otp.crypto.as_ref()) {
            lines.extend(block(&crypto.render()));
        }
        if let Some(project) = &self.project {
            lines.push(String::new());
            lines.extend(block(&project.render()));
        }
        if !self.targets.is_empty() {
            lines.push(String::new());
            lines.extend(block(&render_targets(&self.targets)));
        }
        lines.push(String::new());
        lines.join("\n")
    }
}

/// One rendered block, as the lines [`Report::render_text`] joins.
///
/// Every `render` in this module ends its last line with a newline, which is
/// what makes each one usable on its own; the report joins lines instead, so
/// the terminator is taken off here rather than each block being written twice.
fn block(text: &str) -> Vec<String> {
    text.trim_end_matches('\n')
        .lines()
        .map(str::to_owned)
        .collect()
}

/// Looks a program up on `PATH` and, if present, asks it for its version.
fn probe_tool(probe: &Probe, path_var: Option<&OsStr>) -> ToolReport {
    let Some(path) = find_in_path(probe.name, path_var) else {
        return ToolReport {
            name: probe.name.to_owned(),
            found: false,
            version: None,
            path: None,
        };
    };

    let version = match run_with_timeout(&path, probe.args, PROBE_TIMEOUT) {
        Ok(output) if output.success => (probe.parse)(&output.stdout),
        Ok(_) | Err(_) => None,
    };

    ToolReport {
        name: probe.name.to_owned(),
        found: true,
        version,
        path: Some(path),
    }
}

/// Parses `gleam --version`, which prints `gleam <semver>`.
fn parse_gleam_version(stdout: &str) -> Option<String> {
    last_token_of_first_line(stdout)
}

/// Parses the OTP release and ERTS version printed by the `erl` probe.
///
/// The probe prints exactly `<otp_release> <erts_version>`, for example
/// `29 17.0.5`.
fn parse_erl_version(stdout: &str) -> Option<String> {
    let line = stdout.lines().next()?;
    let mut tokens = line.split_whitespace();
    let release = tokens.next()?;
    let erts = tokens.next()?;
    Some(format!("OTP {release}, erts {erts}"))
}

/// Parses `strip --version`, whose first line ends with the binutils version.
fn parse_strip_version(stdout: &str) -> Option<String> {
    last_token_of_first_line(stdout)
}

/// Parses `docker version --format {{.Server.Version}}`, a bare version.
fn parse_docker_version(stdout: &str) -> Option<String> {
    let line = stdout.lines().next()?.trim();
    (!line.is_empty()).then(|| line.to_owned())
}

/// Returns the last whitespace-separated token of the first non-empty line.
fn last_token_of_first_line(stdout: &str) -> Option<String> {
    stdout
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())?
        .split_whitespace()
        .next_back()
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::*;

    #[cfg(unix)]
    use crate::process::test_support::script;

    #[test]
    fn gleam_version_is_the_trailing_token() {
        assert_eq!(
            parse_gleam_version("gleam 1.18.1\n").as_deref(),
            Some("1.18.1")
        );
    }

    #[test]
    fn strip_version_is_the_trailing_token_of_the_banner() {
        assert_eq!(
            parse_strip_version("GNU strip (GNU Binutils for Ubuntu) 2.42\nCopyright (C) 2024\n")
                .as_deref(),
            Some("2.42")
        );
    }

    #[test]
    fn erl_version_combines_release_and_erts() {
        assert_eq!(
            parse_erl_version("29 17.0.5").as_deref(),
            Some("OTP 29, erts 17.0.5")
        );
    }

    #[test]
    fn erl_version_needs_both_fields() {
        assert_eq!(parse_erl_version("29"), None);
        assert_eq!(parse_erl_version(""), None);
    }

    #[test]
    fn docker_version_is_the_bare_line() {
        assert_eq!(parse_docker_version("29.7.2\n").as_deref(), Some("29.7.2"));
    }

    #[test]
    fn empty_output_yields_no_version() {
        assert_eq!(parse_docker_version("\n"), None);
        assert_eq!(parse_gleam_version(""), None);
        assert_eq!(parse_strip_version("   \n"), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_missing_tool_is_reported_as_not_found() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path_var = std::env::join_paths([dir.path()]).expect("join paths");
        let report = probe_tool(&PROBES[0], Some(&path_var));
        assert_eq!(
            report,
            ToolReport {
                name: "gleam".to_owned(),
                found: false,
                version: None,
                path: None,
            }
        );
        assert_eq!(report.render(), "gleam: not found");
    }

    #[cfg(unix)]
    #[test]
    fn a_tool_that_fails_is_found_without_a_version() {
        let dir = tempfile::tempdir().expect("tempdir");
        script(dir.path(), "gleam", "exit 3");
        let path_var = std::env::join_paths([dir.path()]).expect("join paths");
        let report = probe_tool(&PROBES[0], Some(&path_var));
        assert!(report.found);
        assert_eq!(report.version, None);
        assert!(
            report
                .render()
                .starts_with("gleam: found, version unknown (")
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_working_tool_reports_its_version_and_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        script(dir.path(), "gleam", "echo gleam 1.18.1");
        let path_var = std::env::join_paths([dir.path()]).expect("join paths");
        let report = probe_tool(&PROBES[0], Some(&path_var));
        assert!(report.found);
        assert_eq!(report.version.as_deref(), Some("1.18.1"));
        assert!(report.render().starts_with("gleam: 1.18.1 ("));
    }

    #[test]
    fn the_erl_probe_cannot_drop_a_crash_dump_in_the_working_directory() {
        let erl = PROBES
            .iter()
            .find(|probe| probe.name == "erl")
            .expect("an erl probe");
        let guard = erl
            .args
            .iter()
            .position(|arg| *arg == "-env")
            .expect("the erl probe sets an emulator environment variable");
        assert_eq!(erl.args.get(guard + 1).copied(), Some("ERL_CRASH_DUMP"));
        assert_eq!(erl.args.get(guard + 2).copied(), Some(NULL_DEVICE));
    }

    #[test]
    fn the_probe_list_is_the_documented_one() {
        let names: Vec<&str> = PROBES.iter().map(|probe| probe.name).collect();
        assert_eq!(names, ["gleam", "erl", "strip", "docker"]);
    }

    #[test]
    fn the_text_report_has_one_line_per_subject() {
        let report = Report {
            format_version: FORMAT_VERSION,
            host_target: Target::host(),
            rustc_required: false,
            cache_dir: Some(PathBuf::from("/home/u/.cache/ginary")),
            cache_dir_source: Some("HOME"),
            cache_dir_error: None,
            cache_probe: None,
            otp: None,
            otp_error: None,
            project: None,
            targets: Vec::new(),
            tools: vec![ToolReport {
                name: "gleam".to_owned(),
                found: false,
                version: None,
                path: None,
            }],
        };
        let text = report.render_text();
        assert!(text.contains(&format!("host target: {}\n", Target::host())));
        assert!(text.contains("rustc/cargo: not required"));
        assert!(text.contains("cache dir: /home/u/.cache/ginary (from HOME)\n"));
        assert!(text.contains("gleam: not found\n"));
        assert!(text.ends_with("otp: not found\n"), "{text}");
    }

    /// Regression for the A1a review: `gather` dropped the `OtpError`, so an
    /// Erlang that is present but unusable rendered exactly like no Erlang at
    /// all and every actionable message the `otp` module carries was
    /// unreachable.
    #[test]
    fn a_failed_discovery_renders_the_reason_it_failed() {
        let report = Report::gather_from(
            &[],
            None,
            &cache_snapshot("/srv/ginary-cache"),
            Err("`/opt/broken` has no `erts-*` directory".to_owned()),
            None,
            Vec::new(),
        );
        assert_eq!(report.otp, None);
        assert_eq!(
            report.otp_error.as_deref(),
            Some("`/opt/broken` has no `erts-*` directory")
        );
        assert!(
            report
                .render_text()
                .contains("otp: unusable (`/opt/broken` has no `erts-*` directory)"),
            "{}",
            report.render_text()
        );
    }

    #[test]
    fn a_successful_discovery_records_no_reason() {
        let report = Report::gather_from(
            &[],
            None,
            &cache_snapshot("/srv/ginary-cache"),
            Ok(OtpReport {
                root: PathBuf::from("/opt/otp"),
                release: 29,
                erts_vsn: "17.0.5".to_owned(),
                otp_version: "29.0.5".to_owned(),
                crypto: None,
            }),
            None,
            Vec::new(),
        );
        assert_eq!(report.otp_error, None);
        let text = report.render_text();
        assert!(
            text.contains("otp: 29.0.5 (release 29, erts 17.0.5)"),
            "{text}"
        );
        assert!(text.contains("otp root: /opt/otp"), "{text}");
    }

    #[test]
    fn an_unresolved_cache_dir_still_renders_a_line() {
        let report = Report {
            format_version: FORMAT_VERSION,
            host_target: Target::host(),
            rustc_required: false,
            cache_dir: None,
            cache_dir_source: None,
            cache_dir_error: Some("no HOME".to_owned()),
            cache_probe: None,
            otp: None,
            otp_error: None,
            project: None,
            targets: Vec::new(),
            tools: Vec::new(),
        };
        assert!(
            report
                .render_text()
                .contains("cache dir: unresolved (no HOME)")
        );
    }

    /// A snapshot that resolves to `dir` through `GINARY_CACHE_DIR`.
    fn cache_snapshot(dir: &str) -> EnvSnapshot {
        EnvSnapshot {
            ginary_cache_dir: Some(OsString::from(dir)),
            ..EnvSnapshot::default()
        }
    }

    #[cfg(unix)]
    #[test]
    fn gathering_probes_the_given_path_and_never_needs_rustc() {
        let dir = tempfile::tempdir().expect("tempdir");
        script(dir.path(), "gleam", "echo gleam 4.5.6");
        let path_var = std::env::join_paths([dir.path()]).expect("join paths");

        let report = Report::gather_from(
            &PROBES[..1],
            Some(&path_var),
            &cache_snapshot("/srv/ginary-cache"),
            Err("no OTP was looked for".to_owned()),
            None,
            Vec::new(),
        );

        assert!(!report.rustc_required);
        assert_eq!(report.format_version, FORMAT_VERSION);
        assert_eq!(report.host_target, Target::host());
        assert_eq!(report.tools.len(), 1);
        assert_eq!(report.tools[0].name, "gleam");
        assert_eq!(report.tools[0].version.as_deref(), Some("4.5.6"));
    }

    #[test]
    fn gathering_takes_the_cache_directory_from_the_snapshot() {
        let report = Report::gather_from(
            &[],
            None,
            &cache_snapshot("/srv/ginary-cache"),
            Err("no OTP was looked for".to_owned()),
            None,
            Vec::new(),
        );
        assert_eq!(report.cache_dir, Some(PathBuf::from("/srv/ginary-cache")));
        assert_eq!(report.cache_dir_source, Some("GINARY_CACHE_DIR"));
        assert_eq!(report.cache_dir_error, None);
        assert!(report.tools.is_empty());
    }

    #[test]
    fn gathering_records_why_the_cache_directory_is_unresolved() {
        let report = Report::gather_from(
            &[],
            None,
            &EnvSnapshot::default(),
            Err("no OTP".to_owned()),
            None,
            Vec::new(),
        );
        assert_eq!(report.cache_dir, None);
        assert_eq!(report.cache_dir_source, None);
        assert!(report.cache_dir_error.is_some(), "{report:?}");
    }
}
