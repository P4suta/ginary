// SPDX-License-Identifier: MIT OR Apache-2.0
//! Writing a macOS artifact: the payload inside `__LINKEDIT`, and the ad-hoc
//! signature over it.
//!
//! An ELF or a PE artifact is the stub with the payload and the trailer
//! appended after it. A Mach-O cannot be built that way and left there: bytes
//! appended past `__LINKEDIT` fail `codesign --strict`, and an arm64 kernel
//! refuses to map a page it cannot verify the signature of at all — an
//! *unsigned* binary, not merely one whose signature does not match.
//!
//! The obvious next idea — carve the payload into a brand new
//! `__GINARY,__payload` section — is what ginary tried first, and two real
//! Macs falsified it. A new section needs a new `LC_SEGMENT_64` load command,
//! and a linker leaves almost no room in the load-command area (the sample
//! stub has forty spare bytes and a section needs a hundred and fifty-two), so
//! adding one forces every following byte of code and data to slide forward.
//! Sliding it invalidates the entry point *and* every rebase target the
//! `LC_DYLD_CHAINED_FIXUPS` stream encodes as an offset from the image base:
//! the signature verifies (it covers whatever bytes are there) and the process
//! still segfaults the instant dyld applies a fixup or the kernel jumps to the
//! moved entry. See ADR
//! [0016](../../../docs/adr/0016-macho-section-payload-and-adhoc-signing.md)
//! for the run that proved this.
//!
//! So this module moves nothing. It appends the payload after `__LINKEDIT`'s
//! existing content, grows `__LINKEDIT`'s `filesize`/`vmsize` so the segment
//! still ends the file, reuses the `LC_CODE_SIGNATURE` command the linker
//! already left (no load command is added, so no content slides), and computes
//! a fresh ad-hoc `CodeDirectory` over the finished bytes — payload included.
//! This is exactly how `codesign` itself embeds a signature: as more bytes at
//! the end of `__LINKEDIT`, covered by the hashes, described by no other load
//! command. The entry point, every segment, and every fixup keep the file
//! offsets and addresses the linker gave them.
//!
//! The payload is found again by [`crate::payload::locate`]: the finished file
//! ends with the ad-hoc signature, and the 64-byte trailer sits immediately
//! before it, so `locate` reads `LC_CODE_SIGNATURE`'s `dataoff` and parses the
//! trailer at `dataoff - 64`. An unsigned build (only the tests ask for one)
//! has no signature after it, so the trailer is the last thing in the file and
//! the ordinary end-of-file reader finds it. There is no signature stripping,
//! no impersonation of another signer, and no identity claimed: an ad-hoc
//! signature asserts nothing about who built the binary, only that these are
//! the bytes it was built with.
//!
//! Real code-signing verification (`codesign --verify`, Gatekeeper, an actual
//! launch) needs a Mac; the macOS CI runners are where it is confirmed.

use std::path::Path;

use sha2::{Digest, Sha256};

use crate::trailer::{TRAILER_LEN, Trailer};

/// Whether [`inject_and_sign`] applies an ad-hoc signature.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodeSign {
    /// Grow `__LINKEDIT` over the payload and apply a plain, unsigned ad-hoc
    /// `CodeDirectory` over the finished file.
    Adhoc,
    /// Append the payload after the file and stop, dropping any
    /// `LC_CODE_SIGNATURE` the stub carried.
    ///
    /// Exists so that [`crate::payload::locate`] can be tested against a
    /// Mach-O carrying a payload without also depending on the signer; a real
    /// arm64 build always signs, because the kernel will not otherwise map it.
    None,
}

/// How [`inject_and_sign`] is asked to sign a stub.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MacSignCfg {
    /// Whether to sign after the payload is written.
    pub codesign: CodeSign,
}

/// What [`inject_and_sign`] did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InjectReport {
    /// The absolute file offset of the payload region's first byte — the
    /// packed payload, immediately followed by its 64-byte trailer — in the
    /// file [`inject_and_sign`] wrote.
    pub payload_offset: u64,
    /// The payload region's size in bytes: the payload plus its 64-byte
    /// trailer.
    pub payload_size: u64,
    /// Whether an ad-hoc signature was applied.
    pub signed: bool,
    /// Where the `LC_CODE_SIGNATURE` the finished file carries came from, or
    /// [`None`] when nothing was signed.
    ///
    /// An arm64 Mach-O always arrives with one, because the linker ad-hoc
    /// signs every arm64 image it produces; an x86_64 one often does not, and
    /// then the command is added into the load-command slack. Both are correct
    /// outcomes and they are not interchangeable, so the report says which one
    /// happened rather than leaving a caller to infer it from the bytes.
    pub code_signature: Option<CodeSignatureSlot>,
    /// The stub's `cputype`, spelled the way [`crate::macho::MachoFacts`]
    /// spells it.
    pub cputype: String,
}

/// Where the `LC_CODE_SIGNATURE` a signed artifact carries came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodeSignatureSlot {
    /// The stub already carried one, and it was pointed at the new signature.
    /// No load command was added, so not one byte of the stub moved.
    Reused,
    /// The stub carried none, and a 16-byte `linkedit_data_command` was
    /// written into the slack a linker leaves between the last load command
    /// and the first section. `ncmds` and `sizeofcmds` grow by one command;
    /// every file offset in the image stays where the linker put it, because
    /// the bytes the command occupies were spare.
    Added,
}

