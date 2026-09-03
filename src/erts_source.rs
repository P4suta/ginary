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
//! the real emulator is read and the target, the linkage and the minimum glibc
//! are derived from *it*. A tarball whose name says `aarch64` and whose
//! emulator is an x86-64 binary is caught here, at the one place that looks,
//! rather than by a user whose loader refuses the artifact. Nothing downstream
//! trusts a provenance string.
//!
//! Which emulator is read is a property of the tree, and which *reader* reads
//! it is a property of the emulator. A Windows root — the contents of
//! `otp_win64_<version>.zip` — has no `beam.smp` at all: its `beam.smp.dll` is
//! a PE image `erl.exe` loads, read straight from `object` for the reason
//! `crate::stub` gives, because a PE names no interpreter and the machine
//! field is the whole question. That half is a layout test,
//! [`crate::assemble::is_windows_erts_bin`], so assembly and this module cannot
//! disagree about what a tree is, and a Windows tree handed to a Linux build is
//! a target mismatch rather than a missing `beam.smp`.
//!
//! A *unix* tree cannot be told apart by its layout, because macOS lays
//! `erts-<vsn>/bin` out exactly as Linux does; the only difference a build can
//! observe is that one `beam.smp` is a Mach-O and the other an ELF. So the
//! reader is chosen by the emulator's own first bytes — [`emulator_format`] —
//! and a macOS runtime is read with [`crate::macho`] where a Linux one is read
//! with [`crate::elf`]. Reading the first with the second is what stopped both
//! macOS jobs of the first CI run that ever reached `ginary build`; see
//! `tests/regressions/e7_a_macos_runtime_was_read_as_an_elf.rs`.
//!
//! Four of the five sources are available. `host` and `dir:PATH` need nothing
//! but a path and resolve through [`resolve`]; `catalog` and `tarball:PATH`
//! need a cache root, a catalogue and a network policy, so they resolve
//! through [`resolve_in`], which is given them, and [`resolve`] answers
//! [`ErtsError::NeedsContext`] for both. `docker:IMAGE` parses — so a
//! configuration can be written ahead of the milestone that implements it —
//! and is refused at build time by [`ErtsError::NotYetAvailable`], which names
//! the milestone [`ErtsSourceSpec::milestone`] reports, so that a build and
//! `ginary doctor` never give two answers for one value.
//!
//! **A catalogue is an index, never evidence.** The catalogue arm checks every
//! claim an entry makes — the target, the linkage, the libc — against the
//! emulator that was actually extracted, and [`ErtsError::CatalogClaim`] names
//! both sides of a disagreement.
//!
//! The inspection is injectable. [`resolve_with`] takes the function that reads
//! an ELF, so the plumbing — the provenance strings, the mismatch message, the
//! `nif_loading` rule — is testable against a `FakeOtp` tree on a machine that
//! has no cross-compiled runtime to read.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use object::Object as _;

use crate::catalog::{CatalogError, CatalogPaths, EnsureContext, OtpReq};
use crate::diag::Diag;
use crate::download::Net;
use crate::elf::{ElfError, ElfInfo};
use crate::manifest::{LibcRequirement, OtpProvenance};
use crate::otp::{OtpError, OtpInfo};
use crate::target::{Libc, Linkage, Target};

/// The emulator a unix ERTS installation holds, and the file that is read.
pub const EMULATOR: &str = "beam.smp";

/// The emulator a Windows ERTS installation holds, and the file that is read.
///
/// The unix tree's `beam.smp` is a program `erlexec` execs; the Windows tree's
/// is a DLL `erl.exe` loads into its own process. Both are the runtime itself,
/// so both are what the machine is read off.
pub const WINDOWS_EMULATOR: &str = crate::target::WINDOWS_EMULATOR_DLL;

