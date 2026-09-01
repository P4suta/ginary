// SPDX-License-Identifier: MIT OR Apache-2.0
//! Writing a macOS artifact: the payload section, and the ad-hoc signature
//! over it.
//!
//! An ELF or a PE artifact is the stub with the payload and the trailer
//! appended after it. A Mach-O cannot be built that way: bytes appended past
//! `__LINKEDIT` fail `codesign --strict`, and an arm64 kernel refuses to map
//! a page it cannot verify the signature of at all — an *unsigned* binary,
//! not merely one whose signature does not match. See ADR
//! [0016](../../../docs/adr/0016-macho-section-payload-and-adhoc-signing.md)
//! for the two findings and their sources.
//!
//! So this module does what every macOS app-bundler does: it writes the
//! payload into a dedicated `__GINARY,__payload` section — a section is
//! ordinary content a Mach-O carries, not an evasion of anything — and then
//! applies a plain, unsigned, ad-hoc `CodeDirectory` over ginary's own
//! output, which is what makes the kernel willing to load it. There is no
//! signature stripping, no impersonation of another signer, and no identity
//! claimed: an ad-hoc signature asserts nothing about who built the binary,
//! only that these are the bytes it was built with.
//!
//! Real code-signing verification (`codesign --verify`, Gatekeeper, an actual
//! launch) needs a Mac and is out of scope on this host; see
//! `docs/dev/log/D3.md` for exactly what was and was not checked here.

use std::path::Path;

use sha2::{Digest, Sha256};

/// Whether [`inject_and_sign`] applies an ad-hoc signature.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodeSign {
    /// Apply a plain, unsigned ad-hoc `CodeDirectory` after the section is
    /// written.
    Adhoc,
    /// Write the section and stop.
    ///
    /// Exists so that [`crate::payload::locate`] can be tested against a
    /// Mach-O carrying the section without also depending on the signer.
    None,
}

/// How [`inject_and_sign`] is asked to sign a stub.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MacSignCfg {
    /// Whether to sign after the section is written.
    pub codesign: CodeSign,
}

/// What [`inject_and_sign`] did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InjectReport {
    /// The absolute file offset of the `__GINARY,__payload` section's first
    /// byte, in the file [`inject_and_sign`] wrote.
    pub section_offset: u64,
    /// The section's size in bytes: the 64-byte trailer plus the payload.
    pub section_size: u64,
    /// Whether an ad-hoc signature was applied.
    pub signed: bool,
    /// The stub's `cputype`, spelled the way [`crate::macho::MachoFacts`]
    /// spells it.
    pub cputype: String,
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
    /// write a section into, so it is the one that checks
    /// [`crate::macho::MachoFacts::is_fat`] and refuses here.
    #[error("the stub is a fat Mach-O carrying more than one architecture; a thin one is required")]
    Fat,
    /// `stub_bytes` already carries a `__GINARY,__payload` section.
    #[error(
        "the stub already carries a __GINARY,__payload section; a payload may not be added twice"
    )]
    AlreadySectioned,
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

/// Writes `payload_with_trailer` into a `__GINARY,__payload` section of
/// `stub_bytes` and, per `cfg`, applies an ad-hoc code signature, writing the
/// result to `out`.
///
/// `payload_with_trailer` is exactly what [`crate::payload::pack`] and
/// [`crate::trailer::Trailer::to_bytes`] already produce for the ELF and PE
/// path — the section's first 64 bytes are that trailer, with
/// `payload_offset` relative to the section's own start rather than to the
/// file, and the payload follows immediately after; see
/// `docs/format.md` and [`crate::payload::locate`].
///
/// # Errors
///
/// [`SignMacosError::NotAMachO`] when `stub_bytes` is not a Mach-O this crate
/// can read — including when its geometry cannot be safely rewritten, once
/// reading has already confirmed it is a whole Mach-O — [`SignMacosError::Fat`]
/// when it is a fat one, [`SignMacosError::AlreadySectioned`] when it already
/// carries the section, and [`SignMacosError::Io`] when `out` could not be
/// written.
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
    let body = writer.build()?;

    std::fs::write(out, &body).map_err(|source| SignMacosError::Io {
        path: out.to_path_buf(),
        source,
    })?;

    Ok(InjectReport {
        section_offset: writer.section_fileoff,
        section_size: writer.section_size,
        signed: cfg.codesign == CodeSign::Adhoc,
        cputype: facts.cputype,
    })
}

