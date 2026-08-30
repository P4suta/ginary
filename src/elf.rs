// SPDX-License-Identifier: MIT OR Apache-2.0
//! Read-only inspection of ELF binaries.
//!
//! Three questions are asked of every ELF file that reaches an artifact, and
//! this module is the only place that answers them:
//!
//! - **What is it?** `src/strip.rs` detects an ELF by its magic bytes rather
//!   than by its name — a NIF under `priv/lib` may be called anything — and
//!   re-inspects each file after `strip` has rewritten it, because a strip that
//!   produced something that is no longer the same machine and class did not
//!   strip, it destroyed.
//! - **What does it need?** `src/report.rs` collects `DT_NEEDED` and the
//!   highest `GLIBC_x.y` any file requires, which together are the artifact's
//!   real portability floor. A user whose target machine is older than that
//!   floor should learn it from the build, not from a runtime loader error.
//! - **Is it already stripped?** A staged tree that is smaller than expected
//!   is usually a runtime that someone else stripped first.
//!
//! Everything here is read-only, and everything here takes bytes ginary did not
//! write, so like `src/beam.rs` it follows the never-panic rule: a random byte
//! vector, a truncated binary and a text file are typed errors, never a panic.
//! The parsing itself is the `object` crate's; this module is the vocabulary
//! ginary uses on top of it, so that no other module has to know what a
//! `.gnu.version_r` entry looks like.

use std::path::{Path, PathBuf};

use object::Object as _;
use object::elf::{FileHeader32, FileHeader64};
use object::read::elf::{ElfFile, ProgramHeader as _, SectionHeader as _};
use serde::Serialize;

/// The four bytes every ELF file starts with, `\x7fELF`.
pub const ELF_MAGIC: [u8; 4] = [0x7f, b'E', b'L', b'F'];

/// The prefix of a symbol version naming a glibc release.
pub const GLIBC_VERSION_PREFIX: &str = "GLIBC_";

/// What kind of object an ELF file is, from `e_type`.
///
/// The one header field that says whether a file is a program or a library,
/// and the reason it is here is `src/strip.rs`: a shared object gets
/// `--strip-unneeded` and everything else gets `--strip-all`, and no other
/// field answers that question. [`ElfInfo::interp`] does not — a statically
/// linked program has no interpreter either, and a real shared library may
/// well have one, as glibc's own `libc.so.6` does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ElfKind {
    /// `ET_REL`, an object file.
    Relocatable,
    /// `ET_EXEC`, a non-relocatable program.
    Executable,
    /// `ET_DYN`, a shared object.
    ///
    /// A position-independent *executable* is also an `ET_DYN`, which the
    /// header does not distinguish and this enum therefore does not either;
    /// [`ElfInfo::is_pie`] is what separates the two.
    SharedObject,
    /// `ET_CORE`, a core dump.
    Core,
    /// Anything else, carrying the number the header held.
    Other(u16),
}

impl ElfKind {
    /// The kind `e_type` names.
    fn of(e_type: object::elf::FileType) -> Self {
        match e_type {
            object::elf::ET_REL => Self::Relocatable,
            object::elf::ET_EXEC => Self::Executable,
            object::elf::ET_DYN => Self::SharedObject,
            object::elf::ET_CORE => Self::Core,
            other => Self::Other(other.0),
        }
    }
}

/// What ginary reads out of an ELF file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ElfInfo {
    /// The ELF class: `32` or `64`.
    pub class: u8,
    /// What kind of object the file is.
    pub kind: ElfKind,
    /// The machine, spelled the way [`crate::target::Arch`] spells it —
    /// `x86_64`, `aarch64` — or the `object` crate's debug name for one ginary
    /// has no name of its own for.
    pub machine: String,
    /// The program interpreter from `PT_INTERP`, if the file has one.
    ///
    /// `None` for a shared object, for a static binary and for anything else
    /// the kernel does not hand to a dynamic loader.
    pub interp: Option<String>,
    /// The `DT_NEEDED` entries, in the order the dynamic section lists them.
    pub needed: Vec<String>,
    /// The highest `GLIBC_x.y` version the file requires, without the prefix.
    ///
    /// `Some("2.38")`, not `Some("GLIBC_2.38")`: the number is the datum, and
    /// the report puts the prefix back when it prints it. Compared
    /// numerically, component by component, so `2.9` is below `2.38`.
    pub glibc_max: Option<String>,
    /// Whether the file is a position-independent executable.
    pub is_pie: bool,
    /// Whether the file has no `.symtab`, which is what `strip` removes.
    pub stripped: bool,
}

