// SPDX-License-Identifier: MIT OR Apache-2.0
//! Finding the stub a target's artifact is built from, and proving it is one.
//!
//! A build for the host uses the running executable: it is a ginary of this
//! version, for this target, and it is right there. A build for any other
//! target needs a *stub* — the same ginary, cross-compiled, usually with
//! `--no-default-features` so that it carries the launcher and nothing else.
//!
//! [`locate`] answers where one comes from, in a fixed order:
//!
//! 1. `--stub PATH`, which is an instruction rather than a search;
//! 2. `$GINARY_STUB_DIR/ginary-stub-<version>-<target>`, then
//!    `$GINARY_STUB_DIR/ginary-<version>-<target>`, which is what
//!    `mise run stubs:build` fills;
//! 3. the running executable, when the target is the host;
//! 4. `<cache>/stubs/<version>/<target>`, where a downloaded stub is kept.
//!
//! Nothing below that is a search: a release download arrives with the
//! catalogue milestone, so a target with no stub anywhere is
//! [`StubError::NotFound`], naming every path that was tried.
//!
//! [`verify`] is the other half, and it is deliberately paranoid. A stub is
//! *version-locked*: the launcher in it reads the payload this ginary writes,
//! so a stub from another ginary is refused by name rather than trusted to be
//! compatible. Then the marker's target, then the file itself — an ELF or PE
//! header that disagrees with the marker means the marker was copied, not
//! built — and finally the trailer: a file that already carries one is an
//! artifact, and appending a second payload to it would produce a file nobody
//! can read.

use std::fmt;
use std::path::{Path, PathBuf};

use object::Object as _;

use crate::stubid::{self, StubId, StubIdError};
use crate::target::{ElfTarget, Os, Target};
use crate::trailer::{Trailer, TrailerError};

/// The version of this ginary, which every stub it uses has to carry.
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The environment variable that names a directory of prebuilt stubs.
pub const STUB_DIR_VAR: &str = "GINARY_STUB_DIR";

/// The subdirectory of the cache root that holds downloaded stubs.
pub const CACHE_SUBDIR: &str = "stubs";

/// The largest file [`verify`] will read into memory.
///
/// A stub is one to five megabytes. Sixty-four is room for a debug build and
/// still a bound, so that pointing `--stub` at a disk image reports a refusal
/// rather than filling the machine's memory.
pub const MAX_STUB_BYTES: u64 = 64 * 1024 * 1024;

/// Where a located stub came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StubSource {
    /// `--stub PATH` named it.
    Explicit(PathBuf),
    /// It was found in the directory `GINARY_STUB_DIR` names.
    EnvDir(PathBuf),
    /// It is the running executable, because the target is the host.
    SelfExe,
    /// It was found under the cache root's stub directory.
    Cache(PathBuf),
}

/// Where [`locate`] is allowed to look.
///
/// The two optional fields are the two ways a user overrides the search;
/// `cache_dir` is the resolved cache root, so that a test — and `--offline` —
/// can point the whole search somewhere else.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StubOpts {
    /// `--stub PATH`, which is used or refused, never fallen back from.
    pub explicit: Option<PathBuf>,
    /// The directory `GINARY_STUB_DIR` names, when it is set.
    pub env_dir: Option<PathBuf>,
    /// The cache root, the parent of [`CACHE_SUBDIR`].
    pub cache_dir: PathBuf,
}