// ----------------------------------------------------------- the writer --

/// The page alignment the payload segment's `vmsize` and `filesize` are
/// rounded up to, matching Apple Silicon's page size. Nothing this crate
/// reads depends on the exact value; only a real kernel's page-in would.
const SEGMENT_PAGE_ALIGN: u64 = 0x4000;

/// The page size Apple's ad-hoc `CodeDirectory` hashes over, independent of
/// [`SEGMENT_PAGE_ALIGN`]: `page_size` field value `12`, `log2(4096)`.
const CODE_DIRECTORY_PAGE: usize = 4096;

/// The length of a 64-bit Mach-O header.
const HEADER_LEN: usize = 32;
/// The length of one `segment_command_64`, header fields only.
const SEGMENT_CMD_LEN: usize = 72;
/// The length of one `section_64`.
const SECTION_LEN: usize = 80;
/// The length of a `linkedit_data_command`.
const LINKEDIT_DATA_CMD_LEN: usize = 16;

const LC_SEGMENT_64: u32 = 0x19;
const LC_SYMTAB: u32 = 0x2;
const LC_DYSYMTAB: u32 = 0xb;
const LC_CODE_SIGNATURE: u32 = 0x1d;
const LC_DYLD_INFO: u32 = 0x22;
const LC_DYLD_INFO_ONLY: u32 = 0x8000_0022;
const LC_FUNCTION_STARTS: u32 = 0x26;
const LC_DATA_IN_CODE: u32 = 0x29;
const LC_DYLIB_CODE_SIGN_DRS: u32 = 0x2b;
const LC_DYLD_EXPORTS_TRIE: u32 = 0x8000_0033;
const LC_DYLD_CHAINED_FIXUPS: u32 = 0x8000_0034;

/// Everything [`inject_and_sign`] needs to write the finished file, planned
/// once and then assembled by [`Writer::build`].
///
/// The technique — insert the payload segment where `__LINKEDIT` used to
/// start, push `__LINKEDIT` and every offset a `symtab`/`dysymtab`/`dyld_info`
/// /`linkedit_data` command points into it by the same amount, and drop
/// whatever `LC_CODE_SIGNATURE` the stub carried rather than trust a
/// signature that covered different bytes — is adapted from `libsui` 0.16.4's
/// `Macho::write_section` (`denoland/sui`, MIT, Copyright (c) 2024 Divy
/// Srivastava and the Deno authors), not depended on directly: its arm64
/// path hardcodes the new segment's name to `__SUI`, which this crate has no
/// way to override from outside the crate, and this artifact needs
/// `__GINARY`. `docs/dev/log/D3.md` records this decision in full; the ADR
/// covers why the section goes before `__LINKEDIT` at all.
struct Writer {
    header: [u8; HEADER_LEN],
    commands: Vec<Vec<u8>>,
    pre_linkedit: Vec<u8>,
    section_data: Vec<u8>,
    section_padding: usize,
    post_linkedit: Vec<u8>,
    section_fileoff: u64,
    section_size: u64,
    sign: bool,
    codesig_index: Option<usize>,
    linkedit_index: Option<usize>,
    text_fileoff: u64,
    text_filesize: u64,
}

