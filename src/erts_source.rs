// SPDX-License-Identifier: MIT OR Apache-2.0
//! Where the bundled BEAM runtime comes from, and what it really is.
//!
//! An artifact carries an ERTS installation, and there are five places one can
//! come from: the host's own OTP, a directory somebody unpacked, a tarball, the
//! signed catalogue, or a container image. [`ErtsSourceSpec`] is the spelling a
//! `gleam.toml` uses for each; [`resolve`] turns one into a [`ResolvedErts`],
//! which is what the rest of the build reads.
//!
//! **This module is the single trust anchor.** Whatever the runtime came from,
//! the real `beam.smp` is read with [`crate::elf`] and the target, the linkage
//! and the minimum glibc are derived from *it*. A tarball whose name says
//! `aarch64` and whose emulator is an x86-64 binary is caught here, at the one
//! place that looks, rather than by a user whose loader refuses the artifact.
//! Nothing downstream trusts a provenance string.
//!
//! Two of the five sources are available today. `host` and `dir:PATH` resolve;
//! `tarball:PATH`, `catalog` and `docker:IMAGE` parse — so a configuration can
//! be written ahead of the milestone that implements them — and are refused at
//! build time by [`ErtsError::NotYetAvailable`], which names the milestone.
//!
//! The inspection is injectable. [`resolve_with`] takes the function that reads
//! an ELF, so the plumbing — the provenance strings, the mismatch message, the
//! `nif_loading` rule — is testable against a `FakeOtp` tree on a machine that
//! has no cross-compiled runtime to read.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use crate::elf::{ElfError, ElfInfo};
use crate::manifest::{LibcRequirement, OtpProvenance};
use crate::otp::{OtpError, OtpInfo};
use crate::target::{Libc, Linkage, Target};

/// The emulator every ERTS installation holds, and the file that is read.
pub const EMULATOR: &str = "beam.smp";

/// The milestone the three unavailable sources arrive with.
pub const CATALOG_MILESTONE: &str = "catalog";

/// What [`ErtsError::UnknownRuntimeTarget`] prints for an emulator that names
/// no program interpreter.
pub const NO_INTERPRETER: &str = "none";

/// Where one build's BEAM runtime comes from.
///
/// The spelling in `[tools.ginary.target.<name>] erts`, parsed by
/// [`ErtsSourceSpec::from_str`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ErtsSourceSpec {
    /// `host`: the OTP installation `erl` reports.
    ///
    /// `--otp-root` names a directory rather than a discovery, so a build that
    /// passes one resolves [`ErtsSourceSpec::Dir`] instead; see
    /// [`crate::bundle`].
    Host,
    /// `dir:PATH`: a runtime root somebody already unpacked.
    Dir(PathBuf),
    /// `tarball:PATH`: an archive of a runtime root.
    Tarball(PathBuf),
    /// `catalog`: the signed prebuilt-OTP catalogue.
    Catalog,
    /// `docker:IMAGE`: a runtime copied out of a container image.
    Docker(String),
}

impl ErtsSourceSpec {
    /// The spelling this source was written as, which is its provenance.
    pub fn label(&self) -> String {
        match self {
            Self::Host => "host".to_owned(),
            Self::Dir(path) => format!("dir:{}", path.display()),
            Self::Tarball(path) => format!("tarball:{}", path.display()),
            Self::Catalog => "catalog".to_owned(),
            Self::Docker(image) => format!("docker:{image}"),
        }
    }

    /// The milestone this source arrives with, or [`None`] when it is here.
    pub const fn milestone(&self) -> Option<&'static str> {
        match self {
            Self::Host | Self::Dir(_) => None,
            Self::Tarball(_) | Self::Catalog | Self::Docker(_) => Some(CATALOG_MILESTONE),
        }
    }
}

/// Why an `erts` value is not an ERTS source.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SpecError {
    /// The value is not one of the five spellings.
    #[error(
        "expected `host`, `catalog`, `dir:PATH`, `tarball:PATH` or `docker:IMAGE`, not `{value}`"
    )]
    Unknown {
        /// The value that was refused.
        value: String,
    },
    /// A `dir:` or `tarball:` prefix with nothing after it.
    #[error("`{prefix}:` names no path")]
    EmptyPath {
        /// The prefix that named nothing, without its colon.
        prefix: &'static str,
    },
    /// A `docker:` prefix with nothing after it.
    #[error("`docker:` names no image")]
    EmptyImage,
}

impl FromStr for ErtsSourceSpec {
    type Err = SpecError;