/// The length of the `linkedit_data_command` an `LC_CODE_SIGNATURE` is: a
/// command header (`cmd`, `cmdsize`) and the two `u32` fields `dataoff` and
/// `datasize`.
///
/// This is the whole reason the missing-signature case is fixable at all. A
/// new `LC_SEGMENT_64` with one `section_64` in it — what carving a payload
/// section would have needed — is 152 bytes, and the sample stub has 40 spare;
/// 16 fits where 152 does not.
pub const CODE_SIGNATURE_COMMAND_LEN: u64 = 16;

/// What a stub's load-command area has room for.
///
/// A Mach-O's load commands run from the end of its 32-byte header to
/// `sizeofcmds` bytes later, and the first section's file offset is where the
/// image's own content begins. A linker rounds the gap between the two up, so
/// there is normally a little slack: bytes that belong to no command and no
/// section, and that a new load command can therefore be written into without
/// moving anything.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoadCommandSlack {
    /// Where the load commands end: 32 plus the header's `sizeofcmds`.
    pub commands_end: u64,
    /// The lowest file offset any section in any segment begins at — the
    /// first byte of content that a growing command area would overwrite.
    pub first_content_offset: u64,
    /// How many bytes lie between the two, and are therefore free.
    pub free: u64,
}

/// Why a Mach-O could not be written or signed.
#[derive(Debug, thiserror::Error)]
pub enum SignMacosError {
    /// `stub_bytes` is not a Mach-O this crate can read at all.
    #[error("the stub is not a Mach-O this ginary can read: {source}")]
    NotAMachO {
        /// What [`crate::macho::read`] said.
        #[source]
        source: crate::macho::MachoError,
    },
    /// `stub_bytes` is a fat (universal) Mach-O.
    ///
    /// [`crate::macho::read`] does not refuse a fat binary itself — it is
    /// still a Mach-O — but this function needs exactly one architecture to
    /// grow a segment of, so it is the one that checks
    /// [`crate::macho::MachoFacts::is_fat`] and refuses here.
    #[error("the stub is a fat Mach-O carrying more than one architecture; a thin one is required")]
    Fat,
    /// `stub_bytes` already carries a `__GINARY,__payload` section.
    ///
    /// Older ginary builds wrote the payload into a section; a stub that
    /// already carries one is an artifact, not a fresh stub, and a payload
    /// may not be added twice.
    #[error(
        "the stub already carries a __GINARY,__payload section; a payload may not be added twice"
    )]
    AlreadySectioned,
    /// The stub carries no `LC_CODE_SIGNATURE` to reuse, and the slack before
    /// its first section is too small to add one.
    ///
    /// The load-command area cannot grow past the first section without
    /// relocating every byte of code and data behind it, which invalidates the
    /// entry point and every offset `LC_DYLD_CHAINED_FIXUPS` encodes. So this
    /// is refused rather than attempted, and the numbers that decided it are
    /// reported: how many bytes a command needs, and how many were free.
    #[error(
        "cannot ad-hoc sign a Mach-O with no LC_CODE_SIGNATURE to reuse: adding one needs \
         {needed} bytes of load command and only {free} are free before the first section, and \
         the load-command area cannot grow without relocating code"
    )]
    NoRoomForCodeSignature {
        /// The bytes a `linkedit_data_command` needs:
        /// [`CODE_SIGNATURE_COMMAND_LEN`].
        needed: u64,
        /// The bytes actually free, from
        /// [`LoadCommandSlack::free`].
        free: u64,
    },
    /// The finished file could not be written to `out`.
    #[error("cannot write `{path}`: {source}")]
    Io {
        /// The file that could not be written.
        path: std::path::PathBuf,
        /// The underlying operating system error.
        #[source]
        source: std::io::Error,
    },
}

/// Appends `payload_with_trailer` to `stub_bytes` inside its `__LINKEDIT`
/// segment and, per `cfg`, applies an ad-hoc code signature, writing the
/// result to `out`.
///
/// `payload_with_trailer` is exactly what the ELF and PE path already
/// produces: the 64-byte trailer from [`crate::trailer::Trailer::to_bytes`]
/// followed by the packed payload. This function relays it into the layout a
/// signed Mach-O needs — the packed bytes first, then a trailer whose
/// `payload_offset` is absolute, so [`crate::payload::locate`] finds it the
/// same way it finds an ELF's — and grows `__LINKEDIT` so the ad-hoc
/// signature covers all of it. See `docs/format.md` and [`crate::payload::locate`].
///
/// # Errors
///
/// [`SignMacosError::NotAMachO`] when `stub_bytes` is not a Mach-O this crate
/// can read — including when its geometry cannot be safely rewritten, once
/// reading has already confirmed it is a whole Mach-O — [`SignMacosError::Fat`]
/// when it is a fat one, [`SignMacosError::AlreadySectioned`] when it already
/// carries the section an older ginary wrote, and [`SignMacosError::Io`] when
/// `out` could not be written.
pub fn inject_and_sign(
    stub_bytes: &[u8],
    payload_with_trailer: &[u8],
    out: &Path,
    cfg: &MacSignCfg,
) -> Result<InjectReport, SignMacosError> {
    if !crate::macho::is_macho(stub_bytes) {
        return Err(SignMacosError::NotAMachO {
            source: crate::macho::MachoError::NotMachO,
        });
    }
    let facts =
        crate::macho::read(stub_bytes).map_err(|source| SignMacosError::NotAMachO { source })?;
    if facts.is_fat {
        return Err(SignMacosError::Fat);
    }
    if crate::macho::section(
        stub_bytes,
        crate::macho::PAYLOAD_SEGMENT,
        crate::macho::PAYLOAD_SECTION,
    )
    .is_some()
    {
        return Err(SignMacosError::AlreadySectioned);
    }
    if stub_bytes.get(0..4) != Some(&crate::macho::MH_MAGIC_64.to_le_bytes()[..]) {
        return Err(SignMacosError::NotAMachO {
            source: crate::macho::MachoError::Parse {
                message: "only a little-endian 64-bit thin Mach-O stub can be injected into"
                    .to_owned(),
            },
        });
    }

    let writer = Writer::plan(
        stub_bytes,
        payload_with_trailer,
        cfg.codesign == CodeSign::Adhoc,
    )?;
    let built = writer.build()?;

    std::fs::write(out, &built.body).map_err(|source| SignMacosError::Io {
        path: out.to_path_buf(),
        source,
    })?;

    Ok(InjectReport {
        payload_offset: built.payload_offset,
        payload_size: built.payload_size,
        signed: cfg.codesign == CodeSign::Adhoc,
        code_signature: writer.codesig_slot,
        cputype: facts.cputype,
    })
}

