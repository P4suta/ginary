// SPDX-License-Identifier: MIT OR Apache-2.0
//! The whole build, from a Gleam project to one executable.
//!
//! Every other build-side module answers one question and this one asks them
//! in order: find the project, read its configuration, export the shipment,
//! discover the OTP installation, resolve the closure, stage it, strip it,
//! write the manifest, pack the payload, and append that payload and a trailer
//! to a copy of the ginary binary.
//!
//! ```text
//! gleam::find_project -> config::ProjectConfig -> gleam::export_shipment
//!   -> otp::discover -> closure::app_dependency_closure -> assemble::stage
//!   -> strip::strip -> report::measure -> manifest::Manifest
//!   -> payload::pack -> stub + payload + trailer -> <output>/<app>
//! ```
//!
//! Two decisions are this module's own.
//!
//! **The stub is the running executable, and it may not be an artifact.** A
//! packaged application never reaches the command line at all — its launcher
//! takes every argument — but a copy of one handed to a build through some
//! other route would produce an artifact with two payloads and one trailer.
//! [`check_stub`] refuses that, and it is a separate function precisely so a
//! test can hand it a trailered file without needing a Gleam project.
//!
//! **The work directory is removed whether the build succeeds or not.**
//! Staging happens under `<project>/build/ginary/.work-<pid>/root`, so a
//! failed build leaves the project tree as it found it; `--keep-staging` keeps
//! it and prints where it is.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::assemble::{AssembleError, Category, StageOptions};
use crate::catalog::{CatalogPaths, OtpReq};
use crate::closure::{self, AppSet, ClosureError};
use crate::config::{BuildOptions, ConfigError};
use crate::diag::Diag;
use crate::download::Net;
use crate::error::LauncherError;
use crate::erts_source::{ErtsSourceSpec, SourceContext};
use crate::gleam::{self, GleamError, ProjectDir};
use crate::manifest::{AppRef, LaunchSpec, Manifest, ManifestError, OtpProvenance};
use crate::otp::OtpError;
use crate::payload::PayloadError;
use crate::report::{ReportError, SizeReport};
use crate::strip::{StripError, StripReport};
use crate::stub::StubOpts;
use crate::target::Target;
use crate::trailer::{TRAILER_LEN, Trailer, TrailerError};

/// The prefix of the per-build staging directory, under `build/ginary`.
///
/// The process id is what makes two concurrent builds of one project
/// independent, and what lets a later build recognise the residue of a killed
/// one as residue.
pub const WORK_DIR_PREFIX: &str = ".work-";

/// The staging root inside a work directory.
pub const WORK_STAGE_NAME: &str = "root";

/// The mode the artifact is given.
pub const ARTIFACT_MODE: u32 = 0o755;

/// The program the launcher execs, relative to `erts-<vsn>/bin`.
pub const LAUNCH_PROGRAM: &str = "erlexec";

/// The boot script, relative to the extracted root and without `.boot`.
pub const BOOT_SCRIPT: &str = "bin/no_dot_erlang";

/// Where an `[tools.ginary] vm_args` is copied to inside the artifact.
///
/// Fixed rather than taken from the project: the manifest names a path the
/// launcher joins onto the extracted root, and a value the project chose would
/// be a path from a project reaching into a cache directory.
pub const STAGED_VM_ARGS: &str = "releases/vm.args";

/// Where an `[tools.ginary] sys_config` is copied to.
pub const STAGED_SYS_CONFIG: &str = "releases/sys.config";

/// What `-config` is given: [`STAGED_SYS_CONFIG`] without its extension.
///
/// `erl -config` appends `.config` itself, so passing the suffix would send
/// the runtime looking for `sys.config.config`.
pub const STAGED_CONFIG_ARG: &str = "releases/sys";

/// The program a distributed artifact bundles beyond the required four.
pub const EPMD_BIN: &str = "epmd";

/// The program an artifact under `heart` bundles.
pub const HEART_BIN: &str = "heart";

/// The mode the copied `vm.args` and `sys.config` are given.
///
/// Readable and not executable: they are read by the runtime, not run.
pub const CONFIG_FILE_MODE: u32 = 0o644;

/// What a distributed artifact that names no node is warned about.
///
/// Distribution without `-name` or `-sname` starts a runtime that cannot be
/// reached, which is a build that did half of what was asked and said nothing.
/// It is a warning and not an error because the node name can legitimately
/// come from the environment at run time, through `GINARY_ERL_FLAGS`.
pub const DISTRIBUTION_NO_NAME: &str = "[tools.ginary] distribution is on and neither erl_flags nor the args file names the node \
     with -name or -sname; the runtime will start distribution and have no name to register";

/// The `app_version` recorded when the project declares none.
///
/// `gleam.toml` makes `version` optional and the manifest does not, because
/// the field is what `ginary inspect` prints and what a future upgrade check
/// would compare. A project that declares nothing gets this rather than an
/// empty string, so the value in an artifact is always a version.
pub const UNKNOWN_VERSION: &str = "0.0.0";

/// The closure and staging accounts `--explain` prints before the report.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BuildExplain {
    /// [`crate::closure::explain`] over the resolved application set.
    pub closure: String,
    /// [`crate::assemble::StagedRoot::explain`] over the staged tree.
    pub staged: String,
}

/// One target's half of a build.
///
/// A build produces one artifact per target, and every number below is that
/// artifact's own. The report holds a row per target so that a multi-target
/// build says what each one cost and what runtime each one carries, rather
/// than reporting the first and leaving the rest to be inferred.
#[derive(Clone, Debug, Serialize)]
pub struct TargetBuild {
    /// The target this artifact is for.
    pub target: Target,
    /// The artifact.
    #[serde(serialize_with = "serialize_path")]
    pub out: PathBuf,
    /// The manifest copy written beside it, for a suffixed build.
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_optional_path"
    )]
    pub manifest_copy: Option<PathBuf>,
    /// The stub's length, which is the payload's offset.
    pub stub_len: u64,
    /// The payload's length.
    pub payload_len: u64,
    /// The artifact's length: stub, payload and the 64-byte trailer.
    pub total_len: u64,
    /// The payload's SHA-256, in lower-case hexadecimal.
    pub sha256: String,
    /// What the bundled runtime is and where it came from.
    pub otp: OtpProvenance,
}

impl TargetBuild {
    /// The one line that says what was written and what it is made of.
    pub fn artifact_line(&self) -> String {
        format!(
            "artifact: {} ({} stub + {} payload + {TRAILER_LEN} trailer)",
            self.out.display(),
            self.stub_len,
            self.payload_len
        )
    }

    /// The C library the runtime needs, as the table prints it.
    ///
    /// `gnu 2.38` when a minimum is known, `musl` when the library carries no
    /// symbol versions to derive one from, and `-` on a platform that has one
    /// system C runtime and therefore no choice to record.
    pub fn libc_summary(&self) -> String {
        match &self.otp.libc {
            None => "-".to_owned(),
            Some(libc) => match &libc.min {
                Some(min) => format!("{} {min}", libc.kind),
                None => libc.kind.clone(),
            },
        }
    }
}

/// The table one row per target, as [`BuildReport::render_text`] prints it.
///
/// Six columns: the target, the artifact it was written to, how its runtime is
/// linked, the C library that runtime needs, whether a NIF can be loaded into
/// it, and where the runtime came from.
pub fn render_target_table(targets: &[TargetBuild]) -> String {
    let rows: Vec<[String; 6]> = targets
        .iter()
        .map(|target| {
            [
                target.target.name(),
                target.out.display().to_string(),
                target.otp.linkage.clone(),
                target.libc_summary(),
                if target.otp.nif_loading { "yes" } else { "no" }.to_owned(),
                target.otp.source.clone(),
            ]
        })
        .collect();
    crate::closure::render_table(
        ["target", "artifact", "linkage", "libc", "nif", "erts"],
        &rows,
    )
}