/// The milestone `docker:IMAGE` arrives with.
pub const IMAGE_MILESTONE: &str = "container image";

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
    ///
    /// Four of the five are here: C3 implemented `catalog` and `tarball:`, and
    /// [`resolve_in`] performs them. Only a runtime copied out of a container
    /// image is still to come.
    pub const fn milestone(&self) -> Option<&'static str> {
        match self {
            Self::Host | Self::Dir(_) | Self::Tarball(_) | Self::Catalog => None,
            Self::Docker(_) => Some(IMAGE_MILESTONE),
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
    /// What resolving this runtime raised that is not an error.
    ///
    /// A catalogue entry whose release is further ahead of this machine's than
    /// ginary has tested is the one entry there is. The build folds these into
    /// [`crate::bundle::BuildReport::warnings`], which is the channel a user
    /// reads; the recorder gets a copy, and a copy is not a report.
    pub warnings: Vec<String>,
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

    /// The same runtime, carrying `warnings`.
    ///
    /// Taken by value so that the resolution is built once and the guard's
    /// sentences are attached to it in one place; the other arms attach none.
    #[must_use]
    pub fn warning_with(mut self, warnings: Vec<String>) -> Self {
        self.warnings = warnings;
        self
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

/// The emulator of a unix runtime root: `<root>/erts-<vsn>/bin/beam.smp`.
pub fn emulator_path(otp: &OtpInfo) -> PathBuf {
    otp.erts_bin.join(EMULATOR)
}

/// The emulator of a Windows runtime root:
/// `<root>/erts-<vsn>/bin/beam.smp.dll`.
pub fn windows_emulator_path(otp: &OtpInfo) -> PathBuf {
    otp.erts_bin.join(WINDOWS_EMULATOR)
}

/// Why a runtime could not be resolved.
#[derive(Debug, thiserror::Error)]
pub enum ErtsError {
    /// The host OTP could not be discovered, or a root is not a runtime.
    #[error("cannot use the runtime")]
    Otp(#[from] OtpError),
    /// The source parses and the milestone that implements it is not here.
    #[error(
        "the ERTS source `{spec}` arrives with the {milestone} milestone; `host`, `dir:PATH`, \
         `tarball:PATH` and `catalog` are available today"
    )]
    NotYetAvailable {
        /// The source, as it was written.
        spec: String,
        /// The milestone it arrives with, as [`ErtsSourceSpec::milestone`]
        /// reports it, so that a build and `ginary doctor` cannot disagree.
        milestone: &'static str,
    },
    /// The source is here and was resolved without what it needs to resolve.
    ///
    /// `catalog` and `tarball:PATH` both fill a cache, so both need a cache
    /// root, a catalogue and a network policy; [`resolve_in`] is given them
    /// and [`resolve`] is not. A build always takes the first, so this reaches
    /// a user only through a caller that describes a runtime rather than
    /// building one.
    #[error(
        "the ERTS source `{spec}` needs a cache root, a catalog and a network policy; it \
         resolves through a build rather than through a bare inspection"
    )]
    NeedsContext {
        /// The source, as it was written.
        spec: String,
    },
    /// The Windows emulator is not a PE image at all.
    #[error(
        "the emulator at {path} is not a Windows PE image ({reason}); a runtime for another \
         target has to be a real cross-built tree, not a stand-in"
    )]
    NotAPeRuntime {
        /// The emulator that was read.
        path: PathBuf,
        /// What the reader said.
        reason: String,
    },
    /// The Windows emulator is a PE for a machine ginary has no name for.
    #[error(
        "the emulator at {path} is a PE for {machine}, which is not a target ginary packages for"
    )]
    UnknownWindowsRuntimeTarget {
        /// The emulator that was read.
        path: PathBuf,
        /// The machine its header named, as `object` spells it.
        machine: String,
    },
    /// The macOS emulator is not a Mach-O binary at all.
    #[error(
        "the emulator at {path} is not a Mach-O binary ({reason}); a runtime for another \
         target has to be a real cross-built tree, not a stand-in"
    )]
    NotAMachoRuntime {
        /// The emulator that was read.
        path: PathBuf,
        /// What the reader said.
        reason: String,
    },
    /// The macOS emulator is a universal binary, so it is more than one
    /// runtime and none of them is the one to bundle.
    ///
    /// An artifact carries one architecture, so a fat emulator has no
    /// `cputype` to read the target off and no single set of bytes to ship.
    /// It is a runtime that has to be thinned before it can be bundled, which
    /// is a thing the user does to their own tree and not a thing ginary does
    /// to somebody's installation behind their back.
    #[error(
        "the emulator at {path} is a universal binary carrying more than one architecture; \
         bundle a runtime built for one target, or thin this one with `lipo -thin`"
    )]
    UniversalRuntime {
        /// The emulator that was read.
        path: PathBuf,
    },
    /// The macOS emulator is a Mach-O for a `cputype` ginary has no target
    /// for.
    #[error(
        "the emulator at {path} is a Mach-O for {cputype}, which is not a target ginary \
         packages for"
    )]
    UnknownMacosRuntimeTarget {
        /// The emulator that was read.
        path: PathBuf,
        /// The `cputype` its header named, as [`crate::macho`] spells it.
        cputype: String,
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
    /// The catalogue could not be read, or held no such runtime.
    #[error("cannot use the catalog")]
    Catalog(#[from] CatalogError),
    /// The catalogue's claim about a runtime is not what the emulator says.
    ///
    /// The catalogue is an index and the emulator is the evidence. A mismatch
    /// is a hard error naming both, because an entry that lies about its
    /// linkage would put `nif_loading: true` into the manifest of a runtime
    /// that cannot `dlopen` anything.
    #[error(
        "the catalog says {entry} has {field} {claimed}, and the emulator at {path} has \
         {actual}; the catalog is an index, not evidence"
    )]
    CatalogClaim {
        /// The entry, as [`catalog_provenance`] spells it.
        entry: String,
        /// Which claim disagrees: `target`, `linkage` or `libc`.
        field: &'static str,
        /// What the catalogue said.
        claimed: String,
        /// What the emulator says.
        actual: String,
        /// The emulator that was read.
        path: PathBuf,
    },
}