/// Measures the load-command slack of the thin 64-bit Mach-O `stub`.
///
/// This is the measurement the missing-`LC_CODE_SIGNATURE` case turns on: a
/// command can be added exactly when [`LoadCommandSlack::free`] is at least
/// [`CODE_SIGNATURE_COMMAND_LEN`]. It is a function of its own rather than a
/// step inside [`inject_and_sign`] so that a test — and a person holding a
/// stub they are unsure of — can ask the question without writing a file.
///
/// # Errors
///
/// [`SignMacosError::NotAMachO`] when `stub` is not a thin 64-bit
/// little-endian Mach-O, or when its header, its load commands or one of its
/// sections runs past the end of the buffer.
pub fn load_command_slack(stub: &[u8]) -> Result<LoadCommandSlack, SignMacosError> {
    if stub.get(0..4) != Some(&crate::macho::MH_MAGIC_64.to_le_bytes()[..]) {
        return Err(parse_error(
            "only a little-endian 64-bit thin Mach-O has a load-command area this can measure",
        ));
    }
    let geometry = command_geometry(stub)?;
    // A file with no section at all has nothing the command area could
    // overwrite before the end of the file, so the file's own length is the
    // honest bound; every stub a real linker produces has one.
    let first_content_offset = geometry.first_content_offset.unwrap_or(geometry.file_len);
    Ok(LoadCommandSlack {
        commands_end: geometry.commands_end,
        first_content_offset,
        free: first_content_offset.saturating_sub(geometry.commands_end),
    })
}

/// Where a thin 64-bit Mach-O's load commands end, and where its content
/// begins.
#[derive(Clone, Copy, Debug)]
struct CommandGeometry {
    /// `32 + sizeofcmds`, checked against the buffer's length.
    commands_end: u64,
    /// The lowest non-zero section file offset in the image, or [`None`] when
    /// the image declares no section at all.
    first_content_offset: Option<u64>,
    /// The buffer's length.
    file_len: u64,
}

/// Walks `stub`'s load commands and reports [`CommandGeometry`].
///
/// The walk is the same one [`Writer::plan`] does, kept separate so that a
/// caller measuring a stub does not have to plan a write of it.
fn command_geometry(stub: &[u8]) -> Result<CommandGeometry, SignMacosError> {
    let ncmds = get_u32(stub, 16)?;
    let sizeofcmds = get_u32(stub, 20)? as usize;
    let commands_end = HEADER_LEN
        .checked_add(sizeofcmds)
        .ok_or_else(|| parse_error("the load command area overflows"))?;
    if stub.len() < commands_end {
        return Err(parse_error(
            "the load commands run past the end of the file",
        ));
    }

    let mut offset = HEADER_LEN;
    let mut first_content_offset: Option<u64> = None;
    for _ in 0..ncmds {
        let cmd = get_u32(stub, offset)?;
        let size_field = offset
            .checked_add(4)
            .ok_or_else(|| parse_error("a load command offset overflows"))?;
        let cmdsize = get_u32(stub, size_field)? as usize;
        if cmdsize < 8 {
            return Err(parse_error("a load command is shorter than its own header"));
        }
        let end = offset
            .checked_add(cmdsize)
            .ok_or_else(|| parse_error("a load command's size overflows"))?;
        if end > stub.len() {
            return Err(parse_error("a load command runs past the end of the file"));
        }
        if cmd == LC_SEGMENT_64 {
            let raw = &stub[offset..end];
            if raw.len() < SEGMENT_CMD_LEN {
                return Err(parse_error("a segment command is shorter than 72 bytes"));
            }
            first_content_offset = min_section_offset(raw, first_content_offset)?;
        }
        offset = end;
    }

    Ok(CommandGeometry {
        commands_end: commands_end as u64,
        first_content_offset,
        file_len: stub.len() as u64,
    })
}

// ----------------------------------------------------------- the writer --