/// What one build produced.
#[derive(Clone, Debug, Serialize)]
pub struct BuildReport {
    /// The application, which is the project name and the artifact's name.
    pub app: String,
    /// The artifact.
    #[serde(serialize_with = "serialize_path")]
    pub out: PathBuf,
    /// The stub's length, which is the payload's offset.
    pub stub_len: u64,
    /// The payload's length.
    pub payload_len: u64,
    /// The artifact's length: stub, payload and the 64-byte trailer.
    pub total_len: u64,
    /// The payload's SHA-256, in lower-case hexadecimal.
    pub sha256: String,
    /// What the strip phase did.
    pub strip: StripReport,
    /// The size breakdown and the `needs:` line.
    pub size_report: SizeReport,
    /// The manifest that was packed.
    pub manifest: Manifest,
    /// One row per target this build produced, in the order they were named.
    pub targets: Vec<TargetBuild>,
    /// The work directory, when `--keep-staging` kept it.
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_optional_path"
    )]
    pub staging: Option<PathBuf>,
    /// What the build could not do, having produced the artifact anyway.
    ///
    /// Empty on an ordinary build. A work directory that could not be removed
    /// is one entry, and a catalogue runtime whose release is further ahead of
    /// this machine's than ginary has tested is another: the artifact is
    /// complete and something about it still has to be said out loud.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    /// The accounts `--explain` asked for.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub explain: Option<BuildExplain>,
}

/// Serialises a path the way every report renders one: lossily.
///
/// serde's own implementation for [`PathBuf`] fails on a path that is not
/// UTF-8, and on Linux that is an ordinary file name. Failing there would fail
/// the *report* of a build that had already written its artifact, so the JSON
/// form uses the same lossy rendering the text form has always used.
fn serialize_path<S: serde::Serializer>(path: &Path, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&path.display().to_string())
}

/// [`serialize_path`] for the optional one, which is skipped when it is
/// [`None`].
fn serialize_optional_path<S: serde::Serializer>(
    path: &Option<PathBuf>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match path {
        Some(path) => serialize_path(path, serializer),
        None => serializer.serialize_none(),
    }
}

impl BuildReport {
    /// The one line that says what was written and what it is made of.
    ///
    /// ```text
    /// artifact: build/ginary/hello_ffi (5242880 stub + 9437184 payload + 64 trailer)
    /// ```
    pub fn artifact_line(&self) -> String {
        format!(
            "artifact: {} ({} stub + {} payload + {TRAILER_LEN} trailer)",
            self.out.display(),
            self.stub_len,
            self.payload_len
        )
    }

    /// The size report's table, the `needs:` line, whatever the build could
    /// not do, and [`Self::artifact_line`].
    ///
    /// The artifact line stays last whatever else is printed, because it is
    /// the line a caller quotes.
    pub fn render_text(&self) -> String {
        let mut text = self.size_report.render_text();
        for warning in &self.warnings {
            text.push_str(&format!("warning: {warning}\n"));
        }
        text.push('\n');
        // One artifact is named on its own line, because that is the line a
        // caller quotes; several are a table, because six facts about each of
        // seven targets is not something a reader parses out of prose.
        if self.targets.len() > 1 {
            text.push_str(&render_target_table(&self.targets));
        } else {
            text.push_str(&self.artifact_line());
            text.push('\n');
        }
        text
    }
}

/// The staging root one build stages into.
///
/// `<project>/build/ginary/.work-<pid>/root`, whatever `--out` says: the work
/// directory belongs to the project rather than to the destination, so an
/// artifact written to `/usr/local/bin` does not stage there.
pub fn work_root(project: &Path, pid: u32) -> PathBuf {
    work_dir(project, pid).join(WORK_STAGE_NAME)
}

/// The per-build work directory, the parent of [`work_root`].
///
/// This is the directory `--keep-staging` keeps and prints, and the one a
/// build removes on its way out however it ended.
pub fn work_dir(project: &Path, pid: u32) -> PathBuf {
    project
        .join(crate::config::DEFAULT_OUTPUT)
        .join(format!("{WORK_DIR_PREFIX}{pid}"))
}

/// Checks that `stub` is the ginary command line tool and returns its length.
///
/// # Errors
///
/// [`BundleError::BundledStub`] when the file already carries a trailer —
/// whole or damaged, because the magic is what says a payload was appended —
/// and [`BundleError::Io`] when the file cannot be opened, stat'd or read.
pub fn check_stub(stub: &Path) -> Result<u64, BundleError> {
    let file = std::fs::File::open(stub).map_err(|source| BundleError::Io {
        what: format!("cannot read the stub at {}", stub.display()),
        source,
    })?;
    let len = file
        .metadata()
        .map_err(|source| BundleError::Io {
            what: format!("cannot stat the stub at {}", stub.display()),
            source,
        })?
        .len();

    // A trailer that is there and *damaged* is refused for the same reason a
    // whole one is: the magic says a payload was appended to this file, and
    // appending a second one would produce an artifact nobody can read. A
    // trailer that could not be *read* is a different fault and gets a
    // different message: "install plain ginary" is no remedy for an EIO.
    match Trailer::read_from(&file) {
        Ok(None) => Ok(len),
        Ok(Some(_)) => Err(BundleError::BundledStub {
            path: stub.to_path_buf(),
        }),
        Err(TrailerError::Io(source)) => Err(BundleError::Io {
            what: format!(
                "cannot read the last {TRAILER_LEN} bytes of {}",
                stub.display()
            ),
            source,
        }),
        Err(_) => Err(BundleError::BundledStub {
            path: stub.to_path_buf(),
        }),
    }
}

/// Builds one artifact out of the project `opts` names.
///
/// The stub is the running executable, found through
/// [`crate::selfexe::open_self`].
///
/// # Errors
///
/// [`BundleError`], wrapping whichever phase failed. Nothing is left behind
/// on any of those paths: the work directory is removed and the artifact is
/// written through a temporary file that is only renamed into place once it is
/// complete.
pub fn build(opts: &BuildOptions, diag: &Diag) -> Result<BuildReport, BundleError> {
    let (_file, path) = crate::selfexe::open_self()?;
    build_with_stub(opts, &path, diag)
}

/// [`build`], with the stub named explicitly.
///
/// The seam the "a bundled executable cannot build" rule is tested through,
/// and the seam a cross-target build will later pass a downloaded stub to.
///
/// # Errors
///
/// As [`build`], plus [`BundleError::BundledStub`] when `stub` is itself a
/// packaged application.
pub fn build_with_stub(
    opts: &BuildOptions,
    stub: &Path,
    diag: &Diag,
) -> Result<BuildReport, BundleError> {
    // First, because it is the cheapest failure and the one whose remedy is
    // "install plain ginary": a build that exported a project and staged a
    // runtime before saying so would have wasted minutes to say it.
    let stub_len = check_stub(stub)?;

    // Second, and for the same reason: everything below is a fault in the
    // command line or in `gleam.toml`, and each of them is cheaper to report
    // than the export that would otherwise come first.
    let targets = build_targets(opts)?;
    let stubs = resolve_stubs(opts, stub, stub_len, &targets)?;
    check_cross_erts(opts, &targets)?;

    let project = ProjectDir::new(opts.root.clone());
    let shipment = if opts.skip_export {
        gleam::existing_shipment(&project)?
    } else {
        gleam::export_shipment(&project, diag)?
    };

    let work = work_dir(&opts.root, std::process::id());
    let outcome = build_each_target(opts, &stubs, &shipment, &work, diag);

    if opts.keep_staging {
        outcome.map(|mut report| {
            report.staging = Some(work);
            report
        })
    } else {
        // However it ended: a failed build leaves the project tree as it found
        // it, and a successful one leaves only the artifact. A removal that
        // does not happen is never fatal and never silent.
        match (remove_work_dir(&work), outcome) {
            (None, outcome) => outcome,
            (Some(warning), Ok(mut report)) => {
                diag.kv("cleanup", &[("warning", &warning)]);
                report.warnings.push(warning);
                Ok(report)
            }
            // The build already failed and its error is what the user is
            // about to read; the residue goes to the recorder, because a
            // second headline would bury the first.
            (Some(warning), Err(error)) => {
                diag.kv("cleanup", &[("warning", &warning)]);
                Err(error)
            }
        }
    }
}

