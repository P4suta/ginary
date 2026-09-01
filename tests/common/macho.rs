// SPDX-License-Identifier: MIT OR Apache-2.0
//! Hand-fabricated Mach-O bytes for `src/macho.rs`, `src/payload.rs`'s
//! `locate` and `src/sign_macos.rs`.
//!
//! Not gated behind the `cli` feature. `src/macho.rs` is on the launcher
//! path — a macOS artifact locates its own payload through it with no `cli`
//! feature compiled in at all — so its tests, and this helper module, have
//! to build under `--no-default-features` as well as the default build,
//! which is why this lives beside `tests/common/payload.rs` rather than
//! beside the `cli`-gated `tests/common/native.rs`.
//!
//! Every header here is written field by field, the same technique
//! `tests/common/native.rs` uses for ELF and PE: there is no macOS toolchain
//! on this machine, and the claims under test are about header fields —
//! `cputype`, a section's file offset, whether an `LC_CODE_SIGNATURE` load
//! command is present — that a hand-built header proves as well as a linker
//! would. `tests/fixtures/macho/` is the other half: a real, unmodified
//! Mach-O binary, for the tests that have to hold against something this
//! module did not write.

use std::path::{Path, PathBuf};

/// `MH_MAGIC_64`, the 64-bit little-endian magic a thin Mach-O begins with.
pub const MH_MAGIC_64: u32 = 0xfeed_facf;
/// `FAT_MAGIC`, a fat binary's magic as it sits on disk (big-endian).
pub const FAT_MAGIC: u32 = 0xcafe_babe;

/// `CPU_TYPE_X86_64`.
pub const CPU_TYPE_X86_64: u32 = 0x0100_0007;
/// `CPU_TYPE_ARM64`.
pub const CPU_TYPE_ARM64: u32 = 0x0100_000c;

/// `MH_EXECUTE`, the `filetype` of a Mach-O program.
pub const MH_EXECUTE: u32 = 2;
/// `MH_DYLIB`, the `filetype` of a Mach-O shared library.
pub const MH_DYLIB: u32 = 6;

/// `LC_SEGMENT_64`.
const LC_SEGMENT_64: u32 = 0x19;
/// `LC_CODE_SIGNATURE`.
const LC_CODE_SIGNATURE: u32 = 0x1d;

/// The length of a 64-bit Mach-O header.
const HEADER_LEN: u64 = 32;
/// The length of one `segment_command_64`, header fields only (no sections).
const SEGMENT_CMD_LEN: u64 = 72;
/// The length of one `section_64`.
const SECTION_LEN: u64 = 80;
/// The length of a `linkedit_data_command`, which `LC_CODE_SIGNATURE` uses.
const CODESIG_CMD_LEN: u64 = 16;

/// A thin, 64-bit Mach-O header with no load commands at all: eight fields,
/// `ncmds` zero, and nothing after them. Describes a whole (if empty) object
/// rather than a truncated one.
pub fn thin_header(cpu_type: u32, filetype: u32) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(HEADER_LEN as usize);
    bytes.extend_from_slice(&MH_MAGIC_64.to_le_bytes());
    bytes.extend_from_slice(&cpu_type.to_le_bytes());
    bytes.extend_from_slice(&0i32.to_le_bytes()); // cpusubtype
    bytes.extend_from_slice(&filetype.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes()); // ncmds
    bytes.extend_from_slice(&0u32.to_le_bytes()); // sizeofcmds
    bytes.extend_from_slice(&0u32.to_le_bytes()); // flags
    bytes.extend_from_slice(&0u32.to_le_bytes()); // reserved
    assert_eq!(bytes.len() as u64, HEADER_LEN, "the header is 32 bytes");
    bytes
}

/// A file that begins with the thin 64-bit magic and holds nothing else: not
/// even a whole header.
pub fn magic_only() -> Vec<u8> {
    let mut bytes = MH_MAGIC_64.to_le_bytes().to_vec();
    bytes.extend_from_slice(b"junk");
    bytes
}

