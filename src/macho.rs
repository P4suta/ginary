// SPDX-License-Identifier: MIT OR Apache-2.0
//! Read-only inspection of Mach-O binaries.
//!
//! macOS is the one platform where the payload does not sit at the end of
//! the file: appending bytes after a Mach-O's `__LINKEDIT` segment breaks
//! `codesign --strict`, and the arm64 kernel refuses to map a page whose
//! signature it cannot verify. So a macOS artifact carries its payload in a
//! dedicated `__GINARY,__payload` section instead, ad-hoc signed after the
//! section is written; see [`crate::sign_macos`] and `docs/format.md`.
//!
//! This module answers the questions [`crate::payload::locate`] and
//! `stub::verify` ask of a Mach-O file: which CPU it is built for, whether it
//! is a fat binary (neither caller may proceed past one — a stub and a
//! launcher both need one architecture, not a bundle of them, though saying
//! so is [`MachoFacts::is_fat`] rather than a refusal this module makes on
//! its own), whether it already carries a code signature, and where a named
//! section is.
//!
//! Like [`crate::elf`], every byte this module reads may have come from
//! someone other than ginary, so nothing here may panic: a random byte
//! vector, a truncated download and a file that is a different format
//! entirely are typed errors, never a crash.

use std::fs;
use std::path::{Path, PathBuf};

use object::read::File as ObjectFile;
use object::read::macho::LoadCommandIterator;
use object::{Architecture, Object as _, ObjectSection as _};

use crate::target::{Arch, Libc, Os, Target};

/// `MH_MAGIC`, the 32-bit little-endian thin magic.
pub const MH_MAGIC: u32 = 0xfeed_face;
/// `MH_CIGAM`, the 32-bit thin magic read on the other byte order.
pub const MH_CIGAM: u32 = 0xcefa_edfe;
/// `MH_MAGIC_64`, the 64-bit little-endian thin magic.
pub const MH_MAGIC_64: u32 = 0xfeed_facf;
/// `MH_CIGAM_64`, the 64-bit thin magic read on the other byte order.
pub const MH_CIGAM_64: u32 = 0xcffa_edfe;
/// `FAT_MAGIC`, a fat binary's magic, as it sits on disk (big-endian).
pub const FAT_MAGIC: u32 = 0xcafe_babe;
/// `FAT_CIGAM`, the same magic read on the other byte order.
pub const FAT_CIGAM: u32 = 0xbeba_feca;

/// The four thin-Mach-O magics, 32- and 64-bit, in both byte orders.
pub const THIN_MAGICS: [u32; 4] = [MH_MAGIC, MH_CIGAM, MH_MAGIC_64, MH_CIGAM_64];

/// The two fat-binary magics.
pub const FAT_MAGICS: [u32; 2] = [FAT_MAGIC, FAT_CIGAM];

/// The segment a macOS artifact's payload section lives in.
///
/// [`crate::sign_macos::inject_and_sign`] writes it, [`crate::payload::locate`]
/// and `stub::verify` look it up by this name and [`PAYLOAD_SECTION`].
pub const PAYLOAD_SEGMENT: &str = "__GINARY";

/// The section, within [`PAYLOAD_SEGMENT`], a macOS artifact's payload lives
/// in.
pub const PAYLOAD_SECTION: &str = "__payload";

/// `CPU_TYPE_X86_64`, the `cputype` a 64-bit x86 Mach-O names.
pub const CPU_TYPE_X86_64: u32 = 0x0100_0007;
/// `CPU_TYPE_ARM64`, the `cputype` an arm64 Mach-O names.
pub const CPU_TYPE_ARM64: u32 = 0x0100_000c;

/// The largest file [`read`] will hold in memory to inspect.
///
/// A stub or a packaged application is a handful of megabytes; a hundred is
/// headroom without letting a multi-gigabyte file be read whole just to look
/// at its header.
pub const MAX_MACHO_BYTES: u64 = 100 * 1024 * 1024;