/// Removes one build's work directory, reporting what stopped it.
///
/// Returns [`None`] when the directory is gone and one warning line — naming
/// the directory and what the operating system said — when it is not. A
/// staging tree is tens of megabytes inside the user's project, so a removal
/// that fails on `EACCES`, on `EBUSY` or half way through may not be a
/// removal that is silently dropped; it is also not a reason to fail a build
/// that produced a complete artifact.
pub fn remove_work_dir(work: &Path) -> Option<String> {
    match std::fs::remove_dir_all(work) {
        Ok(()) => None,
        // Already gone is removed: `--keep-staging` is the flag that keeps a
        // work directory, and a build whose staging never got as far as
        // creating one has nothing to report.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => Some(format!(
            "the staging directory {} could not be removed ({error}); it is still in the project",
            work.display()
        )),
    }
}

/// The targets one build produces.
///
/// [`BuildOptions::merge`] resolves at least the host, so a list that is empty
/// here came from options something else assembled, and building the host
/// anyway would produce an artifact nobody asked for. Nothing else is refused
/// here: which stub a target needs is [`resolve_stubs`]'s question and which
/// runtime it needs is [`check_cross_erts`]'s.
///
/// # Errors
///
/// [`BundleError::NoTargets`] when the list is empty.
fn build_targets(opts: &BuildOptions) -> Result<Vec<Target>, BundleError> {
    if opts.targets.is_empty() {
        return Err(BundleError::NoTargets);
    }
    Ok(opts.targets.clone())
}

/// One target and the file its artifact is built on top of.
///
/// Resolved before the project is exported, for the reason
/// [`BundleError::BundledStub`] is: a stub that is missing, is another
/// ginary's or is for another machine is a fault in the command line, and a
/// build that staged a runtime before saying so would have spent minutes to
/// say it.
#[derive(Clone, Debug)]
struct TargetStub {
    /// The target this stub is for.
    target: Target,
    /// The file the artifact starts with.
    path: PathBuf,
    /// Its length, which is the payload's offset.
    len: u64,
}

/// Where [`crate::stub::locate`] may look, for this process.
///
/// `--stub` first, then `GINARY_STUB_DIR`, then the resolved cache root. The
/// cache root is required even when nothing is in it, because a search that
/// silently dropped its last source would report a shorter list of paths than
/// it tried.
///
/// # Errors
///
/// [`BundleError::CacheDir`] when no cache root can be resolved at all.
fn stub_opts(opts: &BuildOptions) -> Result<StubOpts, BundleError> {
    let cache = crate::cache_dir::resolve(&crate::cache_dir::EnvSnapshot::from_env())
        .map_err(BundleError::CacheDir)?;
    Ok(StubOpts {
        explicit: opts.stub.clone(),
        env_dir: std::env::var_os(crate::stub::STUB_DIR_VAR).map(PathBuf::from),
        cache_dir: cache.path,
    })
}

/// The stub every target is built from, resolved and proved.
///
/// The host is the running executable unless `--stub` names something else:
/// it is a ginary of this version, for this target, and it is already open.
/// Every other target is located and then verified, which is where a stub of
/// another ginary, for another machine, or one that is really an artifact is
/// refused by name.
///
/// # Errors
///
/// [`BundleError::Stub`] naming the target and the file, and
/// [`BundleError::CacheDir`] when the search cannot even be described.
fn resolve_stubs(
    opts: &BuildOptions,
    self_stub: &Path,
    self_len: u64,
    targets: &[Target],
) -> Result<Vec<TargetStub>, BundleError> {
    let host = Target::host();
    let mut resolved = Vec::with_capacity(targets.len());
    for target in targets.iter().copied() {
        if target == host && opts.stub.is_none() {
            resolved.push(TargetStub {
                target,
                path: self_stub.to_path_buf(),
                len: self_len,
            });
            continue;
        }
        let search = stub_opts(opts)?;
        let (path, _source) =
            crate::stub::locate(&target, &search).map_err(|source| BundleError::Stub {
                target,
                source: Box::new(source),
            })?;
        crate::stub::verify(&path, &target).map_err(|source| BundleError::Stub {
            target,
            source: Box::new(source),
        })?;
        let len = std::fs::metadata(&path)
            .map_err(|source| BundleError::Io {
                what: format!("cannot stat the stub at {}", path.display()),
                source,
            })?
            .len();
        resolved.push(TargetStub { target, path, len });
    }
    Ok(resolved)
}

/// Refuses a cross target whose runtime nothing names.
///
/// A target other than the host has to be told where its runtime comes from:
/// `erts = "catalog"` to take it out of the prebuilt-OTP catalogue, or a
/// directory or a tarball for one somebody unpacked. Checked before the export
/// for the same reason the stub is: it is a fault in `gleam.toml`, and finding
/// it after `gleam export` would cost minutes.
///
/// The question is whether the target names an `erts`, not whether it has a
/// sub-table: three of that table's four keys — `otp_variant`, `native` and
/// `codesign` — are recorded rather than acted on today, so a table holding
/// only those says nothing about where the runtime comes from and
/// [`crate::config::TargetConfig::erts_spec`] would answer `Host` for it. See
/// `tests/regressions/c2_a_target_sub_table_with_no_erts_passed_the_guard.rs`.
///
/// # Errors
///
/// [`BundleError::CrossErtsNotConfigured`] for the first such target.
fn check_cross_erts(opts: &BuildOptions, targets: &[Target]) -> Result<(), BundleError> {
    if opts.otp_root.is_some() {
        return Ok(());
    }
    let host = Target::host();
    for target in targets.iter().copied() {
        let named = opts
            .target_config
            .get(&target.name())
            .is_some_and(|config| config.erts.is_some());
        if target != host && !named {
            return Err(BundleError::CrossErtsNotConfigured { target });
        }
    }
    Ok(())
}

/// Where one target's runtime comes from.
///
/// The target's own `[tools.ginary.target.<name>] erts`, unless `--otp-root`
/// named a directory: a flag the user typed just now wins over the project's
/// own configuration, as every other flag in [`BuildOptions::merge`] does, and
/// what it names is a runtime root, which is exactly a `dir:` source.
///
/// # Errors
///
/// [`BundleError::Config`] carrying [`ConfigError::ErtsSource`] when the
/// configured value is not one of the five spellings, which
/// [`crate::config::ToolsConfig::validate_targets`] refuses before a build can
/// reach this.
fn erts_spec_for(
    opts: &BuildOptions,
    target: Target,
) -> Result<crate::erts_source::ErtsSourceSpec, BundleError> {
    if let Some(root) = &opts.otp_root {
        return Ok(crate::erts_source::ErtsSourceSpec::Dir(root.clone()));
    }
    let name = target.name();
    opts.erts_spec(target).map_err(|error| {
        BundleError::Config(ConfigError::ErtsSource {
            path: opts.root.join(crate::gleam::MANIFEST_NAME),
            value: opts
                .target_config
                .get(&name)
                .and_then(|config| config.erts.clone())
                .unwrap_or_default(),
            target: name,
            reason: error.to_string(),
        })
    })
}