/// Which object-file reader an emulator's own first bytes call for.
///
/// The flavour of an OTP tree is a property of the *tree* — a Windows runtime
/// spells its emulator `beam.smp.dll` and keeps an `erl.ini` beside it — but
/// the flavour of a *unix* tree is not: macOS and Linux lay out
/// `erts-<vsn>/bin` identically, and the only difference a build can see is
/// that one `beam.smp` is a Mach-O and the other is an ELF. Reading a macOS
/// runtime with the ELF reader is what killed both macOS jobs of run
/// <https://github.com/P4suta/ginary/actions/runs/33702776627>:
///
/// ```text
/// error: cannot resolve the runtime to bundle
///   caused by: the emulator at .../erts-17.0.5/bin/beam.smp is not an ELF
///   binary (not an ELF file); a runtime for another target has to be a real
///   cross-built tree, not a stand-in
/// ```
///
/// So the reader is chosen by the emulator's own magic, which is the same
/// trust anchor the rest of this module already uses: nothing the
/// configuration spelled is believed about a runtime.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmulatorFormat {
    /// `\x7fELF`: a Linux runtime, read by [`crate::elf`].
    Elf,
    /// A thin or fat Mach-O magic: a macOS runtime, read by [`crate::macho`].
    MachO,
    /// `MZ`: a Windows runtime, read straight from `object`.
    Pe,
}

/// The format of an emulator, from the bytes at the head of the file.
///
/// [`None`] when `head` is shorter than the shortest magic or begins with
/// none of them — a shell script where the emulator belongs, or a text file
/// somebody renamed — which every caller reports as the runtime it could not
/// read rather than guessing at.
pub fn emulator_format(head: &[u8]) -> Option<EmulatorFormat> {
    if crate::elf::is_elf(head) {
        Some(EmulatorFormat::Elf)
    } else if crate::macho::is_macho(head) {
        Some(EmulatorFormat::MachO)
    } else if head.starts_with(PE_MAGIC) {
        Some(EmulatorFormat::Pe)
    } else {
        None
    }
}

/// How many bytes of an emulator [`emulator_format`] needs.
///
/// Four, the longest magic it knows. A shorter file is read as far as it
/// goes, and one too short to carry any magic at all classifies as no format
/// rather than as a guess.
const EMULATOR_MAGIC_LEN: usize = 4;

/// `MZ`, the two bytes every PE image begins with.
///
/// The DOS stub's own magic, which is what a file *starts* with; the `PE\0\0`
/// signature sits at an offset the stub names, and reading it would be
/// parsing rather than classifying.
const PE_MAGIC: &[u8] = b"MZ";

/// The first [`EMULATOR_MAGIC_LEN`] bytes of `path`, or as many as it holds,
/// or an empty vector when it cannot be opened or read at all.
///
/// A file nobody can read classifies as no format at all, which sends
/// [`resolve_with`] down the arm it has always taken for an unreadable
/// emulator: the reader itself reports what it could not read, in its own
/// words, instead of this function inventing a second sentence for the same
/// failure.
fn emulator_magic(path: &Path) -> Vec<u8> {
    use std::io::Read as _;

    let mut head = [0u8; EMULATOR_MAGIC_LEN];
    let Ok(mut file) = std::fs::File::open(path) else {
        return Vec::new();
    };
    let mut filled = 0;
    while filled < head.len() {
        match file.read(&mut head[filled..]) {
            Ok(0) => break,
            Ok(read) => filled += read,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return Vec::new(),
        }
    }
    head[..filled].to_vec()
}