/// What ginary reads out of a Mach-O file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MachoFacts {
    /// The `cputype`, spelled `x86_64` or `arm64` for the two this crate
    /// packages for, or the `object` crate's debug name for one it does not.
    pub cputype: String,
    /// [`crate::target::Target`] for a `cputype` this crate has a target for,
    /// and [`None`] for one it does not.
    ///
    /// macOS names its dynamic linker inside `LC_LOAD_DYLINKER` rather than in
    /// a program header the way an ELF's `PT_INTERP` does, and every macOS
    /// target shares the same one, so there is no libc distinction to make:
    /// the `cputype` is the whole of the target.
    pub target: Option<crate::target::Target>,
    /// Whether the file is a fat (universal) binary.
    ///
    /// A fat binary carries more than one architecture and therefore no
    /// single `cputype`: [`read`] does not refuse one — it is still a Mach-O,
    /// and saying so is useful — but [`MachoFacts::cputype`] is empty,
    /// [`MachoFacts::target`] is [`None`] and [`MachoFacts::sections`] is
    /// empty, because none of the three has one answer without first
    /// choosing an architecture. Refusing to proceed past a fat binary is
    /// each *caller*'s decision: [`crate::payload::locate`] and
    /// `stub::verify` both need exactly one architecture and both check this
    /// field themselves.
    pub is_fat: bool,
    /// Whether an `LC_CODE_SIGNATURE` load command is present.
    pub has_code_signature: bool,
    /// Every section the file's load commands name: `(segment, section, file
    /// offset, size)`, in load-command order.
    pub sections: Vec<(String, String, u64, u64)>,
}

/// Why a Mach-O file could not be inspected.
#[derive(Debug, thiserror::Error)]
pub enum MachoError {
    /// The bytes do not begin with a thin or a fat Mach-O magic.
    #[error("not a Mach-O file")]
    NotMachO,
    /// The file starts like a Mach-O and is not a whole, parseable one.
    #[error("cannot parse the Mach-O file: {message}")]
    Parse {
        /// What the parser said.
        message: String,
    },
    /// The file is larger than [`MAX_MACHO_BYTES`].
    #[error("{path} is {len} bytes, and no more than {MAX_MACHO_BYTES} are read to inspect one")]
    TooLarge {
        /// The file that was refused.
        path: PathBuf,
        /// Its actual length.
        len: u64,
    },
    /// The file could not be read.
    #[error("cannot read `{path}`: {source}")]
    Io {
        /// The file that could not be read.
        path: PathBuf,
        /// The underlying operating system error.
        #[source]
        source: std::io::Error,
    },
}

/// Whether `bytes` begins with a thin or a fat Mach-O magic.
///
/// This is the same kind of question [`crate::elf::is_elf`] answers for ELF:
/// a cheap check on the first four bytes, used to decide whether a file is
/// worth handing to [`read`] at all, before anything is parsed.
pub fn is_macho(bytes: &[u8]) -> bool {
    magic_of(bytes).is_some_and(|magic| THIN_MAGICS.contains(&magic) || FAT_MAGICS.contains(&magic))
}

/// The first four bytes of `bytes`, read as a little-endian `u32`, or
/// [`None`] when there are fewer than four.
///
/// Every magic this module recognises is compared against the value its four
/// bytes produce when read this way, whichever byte order the file was
/// actually written in: [`MH_CIGAM`] and [`MH_CIGAM_64`] are exactly the
/// swapped forms of [`MH_MAGIC`] and [`MH_MAGIC_64`] for that reason, and the
/// same holds for the fat pair.
fn magic_of(bytes: &[u8]) -> Option<u32> {
    let four = bytes.get(0..4)?;
    Some(u32::from_le_bytes([four[0], four[1], four[2], four[3]]))
}