impl Writer {
    /// Reads `stub`'s load commands, drops any existing `LC_CODE_SIGNATURE`,
    /// shifts everything `__LINKEDIT` and its referencing commands point at,
    /// and plans the new `__GINARY,__payload` segment — without writing
    /// anything yet.
    fn plan(stub: &[u8], payload: &[u8], sign: bool) -> Result<Self, SignMacosError> {
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

        let mut commands: Vec<(u32, Vec<u8>)> = Vec::with_capacity(ncmds as usize);
        let mut offset = HEADER_LEN;
        let mut had_old_codesig = false;
        let mut old_linkedit_fileoff: Option<u64> = None;
        let mut old_linkedit_vmaddr: u64 = 0;
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
                if segment_name(&raw) == "__LINKEDIT" {
                    old_linkedit_fileoff = Some(get_u64(&raw, 40)?);
                    old_linkedit_vmaddr = get_u64(&raw, 24)?;
                }
            }
            if cmd == LC_CODE_SIGNATURE {
                had_old_codesig = true;
            } else {
                commands.push((cmd, raw));
            }
            offset = end;
        }

        // Every real Mach-O has `__LINKEDIT` last; a hand-fabricated stub
        // with no load commands at all does not, and the payload simply
        // lands at the end of the file — there is nothing else to shift.
        let old_linkedit_fileoff = old_linkedit_fileoff.unwrap_or(stub.len() as u64);

        let removed_len = if had_old_codesig {
            LINKEDIT_DATA_CMD_LEN as u64
        } else {
            0
        };
        let added_len = (SEGMENT_CMD_LEN + SECTION_LEN) as u64
            + if sign {
                LINKEDIT_DATA_CMD_LEN as u64
            } else {
                0
            };
        // Always positive: one new segment-and-section command (152 bytes)
        // is added unconditionally, and at most one 16-byte command is ever
        // removed.
        let header_growth = added_len
            .checked_sub(removed_len)
            .ok_or_else(|| parse_error("the load command area shrank unexpectedly"))?;

        let section_size = payload.len() as u64;
        let section_filesize = align_up(section_size, SEGMENT_PAGE_ALIGN);

        for (cmd, raw) in &mut commands {
            match *cmd {
                LC_SEGMENT_64 => {
                    shift_segment(
                        raw,
                        header_growth,
                        old_linkedit_fileoff,
                        old_linkedit_vmaddr,
                        section_filesize,
                    )?;
                }
                LC_SYMTAB => {
                    shift_fields(
                        raw,
                        &[8, 16],
                        header_growth,
                        old_linkedit_fileoff,
                        section_filesize,
                    )?;
                }
                LC_DYSYMTAB => {
                    shift_fields(
                        raw,
                        &[32, 40, 48, 56, 64, 72],
                        header_growth,
                        old_linkedit_fileoff,
                        section_filesize,
                    )?;
                }
                LC_DYLD_INFO | LC_DYLD_INFO_ONLY => {
                    shift_fields(
                        raw,
                        &[8, 16, 24, 32, 40],
                        header_growth,
                        old_linkedit_fileoff,
                        section_filesize,
                    )?;
                }
                LC_FUNCTION_STARTS
                | LC_DATA_IN_CODE
                | LC_DYLIB_CODE_SIGN_DRS
                | LC_DYLD_EXPORTS_TRIE
                | LC_DYLD_CHAINED_FIXUPS => {
                    shift_fields(
                        raw,
                        &[8],
                        header_growth,
                        old_linkedit_fileoff,
                        section_filesize,
                    )?;
                }
                _ => {}
            }
        }

        let section_fileoff = old_linkedit_fileoff
            .checked_add(header_growth)
            .ok_or_else(|| parse_error("the payload section's offset overflows"))?;
        // The same shift, in VM address space: `__LINKEDIT`'s own `vmaddr`
        // moves by `header_growth` for the same reason its `fileoff` does —
        // the load-command area it sits after grew — and the new segment
        // takes the address `__LINKEDIT` would have landed at without the
        // payload segment's own `section_filesize` on top of that, exactly
        // mirroring `section_fileoff` above.
        let section_vmaddr = old_linkedit_vmaddr
            .checked_add(header_growth)
            .ok_or_else(|| parse_error("the payload section's vmaddr overflows"))?;

        let mut seg_and_sect = Vec::with_capacity(SEGMENT_CMD_LEN + SECTION_LEN);
        seg_and_sect.extend_from_slice(&LC_SEGMENT_64.to_le_bytes());
        seg_and_sect.extend_from_slice(
            &u32::try_from(SEGMENT_CMD_LEN + SECTION_LEN)
                .unwrap_or(u32::MAX)
                .to_le_bytes(),
        );
        seg_and_sect.extend_from_slice(&name16(crate::macho::PAYLOAD_SEGMENT));
        seg_and_sect.extend_from_slice(&section_vmaddr.to_le_bytes()); // vmaddr
        seg_and_sect.extend_from_slice(&section_filesize.to_le_bytes()); // vmsize
        seg_and_sect.extend_from_slice(&section_fileoff.to_le_bytes()); // fileoff
        seg_and_sect.extend_from_slice(&section_filesize.to_le_bytes()); // filesize
        seg_and_sect.extend_from_slice(&1i32.to_le_bytes()); // maxprot: VM_PROT_READ
        seg_and_sect.extend_from_slice(&1i32.to_le_bytes()); // initprot
        seg_and_sect.extend_from_slice(&1u32.to_le_bytes()); // nsects
        seg_and_sect.extend_from_slice(&0u32.to_le_bytes()); // flags
        seg_and_sect.extend_from_slice(&name16(crate::macho::PAYLOAD_SECTION));
        seg_and_sect.extend_from_slice(&name16(crate::macho::PAYLOAD_SEGMENT));
        seg_and_sect.extend_from_slice(&section_vmaddr.to_le_bytes()); // addr
        seg_and_sect.extend_from_slice(&section_size.to_le_bytes()); // size
        seg_and_sect.extend_from_slice(
            &u32::try_from(section_fileoff)
                .unwrap_or(u32::MAX)
                .to_le_bytes(),
        ); // offset
        seg_and_sect.extend_from_slice(&0u32.to_le_bytes()); // align
        seg_and_sect.extend_from_slice(&0u32.to_le_bytes()); // reloff
        seg_and_sect.extend_from_slice(&0u32.to_le_bytes()); // nreloc
        seg_and_sect.extend_from_slice(&0u32.to_le_bytes()); // flags: S_REGULAR
        seg_and_sect.extend_from_slice(&0u32.to_le_bytes()); // reserved1
        seg_and_sect.extend_from_slice(&0u32.to_le_bytes()); // reserved2
        seg_and_sect.extend_from_slice(&0u32.to_le_bytes()); // reserved3
        commands.push((LC_SEGMENT_64, seg_and_sect));

        let codesig_index = if sign {
            let mut cs = Vec::with_capacity(LINKEDIT_DATA_CMD_LEN);
            cs.extend_from_slice(&LC_CODE_SIGNATURE.to_le_bytes());
            cs.extend_from_slice(
                &u32::try_from(LINKEDIT_DATA_CMD_LEN)
                    .unwrap_or(u32::MAX)
                    .to_le_bytes(),
            );
            cs.extend_from_slice(&0u32.to_le_bytes()); // dataoff, patched in `build`
            cs.extend_from_slice(&0u32.to_le_bytes()); // datasize, patched in `build`
            commands.push((LC_CODE_SIGNATURE, cs));
            Some(commands.len() - 1)
        } else {
            None
        };

        let linkedit_index = commands
            .iter()
            .position(|(cmd, raw)| *cmd == LC_SEGMENT_64 && segment_name(raw) == "__LINKEDIT");
        let (text_fileoff, text_filesize) = commands
            .iter()
            .find(|(cmd, raw)| *cmd == LC_SEGMENT_64 && segment_name(raw) == "__TEXT")
            .map(|(_, raw)| (get_u64(raw, 40).unwrap_or(0), get_u64(raw, 48).unwrap_or(0)))
            .unwrap_or((0, 0));

        let split_at = old_linkedit_fileoff
            .saturating_sub(old_lc_end as u64)
            .min(stub.len().saturating_sub(old_lc_end) as u64) as usize;
        let pre_linkedit = stub[old_lc_end..old_lc_end + split_at].to_vec();
        let post_linkedit = stub[old_lc_end + split_at..].to_vec();

        let section_padding = section_filesize.saturating_sub(section_size) as usize;

        Ok(Self {
            header,
            commands: commands.into_iter().map(|(_, raw)| raw).collect(),
            pre_linkedit,
            section_data: payload.to_vec(),
            section_padding,
            post_linkedit,
            section_fileoff,
            section_size,
            sign,
            codesig_index,
            linkedit_index,
            text_fileoff,
            text_filesize,
        })
    }

    /// Assembles the planned commands and file content into the finished
    /// bytes, computing and appending the ad-hoc signature last, when asked.
    fn build(&self) -> Result<Vec<u8>, SignMacosError> {
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
                + self.pre_linkedit.len()
                + self.section_data.len()
                + self.section_padding
                + self.post_linkedit.len()
                + 1024,
        );
        body.extend_from_slice(&header);

        let mut command_offsets = Vec::with_capacity(self.commands.len());
        for raw in &self.commands {
            command_offsets.push(body.len());
            body.extend_from_slice(raw);
        }
        body.extend_from_slice(&self.pre_linkedit);
        body.extend_from_slice(&self.section_data);
        body.extend(std::iter::repeat_n(0u8, self.section_padding));
        body.extend_from_slice(&self.post_linkedit);

        if self.sign {
            let sig_off = body.len();
            let blob = build_ad_hoc_signature(&body, self.text_fileoff, self.text_filesize);
            let sz = blob.len();

            if let Some(index) = self.codesig_index
                && let Some(&at) = command_offsets.get(index)
            {
                put_u32(
                    &mut body,
                    at + 8,
                    u32::try_from(sig_off).unwrap_or(u32::MAX),
                )?;
                put_u32(&mut body, at + 12, u32::try_from(sz).unwrap_or(u32::MAX))?;
            }
            if let Some(index) = self.linkedit_index
                && let Some(&at) = command_offsets.get(index)
            {
                let linkedit_fileoff = get_u64(&body, at + 40)?;
                let seg_size = (sig_off as u64)
                    .saturating_add(sz as u64)
                    .saturating_sub(linkedit_fileoff);
                put_u64(&mut body, at + 32, seg_size)?; // vmsize
                put_u64(&mut body, at + 48, seg_size)?; // filesize
            }
            body.extend_from_slice(&blob);
        }

        Ok(body)
    }
}