/// Resolves one runtime and checks that it is the one the build asked for.
///
/// `Host` discovers the installation `erl` reports and `Dir` inspects the root
/// it names; both then read the emulator — `beam.smp` on a unix tree,
/// `beam.smp.dll` on a Windows one — and the target, the linkage and the
/// minimum glibc come out of that file rather than out of the spelling.
/// `catalog` and `tarball:PATH` are implemented and need a context this entry
/// point has not got, so both earn [`ErtsError::NeedsContext`]; `docker:IMAGE`
/// earns [`ErtsError::NotYetAvailable`].
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
/// It is the **ELF** seam. A tree
/// [`crate::assemble::is_windows_erts_bin`] recognises is resolved by the
/// Windows arm below, which reads a PE and never calls `inspect`; a Windows
/// fixture therefore holds a real PE image rather than a script.
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
        // The two that are implemented and need a context this entry point
        // does not have. Refused here rather than by a check above the match,
        // so that adding a variant is a compile error in the one place that
        // decides what a source means.
        ErtsSourceSpec::Tarball(_) | ErtsSourceSpec::Catalog => {
            return Err(ErtsError::NeedsContext { spec: spec.label() });
        }
        // The one that is not here yet, refused with the milestone
        // `ErtsSourceSpec::milestone` reports, which is what `ginary doctor`
        // prints for the same value.
        ErtsSourceSpec::Docker(_) => {
            return Err(ErtsError::NotYetAvailable {
                spec: spec.label(),
                milestone: IMAGE_MILESTONE,
            });
        }
    };

    // Which object file is the evidence is a property of the tree, not of the
    // request: a unix runtime's emulator is an ELF program and a Windows
    // runtime's is a DLL, and neither reader can read the other's file. The
    // flavour test is `assemble`'s, so assembly and this function cannot
    // disagree about what a tree is. `inspect` is the ELF seam and is not
    // consulted on this arm; a Windows emulator is read straight from
    // `object`, the way `crate::stub` reads a Windows stub, because a PE names
    // no interpreter and the machine field is the whole question.
    if crate::assemble::is_windows_erts_bin(&otp.erts_bin) {
        return resolve_windows(spec, requested, otp);
    }

    // The trust anchor: from here on nothing the configuration said is used,
    // and every fact comes out of the emulator's own header.
    let emulator = emulator_path(&otp);

    // Which reader reads it is the emulator's own to say. The tree cannot:
    // macOS lays `erts-<vsn>/bin` out exactly as Linux does, so the only
    // difference a build can observe is that one `beam.smp` is a Mach-O and
    // the other an ELF, and handing the first to the ELF reader is what
    // stopped both macOS jobs of the first run that ever reached
    // `ginary build`. Only a Mach-O is diverted: everything else keeps the
    // reader it has always had, an unreadable file included, so that
    // `inspect` stays the one place that says what it could not read.
    if emulator_format(&emulator_magic(&emulator)) == Some(EmulatorFormat::MachO) {
        return resolve_macos(spec, requested, otp, emulator);
    }

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
        // `host` and `dir:` make no claim a guard could disagree with.
        warnings: Vec::new(),
    })
}