/// The page alignment `__LINKEDIT`'s grown `vmsize` is rounded up to, matching
/// Apple Silicon's page size. A segment's `vmsize` must be a whole number of
/// pages; nothing this crate reads depends on the exact value, only a real
/// kernel's page-in would.
const SEGMENT_PAGE_ALIGN: u64 = 0x4000;

/// The page size Apple's ad-hoc `CodeDirectory` hashes over, independent of
/// [`SEGMENT_PAGE_ALIGN`]: `page_size` field value `12`, `log2(4096)`.
const CODE_DIRECTORY_PAGE: usize = 4096;

/// The boundary a code signature begins on, as every linker-produced one does.
///
/// `codesign --verify --strict` reads `LC_CODE_SIGNATURE`'s `dataoff` and
/// expects a superblob there; an unaligned one is a shape no `ld` emits and
/// nothing else in the file needs, so the gap is padded and the padding is
/// inside what the hashes cover.
const SIGNATURE_ALIGNMENT: u64 = 16;

/// `CSMAGIC_EMBEDDED_SIGNATURE`, the superblob a signed Mach-O carries.
const CSMAGIC_EMBEDDED_SIGNATURE: u32 = 0xfade_0cc0;

/// `CSMAGIC_CODEDIRECTORY`, the one blob this module writes inside it.
const CSMAGIC_CODEDIRECTORY: u32 = 0xfade_0c02;

/// The identifier an ad-hoc `CodeDirectory` names, NUL-terminated.
///
/// An ad-hoc signature asserts no identity. Nothing reads it back.
const AD_HOC_IDENTIFIER: &[u8] = b"a.out\0";

/// The length of one SHA-256, the only hash this module writes.
const AD_HOC_HASH_LEN: usize = 32;

/// The fixed part of a `CodeDirectory` blob, up to `execSegFlags`: version
/// `0x0002_0400`, which is `CS_SUPPORTSEXECSEG`.
const AD_HOC_CD_HEADER_LEN: usize = 88;

/// `CSMAGIC_EMBEDDED_SIGNATURE`'s own header plus the one blob index entry
/// that follows it.
const AD_HOC_SUPERBLOB_AND_BLOB_LEN: usize = 20;

/// Where each part of the ad-hoc signature written at a given file offset
/// lands, and how long the whole of it is.
///
/// One producer for the length. [`Writer::build`] has to write `datasize` into
/// the load commands *before* it hashes the pages those commands sit on, so it
/// needs the size of a blob that does not exist yet; deriving it twice, once
/// there and once in [`build_ad_hoc_signature`], is two things that can drift
/// apart. Every field is a function of `sig_off` alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AdHocLayout {
    /// One code slot per [`CODE_DIRECTORY_PAGE`]-byte page below `sig_off`.
    n_hashes: usize,
    /// Where [`AD_HOC_IDENTIFIER`] starts, relative to the blob.
    ident_offset: usize,
    /// Where code slot 0 starts, relative to the blob.
    hash_offset: usize,
    /// The `CodeDirectory` blob's own length.
    cd_len: usize,
    /// The whole superblob's length: `datasize`.
    total_len: usize,
}

impl AdHocLayout {
    /// The layout of the signature that begins at `sig_off` and covers
    /// everything below it.
    const fn at(sig_off: usize) -> Self {
        let n_hashes = sig_off.div_ceil(CODE_DIRECTORY_PAGE);
        let ident_offset = AD_HOC_CD_HEADER_LEN;
        let hash_offset = ident_offset + AD_HOC_IDENTIFIER.len();
        let cd_len = hash_offset + n_hashes * AD_HOC_HASH_LEN;
        Self {
            n_hashes,
            ident_offset,
            hash_offset,
            cd_len,
            total_len: AD_HOC_SUPERBLOB_AND_BLOB_LEN + cd_len,
        }
    }
}

/// The length of a 64-bit Mach-O header.
const HEADER_LEN: usize = 32;
/// The length of one `segment_command_64`, header fields only.
const SEGMENT_CMD_LEN: usize = 72;

const LC_SEGMENT_64: u32 = 0x19;
const LC_CODE_SIGNATURE: u32 = 0x1d;

/// Everything [`inject_and_sign`] needs to write the finished file, planned
/// once and then assembled by [`Writer::build`].
///
/// No existing byte of the stub moves. The load commands are rewritten only to
/// drop a stale `LC_CODE_SIGNATURE` (for an unsigned build) and to leave room
/// for the fields [`Writer::build`] patches once the payload's size is known;
/// the command area is padded back to the offset the stub's own content starts
/// at, so every segment and section keeps the file offset the linker gave it.
struct Writer {
    /// The Mach-O header, `ncmds`/`sizeofcmds` patched in [`Writer::build`].
    header: [u8; HEADER_LEN],
    /// The load commands to write, in order, with any stale
    /// `LC_CODE_SIGNATURE` removed and, when signing, a fresh one kept.
    commands: Vec<Vec<u8>>,
    /// The zero bytes written after the commands so the stub's own content
    /// still begins at [`Writer::content`]'s original file offset.
    lc_padding: usize,
    /// The stub's content from the end of its load commands up to
    /// [`Writer::content_end`]: every segment and section, unchanged.
    content: Vec<u8>,
    /// The packed payload bytes, without any trailer.
    payload: Vec<u8>,
    /// SHA-256 of the payload, from the trailer the caller supplied.
    payload_sha256: [u8; 32],
    /// Whether an ad-hoc signature is applied over the finished file.
    sign: bool,
    /// The index into [`Writer::commands`] of the `LC_CODE_SIGNATURE` to
    /// patch, when signing.
    codesig_index: Option<usize>,
    /// Where that command came from, for [`InjectReport::code_signature`].
    codesig_slot: Option<CodeSignatureSlot>,
    /// The index into [`Writer::commands`] of the `__LINKEDIT` segment to
    /// grow, when signing.
    linkedit_index: Option<usize>,
    /// `__TEXT`'s `fileoff`, for the `CodeDirectory`'s executable-segment base.
    text_fileoff: u64,
    /// `__TEXT`'s `filesize`, for the executable-segment limit.
    text_filesize: u64,
}

