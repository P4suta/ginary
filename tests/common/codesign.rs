// SPDX-License-Identifier: MIT OR Apache-2.0
//! Reading back the embedded code signature `src/sign_macos.rs` writes, and
//! recomputing what it claims.
//!
//! A Mach-O the kernel will run carries a `CSMAGIC_EMBEDDED_SIGNATURE`
//! superblob whose `CodeDirectory` holds one SHA-256 per 4096-byte page of
//! the file *below* the signature. The kernel does not read the signature to
//! be polite: it hashes each page as it faults it in and compares, and a page
//! whose hash does not match is a page it will not map — which it reports by
//! killing the process with `SIGKILL` before `main` runs.
//!
//! That is a claim about arithmetic over bytes, not about macOS, so it is
//! checkable here. Nothing in this module calls into `src/sign_macos.rs`: the
//! load commands are walked, the blob is parsed field by field from the
//! layout Apple's `cs_blobs.h` fixes, and the page hashes are recomputed with
//! `sha2` — so a test written against it is a test of the signer rather than
//! a restatement of it.
//!
//! Not gated behind the `cli` feature, for the reason
//! [`crate::common::macho`] is not.

use sha2::{Digest, Sha256};

/// `LC_SEGMENT_64`.
const LC_SEGMENT_64: u32 = 0x19;
/// `LC_CODE_SIGNATURE`.
const LC_CODE_SIGNATURE: u32 = 0x1d;
/// The length of a 64-bit Mach-O header.
const HEADER_LEN: usize = 32;

/// `CSMAGIC_EMBEDDED_SIGNATURE`, the superblob a signed Mach-O carries.
pub const CSMAGIC_EMBEDDED_SIGNATURE: u32 = 0xfade_0cc0;
/// `CSMAGIC_CODEDIRECTORY`, the one blob this crate writes inside it.
pub const CSMAGIC_CODEDIRECTORY: u32 = 0xfade_0c02;
/// `CSSLOT_CODEDIRECTORY`, the slot index the directory occupies.
pub const CSSLOT_CODEDIRECTORY: u32 = 0;
/// `CS_ADHOC`: the signature asserts no identity.
pub const CS_ADHOC: u32 = 0x0000_0002;
/// `CS_LINKER_SIGNED`: the flag a linker sets on a signature it produced as
/// part of the link. A binary ginary rewrote and re-signed did not come from a
/// linker, so it may not carry this: the flag claims a provenance that is no
/// longer true. See `docs/dev/log/E9.md`.
pub const CS_LINKER_SIGNED: u32 = 0x0002_0000;
/// The alignment `ld` gives the code signature, and `codesign` expects.
pub const SIGNATURE_ALIGNMENT: u64 = 16;

/// One `segment_command_64`, as much of it as a test asks about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Segment {
    /// The `segname` field, trimmed at its first NUL.
    pub name: String,
    /// `vmaddr`.
    pub vmaddr: u64,
    /// `vmsize`.
    pub vmsize: u64,
    /// `fileoff`.
    pub fileoff: u64,
    /// `filesize`.
    pub filesize: u64,
}

/// The `CodeDirectory` blob, parsed field by field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodeDirectory {
    /// `version`.
    pub version: u32,
    /// `flags`.
    pub flags: u32,
    /// `nSpecialSlots`.
    pub n_special_slots: u32,
    /// `nCodeSlots`.
    pub n_code_slots: u32,
    /// `codeLimit`: how many bytes of the file the hashes cover.
    pub code_limit: u64,
    /// `hashSize`, in bytes.
    pub hash_size: u8,
    /// `hashType`; `2` is SHA-256.
    pub hash_type: u8,
    /// `pageSize`, as the base-2 logarithm the field actually holds.
    pub page_size_log2: u8,
    /// The identifier string, without its NUL.
    pub identifier: String,
    /// `execSegBase`.
    pub exec_seg_base: u64,
    /// `execSegLimit`.
    pub exec_seg_limit: u64,
    /// The code slot hashes, in slot order.
    pub hashes: Vec<Vec<u8>>,
}

impl CodeDirectory {
    /// The page size the hashes are taken over.
    pub fn page_size(&self) -> usize {
        1usize << self.page_size_log2
    }
}

/// Everything a test asks of the signature a file carries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmbeddedSignature {
    /// `dataoff` of the `LC_CODE_SIGNATURE` command: where the superblob
    /// starts in the file.
    pub data_offset: u64,
    /// `datasize`: how long it is.
    pub data_size: u64,
    /// The superblob's own `magic`.
    pub magic: u32,
    /// How many blobs the superblob indexes.
    pub blob_count: u32,
    /// The slot type of the first indexed blob.
    pub first_slot: u32,
    /// The parsed `CodeDirectory`.
    pub code_directory: CodeDirectory,
}