/// The macOS arm of [`resolve_with`].
///
/// The same shape as the Windows one and the same trust anchor: the machine
/// comes off the emulator's own Mach-O header rather than off the spelling
/// that named the tree, so an x86-64 runtime in an aarch64 build is
/// [`ErtsError::TargetMismatch`] here rather than a loader error on somebody
/// else's machine.
///
/// Three facts are not read, because a macOS runtime cannot vary in them. The
/// emulator resolves `libSystem` at load time, so the linkage is
/// [`Linkage::Dynamic`] and NIFs load; and macOS has one system C library, so
/// the target's libc is [`Libc::None`] and there is no symbol-version floor to
/// record. That is what [`crate::macho::MachoFacts::target`] already
/// documents: the `cputype` is the whole of the target.
fn resolve_macos(
    spec: &ErtsSourceSpec,
    requested: &Target,
    otp: OtpInfo,
    emulator: PathBuf,
) -> Result<ResolvedErts, ErtsError> {
    let facts = crate::macho::inspect(&emulator).map_err(|source| ErtsError::NotAMachoRuntime {
        path: emulator.clone(),
        reason: source.to_string(),
    })?;
    if facts.is_fat {
        return Err(ErtsError::UniversalRuntime { path: emulator });
    }
    let Some(target) = facts.target else {
        return Err(ErtsError::UnknownMacosRuntimeTarget {
            path: emulator,
            cputype: facts.cputype,
        });
    };
    if target != *requested {
        return Err(ErtsError::TargetMismatch {
            path: otp.root.clone(),
            requested: *requested,
            actual: target,
        });
    }

    Ok(ResolvedErts {
        libc_min: None,
        nif_loading: Linkage::Dynamic.loads_nifs(),
        provenance: match spec {
            ErtsSourceSpec::Host => format!("host:{}", otp.root.display()),
            other => other.label(),
        },
        otp,
        target,
        linkage: Linkage::Dynamic,
        warnings: Vec::new(),
    })
}

/// The Windows arm of [`resolve_with`].
///
/// The same shape as the unix one and the same trust anchor: the machine comes
/// off the emulator's own PE header rather than off the spelling that named
/// the tree, so a Linux tree in a Windows build and an aarch64 tree in an
/// x86-64 build are both [`ErtsError::TargetMismatch`] here rather than a
/// loader error on somebody else's machine.
///
/// Three facts are not read, because a Windows runtime cannot vary in them. It
/// is a set of DLLs `erl.exe` loads, so the linkage is [`Linkage::Dynamic`] and
/// NIFs load; and Windows has one system C runtime, so the target's libc is
/// [`Libc::None`] and there is no version floor to record.
fn resolve_windows(
    spec: &ErtsSourceSpec,
    requested: &Target,
    otp: OtpInfo,
) -> Result<ResolvedErts, ErtsError> {
    let emulator = windows_emulator_path(&otp);
    let arch = read_pe_machine(&emulator)?;
    let target = Target::new(crate::target::Os::Windows, arch, Libc::None);
    if target != *requested {
        return Err(ErtsError::TargetMismatch {
            path: otp.root.clone(),
            requested: *requested,
            actual: target,
        });
    }

    Ok(ResolvedErts {
        libc_min: None,
        nif_loading: Linkage::Dynamic.loads_nifs(),
        provenance: match spec {
            ErtsSourceSpec::Host => format!("host:{}", otp.root.display()),
            other => other.label(),
        },
        otp,
        target,
        linkage: Linkage::Dynamic,
        warnings: Vec::new(),
    })
}

/// The architecture a PE image's COFF header names.
///
/// # Errors
///
/// [`ErtsError::NotAPeRuntime`] when the file cannot be read or is not a PE at
/// all — an ELF `beam.smp` renamed into a Windows tree lands here — and
/// [`ErtsError::UnknownWindowsRuntimeTarget`] when it is a PE for a machine
/// ginary has no target for.
fn read_pe_machine(emulator: &Path) -> Result<crate::target::Arch, ErtsError> {
    let bytes = std::fs::read(emulator).map_err(|source| ErtsError::NotAPeRuntime {
        path: emulator.to_path_buf(),
        reason: source.to_string(),
    })?;
    let file = object::read::File::parse(&*bytes).map_err(|source| ErtsError::NotAPeRuntime {
        path: emulator.to_path_buf(),
        reason: source.to_string(),
    })?;
    let format = file.format();
    let architecture = file.architecture();
    if format != object::BinaryFormat::Pe {
        return Err(ErtsError::NotAPeRuntime {
            path: emulator.to_path_buf(),
            reason: format!("it is {format:?}"),
        });
    }
    match architecture {
        object::Architecture::X86_64 => Ok(crate::target::Arch::X86_64),
        object::Architecture::Aarch64 => Ok(crate::target::Arch::Aarch64),
        other => Err(ErtsError::UnknownWindowsRuntimeTarget {
            path: emulator.to_path_buf(),
            machine: format!("{other:?}"),
        }),
    }
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

/// What the three sources that are not `host` or `dir:` need to resolve.
///
/// Taken rather than read: the catalogue paths, the cache root and the network
/// policy are all decisions the caller has already made, and a resolution that
/// read them itself could not be tested in parallel.
#[derive(Clone, Copy, Debug)]
pub struct SourceContext<'a> {
    /// Where a catalogue may be read from.
    pub catalog_paths: &'a CatalogPaths,
    /// The root of the OTP cache, [`crate::catalog::cache_root`].
    pub cache_root: &'a Path,
    /// Whether this build may fetch, and where the bases point.
    pub net: &'a Net,
    /// The OTP release the shipment was compiled by.
    pub host_release: u32,
    /// Which version of OTP the configuration asked for.
    pub otp_version: &'a OtpReq,
    /// `otp_variant`, when the configuration named one.
    pub variant: Option<&'a str>,
    /// Where the phases are reported.
    pub diag: &'a Diag,
}