/// What [`Writer::build`] produced.
struct Built {
    /// The finished file's bytes.
    body: Vec<u8>,
    /// The absolute file offset of the trailer that begins the payload region.
    payload_offset: u64,
    /// The trailer plus the payload, in bytes.
    payload_size: u64,
}

impl Writer {
    /// Reads `stub`'s load commands, drops any stale `LC_CODE_SIGNATURE` when
    /// not signing, and records where `__LINKEDIT`, `__TEXT` and the payload go
    /// — without writing anything yet.
    fn plan(stub: &[u8], payload_with_trailer: &[u8], sign: bool) -> Result<Self, SignMacosError> {
        if (payload_with_trailer.len() as u64) < TRAILER_LEN {
            return Err(parse_error(
                "the payload handed to the Mach-O signer is shorter than its own trailer",
            ));
        }
        let split = TRAILER_LEN as usize;
        let payload = payload_with_trailer[split..].to_vec();
        let mut payload_sha256 = [0u8; 32];
        payload_sha256.copy_from_slice(&payload_with_trailer[24..56]);

        let mut header = [0u8; HEADER_LEN];
        header.copy_from_slice(
            stub.get(0..HEADER_LEN)
                .ok_or_else(|| parse_error("the file is shorter than a Mach-O header"))?,
        );
        let ncmds = get_u32(stub, 16)?;
        let sizeofcmds = get_u32(stub, 20)? as usize;
        let old_lc_end = HEADER_LEN
            .checked_add(sizeofcmds)
            .ok_or_else(|| parse_error("the load command area overflows"))?;
        if stub.len() < old_lc_end {
            return Err(parse_error(
                "the load commands run past the end of the file",
            ));
        }

        let mut commands: Vec<Vec<u8>> = Vec::with_capacity(ncmds as usize);
        let mut offset = HEADER_LEN;
        let mut old_codesig_dataoff: Option<u64> = None;
        let mut first_content_off: Option<u64> = None;
        for _ in 0..ncmds {
            let cmd = get_u32(stub, offset)?;
            let size_field = offset
                .checked_add(4)
                .ok_or_else(|| parse_error("a load command offset overflows"))?;
            let cmdsize = get_u32(stub, size_field)? as usize;
            if cmdsize < 8 {
                return Err(parse_error("a load command is shorter than its own header"));
            }
            let end = offset
                .checked_add(cmdsize)
                .ok_or_else(|| parse_error("a load command's size overflows"))?;
            if end > stub.len() {
                return Err(parse_error("a load command runs past the end of the file"));
            }
            let raw = stub[offset..end].to_vec();
            if cmd == LC_SEGMENT_64 {
                if raw.len() < SEGMENT_CMD_LEN {
                    return Err(parse_error("a segment command is shorter than 72 bytes"));
                }
                first_content_off = min_section_offset(&raw, first_content_off)?;
            }
            if cmd == LC_CODE_SIGNATURE {
                old_codesig_dataoff = Some(u64::from(get_u32(&raw, 8)?));
                // A stale signature covers the wrong bytes. It is kept only when
                // this build will re-sign and patch it; otherwise it is dropped.
                if sign {
                    commands.push(raw);
                }
            } else {
                commands.push(raw);
            }
            offset = end;
        }

        // A signed build needs a command to point at the signature. The linker
        // ad-hoc signs every arm64 image it produces, so an arm64 stub always
        // arrives with one to reuse; an x86_64 stub often does not, and then a
        // sixteen-byte `linkedit_data_command` is written into the slack that
        // sits between the last load command and the first section. Those bytes
        // belong to no command and no section, so the command area grows into
        // spare room and every file offset in the image stays where the linker
        // put it. Only a stub whose slack is genuinely smaller than a command
        // is refused, and the refusal names both numbers.
        let mut codesig_slot = None;
        let codesig_index = if sign {
            match commands
                .iter()
                .position(|raw| get_u32(raw, 0).unwrap_or(0) == LC_CODE_SIGNATURE)
            {
                Some(index) => {
                    codesig_slot = Some(CodeSignatureSlot::Reused);
                    Some(index)
                }
                None => {
                    let free = first_content_off
                        .unwrap_or(stub.len() as u64)
                        .saturating_sub(old_lc_end as u64);
                    if free < CODE_SIGNATURE_COMMAND_LEN {
                        return Err(SignMacosError::NoRoomForCodeSignature {
                            needed: CODE_SIGNATURE_COMMAND_LEN,
                            free,
                        });
                    }
                    commands.push(new_code_signature_command());
                    codesig_slot = Some(CodeSignatureSlot::Added);
                    Some(commands.len() - 1)
                }
            }
        } else {
            None
        };

        // Where the payload begins. A signed build drops the old signature's
        // bytes and writes the payload where they were; an unsigned one keeps
        // the whole stub and appends after it.
        let content_end = match old_codesig_dataoff {
            Some(dataoff) if sign => dataoff,
            _ => stub.len() as u64,
        };
        let content_end = usize::try_from(content_end)
            .ok()
            .filter(|end| *end >= old_lc_end && *end <= stub.len())
            .ok_or_else(|| parse_error("the payload boundary is outside the file"))?;

        let new_sizeofcmds: usize = commands.iter().map(Vec::len).sum();
        let new_lc_end = HEADER_LEN
            .checked_add(new_sizeofcmds)
            .ok_or_else(|| parse_error("the rewritten load command area overflows"))?;
        // The command area shrinks (a dropped signature), stays the same, or
        // grows by exactly one command into the slack. When it shrinks the
        // padding makes the difference up so nothing moves; when it grows it
        // eats slack, and the stub's own bytes are taken from past it — either
        // way the first content byte lands on the offset the linker gave it.
        let lc_padding = old_lc_end.saturating_sub(new_lc_end);
        let content_from = old_lc_end.max(new_lc_end);
        if let Some(first) = first_content_off
            && (new_lc_end as u64) > first
        {
            return Err(parse_error(
                "the rewritten load commands would overwrite the stub's own content",
            ));
        }

        let linkedit_index = commands.iter().position(|raw| {
            get_u32(raw, 0).unwrap_or(0) == LC_SEGMENT_64 && segment_name(raw) == "__LINKEDIT"
        });

        let (text_fileoff, text_filesize) = commands
            .iter()
            .find(|raw| {
                get_u32(raw, 0).unwrap_or(0) == LC_SEGMENT_64 && segment_name(raw) == "__TEXT"
            })
            .map(|raw| (get_u64(raw, 40).unwrap_or(0), get_u64(raw, 48).unwrap_or(0)))
            .unwrap_or((0, 0));

        let content = stub
            .get(content_from..content_end)
            .ok_or_else(|| parse_error("the payload boundary is outside the file"))?
            .to_vec();

        Ok(Self {
            header,
            commands,
            lc_padding,
            content,
            payload,
            payload_sha256,
            sign,
            codesig_index,
            codesig_slot,
            linkedit_index,
            text_fileoff,
            text_filesize,
        })
    }