/// Every segment of a thin little-endian 64-bit Mach-O, in load-command
/// order.
///
/// # Panics
///
/// If `file` is not one, or is truncated inside its load commands. Every
/// caller has just written it.
pub fn segments(file: &[u8]) -> Vec<Segment> {
    let mut found = Vec::new();
    for (cmd, raw) in load_commands(file) {
        if cmd != LC_SEGMENT_64 {
            continue;
        }
        found.push(Segment {
            name: name16(&raw[8..24]),
            vmaddr: u64le(&raw, 24),
            vmsize: u64le(&raw, 32),
            fileoff: u64le(&raw, 40),
            filesize: u64le(&raw, 48),
        });
    }
    found
}

/// One named segment, or [`None`] when the file carries no such segment.
///
/// # Panics
///
/// If `file` is not a thin little-endian 64-bit Mach-O.
pub fn segment(file: &[u8], name: &str) -> Option<Segment> {
    segments(file).into_iter().find(|seg| seg.name == name)
}

/// The embedded signature `file` carries, or [`None`] when it has no
/// `LC_CODE_SIGNATURE` at all.
///
/// # Panics
///
/// If `file` is not a thin little-endian 64-bit Mach-O, or if it carries a
/// signature command whose blob is truncated or is not a superblob wrapping a
/// `CodeDirectory` — every one of those is a defect in the writer under test,
/// and a panic naming it is the report.
pub fn signature(file: &[u8]) -> Option<EmbeddedSignature> {
    let (_, raw) = load_commands(file)
        .into_iter()
        .find(|(cmd, _)| *cmd == LC_CODE_SIGNATURE)?;
    let data_offset = u64::from(u32le(&raw, 8));
    let data_size = u64::from(u32le(&raw, 12));
    let start = usize::try_from(data_offset).expect("a signature offset that fits this machine");
    let end = start + usize::try_from(data_size).expect("a signature size that fits this machine");
    assert!(
        end <= file.len(),
        "the LC_CODE_SIGNATURE command names bytes {start}..{end} and the file is {} long",
        file.len()
    );
    let blob = &file[start..end];

    let magic = u32be(blob, 0);
    let blob_count = u32be(blob, 8);
    assert!(
        blob_count >= 1,
        "a superblob that indexes nothing carries no CodeDirectory"
    );
    let first_slot = u32be(blob, 12);
    let cd_at = usize::try_from(u32be(blob, 16)).expect("a blob offset that fits this machine");

    Some(EmbeddedSignature {
        data_offset,
        data_size,
        magic,
        blob_count,
        first_slot,
        code_directory: code_directory(&blob[cd_at..]),
    })
}

/// Parses one `CodeDirectory` blob starting at `blob[0]`.
fn code_directory(blob: &[u8]) -> CodeDirectory {
    assert_eq!(
        u32be(blob, 0),
        CSMAGIC_CODEDIRECTORY,
        "the first indexed blob is a CodeDirectory"
    );
    let hash_offset = usize::try_from(u32be(blob, 16)).expect("a hash offset that fits");
    let ident_offset = usize::try_from(u32be(blob, 20)).expect("an ident offset that fits");
    let n_special_slots = u32be(blob, 24);
    let n_code_slots = u32be(blob, 28);
    let hash_size = blob[36];
    let hash_type = blob[37];
    let page_size_log2 = blob[39];
    let code_limit64 = u64be(blob, 56);
    let code_limit = if code_limit64 == 0 {
        u64::from(u32be(blob, 32))
    } else {
        code_limit64
    };

    let ident_end = blob[ident_offset..]
        .iter()
        .position(|&byte| byte == 0)
        .map_or(blob.len(), |at| ident_offset + at);
    let identifier = String::from_utf8_lossy(&blob[ident_offset..ident_end]).into_owned();

    let size = usize::from(hash_size);
    let mut hashes = Vec::with_capacity(n_code_slots as usize);
    for slot in 0..n_code_slots as usize {
        let at = hash_offset + slot * size;
        assert!(
            at + size <= blob.len(),
            "code slot {slot} runs past the end of the CodeDirectory"
        );
        hashes.push(blob[at..at + size].to_vec());
    }

    CodeDirectory {
        version: u32be(blob, 8),
        flags: u32be(blob, 12),
        n_special_slots,
        n_code_slots,
        code_limit,
        hash_size,
        hash_type,
        page_size_log2,
        identifier,
        exec_seg_base: u64be(blob, 64),
        exec_seg_limit: u64be(blob, 72),
        hashes,
    }
}