/// What the catalogue and tarball sources need, gathered once per build.
///
/// [`None`] when every target's runtime comes from a directory or from this
/// machine's own OTP: resolving a cache root and asking `erl` which release it
/// is would then be work for an answer nothing reads, and a machine with no
/// writable cache would fail a build that needs no cache.
struct RuntimeSources {
    /// Where a catalogue may be read from.
    catalog_paths: CatalogPaths,
    /// The root of the OTP cache.
    cache_root: PathBuf,
    /// Whether this build may fetch, and where the bases point.
    net: Net,
    /// The release the shipment is compiled by.
    host_release: u32,
    /// Which version of OTP the build asks the catalogue for.
    otp_version: OtpReq,
}

/// Gathers [`RuntimeSources`] when at least one target needs a cache.
///
/// The host release comes from the OTP that will compile the shipment, because
/// that is what the version rule is about: a runtime older than the compiler
/// cannot load the modules the compiler produced. It is read once, and only
/// when a catalogue is actually going to be consulted.
///
/// # Errors
///
/// [`BundleError::CacheDir`] when nothing says where the cache is, and
/// [`BundleError::Otp`] when the host installation cannot be found.
fn runtime_sources(
    opts: &BuildOptions,
    stubs: &[TargetStub],
) -> Result<Option<RuntimeSources>, BundleError> {
    let mut needed = false;
    for entry in stubs {
        if opts.otp_root.is_none()
            && matches!(
                opts.erts_spec(entry.target),
                Ok(ErtsSourceSpec::Catalog | ErtsSourceSpec::Tarball(_))
            )
        {
            needed = true;
        }
    }
    if !needed {
        return Ok(None);
    }

    let dirs = crate::cache_dir::resolve(&crate::cache_dir::EnvSnapshot::from_env())?;
    let host_release = crate::otp::discover(None)?.release;
    let cache_root = crate::catalog::cache_root(&dirs.path);
    Ok(Some(RuntimeSources {
        catalog_paths: CatalogPaths {
            explicit: std::env::var_os(crate::catalog::CATALOG_ENV_VAR)
                .filter(|value| !value.is_empty())
                .map(PathBuf::from),
            cache: Some(cache_root.join(crate::catalog::CATALOG_FILE)),
        },
        cache_root,
        net: Net::from_vars(false, &Net::env_vars()),
        host_release,
        otp_version: OtpReq::Host(host_release),
    }))
}

/// The installation whose `bin/erl` strips the staged modules.
///
/// [`None`] when the bundled runtime's own is the right one, which is every
/// build for this machine. A `.beam` is portable and an emulator is not: a
/// runtime cross-built for another target cannot run
/// `beam_lib:strip_files/1` here — its `bin/erl` execs an emulator this kernel
/// will not load — so the *host's* installation strips the modules, which
/// produces the same bytes because the modules are the same modules.
///
/// # Errors
///
/// [`BundleError::Otp`] when the runtime is for another target and the host
/// has no usable installation to strip with. A build that got this far has
/// already run `gleam export`, so an `erl` that cannot be found here is a
/// machine that changed under the build rather than a configuration to work
/// around.
fn beam_stripper(
    erts: &crate::erts_source::ResolvedErts,
) -> Result<Option<crate::otp::OtpInfo>, BundleError> {
    if erts.target == Target::host() {
        return Ok(None);
    }
    Ok(Some(crate::otp::discover(None)?))
}

/// Builds one artifact per target and folds them into one report.
///
/// The report's own `strip`, `size_report`, `manifest` and `explain` are the
/// first target's, because they are the ones every earlier milestone printed
/// and the ones a caller already reads; [`BuildReport::targets`] holds a row
/// for each, in the order the targets were named. [`BuildReport::warnings`] is
/// the exception and is every target's: a warning is something a build could
/// not do, and dropping the second target's would be the silent skip
/// `CLAUDE.md` forbids. A runtime's own warnings —
/// [`crate::erts_source::ResolvedErts::warnings`], which carries the version
/// guard's "further ahead than ginary has tested" — join them there rather
/// than stopping at the recorder. A build with more than one target prefixes
/// each warning with the target that raised it, so a line naming a runtime
/// file says which artifact is missing it.
///
/// Sequential, and every target stages into the same work root: the tree one
/// target staged is packed and measured before the next replaces it, which is
/// what lets the whole build have one work directory and one removal site.
///
/// # Errors
///
/// The first failure, from whichever phase and whichever target raised it. A
/// build that wrote one target's artifact and failed on the next reports the
/// failure: a partial multi-target build is not a build that succeeded.
/// [`BundleError::NoTargets`] when `targets` is empty, which [`build_targets`]
/// has already refused.
fn build_each_target(
    opts: &BuildOptions,
    stubs: &[TargetStub],
    shipment: &Path,
    work: &Path,
    diag: &Diag,
) -> Result<BuildReport, BundleError> {
    let mut whole: Option<BuildReport> = None;
    let mut rows: Vec<TargetBuild> = Vec::with_capacity(stubs.len());
    let mut warnings: Vec<String> = Vec::new();
    let attributed = stubs.len() > 1;
    let sources = runtime_sources(opts, stubs)?;

    for entry in stubs {
        let target = &entry.target;
        let spec = erts_spec_for(opts, *target)?;
        let erts = {
            let _phase = diag.phase("erts");
            match &sources {
                Some(sources) => crate::erts_source::resolve_in(
                    &spec,
                    target,
                    &SourceContext {
                        catalog_paths: &sources.catalog_paths,
                        cache_root: &sources.cache_root,
                        net: &sources.net,
                        host_release: sources.host_release,
                        otp_version: &sources.otp_version,
                        variant: opts
                            .target_config
                            .get(&target.name())
                            .and_then(|config| config.otp_variant.as_deref()),
                        diag,
                    },
                )?,
                None => crate::erts_source::resolve(&spec, target)?,
            }
        };
        diag.kv(
            "erts",
            &[
                ("target", &target.name()),
                ("source", &erts.provenance),
                ("linkage", erts.linkage.as_str()),
            ],
        );

        let set = {
            let _phase = diag.phase("closure");
            closure::app_dependency_closure(
                shipment,
                &erts.otp.lib,
                std::slice::from_ref(&opts.app),
                &opts.otp_applications,
            )?
        };
        diag.kv("closure", &[("apps", &set.len().to_string())]);

        let job = TargetJob {
            target: *target,
            erts: &erts,
            set: &set,
        };
        let mut report = assemble_and_write(opts, &job, &entry.path, entry.len, work, diag)?;
        // The runtime's own warnings are this target's warnings. They go in
        // ahead of the ones the assembly raised, because a runtime a user was
        // warned about is what every later line is about, and they take the
        // same target attribution as everything else below.
        report.warnings.splice(0..0, erts.warnings.iter().cloned());
        rows.extend(report.targets.iter().cloned());
        warnings.extend(
            std::mem::take(&mut report.warnings)
                .into_iter()
                .map(|warning| {
                    if attributed {
                        format!("{}: {warning}", target.name())
                    } else {
                        warning
                    }
                }),
        );
        if whole.is_none() {
            whole = Some(report);
        }
    }

    match whole {
        Some(mut report) => {
            report.targets = rows;
            report.warnings = warnings;
            Ok(report)
        }
        None => Err(BundleError::NoTargets),
    }
}