    /// Parses `host`, `catalog`, `dir:PATH`, `tarball:PATH` or `docker:IMAGE`.
    ///
    /// # Errors
    ///
    /// [`SpecError`] naming what was expected and what was found.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // The prefixed forms first, and each takes everything after its *first*
        // colon: a Docker tag holds a colon of its own, so splitting on every
        // one would turn `docker:erlang:29-alpine` into an image called
        // `erlang`.
        if let Some(rest) = s.strip_prefix("dir:") {
            return non_empty(rest, "dir").map(|path| Self::Dir(PathBuf::from(path)));
        }
        if let Some(rest) = s.strip_prefix("tarball:") {
            return non_empty(rest, "tarball").map(|path| Self::Tarball(PathBuf::from(path)));
        }
        if let Some(rest) = s.strip_prefix("docker:") {
            return if rest.is_empty() {
                Err(SpecError::EmptyImage)
            } else {
                Ok(Self::Docker(rest.to_owned()))
            };
        }
        match s {
            "host" => Ok(Self::Host),
            "catalog" => Ok(Self::Catalog),
            _ => Err(SpecError::Unknown {
                value: s.to_owned(),
            }),
        }
    }
}

/// The path after a `dir:` or `tarball:` prefix, refusing an empty one.
///
/// A prefix with nothing after it is a value somebody meant to finish: it
/// names no file, and accepting it would turn a half-written setting into a
/// resolution against the current directory.
fn non_empty<'a>(rest: &'a str, prefix: &'static str) -> Result<&'a str, SpecError> {
    if rest.is_empty() {
        Err(SpecError::EmptyPath { prefix })
    } else {
        Ok(rest)
    }
}

/// The facts [`resolve_with`] reads out of an emulator.
///
/// A narrow view of [`ElfInfo`] on purpose: this is the seam a test injects,
/// and a seam that took the whole structure would make a test write fields
/// that decide nothing here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ElfFacts {
    /// The machine, spelled as [`ElfInfo::machine`] spells it.
    pub machine: String,
    /// The program interpreter, absent for a static binary.
    pub interp: Option<String>,
    /// The `DT_NEEDED` entries.
    pub needed: Vec<String>,
    /// The highest `GLIBC_x.y` the emulator requires, without the prefix.
    pub glibc_max: Option<String>,
}

impl ElfFacts {
    /// The four fields of an inspected ELF this module reads.
    pub fn of(info: &ElfInfo) -> Self {
        Self {
            machine: info.machine.clone(),
            interp: info.interp.clone(),
            needed: info.needed.clone(),
            glibc_max: info.glibc_max.clone(),
        }
    }
}

/// One runtime, as the emulator itself describes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedErts {
    /// The runtime root, its release and its ERTS version.
    pub otp: OtpInfo,
    /// The target the emulator is really for.
    pub target: Target,
    /// How the emulator is linked.
    pub linkage: Linkage,
    /// The lowest glibc the emulator will run against, for a gnu target.
    pub libc_min: Option<String>,
    /// Whether a NIF can be loaded into this runtime.
    pub nif_loading: bool,
    /// Where it came from, as [`ErtsSourceSpec::label`] spells it, with the
    /// resolved root appended for the two sources that have one.
    pub provenance: String,
}

impl ResolvedErts {
    /// The C library the runtime needs, or [`None`] when it needs none.
    pub fn libc_kind(&self) -> Option<&'static str> {
        match self.target.libc {
            Libc::Gnu => Some("gnu"),
            Libc::Musl => Some("musl"),
            Libc::None => None,
        }
    }

    /// The block `ginary.json` records about the runtime.
    pub fn provenance_block(&self) -> OtpProvenance {
        OtpProvenance {
            linkage: self.linkage.as_str().to_owned(),
            libc: self.libc_kind().map(|kind| LibcRequirement {
                kind: kind.to_owned(),
                min: self.libc_min.clone(),
            }),
            nif_loading: self.nif_loading,
            source: self.provenance.clone(),
        }
    }
}

/// The emulator of a runtime root: `<root>/erts-<vsn>/bin/beam.smp`.
pub fn emulator_path(otp: &OtpInfo) -> PathBuf {
    otp.erts_bin.join(EMULATOR)
}

/// Why a runtime could not be resolved.
#[derive(Debug, thiserror::Error)]
pub enum ErtsError {
    /// The host OTP could not be discovered, or a root is not a runtime.
    #[error("cannot use the runtime")]
    Otp(#[from] OtpError),
    /// The source parses and the milestone that implements it is not here.
    #[error(
        "the ERTS source `{spec}` arrives with the {milestone} milestone; only `host` and \
         `dir:PATH` are available today"
    )]
    NotYetAvailable {
        /// The source, as it was written.
        spec: String,
        /// The milestone it arrives with.
        milestone: &'static str,
    },
    /// The emulator is not an ELF binary at all.
    #[error(
        "the emulator at {path} is not an ELF binary ({reason}); a runtime for another target \
         has to be a real cross-built tree, not a stand-in"
    )]
    NotAnElfRuntime {
        /// The emulator that was read.
        path: PathBuf,
        /// What the reader said.
        reason: String,
    },
    /// The emulator is an ELF for a machine or a libc ginary has no name for.
    #[error(
        "the emulator at {path} is for machine `{machine}` with interpreter {interp}, which is \
         not a target ginary packages for"
    )]
    UnknownRuntimeTarget {
        /// The emulator that was read.
        path: PathBuf,
        /// The machine its header named.
        machine: String,
        /// Its interpreter, or `none`.
        interp: String,
    },
    /// The emulator is for a target other than the one being built.
    #[error(
        "the runtime at {path} is for {actual}, and this build is for {requested}; build with \
         `--target {actual}`, or point `erts` at a {requested} runtime"
    )]
    TargetMismatch {
        /// The runtime root that was inspected.
        path: PathBuf,
        /// The target the build asked for.
        requested: Target,
        /// The target the emulator is really for.
        actual: Target,
    },
}