/// Every path [`locate`] would try for `target`, in the order it tries them.
///
/// This is what [`StubError::NotFound`] prints, and it is public so that the
/// order is testable without a directory full of files. The running
/// executable appears only when `target` is the host and only when it can be
/// opened.
pub fn candidate_paths(target: &Target, opts: &StubOpts) -> Vec<(PathBuf, StubSource)> {
    let mut candidates = Vec::with_capacity(4);
    let name = format!("{}{}", target.name(), target.exe_suffix());

    if let Some(path) = &opts.explicit {
        candidates.push((path.clone(), StubSource::Explicit(path.clone())));
    }
    if let Some(dir) = &opts.env_dir {
        // Both spellings, the explicit one first: `stubs:build` writes
        // `ginary-stub-<version>-<target>`, and a stub someone renamed off a
        // release asset is `ginary-<version>-<target>`.
        candidates.push((
            dir.join(format!("ginary-stub-{VERSION}-{name}")),
            StubSource::EnvDir(dir.clone()),
        ));
        candidates.push((
            dir.join(format!("ginary-{VERSION}-{name}")),
            StubSource::EnvDir(dir.clone()),
        ));
    }
    // The running executable is a stub of exactly one target, and it is only a
    // candidate when it can be opened: a ginary whose own file has been
    // replaced under it is not a file a payload may be appended to.
    if *target == Target::host()
        && let Ok((_file, path)) = crate::selfexe::open_self()
    {
        candidates.push((path, StubSource::SelfExe));
    }
    candidates.push((
        opts.cache_dir.join(CACHE_SUBDIR).join(VERSION).join(&name),
        StubSource::Cache(opts.cache_dir.clone()),
    ));
    candidates
}

/// Finds the stub for `target`.
///
/// The first candidate that exists wins; nothing is verified here, because
/// "which file" and "is this file a stub" are two questions and a wrong answer
/// to the second one must name the file it is about.
///
/// # Errors
///
/// [`StubError::Missing`], [`StubError::NotAFile`] or [`StubError::Io`] when
/// `--stub` named a path that cannot be used — an instruction that cannot be
/// followed is never a fallback — and [`StubError::NotFound`] when no
/// candidate exists.
pub fn locate(target: &Target, opts: &StubOpts) -> Result<(PathBuf, StubSource), StubError> {
    let candidates = candidate_paths(target, opts);
    let mut searched: Vec<PathBuf> = Vec::with_capacity(candidates.len());
    for (path, source) in candidates {
        // `--stub` is an instruction rather than a hint: a build that fell
        // back to a stub the user did not name would package the wrong file
        // and say nothing. It is also the one candidate a person typed, so it
        // is the one that earns a sentence about what is actually wrong with
        // it: `Path::is_file` answers `false` for a directory, for a dangling
        // symlink and for every `stat` that fails, and folding all of those
        // into "is not there" sends the reader to check a spelling that is
        // right. The searched candidates keep the cheap question, because a
        // path nobody typed only has to be present or absent.
        if matches!(source, StubSource::Explicit(_)) {
            return match std::fs::metadata(&path) {
                Ok(meta) if meta.is_file() => Ok((path, source)),
                Ok(meta) => Err(StubError::NotAFile {
                    found: describe_file_type(&meta.file_type()).to_owned(),
                    path,
                }),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    Err(StubError::Missing { path })
                }
                Err(source) => Err(StubError::Io {
                    what: format!("cannot look at the stub at {}", path.display()),
                    source,
                }),
            };
        }
        if path.is_file() {
            return Ok((path, source));
        }
        searched.push(path);
    }
    Err(StubError::NotFound {
        target: *target,
        version: VERSION.to_owned(),
        searched,
    })
}

/// What a path that is not a regular file is, as the tail of a sentence.
///
/// Two answers rather than a list, because the caller's `metadata` follows
/// symlinks: what is left after "a directory" is a device, a socket or a fifo,
/// and naming which one would not change what the reader has to do about it.
fn describe_file_type(kind: &std::fs::FileType) -> &'static str {
    if kind.is_dir() {
        "a directory"
    } else {
        "not a regular file"
    }
}