/// Why an ELF file could not be inspected.
#[derive(Debug, thiserror::Error)]
pub enum ElfError {
    /// The bytes do not start with [`ELF_MAGIC`].
    #[error("not an ELF file")]
    NotElf,
    /// The file could not be read.
    #[error("cannot read `{path}`: {source}")]
    Io {
        /// The file that could not be read.
        path: PathBuf,
        /// The underlying operating system error.
        #[source]
        source: std::io::Error,
    },
    /// The file starts like an ELF and is not a whole one.
    #[error("cannot parse the ELF file: {message}")]
    Parse {
        /// What the parser said.
        message: String,
    },
}

/// The highest `GLIBC_x.y` among `versions`, without the prefix.
///
/// Split out and public because the comparison is the part that is easy to get
/// wrong and impossible to test against a real binary: sorting the strings
/// would put `2.9` above `2.38`, and no host installation happens to hold a
/// pair that shows it. Components are compared numerically, left to right, and
/// a component that is not a number sorts below every one that is, so a
/// `GLIBC_2.34-suffixed` oddity cannot win by accident.
///
/// Anything that is not a `GLIBC_` version is ignored: `.gnu.version_r` holds
/// entries for every library a file needs, and only glibc's decide the floor.
pub fn max_glibc_version<'a>(versions: impl IntoIterator<Item = &'a str>) -> Option<String> {
    versions
        .into_iter()
        .filter_map(|version| version.strip_prefix(GLIBC_VERSION_PREFIX))
        .max_by(|left, right| version_key(left).cmp(&version_key(right)))
        .map(str::to_owned)
}

/// The sort key of a version number, compared component by component.
///
/// `None` for a component that is not a number, and `None` sorts below every
/// `Some`, which is the rule the doc comment on [`max_glibc_version`] states:
/// a `2.34-something` cannot outrank a real `2.38` by being longer.
fn version_key(version: &str) -> Vec<Option<u32>> {
    version
        .split('.')
        .map(|part| part.parse::<u32>().ok())
        .collect()
}

/// Whether `bytes` starts with the ELF magic.
///
/// This is how staging decides what to hand to `strip`: an extension says
/// nothing about a file under `priv`, and a `.so` that is really a text script
/// must not be run through a binary tool.
pub fn is_elf(bytes: &[u8]) -> bool {
    bytes.starts_with(&ELF_MAGIC)
}