/// How a catalogue entry is named in a provenance string and in an error.
///
/// `catalog:<version>/<target>/<variant>`: a label, never a path. The variant
/// is always spelled out, so two entries of one version never read alike in a
/// manifest.
pub fn catalog_provenance(version: &str, target: &str, variant: &str) -> String {
    format!("catalog:{version}/{target}/{variant}")
}

/// [`resolve`], with the context the catalogue and tarball sources need.
///
/// `Host` and `Dir` resolve exactly as [`resolve`] does and ignore the
/// context. `Catalog` selects an entry, ensures it is extracted under the
/// cache root and inspects what came out; `Tarball` extracts the archive into
/// the same cache, keyed by its own SHA-256, and inspects that. Both then go
/// through the one trust anchor, and every claim the catalogue made about the
/// runtime — its target, its linkage, its libc — is checked against the
/// emulator, with [`ErtsError::CatalogClaim`] naming both sides of a
/// disagreement.
///
/// # Errors
///
/// [`ErtsError`].
pub fn resolve_in(
    spec: &ErtsSourceSpec,
    requested: &Target,
    ctx: &SourceContext<'_>,
) -> Result<ResolvedErts, ErtsError> {
    resolve_in_with(spec, requested, ctx, |path| {
        crate::elf::inspect(path).map(|info| ElfFacts::of(&info))
    })
}

/// [`resolve_in`], with the emulator inspection injected.
///
/// The seam [`resolve_with`] already has, carried up to the sources that fill
/// a cache first: a runtime extracted out of a fixture tarball carries a shell
/// script where the emulator belongs, so the catalogue plumbing — the cache
/// key, the provenance, the claim check — is reachable on a machine with no
/// cross-built `beam.smp` on it.
///
/// # Errors
///
/// As [`resolve_in`].
pub fn resolve_in_with(
    spec: &ErtsSourceSpec,
    requested: &Target,
    ctx: &SourceContext<'_>,
    inspect: impl Fn(&Path) -> Result<ElfFacts, ElfError>,
) -> Result<ResolvedErts, ErtsError> {
    match spec {
        // The two sources that need no cache resolve exactly as they did
        // before this milestone, through the one function that reads a root.
        ErtsSourceSpec::Host | ErtsSourceSpec::Dir(_) => resolve_with(spec, requested, inspect),
        ErtsSourceSpec::Catalog => resolve_catalog(requested, ctx, inspect),
        ErtsSourceSpec::Tarball(archive) => {
            let root = crate::catalog::ensure_tarball(archive, ctx.cache_root, ctx.diag)?;
            let otp = crate::otp::inspect_root(&root)?;
            let read = read_emulator(&otp, requested, &inspect)?;
            // A tarball makes no claim of its own, so the emulator is held
            // straight to the target being built.
            if read.target != *requested {
                return Err(ErtsError::TargetMismatch {
                    path: otp.root.clone(),
                    requested: *requested,
                    actual: read.target,
                });
            }
            Ok(read.resolved(otp, spec.label()))
        }
        // The same constant `ErtsSourceSpec::milestone` answers with, so the
        // sentence a build prints and the row `ginary doctor` prints cannot
        // name two different milestones for one value.
        ErtsSourceSpec::Docker(_) => Err(ErtsError::NotYetAvailable {
            spec: spec.label(),
            milestone: IMAGE_MILESTONE,
        }),
    }
}