/// Reads `path` and proves it is a stub of this ginary for `want`.
///
/// The gates, in order: the file is under [`MAX_STUB_BYTES`]; it carries
/// exactly one identity marker; the marker's version is this ginary's; its
/// format version is the one this ginary writes; its target is `want`; the
/// file's own object header agrees with `want`; and the file carries no
/// trailer.
///
/// # Errors
///
/// One [`StubError`] per gate, each naming `path`.
pub fn verify(path: &Path, want: &Target) -> Result<StubId, StubError> {
    let file = std::fs::File::open(path).map_err(|source| StubError::Io {
        what: format!("cannot read the stub at {}", path.display()),
        source,
    })?;
    let len = file
        .metadata()
        .map_err(|source| StubError::Io {
            what: format!("cannot stat the stub at {}", path.display()),
            source,
        })?
        .len();
    // Before the read, not after it: the cap exists so that a `--stub` aimed
    // at a disk image is a refusal rather than an allocation.
    if len > MAX_STUB_BYTES {
        return Err(StubError::TooLarge {
            path: path.to_path_buf(),
            len,
            cap: MAX_STUB_BYTES,
        });
    }
    let bytes = std::fs::read(path).map_err(|source| StubError::Io {
        what: format!("cannot read the stub at {}", path.display()),
        source,
    })?;

    let id = stubid::scan(&bytes).map_err(|source| StubError::Marker {
        path: path.to_path_buf(),
        source,
    })?;
    if id.version != VERSION {
        return Err(StubError::VersionMismatch {
            path: path.to_path_buf(),
            stub: id.version,
            ginary: VERSION.to_owned(),
        });
    }
    if id.format_version != crate::manifest::FORMAT_VERSION {
        return Err(StubError::FormatMismatch {
            path: path.to_path_buf(),
            stub: id.format_version,
            supported: crate::manifest::FORMAT_VERSION,
        });
    }
    if id.target != *want {
        return Err(StubError::TargetMismatch {
            path: path.to_path_buf(),
            stub: id.target,
            want: *want,
        });
    }

    // The marker is text and copies with the bytes around it; the object
    // header is what the linker wrote. When they disagree, the header wins.
    check_object(path, &bytes, want)?;

    // Last, because it is the one gate that is about what was done to the file
    // rather than about what the file is: a stub that already carries a
    // payload is an artifact, and appending a second one would produce a file
    // no launcher can read.
    match Trailer::read_from(&file) {
        Ok(None) => Ok(id),
        Ok(Some(_)) => Err(StubError::Trailered {
            path: path.to_path_buf(),
        }),
        Err(TrailerError::Io(source)) => Err(StubError::Io {
            what: format!("cannot read the last bytes of {}", path.display()),
            source,
        }),
        // A trailer that is there and damaged says a payload was appended just
        // as loudly as a whole one does.
        Err(_) => Err(StubError::Trailered {
            path: path.to_path_buf(),
        }),
    }
}

/// Checks the file's own object header against `want`.
///
/// The ELF half is `crate::elf`'s, so that "what is this binary for" is
/// answered in one place; the PE half is read straight from `object`, because
/// a Windows stub has no interpreter and the machine field is the whole
/// question. Mach-O is refused with a sentence rather than guessed at: there
/// is no macOS toolchain here to build a stub with, so the check would be
/// written against nothing.
fn check_object(path: &Path, bytes: &[u8], want: &Target) -> Result<(), StubError> {
    match want.os {
        Os::Macos => Err(StubError::NotYetSupported { target: *want }),
        Os::Linux => {
            if !crate::elf::is_elf(bytes) {
                return Err(not_an_object(path, bytes));
            }
            let info =
                crate::elf::inspect_bytes(bytes).map_err(|error| StubError::NotAnObject {
                    path: path.to_path_buf(),
                    reason: error.to_string(),
                })?;
            let agrees = match Target::from_elf(&info.machine, info.interp.as_deref()) {
                Some(ElfTarget::Dynamic(target)) => target == *want,
                // A static binary names no C library, so the machine is all
                // there is to check: a musl static-pie stub and a static glibc
                // one are the same bytes to the header.
                Some(ElfTarget::StaticLinux(arch)) => arch == want.arch,
                None => false,
            };
            if agrees {
                Ok(())
            } else {
                Err(StubError::ObjectMismatch {
                    path: path.to_path_buf(),
                    want: *want,
                    found: describe_elf(&info),
                })
            }
        }
        Os::Windows => {
            // Answered before `object` is asked, so that an ELF handed to a
            // Windows build is described as the ELF it is.
            if crate::elf::is_elf(bytes) {
                return Err(StubError::ObjectMismatch {
                    path: path.to_path_buf(),
                    want: *want,
                    found: crate::elf::inspect_bytes(bytes)
                        .map_or_else(|_| "an ELF".to_owned(), |info| describe_elf(&info)),
                });
            }
            let file =
                object::read::File::parse(bytes).map_err(|error| StubError::NotAnObject {
                    path: path.to_path_buf(),
                    reason: error.to_string(),
                })?;
            let format = file.format();
            let arch = file.architecture();
            if format == object::BinaryFormat::Pe && arch_of(arch) == Some(want.arch) {
                Ok(())
            } else {
                Err(StubError::ObjectMismatch {
                    path: path.to_path_buf(),
                    want: *want,
                    found: describe_object(format, arch),
                })
            }
        }
    }
}