/// Inspects the ELF file at `path`.
///
/// # Errors
///
/// [`ElfError::Io`] when the file cannot be read, [`ElfError::NotElf`] when it
/// is not an ELF file, and [`ElfError::Parse`] when it is a damaged one.
pub fn inspect(path: &Path) -> Result<ElfInfo, ElfError> {
    let bytes = std::fs::read(path).map_err(|source| ElfError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    inspect_bytes(&bytes)
}

/// Inspects ELF bytes that are already in memory.
///
/// [`inspect`] is this, with the file read first. The split exists because the
/// property tests feed this thousands of random byte vectors, and because
/// stripping already holds the bytes it is about to check.
///
/// # Errors
///
/// [`ElfError::NotElf`] when the bytes are not an ELF file and
/// [`ElfError::Parse`] when they are a damaged one.
pub fn inspect_bytes(bytes: &[u8]) -> Result<ElfInfo, ElfError> {
    if !is_elf(bytes) {
        return Err(ElfError::NotElf);
    }
    match bytes.get(CLASS_BYTE) {
        Some(&ELF_CLASS_32) => read::<FileHeader32<object::Endianness>>(bytes, 32),
        Some(&ELF_CLASS_64) => read::<FileHeader64<object::Endianness>>(bytes, 64),
        Some(class) => Err(ElfError::Parse {
            message: format!("the ELF class byte is {class}, which is neither 32-bit nor 64-bit"),
        }),
        None => Err(ElfError::Parse {
            message: "the file is four bytes of ELF magic and nothing else".to_owned(),
        }),
    }
}

/// The offset of `e_ident[EI_CLASS]`, the byte that says 32-bit or 64-bit.
const CLASS_BYTE: usize = 4;

/// `ELFCLASS32`.
const ELF_CLASS_32: u8 = 1;

/// `ELFCLASS64`.
const ELF_CLASS_64: u8 = 2;

/// Reads one class of ELF file.
///
/// Generic over the header rather than duplicated, because everything below the
/// class byte — program headers, the dynamic table, the version records — is
/// the same shape in both and the `object` crate spells it once.
fn read<Elf>(bytes: &[u8], class: u8) -> Result<ElfInfo, ElfError>
where
    Elf: object::read::elf::FileHeader<Endian = object::Endianness>,
{
    let file = ElfFile::<Elf, &[u8]>::parse(bytes).map_err(parse_failed)?;
    let endian = file.endian();
    let header = file.elf_header();
    let sections = file.elf_section_table();

    let mut interp = None;
    for segment in file.elf_program_headers() {
        if let Some(found) = segment.interpreter(endian, bytes).map_err(parse_failed)? {
            interp = Some(String::from_utf8_lossy(found).into_owned());
            break;
        }
    }

    let dynamic = file.elf_dynamic_table().map_err(parse_failed)?;
    let mut needed = Vec::new();
    let mut flags_1 = 0u64;
    for entry in dynamic.iter() {
        if entry.tag == object::elf::DT_NEEDED {
            if let Ok(name) = dynamic.string(entry) {
                needed.push(String::from_utf8_lossy(name).into_owned());
            }
        } else if entry.tag == object::elf::DT_FLAGS_1 {
            flags_1 = entry.val;
        }
    }

    let mut versions: Vec<String> = Vec::new();
    if let Some((mut needs, link)) = sections.gnu_verneed(endian, bytes).map_err(parse_failed)? {
        let strings = sections
            .strings(endian, bytes, link)
            .map_err(parse_failed)?;
        while let Some((_, mut auxiliaries)) = needs.next().map_err(parse_failed)? {
            while let Some(auxiliary) = auxiliaries.next().map_err(parse_failed)? {
                if let Ok(name) = auxiliary.name(endian, strings) {
                    versions.push(String::from_utf8_lossy(name).into_owned());
                }
            }
        }
    }

    let stripped = !sections
        .iter()
        .any(|section| section.sh_type(endian) == object::elf::SHT_SYMTAB);
    let kind = ElfKind::of(header.e_type(endian));
    let is_pie = kind == ElfKind::SharedObject && (interp.is_some() || flags_1 & PIE_FLAG != 0);

    Ok(ElfInfo {
        class,
        kind,
        machine: machine_name(&file),
        interp,
        glibc_max: max_glibc_version(versions.iter().map(String::as_str)),
        needed,
        is_pie,
        stripped,
    })
}

/// `DF_1_PIE`, the dynamic flag a linker sets on a position-independent
/// executable.
///
/// Spelled here as its numeric value because the `object` crate wraps it in a
/// newtype that this module has no other use for.
const PIE_FLAG: u64 = 0x0800_0000;

/// The architecture, spelled the way [`crate::target::Arch`] spells it.
///
/// A machine ginary has no name of its own for falls back to the `object`
/// crate's, so an artifact built for an unexpected target still says which one
/// rather than saying nothing.
fn machine_name<'data, Elf, R>(file: &ElfFile<'data, Elf, R>) -> String
where
    Elf: object::read::elf::FileHeader,
    R: object::ReadRef<'data>,
{
    match file.architecture() {
        object::Architecture::X86_64 => crate::target::Arch::X86_64.as_str().to_owned(),
        object::Architecture::Aarch64 => crate::target::Arch::Aarch64.as_str().to_owned(),
        other => format!("{other:?}"),
    }
}

/// Turns an `object` parse failure into [`ElfError::Parse`].
///
/// The crate's own error type is opaque and carries only a message, so the
/// message is what travels: it is the only thing that distinguishes a file
/// truncated in its section table from one truncated in its headers.
fn parse_failed(error: object::Error) -> ElfError {
    ElfError::Parse {
        message: error.to_string(),
    }
}
