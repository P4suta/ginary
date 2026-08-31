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

use crate::assemble::{AssembleError, StageOptions};
use crate::closure::{self, AppSet, ClosureError};
use crate::config::{BuildOptions, ConfigError};
use crate::diag::Diag;
use crate::error::LauncherError;
use crate::gleam::{self, GleamError, ProjectDir};
use crate::manifest::{AppRef, LaunchSpec, Manifest, ManifestError};
use crate::otp::{OtpError, OtpInfo};
use crate::payload::PayloadError;
use crate::report::{ReportError, SizeReport};
use crate::strip::{StripError, StripReport};
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
    /// The work directory, when `--keep-staging` kept it.
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_optional_path"
    )]
    pub staging: Option<PathBuf>,
    /// What the build could not do, having produced the artifact anyway.
    ///
    /// Empty on an ordinary build. A work directory that could not be removed
    /// is the one entry there is: the artifact is complete and the project
    /// tree is not as the build found it, and both halves have to be said.
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
        text.push_str(&self.artifact_line());
        text.push('\n');
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

    let project = ProjectDir::new(opts.root.clone());
    let shipment = if opts.skip_export {
        gleam::existing_shipment(&project)?
    } else {
        gleam::export_shipment(&project, diag)?
    };

    let otp = {
        let _phase = diag.phase("otp");
        crate::otp::discover(opts.otp_root.as_deref())?
    };

    let set = {
        let _phase = diag.phase("closure");
        closure::app_dependency_closure(
            &shipment,
            &otp.lib,
            std::slice::from_ref(&opts.app),
            &opts.otp_applications,
        )?
    };
    diag.kv("closure", &[("apps", &set.len().to_string())]);

    let work = work_dir(&opts.root, std::process::id());
    let outcome = assemble_and_write(opts, stub, stub_len, &set, &otp, &work, diag);

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

/// Stages, strips, measures, packs and writes the artifact.
///
/// Split from [`build_with_stub`] so that the work directory has exactly one
/// removal site, on every path out of the build rather than on each of the
/// eight that can fail.
fn assemble_and_write(
    opts: &BuildOptions,
    stub: &Path,
    stub_len: u64,
    set: &AppSet,
    otp: &OtpInfo,
    work: &Path,
    diag: &Diag,
) -> Result<BuildReport, BundleError> {
    let root = work.join(WORK_STAGE_NAME);
    let staged = {
        let _phase = diag.phase("stage");
        crate::assemble::stage(
            set,
            otp,
            &StageOptions {
                extra_bins: opts.erts_extra_bins.clone(),
                remove_junk: true,
                force: true,
            },
            &root,
        )?
    };

    let strip_report = {
        let _phase = diag.phase("strip");
        crate::strip::strip(staged.root(), otp, &opts.strip)?
    };

    // Measured against the listing staging wrote, which still holds the
    // pre-strip sizes; the refresh below is what replaces them.
    let size_report = crate::report::measure(&staged, &strip_report, staged.root())?;
    let staged = staged.refresh()?;

    let manifest = manifest_for(opts, otp, set)?;
    let (payload_len, sha256) = {
        let _phase = diag.phase("pack");
        write_artifact(opts, stub, stub_len, staged.root(), &manifest)?
    };

    let explain = opts.explain.then(|| BuildExplain {
        closure: closure::explain(set),
        staged: staged.explain(),
    });

    Ok(BuildReport {
        app: opts.app.clone(),
        out: opts.out.clone(),
        stub_len,
        payload_len,
        total_len: stub_len
            .saturating_add(payload_len)
            .saturating_add(TRAILER_LEN),
        sha256,
        strip: strip_report,
        size_report,
        manifest,
        staging: None,
        warnings: Vec::new(),
        explain,
    })
}

/// The manifest for one build, derived from what was actually staged.
fn manifest_for(opts: &BuildOptions, otp: &OtpInfo, set: &AppSet) -> Result<Manifest, BundleError> {
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
        target: Target::host(),
        otp_applications,
        gleam_applications,
        launch: LaunchSpec {
            program: LAUNCH_PROGRAM.to_owned(),
            bindir: format!("erts-{}/bin", otp.erts_vsn),
            boot: BOOT_SCRIPT.to_owned(),
            pa,
            eval: format!("'{0}@@main':run('{0}')", opts.app),
            erl_flags: opts.erl_flags.clone(),
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
    stub: &Path,
    stub_len: u64,
    staging: &Path,
    manifest: &Manifest,
) -> Result<(u64, String), BundleError> {
    let dir = opts.out.parent().unwrap_or(Path::new("."));
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

    temp.persist(&opts.out).map_err(|error| BundleError::Io {
        what: format!("cannot write the artifact to {}", opts.out.display()),
        source: error.error,
    })?;

    Ok((packed.len, hex::encode(packed.sha256)))
}

/// Why a build did not produce an artifact.
#[derive(Debug, thiserror::Error)]
pub enum BundleError {
    /// The project's configuration is not usable.
    #[error("cannot read the project configuration")]
    Config(#[from] ConfigError),
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