/// The `ginary` name for an `object` architecture, when there is one.
fn arch_of(arch: object::Architecture) -> Option<crate::target::Arch> {
    match arch {
        object::Architecture::X86_64 => Some(crate::target::Arch::X86_64),
        object::Architecture::Aarch64 => Some(crate::target::Arch::Aarch64),
        _ => None,
    }
}

/// What an ELF really is, as the tail of a sentence.
fn describe_elf(info: &crate::elf::ElfInfo) -> String {
    let libc = match info.interp.as_deref() {
        Some(interp) if interp.contains(crate::target::MUSL_INTERPRETER) => {
            "with a musl interpreter".to_owned()
        }
        Some(interp) if interp.contains(crate::target::GNU_INTERPRETER) => {
            "with a glibc interpreter".to_owned()
        }
        Some(interp) => format!("with the interpreter {interp}"),
        None => "with no interpreter".to_owned(),
    };
    format!("an ELF for {} {libc}", info.machine)
}

/// What a non-ELF object really is, as the tail of a sentence.
fn describe_object(format: object::BinaryFormat, arch: object::Architecture) -> String {
    let kind = match format {
        object::BinaryFormat::Pe => "a PE",
        object::BinaryFormat::MachO => "a Mach-O",
        object::BinaryFormat::Elf => "an ELF",
        _ => "an object file",
    };
    match arch_of(arch) {
        Some(arch) => format!("{kind} for {arch}"),
        None => format!("{kind} for a machine ginary has no name for"),
    }
}

/// The refusal a file that is not an object file at all earns.
///
/// The first two bytes and the length, because that is what tells a shell
/// script from a truncated download without printing the file.
fn not_an_object(path: &Path, bytes: &[u8]) -> StubError {
    let head: String = bytes
        .iter()
        .take(2)
        .map(|byte| {
            if byte.is_ascii_graphic() {
                char::from(*byte).to_string()
            } else {
                format!("\\x{byte:02x}")
            }
        })
        .collect();
    StubError::NotAnObject {
        path: path.to_path_buf(),
        reason: format!("the file is {} bytes and begins `{head}`", bytes.len()),
    }
}