    /// Assembles the header, the (unmoved) content, the payload and the ad-hoc
    /// signature into the finished bytes.
    fn build(&self) -> Result<Built, SignMacosError> {
        let new_sizeofcmds: usize = self.commands.iter().map(Vec::len).sum();
        let mut header = self.header;
        put_u32(
            &mut header,
            16,
            u32::try_from(self.commands.len()).unwrap_or(u32::MAX),
        )?;
        put_u32(
            &mut header,
            20,
            u32::try_from(new_sizeofcmds).unwrap_or(u32::MAX),
        )?;

        let mut body = Vec::with_capacity(
            HEADER_LEN
                + new_sizeofcmds
                + self.lc_padding
                + self.content.len()
                + self.payload.len()
                + TRAILER_LEN as usize
                + 1024,
        );
        body.extend_from_slice(&header);

        let mut command_offsets = Vec::with_capacity(self.commands.len());
        for raw in &self.commands {
            command_offsets.push(body.len());
            body.extend_from_slice(raw);
        }
        // Pad the command area back to where the stub's content started, so
        // every file offset the load commands name is still correct.
        body.extend(std::iter::repeat_n(0u8, self.lc_padding));
        body.extend_from_slice(&self.content);

        let payload_len = self.payload.len() as u64;

        if self.sign {
            // The payload and its trailer end exactly where the signature
            // begins, on a 16-byte boundary, so `locate` reads the trailer at
            // `dataoff - 64`. The alignment padding therefore goes *before* the
            // payload, not after the trailer.
            let projected = (body.len() as u64)
                .saturating_add(payload_len)
                .saturating_add(TRAILER_LEN);
            let pad_before =
                usize::try_from(align_up(projected, SIGNATURE_ALIGNMENT).saturating_sub(projected))
                    .unwrap_or(0);
            body.extend(std::iter::repeat_n(0u8, pad_before));

            let payload_offset = body.len() as u64;
            body.extend_from_slice(&self.payload);
            body.extend_from_slice(
                &Trailer {
                    payload_offset,
                    payload_len,
                    payload_sha256: self.payload_sha256,
                }
                .to_bytes(),
            );

            let sig_off = body.len();
            let sig_size = AdHocLayout::at(sig_off).total_len;

            // Grow `__LINKEDIT` so it still ends the file, now with the payload
            // and the signature inside it, and point `LC_CODE_SIGNATURE` at the
            // signature. Both are patched before the pages are hashed: they live
            // in page 0, and a field written after the hash is a page the kernel
            // refuses to map — which it reports by killing the process rather
            // than by saying so.
            if let Some(index) = self.linkedit_index
                && let Some(&at) = command_offsets.get(index)
            {
                let linkedit_fileoff = get_u64(&body, at + 40)?;
                let filesize = (sig_off as u64)
                    .saturating_add(sig_size as u64)
                    .saturating_sub(linkedit_fileoff);
                put_u64(&mut body, at + 48, filesize)?;
                put_u64(&mut body, at + 32, align_up(filesize, SEGMENT_PAGE_ALIGN))?;
            }
            if let Some(index) = self.codesig_index
                && let Some(&at) = command_offsets.get(index)
            {
                put_u32(
                    &mut body,
                    at + 8,
                    u32::try_from(sig_off).unwrap_or(u32::MAX),
                )?;
                put_u32(
                    &mut body,
                    at + 12,
                    u32::try_from(sig_size).unwrap_or(u32::MAX),
                )?;
            }

            let blob = build_ad_hoc_signature(&body, self.text_fileoff, self.text_filesize);
            body.extend_from_slice(&blob);

            Ok(Built {
                body,
                payload_offset,
                payload_size: payload_len.saturating_add(TRAILER_LEN),
            })
        } else {
            let payload_offset = body.len() as u64;
            body.extend_from_slice(&self.payload);
            body.extend_from_slice(
                &Trailer {
                    payload_offset,
                    payload_len,
                    payload_sha256: self.payload_sha256,
                }
                .to_bytes(),
            );
            Ok(Built {
                body,
                payload_offset,
                payload_size: payload_len.saturating_add(TRAILER_LEN),
            })
        }
    }
}

