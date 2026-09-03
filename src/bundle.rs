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
use crate::manifest::{AppRef, LaunchSpec, Manifest, ManifestError, NativeRef, OtpProvenance};
use crate::native::{self, NativeArtifact, NativeError, ReconcileCtx, TargetNativeCfg};
use crate::otp::OtpError;
use crate::payload::PayloadError;
use crate::report::{ReportError, SizeReport};
use crate::strip::{StripError, StripReport};
use crate::stub::StubOpts;
use crate::target::{Os, Target};
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
///
/// The unix name. [`crate::target::Target::launch_program`] is what a build
/// asks, because a Windows artifact names `erl.exe` instead.
pub const LAUNCH_PROGRAM: &str = crate::target::LAUNCH_PROGRAM;

/// The sentence a Windows build with no Windows runtime is refused with.
///
/// A Windows ERTS tree is `otp_win64_<version>.zip` from `erlang/otp`, and
/// nothing on a Linux or macOS build machine produces one: there is no host
/// runtime to fall back to there and no way to build one. So the refusal names
/// where such a tree comes from rather than what is missing, and a build that
/// already has one unpacked says so with `erts = "dir:<path>"`. A Windows host
/// is the exception [`check_windows_erts`] makes: its own installation is such
/// a tree, so it never reaches this sentence.
pub const WINDOWS_ERTS_FROM_CATALOG: &str =
    "windows ERTS trees arrive with the windows catalog entry";

/// Refuses a Windows build whose runtime cannot be a Windows one.
///
/// `dir:` is the one source that can hold a tree somebody unpacked from the
/// upstream zip, so it is the one source this milestone accepts; every other
/// spelling — the host runtime, a catalogue with no Windows entry in it yet, a
/// Linux tarball, a Docker image — would bundle a runtime that cannot run on
/// the target and would only be found out by whoever ran the artifact.
///
/// A target that is not Windows is always accepted: this check has nothing to
/// say about it.
///
/// `host_os` is the platform the build is *running* on, and it is an argument
/// rather than a `#[cfg]` because it is the whole of what the rule turns on:
/// on a Windows machine the host runtime **is** a Windows ERTS tree, so
/// `erts = "host"` names one and is accepted. Refusing it there was a Linux
/// assumption written into a rule that claims to be about the target.
///
/// # Errors
///
/// [`BundleError::WindowsErtsUnavailable`] naming the source that was asked
/// for and [`WINDOWS_ERTS_FROM_CATALOG`].
pub fn check_windows_erts(
    target: Target,
    spec: &crate::erts_source::ErtsSourceSpec,
    host_os: crate::target::Os,
) -> Result<(), BundleError> {
    if target.os != crate::target::Os::Windows {
        return Ok(());
    }
    if matches!(spec, ErtsSourceSpec::Dir(_)) {
        return Ok(());
    }
    // The host runtime on a Windows machine *is* a Windows ERTS tree, so the
    // one source that could never hold one on Linux is the ordinary answer
    // there. Every other spelling is refused on every host.
    if host_os == crate::target::Os::Windows && matches!(spec, ErtsSourceSpec::Host) {
        return Ok(());
    }
    Err(BundleError::WindowsErtsUnavailable {
        target,
        spec: spec.label(),
    })
}

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

/// The program a distributed artifact bundles beyond the required ones.
///
/// The unix spelling. The build appends the target's [`Target::exe_suffix`],
/// because the value is a file name in the runtime's own `bin` and a Windows
/// tree spells it `epmd.exe`.
pub const EPMD_BIN: &str = "epmd";