/// Inspects the Mach-O bytes already in memory.
///
/// # Errors
///
/// [`MachoError::NotMachO`] when `bytes` begins with neither a thin nor a fat
/// magic, and [`MachoError::Parse`] when it begins with one and is not a
/// whole, parseable Mach-O. A fat binary is not an error here; see
/// [`MachoFacts::is_fat`].
pub fn read(bytes: &[u8]) -> Result<MachoFacts, MachoError> {
    let Some(magic) = magic_of(bytes) else {
        return Err(MachoError::NotMachO);
    };
    if FAT_MAGICS.contains(&magic) {
        // Ambiguous, not broken: see `MachoFacts::is_fat`. Every caller that
        // needs one architecture refuses this itself.
        return Ok(MachoFacts {
            cputype: String::new(),
            target: None,
            is_fat: true,
            has_code_signature: false,
            sections: Vec::new(),
        });
    }
    if !THIN_MAGICS.contains(&magic) {
        return Err(MachoError::NotMachO);
    }

    let object_file = ObjectFile::parse(bytes).map_err(|source| MachoError::Parse {
        message: source.to_string(),
    })?;

    let (cputype, target) = describe_arch(object_file.architecture());

    let mut sections = Vec::with_capacity(object_file.sections().count());
    for section in object_file.sections() {
        let segment = section
            .segment_name()
            .ok()
            .flatten()
            .unwrap_or_default()
            .to_owned();
        let name = section.name().unwrap_or_default().to_owned();
        let (offset, size) = section.file_range().unwrap_or((0, 0));
        sections.push((segment, name, offset, size));
    }

    let has_code_signature = match &object_file {
        ObjectFile::MachO32(file) => {
            let commands = file
                .macho_load_commands()
                .map_err(|source| MachoError::Parse {
                    message: source.to_string(),
                })?;
            carries_code_signature(commands)
        }
        ObjectFile::MachO64(file) => {
            let commands = file
                .macho_load_commands()
                .map_err(|source| MachoError::Parse {
                    message: source.to_string(),
                })?;
            carries_code_signature(commands)
        }
        // `magic_of` already refused every magic that is neither thin nor
        // fat, and the fat case returned above, so `object_file` can only be
        // one of the two Mach-O variants here.
        _ => false,
    };

    Ok(MachoFacts {
        cputype,
        target,
        is_fat: false,
        has_code_signature,
        sections,
    })
}

/// The `cputype` string [`MachoFacts::cputype`] carries, and the
/// [`Target`] it names, for an [`Architecture`] the `object` crate resolved.
///
/// `x86_64` and `arm64` are this crate's own spellings, not the `object`
/// crate's `Debug` names (`Aarch64` would read oddly as a lower-cased
/// `cputype`); anything else falls back to that `Debug` name, lower-cased,
/// with no [`Target`] to offer.
fn describe_arch(architecture: Architecture) -> (String, Option<Target>) {
    match architecture {
        Architecture::X86_64 => (
            "x86_64".to_owned(),
            Some(Target::new(Os::Macos, Arch::X86_64, Libc::None)),
        ),
        Architecture::Aarch64 => (
            "arm64".to_owned(),
            Some(Target::new(Os::Macos, Arch::Aarch64, Libc::None)),
        ),
        other => (format!("{other:?}").to_lowercase(), None),
    }
}

/// Whether any load command in `commands` is an `LC_CODE_SIGNATURE`.
///
/// Both Mach-O widths share one `Endian` type parameter in the `object`
/// crate's unified read API (`Endianness`), so this one function serves the
/// 32- and 64-bit arms of [`read`] alike.
fn carries_code_signature(mut commands: LoadCommandIterator<'_, object::Endianness>) -> bool {
    while let Ok(Some(command)) = commands.next() {
        if command.cmd() == object::macho::LC_CODE_SIGNATURE {
            return true;
        }
    }
    false
}

/// Inspects the Mach-O file at `path`.
///
/// # Errors
///
/// [`MachoError::Io`] when the file cannot be read, [`MachoError::TooLarge`]
/// when it is larger than [`MAX_MACHO_BYTES`], and the errors [`read`] gives
/// for the bytes themselves.
pub fn inspect(path: &Path) -> Result<MachoFacts, MachoError> {
    let metadata = fs::metadata(path).map_err(|source| MachoError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    if metadata.len() > MAX_MACHO_BYTES {
        return Err(MachoError::TooLarge {
            path: path.to_path_buf(),
            len: metadata.len(),
        });
    }
    let bytes = fs::read(path).map_err(|source| MachoError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    read(&bytes)
}

/// The file offset and size of `segment,section` in a thin Mach-O, or
/// [`None`] when the file carries no such section.
///
/// This is what [`crate::payload::locate`] uses to find `__GINARY,__payload`:
/// it wants exactly the two numbers a `pread` needs and nothing else, so it
/// is its own function rather than a search through [`MachoFacts::sections`]
/// at every call site.
pub fn section(bytes: &[u8], segment: &str, section: &str) -> Option<(u64, u64)> {
    let facts = read(bytes).ok()?;
    facts
        .sections
        .into_iter()
        .find(|(seg, sect, _, _)| seg == segment && sect == section)
        .map(|(_, _, offset, size)| (offset, size))
}