/// A fresh, empty `LC_CODE_SIGNATURE`: the sixteen bytes of a
/// `linkedit_data_command` with `dataoff` and `datasize` left at zero for
/// [`Writer::build`] to patch once the signature's place is known.
///
/// This is the whole of what the missing-signature case adds to a stub. It is
/// written into the slack between the last load command and the first section,
/// so `ncmds` and `sizeofcmds` grow and not one byte of content moves.
fn new_code_signature_command() -> Vec<u8> {
    let mut raw = Vec::with_capacity(CODE_SIGNATURE_COMMAND_LEN as usize);
    raw.extend_from_slice(&LC_CODE_SIGNATURE.to_le_bytes());
    raw.extend_from_slice(
        &u32::try_from(CODE_SIGNATURE_COMMAND_LEN)
            .unwrap_or(u32::MAX)
            .to_le_bytes(),
    );
    raw.extend_from_slice(&0u32.to_le_bytes()); // dataoff, patched by `build`
    raw.extend_from_slice(&0u32.to_le_bytes()); // datasize, patched by `build`
    raw
}

/// A plain, unsigned ad-hoc `CodeDirectory` (`CSMAGIC_EMBEDDED_SIGNATURE`
/// wrapping one `CSMAGIC_CODEDIRECTORY` blob) over `body`, SHA-256 per
/// [`CODE_DIRECTORY_PAGE`]-byte page — the same layout `codesign -s -`
/// produces, asserting no identity beyond "these are the bytes ginary
/// built".
fn build_ad_hoc_signature(body: &[u8], text_fileoff: u64, text_filesize: u64) -> Vec<u8> {
    let sig_off = body.len();
    let AdHocLayout {
        n_hashes,
        ident_offset,
        hash_offset,
        cd_len,
        total_len,
    } = AdHocLayout::at(sig_off);
    let mut out = Vec::with_capacity(total_len);
    out.extend_from_slice(&CSMAGIC_EMBEDDED_SIGNATURE.to_be_bytes());
    out.extend_from_slice(&u32::try_from(total_len).unwrap_or(u32::MAX).to_be_bytes());
    out.extend_from_slice(&1u32.to_be_bytes()); // count
    out.extend_from_slice(&0u32.to_be_bytes()); // CSSLOT_CODEDIRECTORY
    out.extend_from_slice(
        &u32::try_from(AD_HOC_SUPERBLOB_AND_BLOB_LEN)
            .unwrap_or(u32::MAX)
            .to_be_bytes(),
    );

    out.extend_from_slice(&CSMAGIC_CODEDIRECTORY.to_be_bytes());
    out.extend_from_slice(&u32::try_from(cd_len).unwrap_or(u32::MAX).to_be_bytes());
    out.extend_from_slice(&0x0002_0400u32.to_be_bytes()); // version
    // flags: CS_ADHOC only. ginary rewrote and re-signed this binary; it did
    // not come out of a linker, so CS_LINKER_SIGNED (0x20000) would claim a
    // provenance that is no longer true. See docs/dev/log/E9.md.
    out.extend_from_slice(&0x0000_0002u32.to_be_bytes());
    out.extend_from_slice(&u32::try_from(hash_offset).unwrap_or(u32::MAX).to_be_bytes());
    out.extend_from_slice(
        &u32::try_from(ident_offset)
            .unwrap_or(u32::MAX)
            .to_be_bytes(),
    );
    out.extend_from_slice(&0u32.to_be_bytes()); // n_special_slots
    out.extend_from_slice(&u32::try_from(n_hashes).unwrap_or(u32::MAX).to_be_bytes());
    out.extend_from_slice(&u32::try_from(sig_off).unwrap_or(u32::MAX).to_be_bytes()); // code_limit
    out.push(u8::try_from(AD_HOC_HASH_LEN).unwrap_or(u8::MAX)); // hash_size
    out.push(2); // hash_type: SHA-256
    out.push(0); // pad1
    out.push(12); // page_size: log2(4096)
    out.extend_from_slice(&0u32.to_be_bytes()); // pad2
    out.extend_from_slice(&0u32.to_be_bytes()); // scatter_offset
    out.extend_from_slice(&0u32.to_be_bytes()); // team_offset
    out.extend_from_slice(&0u32.to_be_bytes()); // pad3
    out.extend_from_slice(&u64::try_from(sig_off).unwrap_or(u64::MAX).to_be_bytes()); // code_limit64
    out.extend_from_slice(&text_fileoff.to_be_bytes()); // exec_seg_base
    out.extend_from_slice(&text_filesize.to_be_bytes()); // exec_seg_limit
    out.extend_from_slice(&1u64.to_be_bytes()); // exec_seg_flags: CS_EXECSEG_MAIN_BINARY
    out.extend_from_slice(AD_HOC_IDENTIFIER);

    let mut hasher = Sha256::new();
    let mut done = 0usize;
    while done < sig_off {
        let take = CODE_DIRECTORY_PAGE.min(sig_off - done);
        hasher.update(&body[done..done + take]);
        out.extend_from_slice(&hasher.finalize_reset());
        done += take;
    }

    out
}