/// The program an artifact under `heart` bundles.
///
/// The unix spelling, suffixed for a Windows target the way [`EPMD_BIN`] is.
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
    // `payload::locate` covers a Mach-O stub that already carries the
    // `__GINARY,__payload` section too, the same way it covers the eof
    // trailer every other target's stub is checked by.
    match crate::payload::locate(&file) {
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

    // Once for the whole build, however many targets it produces: the
    // shipment does not change between them, and reading every object under
    // `priv` three times would say the same thing three times.
    let natives = {
        let _phase = diag.phase("native-scan");
        native::scan_shipment(&shipment)?
    };
    diag.kv("native", &[("objects", &natives.len().to_string())]);

    let work = work_dir(&opts.root, std::process::id());
    let outcome = build_each_target(opts, &stubs, &shipment, &natives, &work, diag);

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
    let cache = crate::cache_dir::resolve(
        &crate::cache_dir::EnvSnapshot::from_env(),
        crate::platform::HOST,
    )
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

    let dirs = crate::cache_dir::resolve(
        &crate::cache_dir::EnvSnapshot::from_env(),
        crate::platform::HOST,
    )?;
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
    natives: &[NativeArtifact],
    work: &Path,
    diag: &Diag,
) -> Result<BuildReport, BundleError> {
    let mut whole: Option<BuildReport> = None;
    let mut rows: Vec<TargetBuild> = Vec::with_capacity(stubs.len());
    // The scan's own warnings first, and unattributed: a file under `priv`
    // that begins like an object and will not parse is a fact about the
    // shipment, which every target of this build shares.
    let mut warnings: Vec<String> = natives
        .iter()
        .filter_map(|artifact| artifact.warning.clone())
        .collect();
    let attributed = stubs.len() > 1;
    let sources = runtime_sources(opts, stubs)?;

    for entry in stubs {
        let target = &entry.target;
        let spec = erts_spec_for(opts, *target)?;
        // Before the runtime is fetched rather than after: a catalogue entry
        // downloaded and then found to be the wrong operating system costs the
        // user minutes and tells them nothing they could not have been told
        // from `gleam.toml`.
        check_windows_erts(*target, &spec, crate::platform::HOST)?;
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
            natives,
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
    /// Every native object the shipment holds, scanned once for the build.
    natives: &'a [NativeArtifact],
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
    let TargetJob {
        target,
        erts,
        set,
        natives,
    } = *job;
    let otp = &erts.otp;
    let out = opts.artifact_path(target);
    let root = work.join(WORK_STAGE_NAME);
    let mut staged = {
        let _phase = diag.phase("stage");
        crate::assemble::stage(
            set,
            otp,
            &StageOptions {
                extra_bins: runtime_bins(opts, target),
                remove_junk: true,
                force: true,
            },
            &root,
        )?
    };

    // Before stripping and before the tree is measured: the two files are part
    // of the artifact, so they belong in the size report and in the listing the
    // payload is packed from.
    let mut warnings = stage_runtime_files(opts, &mut staged, diag)?;

    // What this artifact carries, which is the closure and not the whole
    // shipment: an object in an application nothing depends on never travels,
    // so it is not this target's to answer for. See
    // [`crate::native::staged_only`].
    let natives = native::staged_only(natives, staged.root());
    let natives = natives.as_slice();

    // After staging and before stripping: `apply` rewrites files in the staged
    // tree, and a replacement that arrived after `strip` had run would be the
    // one object in the artifact nobody stripped. A cross-architecture
    // replacement is not stripped either way — `strip` skips every file whose
    // machine is not this host's, and says so in its report.
    let replaced = {
        let _phase = diag.phase("native");
        let overrides = opts
            .target_config
            .get(&target.name())
            .map(|config| &config.native);
        let empty = BTreeMap::new();
        let cfg = TargetNativeCfg {
            overrides: overrides.unwrap_or(&empty),
            hooks: &opts.native_hooks,
        };
        let done = native::reconcile(
            natives,
            &ReconcileCtx {
                target: &target,
                erts_nif_loading: erts.nif_loading,
                cfg: &cfg,
                project_root: &opts.root,
                work_dir: work,
                erts_root: &otp.root,
                erts_version: &otp.erts_vsn,
                otp_version: &otp.otp_version,
                allow_mismatch: opts.allow_native_mismatch,
            },
        )?;
        diag.kv(
            "native",
            &[
                ("target", &target.name()),
                ("replaced", &done.replacements.len().to_string()),
            ],
        );
        native::apply(&done.replacements, staged.root())?;
        warnings.extend(done.warnings);
        done.replacements
    };

    let stripper = beam_stripper(erts)?;
    let strip_report = {
        let _phase = diag.phase("strip");
        crate::strip::strip(staged.root(), stripper.as_ref().unwrap_or(otp), &opts.strip)?
    };

    // Measured against the listing staging wrote, which still holds the
    // pre-strip sizes; the refresh below is what replaces them.
    let size_report = crate::report::measure(&staged, &strip_report, staged.root())?;
    let staged = staged.refresh()?;

    let manifest = manifest_for(
        opts,
        target,
        erts,
        set,
        native_manifest_rows(natives, &replaced, staged.root())?,
    )?;
    let (payload_len, sha256) = {
        let _phase = diag.phase("pack");
        write_artifact(opts, target, &out, stub, stub_len, staged.root(), &manifest)?
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

/// The `native` list one artifact's manifest carries.
///
/// Read back off the *staged tree*, after the replacements have been applied
/// and the tree has been stripped, because the manifest is a record of what
/// the artifact holds rather than of what the shipment did: an object an
/// override replaced records the machine of the file that replaced it, and
/// `ginary verify` holds the manifest to exactly that.
///
/// The list handed in is already the staged one — [`crate::native::staged_only`]
/// narrowed it before the reconciliation — and the existence check below is
/// what makes that a property of this function rather than of its caller: a
/// manifest listing a file the payload does not carry is the finding
/// [`crate::verify::Issue::NativeRowMissing`] exists for.
///
/// # Errors
///
/// [`BundleError::Native`] when a staged object cannot be read.
fn native_manifest_rows(
    natives: &[NativeArtifact],
    replaced: &[crate::native::Replacement],
    root: &Path,
) -> Result<Vec<NativeRef>, BundleError> {
    let mut rows = Vec::new();
    for artifact in natives {
        let relative = artifact.staged_path();
        let path = root.join(&relative);
        if !path.is_file() {
            continue;
        }
        let Some(description) = native::describe_object(&path)? else {
            continue;
        };
        let source = replaced
            .iter()
            .find(|replacement| replacement.artifact_rel_path == artifact.rel_path)
            .map(|replacement| replacement.source.label().to_owned());
        rows.push(NativeRef {
            path: relative,
            kind: manifest_native_kind(description.format),
            machine: description
                .facts
                .as_ref()
                .map(|facts| facts.machine.clone()),
            target: description.facts.as_ref().and_then(|facts| facts.target),
            replaced: source.is_some(),
            source,
        });
    }
    Ok(rows)
}

/// The manifest's word for one container format.
///
/// Two enumerations rather than one because they answer different questions:
/// [`crate::native::ObjectFormat`] is what a build reads, and
/// [`crate::manifest::NativeKind`] is a wire value `docs/format.md` fixes.
const fn manifest_native_kind(format: crate::native::ObjectFormat) -> crate::manifest::NativeKind {
    match format {
        crate::native::ObjectFormat::Elf => crate::manifest::NativeKind::Elf,
        crate::native::ObjectFormat::Pe => crate::manifest::NativeKind::Pe,
        crate::native::ObjectFormat::MachO => crate::manifest::NativeKind::Macho,
    }
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

/// The programs to stage beyond the runtime's required ones.
///
/// The project's own `erts_extra_bins`, plus the one each runtime setting
/// implies: a distributed artifact needs `epmd` and one under `heart` needs
/// `heart`. Asking for a program twice stages it once.
///
/// The two names ginary chooses carry [`Target::exe_suffix`], because a name
/// here is a *file name in the runtime's `bin`* and a Windows tree spells them
/// `epmd.exe` and `heart.exe`. Without the suffix a Windows build with
/// `distribution` or `heart` stopped at staging with "the runtime has no
/// `epmd`" — an error that blamed the runtime for a name ginary picked. The
/// project's own names are left exactly as they were written: they name files
/// in a tree the user is looking at, and a suffix appended to somebody else's
/// spelling would be this function guessing.
fn runtime_bins(opts: &BuildOptions, target: Target) -> Vec<String> {
    let mut bins = opts.erts_extra_bins.clone();
    let mut want = |name: &str| {
        let name = format!("{name}{}", target.exe_suffix());
        if !bins.contains(&name) {
            bins.push(name);
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
    native: Vec<NativeRef>,
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
        launch: launch_spec(opts, target, otp, pa),
        // What the artifact ended up carrying, read back off the staged tree
        // after every replacement was applied; see [`native_manifest_rows`].
        native,
        created_at,
        ginary_version: env!("CARGO_PKG_VERSION").to_owned(),
        extra: BTreeMap::new(),
    })
}

/// The launch specification the manifest carries, for one target.
///
/// Its own function because it is the one part of a manifest that differs
/// between targets, and the difference is a single field: `program` is the
/// name of the program the launcher starts the runtime with, which is
/// `erlexec` everywhere and `erl.exe` on Windows. Everything else — the
/// bindir, the boot script, the code path, the flags — is what was staged and
/// what the project asked for, and is the same text on every machine.
///
/// A seam rather than a block inside [`manifest_for`], so that the rule can be
/// checked without a project, a toolchain and a runtime: the value of this
/// field decides whether a packaged application starts at all, and a build
/// that got it wrong is found out on the user's machine.
fn launch_spec(
    opts: &BuildOptions,
    target: Target,
    otp: &crate::otp::OtpInfo,
    pa: Vec<String>,
) -> LaunchSpec {
    LaunchSpec {
        program: target.launch_program().to_owned(),
        bindir: format!("erts-{}/bin", otp.erts_vsn),
        boot: BOOT_SCRIPT.to_owned(),
        pa,
        eval: format!("'{0}@@main':run('{0}')", opts.app),
        erl_flags: opts.erl_flags.clone(),
        // Every one of these is additive and every one has a serde default,
        // which is what keeps `format_version` at 1: an artifact this build
        // writes still parses in a launcher that predates them.
        args_file: opts.vm_args.as_ref().map(|_| STAGED_VM_ARGS.to_owned()),
        config: opts
            .sys_config
            .as_ref()
            .map(|_| STAGED_CONFIG_ARG.to_owned()),
        distribution: opts.distribution,
        filename_encoding: opts.filename_encoding.clone(),
        heart: opts.heart,
        env: opts.env.clone(),
    }
}

/// Writes `<stub bytes><payload><trailer>` and renames it onto the output.
///
/// The whole artifact is built in a temporary file *in the output directory*,
/// so the rename that publishes it cannot cross a filesystem and cannot leave
/// a half-written executable behind: the destination either does not exist or
/// is a complete artifact.
fn write_artifact(
    opts: &BuildOptions,
    target: Target,
    out: &Path,
    stub: &Path,
    stub_len: u64,
    staging: &Path,
    manifest: &Manifest,
) -> Result<(u64, String), BundleError> {
    if target.os == Os::Macos {
        return write_macos_artifact(opts, out, stub, staging, manifest);
    }

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

/// [`write_artifact`]'s darwin arm: the payload is appended inside the stub's
/// `__LINKEDIT` segment, which is grown to hold it, and the finished file is
/// ad-hoc signed so the signature covers it. See [`crate::sign_macos`] and ADR
/// [0016](../../docs/adr/0016-macho-section-payload-and-adhoc-signing.md)
/// for why a Mach-O cannot be built the plain ELF/PE way, and why the payload
/// cannot live in a new section either.
///
/// Unlike [`write_artifact`] above, this does not go through a temporary
/// file and an atomic rename — [`crate::sign_macos::inject_and_sign`] writes
/// `out` directly, so this function creates `out`'s parent directory and
/// makes the result executable itself, the same two things the ELF/PE arm
/// gets from `NamedTempFile` and its own `set_permissions` call. There is no
/// macOS toolchain on this host to build a darwin stub with, so a build
/// reaching this function through `ginary build --target macos-*` end to end
/// has no coverage here; what is checked on this machine is this function
/// directly (`bundle::tests::write_macos_artifact_*`, against the committed
/// Mach-O fixture standing in for a stub, since `write_macos_artifact` reads
/// `stub`'s bytes without asking `stub::verify` to prove them), the injector
/// underneath it (`tests/sign_macos.rs`), and the honest `StubError::NotFound`
/// a `ginary build --target macos-*` gets without a stub at all.
/// `docs/dev/log/D3.md` records this scope as work for the milestone that
/// can actually run a macOS build end to end.
fn write_macos_artifact(
    opts: &BuildOptions,
    out: &Path,
    stub: &Path,
    staging: &Path,
    manifest: &Manifest,
) -> Result<(u64, String), BundleError> {
    let stub_bytes = std::fs::read(stub).map_err(|source| BundleError::Io {
        what: format!("cannot read the stub at {}", stub.display()),
        source,
    })?;

    if crate::fault::point("pack") == Some("fail") {
        return Err(BundleError::Fault { point: "pack" });
    }

    let mut packed_bytes = Vec::new();
    let packed =
        crate::payload::pack(staging, manifest, opts.compression_level, &mut packed_bytes)?;

    let mut payload_with_trailer = Trailer {
        payload_offset: TRAILER_LEN,
        payload_len: packed.len,
        payload_sha256: packed.sha256,
    }
    .to_bytes()
    .to_vec();
    payload_with_trailer.extend_from_slice(&packed_bytes);

    // Mirrors `write_artifact`'s ELF/PE arm above: the output directory is
    // not assumed to exist, since an ordinary first build has not created it
    // yet.
    let dir = out.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir).map_err(|source| BundleError::Io {
        what: format!("cannot create the output directory {}", dir.display()),
        source,
    })?;

    crate::sign_macos::inject_and_sign(
        &stub_bytes,
        &payload_with_trailer,
        out,
        &crate::sign_macos::MacSignCfg {
            codesign: crate::sign_macos::CodeSign::Adhoc,
        },
    )
    .map_err(BundleError::MacSign)?;

    // Mirrors `write_artifact`'s ELF/PE arm above, which chmods its temp
    // file before publishing it: `inject_and_sign` writes `out` with
    // `std::fs::write`, which gives it the platform default (not
    // executable), so the artifact is unusable until this runs.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(out, std::fs::Permissions::from_mode(ARTIFACT_MODE)).map_err(
            |source| BundleError::Io {
                what: "cannot make the artifact executable".to_owned(),
                source,
            },
        )?;
    }

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
    /// A Windows build named a runtime source that cannot hold a Windows tree.
    #[error(
        "cannot bundle a runtime for {target} from `{spec}`: {WINDOWS_ERTS_FROM_CATALOG}, or from a `dir:` source holding one"
    )]
    WindowsErtsUnavailable {
        /// The target that was being built for.
        target: Target,
        /// The source that was asked for, as it was spelled.
        ///
        /// Not called `source`: that is the name `thiserror` reads as the
        /// error this one wraps, and a runtime source is a string the user
        /// wrote rather than a failure underneath.
        spec: String,
    },
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
    /// A macOS artifact's payload section could not be written or ad-hoc
    /// signed.
    #[error("cannot write the macOS payload section")]
    MacSign(#[from] crate::sign_macos::SignMacosError),
    /// The size report could not be measured.
    #[error("cannot measure the staged tree")]
    Report(#[from] ReportError),
    /// The shipment's native code could not be reconciled with the target.
    ///
    /// The headline says which half of the build refused; the cause is the
    /// table naming every object and the `gleam.toml` keys that answer for it.
    #[error("cannot ship the native code this shipment carries")]
    Native(#[from] NativeError),
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
    use crate::target::{Arch, Libc, Os};

    /// The target the program-list tests name.
    ///
    /// Named rather than [`Target::host`]. `runtime_bins` appends the
    /// *target's* executable suffix, so a test that hands it the host asserts
    /// `epmd` on one machine and `epmd.exe` on another — and the machine where
    /// it is wrong is the one nobody runs it on, which is how the first
    /// Windows runner met three failures nobody had written. See
    /// `tests/regressions/e7_the_unit_tests_asked_the_host_what_platform_it_was.rs`;
    /// `a_windows_build_asks_for_the_programs_by_the_names_the_tree_spells`
    /// below is the other half, and it names its target too.
    fn unix() -> Target {
        Target::new(Os::Linux, Arch::X86_64, Libc::Gnu)
    }

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

        assert_eq!(runtime_bins(&opts, unix()), Vec::<String>::new());
    }

    #[test]
    fn distribution_adds_epmd_and_heart_adds_heart() {
        let root = Path::new("/w/hello");

        assert_eq!(
            runtime_bins(&options(root, "distribution = true\n"), unix()),
            vec![EPMD_BIN.to_owned()],
            "a distributed artifact has to carry the daemon it is allowed to start"
        );
        assert_eq!(
            runtime_bins(&options(root, "heart = true\n"), unix()),
            vec![HEART_BIN.to_owned()],
            "and one under heart has to carry the program that restarts it"
        );
        assert_eq!(
            runtime_bins(
                &options(root, "distribution = true\nheart = true\n"),
                unix()
            ),
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
            runtime_bins(&opts, unix()),
            vec!["epmd".to_owned(), "dyn_erl".to_owned()],
            "the project's own order is kept and nothing is asked for twice"
        );
    }

    #[test]
    fn a_windows_build_asks_for_the_programs_by_the_names_the_tree_spells() {
        let windows = Target::new(Os::Windows, Arch::X86_64, Libc::None);

        assert_eq!(
            runtime_bins(
                &options(Path::new("/w/hello"), "distribution = true\nheart = true\n"),
                windows
            ),
            vec!["epmd.exe".to_owned(), "heart.exe".to_owned()],
            "a Windows ERTS tree ships `epmd.exe` and `heart.exe` and never \
             `epmd` or `heart`, so asking for the unsuffixed names stops the \
             build with an error that blames the runtime for a name ginary chose"
        );
        assert_eq!(
            runtime_bins(
                &options(
                    Path::new("/w/hello"),
                    "distribution = true\nerts_extra_bins = [\"epmd.exe\", \"erl_call.exe\"]\n"
                ),
                windows
            ),
            vec!["epmd.exe".to_owned(), "erl_call.exe".to_owned()],
            "and a project that already spelled it out is not asked for twice"
        );
    }

    // ------------------------------------------------ the launch spec --

    /// A runtime as the manifest reads it: nothing but the version is used
    /// here, and the version is what the bindir is named after.
    fn otp_info() -> crate::otp::OtpInfo {
        crate::otp::OtpInfo {
            root: PathBuf::from("/opt/otp"),
            release: 29,
            erts_vsn: "17.0.5".to_owned(),
            otp_version: "29.0.5".to_owned(),
            erts_bin: PathBuf::from("/opt/otp/erts-17.0.5/bin"),
            lib: PathBuf::from("/opt/otp/lib"),
        }
    }

    #[test]
    fn the_manifest_names_the_program_the_target_starts_its_runtime_with() {
        let opts = options(Path::new("/w/hello"), "");
        let windows = Target::new(Os::Windows, Arch::X86_64, Libc::None);

        assert_eq!(
            launch_spec(&opts, windows, &otp_info(), Vec::new()).program,
            "erl.exe",
            "a Windows runtime has no `erlexec`, so a manifest that named one \
             would send the launcher looking for a file the artifact does not \
             carry"
        );
        for target in crate::target::ALL {
            let spec = launch_spec(&opts, target, &otp_info(), Vec::new());
            assert_eq!(
                spec.program,
                target.launch_program(),
                "{} starts its runtime with {}",
                target.name(),
                target.launch_program()
            );
        }
    }

    #[test]
    fn nothing_else_in_the_launch_spec_depends_on_the_target() {
        let opts = options(Path::new("/w/hello"), "distribution = true\nheart = true\n");
        let pa = vec!["lib/hello/ebin".to_owned()];
        let unix = launch_spec(&opts, Target::host(), &otp_info(), pa.clone());
        let windows = launch_spec(
            &opts,
            Target::new(Os::Windows, Arch::X86_64, Libc::None),
            &otp_info(),
            pa,
        );

        assert_eq!(
            LaunchSpec {
                program: unix.program.clone(),
                ..windows
            },
            unix,
            "the program is the whole of the difference, which is what lets one \
             `launch::plan` serve both launchers"
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

    // --------------------------------------------- write_macos_artifact --

    /// The committed real Mach-O fixture, standing in for a darwin stub —
    /// `write_macos_artifact` only reads `stub`'s bytes, it never asks
    /// `stub::verify` to prove them, so the fixture (which carries no ginary
    /// marker) works here the same way it does in `tests/sign_macos.rs`.
    fn macos_stub_path() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/macho/inet_gethost-aarch64-apple-darwin")
    }

    /// The smallest staging root [`write_macos_artifact`] accepts: one
    /// staged file and the listing `payload::pack` reads back.
    fn macos_staging_tree(root: &Path) {
        use crate::assemble::{LISTING_NAME, StageListing, StagedApp, StagedFile, StagedSource};

        let file_path = root.join("lib/hello/ebin/hello.beam");
        std::fs::create_dir_all(file_path.parent().expect("a parent"))
            .expect("create the staging directory");
        let data = b"FOR1\0\0\0\x04BEAM";
        std::fs::write(&file_path, data).expect("write the staged file");

        let listing = StageListing {
            erts_vsn: "17.0.5".to_owned(),
            otp_release: 29,
            otp_version: "29.0.5".to_owned(),
            apps: vec![StagedApp {
                name: "hello".to_owned(),
                vsn: "1.2.3".to_owned(),
                source: StagedSource::Shipment,
                dir: "lib/hello".to_owned(),
                files: 1,
                bytes: data.len() as u64,
            }],
            files: vec![StagedFile {
                path: "lib/hello/ebin/hello.beam".to_owned(),
                size: data.len() as u64,
                mode: 0o644,
                category: Category::GleamBeam,
            }],
        };
        let json = serde_json::to_string_pretty(&listing).expect("serialise the listing");
        std::fs::write(root.join(LISTING_NAME), format!("{json}\n")).expect("write the listing");
    }

    /// The manifest [`macos_staging_tree`] matches.
    fn macos_manifest() -> Manifest {
        use crate::manifest::{LaunchSpec, OtpProvenance};

        Manifest {
            format_version: crate::manifest::FORMAT_VERSION,
            app: "hello".to_owned(),
            app_version: "1.2.3".to_owned(),
            gleam_version: None,
            otp_release: 29,
            otp_version: "29.0.5".to_owned(),
            erts_version: "17.0.5".to_owned(),
            otp: OtpProvenance {
                linkage: "dynamic".to_owned(),
                libc: None,
                nif_loading: true,
                source: "dir:/opt/otp".to_owned(),
            },
            target: "macos-aarch64".parse().expect("a target name"),
            otp_applications: Vec::new(),
            gleam_applications: vec!["hello".to_owned()],
            launch: LaunchSpec {
                program: "erlexec".to_owned(),
                bindir: "erts-17.0.5/bin".to_owned(),
                boot: "bin/no_dot_erlang".to_owned(),
                pa: vec!["lib/hello/ebin".to_owned()],
                eval: "'hello@@main':run('hello')".to_owned(),
                erl_flags: Vec::new(),
                args_file: None,
                config: None,
                distribution: false,
                filename_encoding: crate::config::DEFAULT_FILENAME_ENCODING.to_owned(),
                heart: false,
                env: BTreeMap::new(),
            },
            native: Vec::new(),
            created_at: "2026-08-31T00:00:00Z".to_owned(),
            ginary_version: env!("CARGO_PKG_VERSION").to_owned(),
            extra: BTreeMap::new(),
        }
    }

    #[test]
    fn write_macos_artifact_creates_the_output_directory_when_it_does_not_exist() {
        // `write_artifact`'s ELF/PE arm creates `out`'s parent before writing
        // (see its own `std::fs::create_dir_all` above); the darwin arm did
        // not, so a build whose output directory does not yet exist — the
        // ordinary case for a first build — failed with a raw `NotFound` IO
        // error instead of producing the artifact.
        let dir = tempfile::tempdir().expect("a temporary directory");
        let staging = dir.path().join("staging");
        std::fs::create_dir_all(&staging).expect("the staging root");
        macos_staging_tree(&staging);
        let opts = options(dir.path(), "");
        let out = dir.path().join("build/output/not/yet/created/hello");

        let result =
            write_macos_artifact(&opts, &out, &macos_stub_path(), &staging, &macos_manifest());

        assert!(
            result.is_ok(),
            "expected the output directory to be created, got {:?}",
            result.err()
        );
        assert!(
            out.is_file(),
            "the artifact must exist at {}",
            out.display()
        );
    }

    #[test]
    fn write_macos_artifact_makes_the_output_file_executable() {
        // `write_artifact`'s ELF/PE arm chmods its temp file to
        // `ARTIFACT_MODE` before publishing it; the darwin arm never touched
        // permissions at all, so its output kept whatever default
        // `std::fs::write` gives a new file (0o644 under an ordinary umask,
        // never executable).
        let dir = tempfile::tempdir().expect("a temporary directory");
        let staging = dir.path().join("staging");
        std::fs::create_dir_all(&staging).expect("the staging root");
        macos_staging_tree(&staging);
        let opts = options(dir.path(), "");
        let out_dir = dir.path().join("build/output");
        std::fs::create_dir_all(&out_dir).expect("the output directory");
        let out = out_dir.join("hello");

        write_macos_artifact(&opts, &out, &macos_stub_path(), &staging, &macos_manifest())
            .expect("writing the macOS artifact succeeds");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&out)
                .expect("the artifact's metadata")
                .permissions()
                .mode()
                & 0o7777;
            assert_eq!(
                mode, ARTIFACT_MODE,
                "expected mode {ARTIFACT_MODE:o}, got {mode:o}"
            );
        }
    }
}