/// Resolves one runtime and checks that it is the one the build asked for.
///
/// `Host` discovers the installation `erl` reports and `Dir` inspects the root
/// it names; both then read the emulator, and the target, the linkage and the
/// minimum glibc come out of that file rather than out of the spelling. The
/// other three sources are refused by [`ErtsError::NotYetAvailable`].
///
/// # Errors
///
/// [`ErtsError`], naming the emulator that was read on every path that got as
/// far as reading one.
pub fn resolve(spec: &ErtsSourceSpec, requested: &Target) -> Result<ResolvedErts, ErtsError> {
    resolve_with(spec, requested, |path| {
        crate::elf::inspect(path).map(|info| ElfFacts::of(&info))
    })
}

/// [`resolve`], with the emulator inspection injected.
///
/// The seam the unit tests use: a `FakeOtp` tree holds a shell script where the
/// emulator belongs, so the plumbing above the ELF reader — the provenance
/// strings, the mismatch, the `nif_loading` rule — is reachable on a machine
/// with no cross-built runtime on it.
///
/// # Errors
///
/// As [`resolve`], with whatever `inspect` reports arriving as
/// [`ErtsError::NotAnElfRuntime`].
pub fn resolve_with(
    spec: &ErtsSourceSpec,
    requested: &Target,
    inspect: impl Fn(&Path) -> Result<ElfFacts, ElfError>,
) -> Result<ResolvedErts, ErtsError> {
    let otp = match spec {
        ErtsSourceSpec::Host => crate::otp::discover(None)?,
        ErtsSourceSpec::Dir(path) => crate::otp::inspect_root(path)?,
        // The three that parse and do not resolve. Refused here rather than by
        // a check above the match, so that adding a variant is a compile error
        // in the one place that decides what a source means.
        ErtsSourceSpec::Tarball(_) | ErtsSourceSpec::Catalog | ErtsSourceSpec::Docker(_) => {
            return Err(ErtsError::NotYetAvailable {
                spec: spec.label(),
                milestone: CATALOG_MILESTONE,
            });
        }
    };

    // The trust anchor: from here on nothing the configuration said is used,
    // and every fact comes out of the emulator's own header.
    let emulator = emulator_path(&otp);
    let facts = inspect(&emulator).map_err(|error| ErtsError::NotAnElfRuntime {
        path: emulator.clone(),
        reason: error.to_string(),
    })?;

    let Some(elf) = Target::from_elf(&facts.machine, facts.interp.as_deref()) else {
        return Err(ErtsError::UnknownRuntimeTarget {
            path: emulator,
            machine: facts.machine,
            interp: facts.interp.unwrap_or_else(|| NO_INTERPRETER.to_owned()),
        });
    };

    let linkage = linkage_of(elf, &facts.needed);
    // A static emulator names no C library, so it is reported as the one the
    // build asked for; a dynamic one named its own and `resolve` leaves that
    // alone.
    let target = elf.resolve(requested.libc);
    if target != *requested {
        return Err(ErtsError::TargetMismatch {
            path: otp.root.clone(),
            requested: *requested,
            actual: target,
        });
    }

    Ok(ResolvedErts {
        // Only a runtime whose own interpreter named glibc, and which resolves
        // it at load time, has a floor to report. musl carries no symbol
        // versions to derive one from; a static emulator resolves nothing at
        // load time whatever it was built against; and an emulator that named
        // no interpreter at all had its C library *assumed* above, so a
        // version out of its symbol table would be a minimum recorded for a
        // library nothing read.
        libc_min: match (elf.target().map(|read| read.libc), linkage) {
            (Some(Libc::Gnu), Linkage::Dynamic) => facts.glibc_max.clone(),
            _ => None,
        },
        nif_loading: linkage.loads_nifs(),
        provenance: match spec {
            // `host` is a spelling that resolves to a directory, so the
            // provenance says which one it resolved to on this machine; every
            // other spelling already names its own source.
            ErtsSourceSpec::Host => format!("host:{}", otp.root.display()),
            other => other.label(),
        },
        otp,
        target,
        linkage,
    })
}

/// What the interpreter and the `DT_NEEDED` set together say about linkage.
///
/// The interpreter decides it: a file the kernel hands to a dynamic loader is
/// dynamically linked, and one with no `PT_INTERP` is static. The needed set is
/// the corroboration — a file that names shared libraries and no loader is not
/// a static runtime, whatever its program headers left out, and calling it one
/// would write `nif_loading: false` into a manifest for a runtime that does
/// load them.
fn linkage_of(elf: crate::target::ElfTarget, needed: &[String]) -> Linkage {
    match elf.linkage() {
        Linkage::Static if !needed.is_empty() => Linkage::Dynamic,
        linkage => linkage,
    }
}