/// The `segname` bytes of a raw `segment_command_64`, trimmed at the first
/// NUL, or the empty string when `raw` is too short to hold one — never a
/// panic, since these bytes may have come from anywhere.
fn segment_name(raw: &[u8]) -> &str {
    let bytes = raw.get(8..24).unwrap_or(&[]);
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    std::str::from_utf8(&bytes[..end]).unwrap_or("")
}

/// The smallest file offset any section in `raw` (a `segment_command_64`)
/// begins at, folded with `current`, ignoring the `0` offset a `__bss`-style
/// section or a header-mapping segment carries.
///
/// This is the first byte of real content after the load commands: the writer
/// must not let the rewritten command area reach it.
fn min_section_offset(raw: &[u8], current: Option<u64>) -> Result<Option<u64>, SignMacosError> {
    let nsects = get_u32(raw, 64)?;
    let mut found = current;
    for index in 0..nsects {
        let section_start = SEGMENT_CMD_LEN
            .checked_add(
                usize::try_from(index)
                    .ok()
                    .and_then(|index| index.checked_mul(80))
                    .ok_or_else(|| parse_error("a section index overflows"))?,
            )
            .ok_or_else(|| parse_error("a section's position overflows"))?;
        let offset_field = section_start
            .checked_add(48)
            .ok_or_else(|| parse_error("a section's offset field overflows"))?;
        if offset_field + 4 > raw.len() {
            // A stub whose declared `nsects` runs past the command it sits in is
            // one `macho::read` already refused; the writer never reaches here.
            break;
        }
        let offset = u64::from(get_u32(raw, offset_field)?);
        if offset > 0 {
            found = Some(found.map_or(offset, |current| current.min(offset)));
        }
    }
    Ok(found)
}

/// A [`SignMacosError::NotAMachO`] wrapping [`crate::macho::MachoError::Parse`],
/// for the write path's own bounds and geometry checks: the stub read
/// cleanly as a whole Mach-O by `macho::read`, and then turned out not to be
/// one this function can safely rewrite.
fn parse_error(message: &str) -> SignMacosError {
    SignMacosError::NotAMachO {
        source: crate::macho::MachoError::Parse {
            message: message.to_owned(),
        },
    }
}

/// `value`, rounded up to the next multiple of `align`.
fn align_up(value: u64, align: u64) -> u64 {
    if align == 0 {
        return value;
    }
    let remainder = value % align;
    if remainder == 0 {
        value
    } else {
        value.saturating_add(align - remainder)
    }
}

/// The little-endian `u32` at `at`.
fn get_u32(buf: &[u8], at: usize) -> Result<u32, SignMacosError> {
    let end = at
        .checked_add(4)
        .ok_or_else(|| parse_error("a field offset overflows"))?;
    let slice = buf
        .get(at..end)
        .ok_or_else(|| parse_error("a field runs past the end of a load command"))?;
    let mut bytes = [0u8; 4];
    bytes.copy_from_slice(slice);
    Ok(u32::from_le_bytes(bytes))
}

/// The little-endian `u64` at `at`.
fn get_u64(buf: &[u8], at: usize) -> Result<u64, SignMacosError> {
    let end = at
        .checked_add(8)
        .ok_or_else(|| parse_error("a field offset overflows"))?;
    let slice = buf
        .get(at..end)
        .ok_or_else(|| parse_error("a field runs past the end of a load command"))?;
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(slice);
    Ok(u64::from_le_bytes(bytes))
}

/// Writes `value` as little-endian at `at`.
fn put_u32(buf: &mut [u8], at: usize, value: u32) -> Result<(), SignMacosError> {
    let end = at
        .checked_add(4)
        .ok_or_else(|| parse_error("a field offset overflows"))?;
    let slice = buf
        .get_mut(at..end)
        .ok_or_else(|| parse_error("a field runs past the end of a load command"))?;
    slice.copy_from_slice(&value.to_le_bytes());
    Ok(())
}

/// Writes `value` as little-endian at `at`.
fn put_u64(buf: &mut [u8], at: usize, value: u64) -> Result<(), SignMacosError> {
    let end = at
        .checked_add(8)
        .ok_or_else(|| parse_error("a field offset overflows"))?;
    let slice = buf
        .get_mut(at..end)
        .ok_or_else(|| parse_error("a field runs past the end of a load command"))?;
    slice.copy_from_slice(&value.to_le_bytes());
    Ok(())
}