/// One target's inputs, as [`build_each_target`] resolved them.
///
/// Three values that travel together and are meaningless apart: the target, the
/// runtime that was resolved *for* it, and the closure that was taken over that
/// runtime's library directory.
struct TargetJob<'a> {
    /// The target this artifact is for.
    target: Target,
    /// The runtime that was resolved for it, and what reading it found.
    erts: &'a crate::erts_source::ResolvedErts,
    /// The applications, resolved against [`TargetJob::erts`]'s library.
    set: &'a AppSet,
}

/// Stages, strips, measures, packs and writes one target's artifact.
///
/// Split from [`build_with_stub`] so that the work directory has exactly one
/// removal site, on every path out of the build rather than on each of the
/// eight that can fail.
fn assemble_and_write(
    opts: &BuildOptions,
    job: &TargetJob<'_>,
    stub: &Path,
    stub_len: u64,
    work: &Path,
    diag: &Diag,
) -> Result<BuildReport, BundleError> {
    let TargetJob { target, erts, set } = *job;
    let otp = &erts.otp;
    let out = opts.artifact_path(target);
    let root = work.join(WORK_STAGE_NAME);
    let mut staged = {
        let _phase = diag.phase("stage");
        crate::assemble::stage(
            set,
            otp,
            &StageOptions {
                extra_bins: runtime_bins(opts),
                remove_junk: true,
                force: true,
            },
            &root,
        )?
    };

    // Before stripping and before the tree is measured: the two files are part
    // of the artifact, so they belong in the size report and in the listing the
    // payload is packed from.
    let warnings = stage_runtime_files(opts, &mut staged, diag)?;

    let stripper = beam_stripper(erts)?;
    let strip_report = {
        let _phase = diag.phase("strip");
        crate::strip::strip(staged.root(), stripper.as_ref().unwrap_or(otp), &opts.strip)?
    };

    // Measured against the listing staging wrote, which still holds the
    // pre-strip sizes; the refresh below is what replaces them.
    let size_report = crate::report::measure(&staged, &strip_report, staged.root())?;
    let staged = staged.refresh()?;

    let manifest = manifest_for(opts, target, erts, set)?;
    let (payload_len, sha256) = {
        let _phase = diag.phase("pack");
        write_artifact(opts, &out, stub, stub_len, staged.root(), &manifest)?
    };

    let manifest_copy = opts.manifest_copy_path(target);
    if let Some(copy) = &manifest_copy {
        write_manifest_copy(copy, &manifest)?;
    }

    let explain = opts.explain.then(|| BuildExplain {
        closure: closure::explain(set),
        staged: staged.explain(),
    });

    Ok(BuildReport {
        app: opts.app.clone(),
        out: out.clone(),
        stub_len,
        payload_len,
        total_len: stub_len
            .saturating_add(payload_len)
            .saturating_add(TRAILER_LEN),
        sha256: sha256.clone(),
        strip: strip_report,
        size_report,
        manifest,
        targets: vec![TargetBuild {
            target,
            out,
            manifest_copy,
            stub_len,
            payload_len,
            total_len: stub_len
                .saturating_add(payload_len)
                .saturating_add(TRAILER_LEN),
            sha256,
            otp: erts.provenance_block(),
        }],
        staging: None,
        warnings,
        explain,
    })
}

/// Writes `<out>-<target>.json`, the manifest a suffixed build copies out.
///
/// The same document the artifact carries as `ginary.json`, pretty-printed so
/// that a person reading a directory of artifacts for machines they cannot run
/// can read it, and so that CI does not have to open an executable for another
/// architecture to find out what is in it.
///
/// # Errors
///
/// [`BundleError::ManifestCopy`] naming the file and what stopped it.
fn write_manifest_copy(path: &Path, manifest: &Manifest) -> Result<(), BundleError> {
    let mut text =
        serde_json::to_string_pretty(manifest).map_err(|error| BundleError::ManifestCopy {
            path: path.to_path_buf(),
            reason: error.to_string(),
        })?;
    text.push('\n');
    std::fs::write(path, text).map_err(|error| BundleError::ManifestCopy {
        path: path.to_path_buf(),
        reason: error.to_string(),
    })
}

/// The programs to stage beyond the required four.
///
/// The project's own `erts_extra_bins`, plus the one each runtime setting
/// implies: a distributed artifact needs `epmd` and one under `heart` needs
/// `heart`. Asking for a program twice stages it once.
fn runtime_bins(opts: &BuildOptions) -> Vec<String> {
    let mut bins = opts.erts_extra_bins.clone();
    let mut want = |name: &str| {
        if !bins.iter().any(|existing| existing == name) {
            bins.push(name.to_owned());
        }
    };
    if opts.distribution {
        want(EPMD_BIN);
    }
    if opts.heart {
        want(HEART_BIN);
    }
    bins
}

/// Copies `vm_args` and `sys_config` into the staged tree, checking both.
///
/// Each file must exist, and each is checked for what it holds before it is
/// copied: an args file may not carry a flag the launcher passes itself, and a
/// `sys.config` must be the one list of terms `file:consult/1` reads. Both
/// checks happen here rather than in [`crate::config`] because both are
/// questions about a file, and everything in that module is pure.
///
/// Returns the warnings the settings earned — today that is the one about a
/// distributed artifact that names no node.
///
/// # Errors
///
/// [`BundleError::RuntimeFile`] when a named file is not there or is not
/// usable, and [`BundleError::Assemble`] when it cannot be written into the
/// staged tree.
fn stage_runtime_files(
    opts: &BuildOptions,
    staged: &mut crate::assemble::StagedRoot,
    diag: &Diag,
) -> Result<Vec<String>, BundleError> {
    let mut vm_args_text = None;
    if let Some(path) = &opts.vm_args {
        let text = read_runtime_file(opts, "vm_args", path)?;
        crate::config::lint_args_file(&text, path).map_err(BundleError::RuntimeFile)?;
        staged.add_file(
            STAGED_VM_ARGS,
            text.as_bytes(),
            CONFIG_FILE_MODE,
            Category::Other,
        )?;
        diag.kv("vm_args", &[("from", &path.display().to_string())]);
        vm_args_text = Some(text);
    }

    if let Some(path) = &opts.sys_config {
        let text = read_runtime_file(opts, "sys_config", path)?;
        crate::config::validate_sys_config(&text, path).map_err(BundleError::RuntimeFile)?;
        staged.add_file(
            STAGED_SYS_CONFIG,
            text.as_bytes(),
            CONFIG_FILE_MODE,
            Category::Other,
        )?;
        diag.kv("sys_config", &[("from", &path.display().to_string())]);
    }

    Ok(runtime_warnings(opts, vm_args_text.as_deref()))
}

/// The warnings one build's runtime settings earn, given its args file.
///
/// Pure, and separate from [`stage_runtime_files`] for that reason: what a
/// build says about a setting is a rule about two lists of strings, and a rule
/// that can only be reached by staging a tree is a rule nothing checks.
fn runtime_warnings(opts: &BuildOptions, vm_args: Option<&str>) -> Vec<String> {
    let mut warnings = Vec::new();
    if opts.distribution && !names_a_node(opts, vm_args) {
        warnings.push(DISTRIBUTION_NO_NAME.to_owned());
    }
    warnings
}