/// A fat (universal) Mach-O header naming `archs`, each `(cputype,
/// cpusubtype)`.
///
/// Fat fields are big-endian on disk, unlike everything in a thin Mach-O:
/// that is what [`FAT_MAGIC`] here is written as, deliberately not swapped.
/// No thin data sits behind the offsets this writes: `macho::read` refuses a
/// fat binary before it would need to follow one.
pub fn fat_header(archs: &[(u32, u32)]) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&FAT_MAGIC.to_be_bytes());
    bytes.extend_from_slice(&(archs.len() as u32).to_be_bytes());
    let offset = 8 + 20 * archs.len() as u32;
    for (cputype, cpusubtype) in archs {
        bytes.extend_from_slice(&cputype.to_be_bytes());
        bytes.extend_from_slice(&cpusubtype.to_be_bytes());
        bytes.extend_from_slice(&offset.to_be_bytes());
        bytes.extend_from_slice(&0u32.to_be_bytes()); // size
        bytes.extend_from_slice(&0u32.to_be_bytes()); // align
    }
    bytes
}

/// What [`with_section`] built.
pub struct Built {
    /// The whole file's bytes.
    pub bytes: Vec<u8>,
    /// The absolute file offset of the planted section's first byte.
    pub section_offset: u64,
    /// The planted section's size, which is `data.len()`.
    pub section_size: u64,
    /// The absolute file offset and size of the ad-hoc `LC_CODE_SIGNATURE`
    /// blob, when `code_signature` was requested.
    pub codesig: Option<(u64, u64)>,
}

/// A thin, 64-bit Mach-O for `cpu_type` carrying one segment named `segname`
/// with one section `sectname` whose bytes are `data`, and — when
/// `code_signature` is `true` — an `LC_CODE_SIGNATURE` load command over a
/// small trailing blob after the section.
///
/// The layout is exactly what a real linker produces for the one property
/// these tests read off it: the load commands end precisely where the
/// section's own file offset begins, and (when present) the code-signature
/// blob comes after the section, as `__LINKEDIT` does in a real binary.
pub fn with_section(
    cpu_type: u32,
    segname: &str,
    sectname: &str,
    data: &[u8],
    code_signature: bool,
) -> Built {
    let ncmds: u32 = if code_signature { 2 } else { 1 };
    let sizeofcmds =
        SEGMENT_CMD_LEN + SECTION_LEN + if code_signature { CODESIG_CMD_LEN } else { 0 };
    let section_offset = HEADER_LEN + sizeofcmds;
    let section_size = data.len() as u64;
    let codesig_offset = section_offset + section_size;
    let codesig_data: &[u8] = b"fake ad-hoc CodeDirectory, RED phase fixture only";
    let codesig_size = codesig_data.len() as u64;

    let mut bytes = Vec::new();
    bytes.extend_from_slice(&MH_MAGIC_64.to_le_bytes());
    bytes.extend_from_slice(&cpu_type.to_le_bytes());
    bytes.extend_from_slice(&0i32.to_le_bytes());
    bytes.extend_from_slice(&MH_EXECUTE.to_le_bytes());
    bytes.extend_from_slice(&ncmds.to_le_bytes());
    bytes.extend_from_slice(&u32::try_from(sizeofcmds).expect("fits").to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes()); // flags
    bytes.extend_from_slice(&0u32.to_le_bytes()); // reserved
    assert_eq!(bytes.len() as u64, HEADER_LEN, "the header is 32 bytes");

    // LC_SEGMENT_64
    bytes.extend_from_slice(&LC_SEGMENT_64.to_le_bytes());
    bytes.extend_from_slice(
        &u32::try_from(SEGMENT_CMD_LEN + SECTION_LEN)
            .expect("fits")
            .to_le_bytes(),
    );
    bytes.extend_from_slice(&name16(segname));
    bytes.extend_from_slice(&0u64.to_le_bytes()); // vmaddr
    bytes.extend_from_slice(&section_size.to_le_bytes()); // vmsize
    bytes.extend_from_slice(&section_offset.to_le_bytes()); // fileoff
    bytes.extend_from_slice(&section_size.to_le_bytes()); // filesize
    bytes.extend_from_slice(&7i32.to_le_bytes()); // maxprot
    bytes.extend_from_slice(&7i32.to_le_bytes()); // initprot
    bytes.extend_from_slice(&1u32.to_le_bytes()); // nsects
    bytes.extend_from_slice(&0u32.to_le_bytes()); // flags

    // section_64
    bytes.extend_from_slice(&name16(sectname));
    bytes.extend_from_slice(&name16(segname));
    bytes.extend_from_slice(&0u64.to_le_bytes()); // addr
    bytes.extend_from_slice(&section_size.to_le_bytes()); // size
    bytes.extend_from_slice(&u32::try_from(section_offset).expect("fits").to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes()); // align
    bytes.extend_from_slice(&0u32.to_le_bytes()); // reloff
    bytes.extend_from_slice(&0u32.to_le_bytes()); // nreloc
    bytes.extend_from_slice(&0u32.to_le_bytes()); // flags (S_REGULAR)
    bytes.extend_from_slice(&0u32.to_le_bytes()); // reserved1
    bytes.extend_from_slice(&0u32.to_le_bytes()); // reserved2
    bytes.extend_from_slice(&0u32.to_le_bytes()); // reserved3

    if code_signature {
        bytes.extend_from_slice(&LC_CODE_SIGNATURE.to_le_bytes());
        bytes.extend_from_slice(&u32::try_from(CODESIG_CMD_LEN).expect("fits").to_le_bytes());
        bytes.extend_from_slice(&u32::try_from(codesig_offset).expect("fits").to_le_bytes());
        bytes.extend_from_slice(&u32::try_from(codesig_size).expect("fits").to_le_bytes());
    }

    assert_eq!(
        bytes.len() as u64,
        section_offset,
        "the load commands must end exactly where the section's file offset begins"
    );
    bytes.extend_from_slice(data);
    let codesig = if code_signature {
        bytes.extend_from_slice(codesig_data);
        Some((codesig_offset, codesig_size))
    } else {
        None
    };

    Built {
        bytes,
        section_offset,
        section_size,
        codesig,
    }
}