/// A plain, unsigned ad-hoc `CodeDirectory` (`CSMAGIC_EMBEDDED_SIGNATURE`
/// wrapping one `CSMAGIC_CODEDIRECTORY` blob) over `body`, SHA-256 per
/// [`CODE_DIRECTORY_PAGE`]-byte page — the same layout `codesign -s -`
/// produces, asserting no identity beyond "these are the bytes ginary
/// built". Adapted from `libsui` 0.16.4's `apple_codesign::MachoSigner`
/// (`denoland/sui`, MIT, Copyright (c) 2024 Divy Srivastava and the Deno
/// authors); see the note on [`Writer`] for why this crate is not depended
/// on directly.
fn build_ad_hoc_signature(body: &[u8], text_fileoff: u64, text_filesize: u64) -> Vec<u8> {
    const CSMAGIC_CODEDIRECTORY: u32 = 0xfade_0c02;
    const CSMAGIC_EMBEDDED_SIGNATURE: u32 = 0xfade_0cc0;
    const CD_HEADER_LEN: usize = 88;
    const HASH_LEN: usize = 32;
    const SUPERBLOB_AND_BLOB_LEN: usize = 20;

    let id: &[u8] = b"a.out\0";
    let sig_off = body.len();
    let n_hashes = sig_off.div_ceil(CODE_DIRECTORY_PAGE);
    let ident_offset = CD_HEADER_LEN;
    let hash_offset = ident_offset + id.len();
    let cd_len = hash_offset + n_hashes * HASH_LEN;
    let total_len = SUPERBLOB_AND_BLOB_LEN + cd_len;

    let mut out = Vec::with_capacity(total_len);
    out.extend_from_slice(&CSMAGIC_EMBEDDED_SIGNATURE.to_be_bytes());
    out.extend_from_slice(&u32::try_from(total_len).unwrap_or(u32::MAX).to_be_bytes());
    out.extend_from_slice(&1u32.to_be_bytes()); // count
    out.extend_from_slice(&0u32.to_be_bytes()); // CSSLOT_CODEDIRECTORY
    out.extend_from_slice(
        &u32::try_from(SUPERBLOB_AND_BLOB_LEN)
            .unwrap_or(u32::MAX)
            .to_be_bytes(),
    );

    out.extend_from_slice(&CSMAGIC_CODEDIRECTORY.to_be_bytes());
    out.extend_from_slice(&u32::try_from(cd_len).unwrap_or(u32::MAX).to_be_bytes());
    out.extend_from_slice(&0x0002_0400u32.to_be_bytes()); // version
    out.extend_from_slice(&0x0002_0002u32.to_be_bytes()); // flags: adhoc | linkerSigned
    out.extend_from_slice(&u32::try_from(hash_offset).unwrap_or(u32::MAX).to_be_bytes());
    out.extend_from_slice(
        &u32::try_from(ident_offset)
            .unwrap_or(u32::MAX)
            .to_be_bytes(),
    );
    out.extend_from_slice(&0u32.to_be_bytes()); // n_special_slots
    out.extend_from_slice(&u32::try_from(n_hashes).unwrap_or(u32::MAX).to_be_bytes());
    out.extend_from_slice(&u32::try_from(sig_off).unwrap_or(u32::MAX).to_be_bytes()); // code_limit
    out.push(u8::try_from(HASH_LEN).unwrap_or(u8::MAX)); // hash_size
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
    out.extend_from_slice(id);

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

/// `name`, NUL-padded to the fixed 16-byte width every Mach-O segment and
/// section name field is; truncated rather than panicking if `name` is
/// somehow longer; every name this module writes is a short internal
/// constant, never truncated in practice.
fn name16(name: &str) -> [u8; 16] {
    let mut out = [0u8; 16];
    let bytes = name.as_bytes();
    let take = bytes.len().min(16);
    out[..take].copy_from_slice(&bytes[..take]);
    out
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

/// Adds `header_growth` to `old`, and `extra` as well when `old` is at or
/// past `old_linkedit_fileoff` — the one rule every offset a
/// `symtab`/`dysymtab`/`dyld_info`/`linkedit_data` command carries follows,
/// because every one of them points somewhere inside `__LINKEDIT`. `0` is
/// left as `0`: every one of these fields uses it to mean "not present",
/// never a real position.
fn shift_value(old: u32, header_growth: u64, old_linkedit_fileoff: u64, extra: u64) -> u32 {
    if old == 0 {
        return 0;
    }
    let extra_here = if u64::from(old) >= old_linkedit_fileoff {
        extra
    } else {
        0
    };
    let shifted = u64::from(old)
        .saturating_add(header_growth)
        .saturating_add(extra_here);
    u32::try_from(shifted).unwrap_or(u32::MAX)
}

/// [`shift_value`]'s 64-bit counterpart, for the two fields that are `u64`
/// rather than `u32`: a segment's own `vmaddr`, and a section's `addr`.
/// Unlike `shift_value` there is no `0`-means-absent case to preserve — an
/// `addr`/`vmaddr` of `0` is a real (if unusual) virtual address, not a
/// sentinel — so every value is shifted unconditionally by `header_growth`,
/// plus `extra` once `old` reaches `old_linkedit_boundary`, mirroring the
/// `fileoff`/`old_linkedit_fileoff` rule [`shift_value`] applies to file
/// offsets.
fn shift_value_u64(
    old: u64,
    header_growth: u64,
    old_linkedit_boundary: u64,
    extra: u64,
) -> Result<u64, SignMacosError> {
    let extra_here = if old >= old_linkedit_boundary {
        extra
    } else {
        0
    };
    old.checked_add(header_growth)
        .and_then(|value| value.checked_add(extra_here))
        .ok_or_else(|| parse_error("a 64-bit address field overflows"))
}

/// Applies [`shift_value`] to the `u32` field at each offset in
/// `field_offsets`, within `raw`.
fn shift_fields(
    raw: &mut [u8],
    field_offsets: &[usize],
    header_growth: u64,
    old_linkedit_fileoff: u64,
    extra: u64,
) -> Result<(), SignMacosError> {
    for &field in field_offsets {
        let old = get_u32(raw, field)?;
        put_u32(
            raw,
            field,
            shift_value(old, header_growth, old_linkedit_fileoff, extra),
        )?;
    }
    Ok(())
}

/// Shifts one `segment_command_64`'s `fileoff` and `vmaddr` (or, when
/// `fileoff` is `0` — the segment mapping the header itself, `__TEXT` in a
/// real Mach-O — grows `filesize` and `vmsize` together instead, so the
/// segment still reaches the same logical end on both axes) and every one of
/// its sections' `offset`.
///
/// `vmaddr` is shifted by exactly the same rule `fileoff` is: unconditionally
/// by `header_growth`, because growing the load-command area of the segment
/// mapping the header (`__TEXT`) pushes every following segment's VM address
/// forward by that much, the same way it pushes every following segment's
/// file offset forward; and by `extra` as well once `vmaddr` reaches
/// `old_linkedit_vmaddr`, mirroring the `fileoff`/`old_linkedit_fileoff` rule
/// exactly — that is what keeps the new `__GINARY` segment and the relocated
/// `__LINKEDIT` from ever landing on the same address (see
/// `tests/regressions/d3_macho_segment_vmaddr_and_vmsize_were_wrong.rs`).
fn shift_segment(
    raw: &mut [u8],
    header_growth: u64,
    old_linkedit_fileoff: u64,
    old_linkedit_vmaddr: u64,
    extra: u64,
) -> Result<(), SignMacosError> {
    let fileoff = get_u64(raw, 40)?;
    if fileoff == 0 {
        let filesize = get_u64(raw, 48)?;
        if filesize > 0 {
            let grown = filesize
                .checked_add(header_growth)
                .ok_or_else(|| parse_error("a segment's filesize overflows"))?;
            put_u64(raw, 48, grown)?;
            let vmsize = get_u64(raw, 32)?;
            let vmsize_grown = vmsize
                .checked_add(header_growth)
                .ok_or_else(|| parse_error("a segment's vmsize overflows"))?;
            put_u64(raw, 32, vmsize_grown)?;
        }
    } else {
        let extra_here = if fileoff >= old_linkedit_fileoff {
            extra
        } else {
            0
        };
        let shifted_off = fileoff
            .checked_add(header_growth)
            .and_then(|value| value.checked_add(extra_here))
            .ok_or_else(|| parse_error("a segment's fileoff overflows"))?;
        put_u64(raw, 40, shifted_off)?;

        let vmaddr = get_u64(raw, 24)?;
        let shifted_vmaddr = shift_value_u64(vmaddr, header_growth, old_linkedit_vmaddr, extra)?;
        put_u64(raw, 24, shifted_vmaddr)?;
    }

    let nsects = get_u32(raw, 64)?;
    for index in 0..nsects {
        let section_start = SEGMENT_CMD_LEN
            .checked_add(
                usize::try_from(index)
                    .ok()
                    .and_then(|index| index.checked_mul(SECTION_LEN))
                    .ok_or_else(|| parse_error("a section index overflows"))?,
            )
            .ok_or_else(|| parse_error("a section's position overflows"))?;
        let addr_field = section_start
            .checked_add(32)
            .ok_or_else(|| parse_error("a section's addr field overflows"))?;
        let old_addr = get_u64(raw, addr_field)?;
        let shifted_addr = shift_value_u64(old_addr, header_growth, old_linkedit_vmaddr, extra)?;
        put_u64(raw, addr_field, shifted_addr)?;

        let field = section_start
            .checked_add(48)
            .ok_or_else(|| parse_error("a section's offset field overflows"))?;
        let old = get_u32(raw, field)?;
        put_u32(
            raw,
            field,
            shift_value(old, header_growth, old_linkedit_fileoff, extra),
        )?;
    }
    Ok(())
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