/// Reads one file `[tools.ginary]` names, or says which key named it.
///
/// The two failures are told apart because they send a user to two different
/// places: a name that resolves to nothing is a mistake in `gleam.toml`, and a
/// file that is there and cannot be read is a mistake in the file. A `sys.config`
/// written in Latin-1 is the case that made the difference matter — it is legal
/// Erlang, `filename_encoding = "latin1"` is a setting ginary offers, and being
/// told the file does not exist would send its author looking for the wrong
/// thing.
fn read_runtime_file(
    opts: &BuildOptions,
    key: &'static str,
    path: &Path,
) -> Result<String, BundleError> {
    let manifest = opts.root.join(crate::gleam::MANIFEST_NAME);
    std::fs::read_to_string(path).map_err(|error| {
        BundleError::RuntimeFile(match error.kind() {
            std::io::ErrorKind::NotFound => ConfigError::MissingFile {
                path: manifest,
                key,
                value: path.display().to_string(),
                missing: path.to_path_buf(),
            },
            _ => ConfigError::UnreadableFile {
                path: manifest,
                key,
                value: path.display().to_string(),
                file: path.to_path_buf(),
                reason: unreadable_reason(&error),
            },
        })
    })
}

/// Why a file that is there could not be read, as a phrase.
///
/// `InvalidData` is spelled out because the operating system had nothing to do
/// with it: it is `read_to_string` refusing bytes that are not UTF-8, and
/// "stream did not contain valid UTF-8" is not a sentence about a file the user
/// can go and look at.
fn unreadable_reason(error: &std::io::Error) -> String {
    match error.kind() {
        std::io::ErrorKind::InvalidData => {
            "it is not valid UTF-8, and ginary reads a configuration file as text".to_owned()
        }
        _ => error.to_string(),
    }
}

/// Whether anything this build passes names the distributed node.
///
/// `erl_flags` and the args file are the two places a `-name` or an `-sname`
/// can come from at build time. `GINARY_ERL_FLAGS` is a third and is not
/// knowable here, which is why the absence of a name is a warning.
fn names_a_node(opts: &BuildOptions, vm_args: Option<&str>) -> bool {
    let is_name = |token: &str| token == "-name" || token == "-sname";
    if opts.erl_flags.iter().any(|flag| is_name(flag)) {
        return true;
    }
    vm_args.is_some_and(|text| {
        crate::config::tokenize_args_file(text)
            .iter()
            .any(|token| is_name(&token.text))
    })
}

/// The manifest for one target, derived from what was actually staged.
fn manifest_for(
    opts: &BuildOptions,
    target: Target,
    erts: &crate::erts_source::ResolvedErts,
    set: &AppSet,
) -> Result<Manifest, BundleError> {
    let otp = &erts.otp;
    let otp_applications: Vec<AppRef> = set
        .otp_apps()
        .into_iter()
        .map(|app| AppRef {
            name: app.name.clone(),
            vsn: app.vsn.clone(),
        })
        .collect();
    let gleam_applications: Vec<String> = set
        .shipment_apps()
        .into_iter()
        .map(|app| app.name.clone())
        .collect();

    // Every shipment application is on the code path, the packaged one first:
    // it is the application whose `-eval` runs and whose `priv` the program
    // reads, and the rest stay in the name order the closure produced.
    let root_entry = format!("lib/{}/ebin", opts.app);
    let mut pa: Vec<String> = gleam_applications
        .iter()
        .map(|name| format!("lib/{name}/ebin"))
        .collect();
    pa.sort_by_key(|entry| *entry != root_entry);

    let created_at = crate::manifest::created_at(
        &crate::manifest::EnvSnapshot::from_env(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_secs()),
    )?;

    Ok(Manifest {
        format_version: crate::manifest::FORMAT_VERSION,
        app: opts.app.clone(),
        app_version: opts
            .app_version
            .clone()
            .unwrap_or_else(|| UNKNOWN_VERSION.to_owned()),
        gleam_version: gleam::gleam_version(),
        otp_release: otp.release,
        otp_version: otp.otp_version.clone(),
        erts_version: otp.erts_vsn.clone(),
        // Every field of it read off the emulator that is being packed, which
        // is what makes the block a record rather than a restatement of the
        // configuration; see [`crate::erts_source`].
        otp: erts.provenance_block(),
        target,
        otp_applications,
        gleam_applications,
        launch: LaunchSpec {
            program: LAUNCH_PROGRAM.to_owned(),
            bindir: format!("erts-{}/bin", otp.erts_vsn),
            boot: BOOT_SCRIPT.to_owned(),
            pa,
            eval: format!("'{0}@@main':run('{0}')", opts.app),
            erl_flags: opts.erl_flags.clone(),
            // Every one of these is additive and every one has a serde
            // default, which is what keeps `format_version` at 1: an artifact
            // this build writes still parses in a launcher that predates them.
            args_file: opts.vm_args.as_ref().map(|_| STAGED_VM_ARGS.to_owned()),
            config: opts
                .sys_config
                .as_ref()
                .map(|_| STAGED_CONFIG_ARG.to_owned()),
            distribution: opts.distribution,
            filename_encoding: opts.filename_encoding.clone(),
            heart: opts.heart,
            env: opts.env.clone(),
        },
        // Nothing is recorded yet: `native.rs` is Phase C, and an empty list
        // is what `docs/format.md` says an artifact with no declared native
        // objects carries.
        native: Vec::new(),
        created_at,
        ginary_version: env!("CARGO_PKG_VERSION").to_owned(),
        extra: BTreeMap::new(),
    })
}

/// Writes `<stub bytes><payload><trailer>` and renames it onto the output.
///
/// The whole artifact is built in a temporary file *in the output directory*,
/// so the rename that publishes it cannot cross a filesystem and cannot leave
/// a half-written executable behind: the destination either does not exist or
/// is a complete artifact.
fn write_artifact(
    opts: &BuildOptions,
    out: &Path,
    stub: &Path,
    stub_len: u64,
    staging: &Path,
    manifest: &Manifest,
) -> Result<(u64, String), BundleError> {
    let dir = out.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir).map_err(|source| BundleError::Io {
        what: format!("cannot create the output directory {}", dir.display()),
        source,
    })?;

    let mut temp = tempfile::NamedTempFile::new_in(dir).map_err(|source| BundleError::Io {
        what: format!("cannot create a temporary file in {}", dir.display()),
        source,
    })?;

    let packed = {
        let mut writer = std::io::BufWriter::new(temp.as_file_mut());
        let mut source = std::fs::File::open(stub).map_err(|source| BundleError::Io {
            what: format!("cannot read the stub at {}", stub.display()),
            source,
        })?;
        let copied = std::io::copy(&mut source, &mut writer).map_err(|source| BundleError::Io {
            what: format!("cannot copy the stub at {}", stub.display()),
            source,
        })?;
        if copied != stub_len {
            return Err(BundleError::StubChanged {
                path: stub.to_path_buf(),
                expected: stub_len,
                actual: copied,
            });
        }

        if crate::fault::point("pack") == Some("fail") {
            return Err(BundleError::Fault { point: "pack" });
        }

        let packed = crate::payload::pack(staging, manifest, opts.compression_level, &mut writer)?;
        let trailer = Trailer {
            payload_offset: stub_len,
            payload_len: packed.len,
            payload_sha256: packed.sha256,
        };
        writer
            .write_all(&trailer.to_bytes())
            .map_err(|source| BundleError::Io {
                what: "cannot write the trailer".to_owned(),
                source,
            })?;
        writer.flush().map_err(|source| BundleError::Io {
            what: "cannot flush the artifact".to_owned(),
            source,
        })?;
        packed
    };

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        temp.as_file()
            .set_permissions(std::fs::Permissions::from_mode(ARTIFACT_MODE))
            .map_err(|source| BundleError::Io {
                what: "cannot make the artifact executable".to_owned(),
                source,
            })?;
    }

    temp.persist(out).map_err(|error| BundleError::Io {
        what: format!("cannot write the artifact to {}", out.display()),
        source: error.error,
    })?;

    Ok((packed.len, hex::encode(packed.sha256)))
}