/// Why a stub could not be used.
#[derive(Debug)]
pub enum StubError {
    /// No candidate path exists.
    NotFound {
        /// The target a stub was wanted for.
        target: Target,
        /// This ginary's version, which the file name carries.
        version: String,
        /// Every path that was tried, in order.
        searched: Vec<PathBuf>,
    },
    /// `--stub` named a path that is not there.
    Missing {
        /// The path, as it was given.
        path: PathBuf,
    },
    /// `--stub` named a path that is there and is not a regular file.
    ///
    /// Separate from [`StubError::Missing`] because the two send the reader to
    /// different places: one is a spelling to check, the other is a path that
    /// is exactly what was typed and is the wrong kind of thing.
    NotAFile {
        /// The path, as it was given.
        path: PathBuf,
        /// What it is instead, as the tail of a sentence.
        found: String,
    },
    /// The file is larger than [`MAX_STUB_BYTES`].
    TooLarge {
        /// The file that was refused.
        path: PathBuf,
        /// Its length.
        len: u64,
        /// The cap it exceeded.
        cap: u64,
    },
    /// The file carries no usable identity marker.
    Marker {
        /// The file that was scanned.
        path: PathBuf,
        /// What the scanner said.
        source: StubIdError,
    },
    /// The stub was built by a different ginary.
    VersionMismatch {
        /// The file that was refused.
        path: PathBuf,
        /// The version its marker names.
        stub: String,
        /// This ginary's version.
        ginary: String,
    },
    /// The stub reads a different payload format.
    FormatMismatch {
        /// The file that was refused.
        path: PathBuf,
        /// The format version its marker names.
        stub: u32,
        /// The format version this ginary writes.
        supported: u32,
    },
    /// The stub's marker names another target.
    TargetMismatch {
        /// The file that was refused.
        path: PathBuf,
        /// The target its marker names.
        stub: Target,
        /// The target the build asked for.
        want: Target,
    },
    /// The file is not an object file this ginary can read.
    NotAnObject {
        /// The file that was refused.
        path: PathBuf,
        /// What the reader said.
        reason: String,
    },
    /// The file's object header disagrees with its marker.
    ///
    /// The marker is text and copies; the header is what the linker wrote.
    /// When they disagree the header is believed and the file is refused.
    ObjectMismatch {
        /// The file that was refused.
        path: PathBuf,
        /// The target the build asked for.
        want: Target,
        /// What the header really says, as a sentence.
        found: String,
    },
    /// Checking the object of this target is not implemented yet.
    ///
    /// Mach-O: there is no macOS toolchain to build a stub with here, so the
    /// check is written where it can be tested rather than guessed at.
    NotYetSupported {
        /// The target whose object format has no check yet.
        target: Target,
    },
    /// The file already carries a trailer, so it is an artifact.
    Trailered {
        /// The file that was refused.
        path: PathBuf,
    },
    /// The file could not be read.
    Io {
        /// What was being done, as a sentence naming the path.
        what: String,
        /// What the operating system said.
        source: std::io::Error,
    },
}

impl fmt::Display for StubError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound {
                target,
                version,
                searched,
            } => {
                writeln!(f, "no stub found for {target} (ginary {version})")?;
                if searched.is_empty() {
                    writeln!(f, "  searched: nothing; no directory was available")?;
                } else {
                    for (index, path) in searched.iter().enumerate() {
                        let label = if index == 0 {
                            "  searched: "
                        } else {
                            "            "
                        };
                        writeln!(f, "{label}{}", path.display())?;
                    }
                }
                write!(
                    f,
                    "  hint: build one with `mise run stubs:build` and set {STUB_DIR_VAR}, or \
                     point `--stub` at it"
                )
            }
            Self::Missing { path } => write!(
                f,
                "the stub {} is not there; `--stub` names a file rather than a search",
                path.display()
            ),
            Self::NotAFile { path, found } => write!(
                f,
                "the stub {} is {found} rather than a file; `--stub` names the stub binary itself",
                path.display()
            ),
            Self::TooLarge { path, len, cap } => write!(
                f,
                "{} is {len} bytes, and a stub may be at most {cap}",
                path.display()
            ),
            Self::Marker { path, source } => {
                write!(f, "{} is not a ginary stub: {source}", path.display())
            }
            Self::VersionMismatch { path, stub, ginary } => write!(
                f,
                "{} is a stub of ginary {stub} and this is ginary {ginary}; stubs are \
                 version-locked, so build or fetch the {ginary} stub",
                path.display()
            ),
            Self::FormatMismatch {
                path,
                stub,
                supported,
            } => write!(
                f,
                "{} reads payload format {stub} and this ginary writes format {supported}",
                path.display()
            ),
            Self::TargetMismatch { path, stub, want } => write!(
                f,
                "{} is a stub for {stub}, and this build is for {want}",
                path.display()
            ),
            Self::NotAnObject { path, reason } => write!(
                f,
                "{} is not an executable this ginary can read ({reason})",
                path.display()
            ),
            Self::ObjectMismatch { path, want, found } => write!(
                f,
                "{} carries a marker for {want} and the file itself is {found}",
                path.display()
            ),
            Self::NotYetSupported { target } => write!(
                f,
                "a stub for {target} cannot be checked here yet; darwin stubs come from the CI \
                 release build"
            ),
            Self::Trailered { path } => write!(
                f,
                "{} is a packaged application rather than a stub; a payload may not be appended \
                 twice",
                path.display()
            ),
            Self::Io { what, .. } => f.write_str(what),
        }
    }
}

impl std::error::Error for StubError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Marker { source, .. } => Some(source),
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}