/// A section body for `__GINARY,__payload`: the 64-byte trailer struct at
/// the section's own start — `payload_offset` relative to the section, which
/// this helper fixes at [`TRAILER_LEN`] because the payload immediately
/// follows it — and then the payload bytes themselves.
///
/// `section_size` for a section built from this body is therefore always
/// `TRAILER_LEN + payload.len()`. See `docs/dev/log/D3.md` ("payload
/// section geometry") for why this is the layout `src/payload.rs::locate`
/// and `src/sign_macos.rs` are written against, ahead of their own
/// implementation.
pub fn payload_section_body(payload: &[u8], payload_sha256: [u8; 32]) -> Vec<u8> {
    use ginary::trailer::{TRAILER_LEN, Trailer};

    let trailer = Trailer {
        payload_offset: TRAILER_LEN,
        payload_len: payload.len() as u64,
        payload_sha256,
    };
    let mut body = trailer.to_bytes().to_vec();
    body.extend_from_slice(payload);
    body
}

/// A Mach-O for `cpu_type` carrying a `__GINARY,__payload` section built by
/// [`payload_section_body`], and nothing else.
pub fn with_payload_section(cpu_type: u32, payload: &[u8], payload_sha256: [u8; 32]) -> Built {
    let body = payload_section_body(payload, payload_sha256);
    with_section(cpu_type, "__GINARY", "__payload", &body, false)
}

/// Sixteen bytes: `name`, NUL-padded, the fixed width every Mach-O segment
/// and section name field is.
///
/// # Panics
///
/// If `name` is longer than 16 bytes.
fn name16(name: &str) -> [u8; 16] {
    let mut out = [0u8; 16];
    let bytes = name.as_bytes();
    assert!(bytes.len() <= 16, "`{name}` is longer than 16 bytes");
    out[..bytes.len()].copy_from_slice(bytes);
    out
}

/// The committed real Mach-O fixture: `erts-17.0.5/bin/inet_gethost` from
/// the `OTP-29.0.5` `erlef/otp_builds` `aarch64-apple-darwin` release —
/// arm64, thin, with `__LINKEDIT` last and already ad-hoc signed. See
/// `tests/fixtures/macho/README.md`.
pub fn real_fixture_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/macho/inet_gethost-aarch64-apple-darwin")
}

/// The bytes of [`real_fixture_path`].
///
/// # Panics
///
/// If the fixture cannot be read, which would mean the repository itself is
/// incomplete rather than that a test input was malformed.
pub fn real_fixture_bytes() -> Vec<u8> {
    std::fs::read(real_fixture_path())
        .expect("tests/fixtures/macho/ is committed to the repository")
}