/// Why a build did not produce an artifact.
#[derive(Debug, thiserror::Error)]
pub enum BundleError {
    /// One target's stub could not be found or could not be proved.
    ///
    /// Boxed because [`crate::stub::StubError`] carries a search list and this
    /// enum is returned by value from every phase of a build.
    #[error("cannot use a stub for {target}")]
    Stub {
        /// The target a stub was wanted for.
        target: Target,
        /// Which of the search's or the proof's gates refused it.
        #[source]
        source: Box<crate::stub::StubError>,
    },
    /// The stub search could not even be described.
    #[error("cannot decide where to look for a stub")]
    CacheDir(#[from] crate::cache_dir::CacheDirError),
    /// A cross target names no runtime, and there is no catalogue yet.
    ///
    /// The stub half of a cross build works; the runtime half still has to be
    /// pointed at a tree for the target, because the signed catalogue that
    /// would fetch one arrives with a later milestone. Bundling the host's own
    /// runtime instead would produce an artifact that cannot start on the
    /// machine it names.
    #[error(
        "no runtime is configured for {target}: a cross build needs \
         `[tools.ginary.target.\"{target}\"] erts = \"catalog\"` in gleam.toml, with a \
         catalog that holds {target} (see `ginary otp list`), or the same table with \
         `erts = \"dir:<a runtime root for {target}>\"`, or `tarball:<file>`, or `--otp-root`"
    )]
    CrossErtsNotConfigured {
        /// The target with no runtime named for it.
        target: Target,
    },
    /// The runtime for one target could not be resolved.
    #[error("cannot resolve the runtime to bundle")]
    Erts(#[from] crate::erts_source::ErtsError),
    /// The build resolved no target at all.
    ///
    /// [`BuildOptions::merge`] cannot produce it: `--target`, then
    /// `[tools.ginary] targets`, then the host, and the last of those always
    /// answers. A caller that assembles the options itself can, and this is an
    /// error rather than a silent host build because an artifact nobody asked
    /// for is worse than a build that stops.
    #[error(
        "this build names no target; `--target` or `[tools.ginary] targets` has to name at \
         least one"
    )]
    NoTargets,
    /// The manifest copy beside a suffixed artifact could not be written.
    #[error("cannot write the manifest copy to {path}: {reason}")]
    ManifestCopy {
        /// The file that could not be written.
        path: PathBuf,
        /// What stopped it.
        reason: String,
    },
    /// The project's configuration is not usable.
    #[error("cannot read the project configuration")]
    Config(#[from] ConfigError),
    /// A file `[tools.ginary]` names is missing or is not what it claims.
    ///
    /// Separate from [`BundleError::Config`] because the headline has to be:
    /// the manifest parsed, and what did not hold up is a file it pointed at.
    #[error("cannot use the runtime configuration the project names")]
    RuntimeFile(#[source] ConfigError),
    /// The project could not be found, or its shipment could not be obtained.
    ///
    /// Neutral about *how* the shipment was to be obtained, because
    /// `--skip-export` reuses one rather than exporting it and a headline
    /// saying "cannot export" would contradict the flag the user typed.
    #[error("cannot obtain the Gleam shipment")]
    Gleam(#[from] GleamError),
    /// The OTP installation is missing or unusable.
    #[error("cannot use the OTP installation")]
    Otp(#[from] OtpError),
    /// An application the project needs could not be resolved.
    #[error("cannot resolve the application closure")]
    Closure(#[from] ClosureError),
    /// The staging root could not be built.
    #[error("cannot stage the runtime")]
    Assemble(#[from] AssembleError),
    /// Stripping failed. A missing tool is a reported skip, not this.
    #[error("cannot strip the staged tree")]
    Strip(#[from] StripError),
    /// The manifest could not be produced.
    #[error("cannot write the manifest")]
    Manifest(#[from] ManifestError),
    /// The payload could not be packed.
    #[error("cannot pack the payload")]
    Payload(#[from] PayloadError),
    /// The size report could not be measured.
    #[error("cannot measure the staged tree")]
    Report(#[from] ReportError),
    /// The stub is itself a packaged application.
    #[error(
        "{path} is a packaged application: a bundled executable cannot build; install plain \
         ginary"
    )]
    BundledStub {
        /// The file that already carries a trailer.
        path: PathBuf,
    },
    /// A file could not be read, written or renamed.
    #[error("{what}")]
    Io {
        /// What was being done, as a sentence naming the path.
        what: String,
        /// What the operating system said.
        #[source]
        source: std::io::Error,
    },
    /// The running executable could not be opened to copy as the stub.
    #[error("cannot open the running executable to copy as the stub")]
    SelfExe(#[from] LauncherError),
    /// The stub changed length between being checked and being copied.
    #[error("{path} changed while it was being copied: {expected} bytes became {actual}")]
    StubChanged {
        /// The file that changed.
        path: PathBuf,
        /// The length [`check_stub`] measured.
        expected: u64,
        /// The length that was actually copied.
        actual: u64,
    },
    /// A fault point armed through `GINARY_FAULT` aborted the build.
    ///
    /// Reachable only under the `fault-injection` feature: the variant is
    /// always compiled, and the [`crate::fault::point`] call that returns it
    /// is what the feature turns from a constant [`None`] into a read of the
    /// environment.
    #[error("the build was aborted at the `{point}` fault point (GINARY_FAULT)")]
    Fault {
        /// The point that fired.
        point: &'static str,
    },
}

#[cfg(test)]
mod tests {
    //! The parts of a build that need neither a project nor a toolchain.
    //!
    //! Three of them are private and stay private: the program list a runtime
    //! setting implies, the question a distributed build asks about its own
    //! flags, and the reading of a file `[tools.ginary]` names. Each is a rule
    //! with no seam an integration test could reach, and each is a rule a
    //! reviewer must be able to see fail.

    use std::path::{Path, PathBuf};

    use super::*;
    use crate::config::{BuildFlags, ProjectConfig};

    /// The options a project whose `[tools.ginary]` is `table` builds with.
    fn options(root: &Path, table: &str) -> BuildOptions {
        let text = format!("name = \"hello\"\n\n[tools.ginary]\n{table}");
        let config = ProjectConfig::from_toml(&text, &root.join("gleam.toml"))
            .expect("the table parses and validates");
        let flags = BuildFlags {
            start: root.to_path_buf(),
            ..BuildFlags::default()
        };
        BuildOptions::merge(root, &config, &flags).expect("the defaults merge")
    }

    // ------------------------------------------------ the program list --

    #[test]
    fn a_plain_build_stages_nothing_beyond_the_required_four() {
        let opts = options(Path::new("/w/hello"), "");

        assert_eq!(runtime_bins(&opts), Vec::<String>::new());
    }

    #[test]
    fn distribution_adds_epmd_and_heart_adds_heart() {
        let root = Path::new("/w/hello");

        assert_eq!(
            runtime_bins(&options(root, "distribution = true\n")),
            vec![EPMD_BIN.to_owned()],
            "a distributed artifact has to carry the daemon it is allowed to start"
        );
        assert_eq!(
            runtime_bins(&options(root, "heart = true\n")),
            vec![HEART_BIN.to_owned()],
            "and one under heart has to carry the program that restarts it"
        );
        assert_eq!(
            runtime_bins(&options(root, "distribution = true\nheart = true\n")),
            vec![EPMD_BIN.to_owned(), HEART_BIN.to_owned()],
            "both settings, both programs"
        );
    }

    #[test]
    fn a_program_the_project_already_asked_for_is_staged_once() {
        let opts = options(
            Path::new("/w/hello"),
            "distribution = true\nerts_extra_bins = [\"epmd\", \"dyn_erl\"]\n",
        );

        assert_eq!(
            runtime_bins(&opts),
            vec!["epmd".to_owned(), "dyn_erl".to_owned()],
            "the project's own order is kept and nothing is asked for twice"
        );
    }

    // ------------------------------------------- the cross runtime rule --

    #[test]
    fn a_host_only_build_needs_no_target_sub_table() {
        let opts = options(Path::new("/w/hello"), "");

        assert!(check_cross_erts(&opts, &[Target::host()]).is_ok());
    }

    #[test]
    fn a_cross_target_with_no_sub_table_is_refused_and_says_what_to_write() {
        // The half of a cross build that is still manual: the stub for another
        // machine can be built today, and the runtime for it cannot be
        // fetched until the catalogue milestone. Bundling the host's own
        // runtime instead would produce an artifact whose name promises a
        // machine it cannot start on, so the build stops and dictates the
        // table to write.
        let target: Target = "linux-aarch64-musl".parse().expect("a target name");
        let opts = options(Path::new("/w/hello"), "");

        let error = check_cross_erts(&opts, &[Target::host(), target])
            .expect_err("a cross target needs a runtime named for it");

        assert!(
            matches!(&error, BundleError::CrossErtsNotConfigured { target: named }
                if *named == target),
            "expected BundleError::CrossErtsNotConfigured, got {error:?}"
        );
        let message = error.to_string();
        assert!(
            message.contains("[tools.ginary.target.\"linux-aarch64-musl\"]")
                && message.contains("erts = \"dir:"),
            "the message dictates the table to write: {message}"
        );
    }

    #[test]
    fn a_cross_target_that_names_its_runtime_passes() {
        let opts = options(
            Path::new("/w/hello"),
            "[tools.ginary.target.linux-aarch64-musl]\nerts = \"dir:/opt/otp-aarch64\"\n",
        );
        let target: Target = "linux-aarch64-musl".parse().expect("a target name");

        assert!(check_cross_erts(&opts, &[target]).is_ok());
    }

    #[test]
    fn a_cross_target_whose_sub_table_names_no_runtime_is_refused() {
        // The table exists and says nothing about the runtime: `otp_variant`
        // is recorded for the catalogue milestone and read by nothing today.
        // A guard that asked whether the sub-table was *there* let this
        // through and left C1's runtime mismatch to raise the alarm minutes
        // later, after the export. See
        // `tests/regressions/c2_a_target_sub_table_with_no_erts_passed_the_guard.rs`.
        let opts = options(
            Path::new("/w/hello"),
            "[tools.ginary.target.linux-aarch64-musl]\notp_variant = \"dynamic\"\n",
        );
        let target: Target = "linux-aarch64-musl".parse().expect("a target name");

        let error = check_cross_erts(&opts, &[target])
            .expect_err("a table that names no runtime names no runtime");

        assert!(
            matches!(&error, BundleError::CrossErtsNotConfigured { target: named }
                if *named == target),
            "expected BundleError::CrossErtsNotConfigured, got {error:?}"
        );
    }

    #[test]
    fn an_otp_root_flag_answers_for_every_target() {
        // `--otp-root` is a runtime root the user typed just now, and it wins
        // over the project's table for every target of the build; a check that
        // ignored it would refuse a build that had already been told where to
        // look.
        let mut opts = options(Path::new("/w/hello"), "");
        opts.otp_root = Some(PathBuf::from("/opt/otp"));
        let target: Target = "linux-aarch64-musl".parse().expect("a target name");

        assert!(check_cross_erts(&opts, &[target]).is_ok());
    }

    // -------------------------------------------------- the node's name --

    #[test]
    fn a_distributed_build_that_names_no_node_earns_one_warning() {
        let opts = options(Path::new("/w/hello"), "distribution = true\n");

        assert_eq!(
            runtime_warnings(&opts, None),
            vec![DISTRIBUTION_NO_NAME.to_owned()],
            "a distributed runtime with no name is a node nothing can reach, and the build \
             says so rather than refusing"
        );
    }

    #[test]
    fn an_sname_in_erl_flags_answers_the_question() {
        let opts = options(
            Path::new("/w/hello"),
            "distribution = true\nerl_flags = [\"-sname\", \"hello\"]\n",
        );

        assert_eq!(runtime_warnings(&opts, None), Vec::<String>::new());
    }

    #[test]
    fn an_sname_in_the_args_file_answers_it_too() {
        // The only case that reads the args file, which is why it is the case
        // a tokenizer change would break silently.
        let opts = options(Path::new("/w/hello"), "distribution = true\n");
        let args_file = "# the node this artifact is\n-sname hello\n+SDio 4\n";

        assert_eq!(
            runtime_warnings(&opts, Some(args_file)),
            Vec::<String>::new()
        );
    }

    #[test]
    fn a_build_that_is_not_distributed_warns_about_nothing() {
        let opts = options(Path::new("/w/hello"), "");

        assert_eq!(runtime_warnings(&opts, None), Vec::<String>::new());
    }

    // --------------------------------------------- reading a named file --

    /// A project root with `config/<name>` in it, holding `bytes`.
    fn project_with(name: &str, bytes: &[u8]) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let config = dir.path().join("config");
        std::fs::create_dir_all(&config).expect("the config directory");
        let path = config.join(name);
        std::fs::write(&path, bytes).expect("write the file");
        (dir, path)
    }

    #[test]
    fn a_file_that_is_not_there_is_reported_as_missing() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let opts = options(dir.path(), "vm_args = \"config/vm.args\"\n");
        let path = dir.path().join("config/vm.args");

        let error =
            read_runtime_file(&opts, "vm_args", &path).expect_err("there is no file at that path");

        let BundleError::RuntimeFile(ConfigError::MissingFile { key, missing, .. }) = &error else {
            panic!("expected ConfigError::MissingFile, got {error:?}");
        };
        assert_eq!(*key, "vm_args");
        assert_eq!(missing, &path);
    }

    #[test]
    fn a_file_that_is_not_utf_8_says_so_rather_than_saying_it_is_missing() {
        // A latin1 `sys.config` is legal Erlang, and `filename_encoding =
        // "latin1"` is one of the three settings ginary offers. Telling that
        // user the file does not exist sends them looking for the wrong thing.
        let (dir, path) = project_with("sys.config", b"[{kernel, [{msg, \"caf\xe9\"}]}].\n");
        let opts = options(dir.path(), "sys_config = \"config/sys.config\"\n");

        let error = read_runtime_file(&opts, "sys_config", &path)
            .expect_err("ginary reads a configuration file as text");

        let BundleError::RuntimeFile(ConfigError::UnreadableFile {
            key, file, reason, ..
        }) = &error
        else {
            panic!("expected ConfigError::UnreadableFile, got {error:?}");
        };
        assert_eq!(*key, "sys_config");
        assert_eq!(file, &path);
        assert!(
            reason.contains("UTF-8"),
            "the reason must be the one the user has to fix: {reason}"
        );
        let message = error.to_string();
        assert!(
            !message.contains("there is no file"),
            "a file that is there is not missing: {message}"
        );
    }

    #[test]
    fn a_directory_where_a_file_was_named_says_what_went_wrong() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = dir.path().join("config");
        std::fs::create_dir_all(&path).expect("the directory");
        let opts = options(dir.path(), "vm_args = \"config\"\n");

        let error = read_runtime_file(&opts, "vm_args", &path)
            .expect_err("a directory is not an args file");

        let BundleError::RuntimeFile(ConfigError::UnreadableFile { file, .. }) = &error else {
            panic!("expected ConfigError::UnreadableFile, got {error:?}");
        };
        assert_eq!(file, &path);
    }
}