/// The catalogue arm: choose an entry, fill the cache, and check every claim.
///
/// The order matters. The entry is chosen and extracted first, and *then* its
/// own `beam.smp` is read; nothing the catalogue said is believed until the
/// emulator has agreed with it, and where the two differ the build stops with
/// [`ErtsError::CatalogClaim`] naming both sides.
fn resolve_catalog(
    requested: &Target,
    ctx: &SourceContext<'_>,
    inspect: impl Fn(&Path) -> Result<ElfFacts, ElfError>,
) -> Result<ResolvedErts, ErtsError> {
    let loaded = crate::catalog::Catalog::load(ctx.catalog_paths)?;
    let origin = loaded.origin.label();
    let selected =
        loaded
            .catalog
            .select(ctx.otp_version, &requested.name(), ctx.variant, &origin)?;
    for warning in &selected.warnings {
        ctx.diag.kv("catalog", &[("warning", warning)]);
    }
    let warnings = selected.warnings.clone();

    let entry = catalog_provenance(selected.version, selected.target, selected.variant);
    let root = crate::catalog::ensure_otp(
        &selected,
        &EnsureContext {
            cache_root: ctx.cache_root,
            catalog_dir: loaded.origin.dir(),
            net: ctx.net,
            diag: ctx.diag,
        },
    )?;

    let otp = crate::otp::inspect_root(&root)?;
    let read = read_emulator(&otp, requested, &inspect)?;

    // The catalogue is an index and the emulator is the evidence.
    let claim = |field: &'static str, claimed: String, actual: String| ErtsError::CatalogClaim {
        entry: entry.clone(),
        field,
        claimed,
        actual,
        path: read.emulator.clone(),
    };
    if selected.target != read.target.name() {
        return Err(claim(
            "target",
            selected.target.to_owned(),
            read.target.name(),
        ));
    }
    if selected.entry.linkage != read.linkage.as_str() {
        return Err(claim(
            "linkage",
            selected.entry.linkage.clone(),
            read.linkage.as_str().to_owned(),
        ));
    }
    let libc = match read.linkage {
        // A fully static emulator resolves nothing at load time, so `none` is
        // the only honest claim about its C library.
        Linkage::Static => "none",
        Linkage::Dynamic => read.libc_kind.unwrap_or("none"),
    };
    if selected.entry.libc.kind != libc {
        return Err(claim(
            "libc",
            selected.entry.libc.kind.clone(),
            libc.to_owned(),
        ));
    }

    // The recorder got a copy above, and a copy is not a report: the sentence
    // travels on the resolved runtime so that `bundle` can put it where a user
    // actually reads one.
    Ok(read.resolved(otp, entry).warning_with(warnings))
}

/// What the trust anchor read out of one runtime's emulator.
struct EmulatorRead {
    /// The emulator that was read.
    emulator: PathBuf,
    /// The target it is really for.
    target: Target,
    /// How it is linked.
    linkage: Linkage,
    /// The C library its own interpreter named, absent for a static build.
    libc_kind: Option<&'static str>,
    /// The lowest glibc it will load against, for a dynamic gnu runtime.
    libc_min: Option<String>,
}

impl EmulatorRead {
    /// The runtime this read describes, under one provenance.
    fn resolved(self, otp: OtpInfo, provenance: String) -> ResolvedErts {
        ResolvedErts {
            otp,
            target: self.target,
            linkage: self.linkage,
            libc_min: self.libc_min,
            nif_loading: self.linkage.loads_nifs(),
            provenance,
            warnings: Vec::new(),
        }
    }
}

/// Reads one runtime's emulator and holds it to the target being built.
///
/// The same three steps [`resolve_with`] makes, factored out because the two
/// sources this milestone adds reach them through a cache rather than through a
/// configured directory.
fn read_emulator(
    otp: &OtpInfo,
    requested: &Target,
    inspect: &impl Fn(&Path) -> Result<ElfFacts, ElfError>,
) -> Result<EmulatorRead, ErtsError> {
    let emulator = emulator_path(otp);
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
    let target = elf.resolve(requested.libc);

    Ok(EmulatorRead {
        emulator,
        target,
        linkage,
        libc_kind: elf.target().map(|read| match read.libc {
            Libc::Gnu => "gnu",
            Libc::Musl => "musl",
            Libc::None => "none",
        }),
        libc_min: match (elf.target().map(|read| read.libc), linkage) {
            (Some(Libc::Gnu), Linkage::Dynamic) => facts.glibc_max.clone(),
            _ => None,
        },
    })
}