/// The SHA-256 of each `page`-byte page of `file[..code_limit]`, the value
/// the kernel computes for itself as it maps the file.
///
/// The last page is short rather than padded, which is what makes the final
/// slot depend on the file's exact length.
///
/// # Panics
///
/// If `code_limit` is longer than `file`, or `page` is zero.
pub fn page_hashes(file: &[u8], code_limit: usize, page: usize) -> Vec<Vec<u8>> {
    assert!(page > 0, "a page size of zero hashes nothing");
    assert!(
        code_limit <= file.len(),
        "codeLimit {code_limit} is past the end of a {}-byte file",
        file.len()
    );
    let mut out = Vec::with_capacity(code_limit.div_ceil(page));
    let mut at = 0;
    while at < code_limit {
        let take = page.min(code_limit - at);
        let mut hasher = Sha256::new();
        hasher.update(&file[at..at + take]);
        out.push(hasher.finalize().to_vec());
        at += take;
    }
    out
}

/// The index of the first code slot whose hash is not the page's own, with
/// what each side said.
///
/// [`None`] when every slot agrees, which is the only state a kernel will
/// run.
pub fn first_bad_slot(file: &[u8], directory: &CodeDirectory) -> Option<(usize, String, String)> {
    let limit = usize::try_from(directory.code_limit).expect("a code limit that fits");
    let recomputed = page_hashes(file, limit, directory.page_size());
    // A directory that claims more slots than the file has pages under
    // `codeLimit` describes pages that are not there; the per-slot loop below
    // is driven by `recomputed`, so it would never look at the surplus slots.
    // Report the count disagreement before the byte-for-byte one.
    if directory.hashes.len() != recomputed.len() {
        let slot = recomputed.len().min(directory.hashes.len());
        let want = recomputed
            .get(slot)
            .map_or_else(|| "(no such page)".to_owned(), hex::encode);
        let got = directory
            .hashes
            .get(slot)
            .map_or_else(|| "(no such slot)".to_owned(), hex::encode);
        return Some((slot, want, got));
    }
    for (slot, want) in recomputed.iter().enumerate() {
        match directory.hashes.get(slot) {
            Some(got) if got == want => {}
            Some(got) => return Some((slot, hex::encode(want), hex::encode(got))),
            None => return Some((slot, hex::encode(want), "(no such slot)".to_owned())),
        }
    }
    None
}

/// Every load command of a thin little-endian 64-bit Mach-O, as
/// `(cmd, raw bytes)`.
fn load_commands(file: &[u8]) -> Vec<(u32, Vec<u8>)> {
    assert!(
        file.len() >= HEADER_LEN,
        "a Mach-O is at least {HEADER_LEN} bytes"
    );
    assert_eq!(
        u32le(file, 0),
        0xfeed_facf,
        "only a thin little-endian 64-bit Mach-O is read here"
    );
    let ncmds = u32le(file, 16) as usize;
    let mut found = Vec::with_capacity(ncmds);
    let mut at = HEADER_LEN;
    for index in 0..ncmds {
        assert!(at + 8 <= file.len(), "load command {index} is truncated");
        let cmd = u32le(file, at);
        let size = u32le(file, at + 4) as usize;
        assert!(size >= 8, "load command {index} is shorter than its header");
        assert!(
            at + size <= file.len(),
            "load command {index} runs past the end of the file"
        );
        found.push((cmd, file[at..at + size].to_vec()));
        at += size;
    }
    found
}

/// A fixed-width name field, trimmed at its first NUL.
fn name16(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

/// A little-endian `u32` at `at`.
fn u32le(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(quad(bytes, at))
}

/// A little-endian `u64` at `at`.
fn u64le(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(oct(bytes, at))
}

/// A big-endian `u32` at `at`; every field of a signature blob is one.
fn u32be(bytes: &[u8], at: usize) -> u32 {
    u32::from_be_bytes(quad(bytes, at))
}

/// A big-endian `u64` at `at`.
fn u64be(bytes: &[u8], at: usize) -> u64 {
    u64::from_be_bytes(oct(bytes, at))
}

/// Four bytes at `at`.
fn quad(bytes: &[u8], at: usize) -> [u8; 4] {
    let mut out = [0u8; 4];
    out.copy_from_slice(
        bytes
            .get(at..at + 4)
            .unwrap_or_else(|| panic!("four bytes at {at} of a {}-byte blob", bytes.len())),
    );
    out
}

/// Eight bytes at `at`.
fn oct(bytes: &[u8], at: usize) -> [u8; 8] {
    let mut out = [0u8; 8];
    out.copy_from_slice(
        bytes
            .get(at..at + 8)
            .unwrap_or_else(|| panic!("eight bytes at {at} of a {}-byte blob", bytes.len())),
    );
    out
}
