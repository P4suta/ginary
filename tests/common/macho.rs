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

// -------------------------------------------- reading a finished Mach-O --
//
// The entry-point reader below reads a *whole, finished* thin 64-bit Mach-O —
// the output of `sign_macos::inject_and_sign` — so a Linux test can hold that
// output to an invariant a runnable artifact has to satisfy but `codesign`
// cannot see: that the entry point still runs the stub's own instructions.
// Segment, signature and `CodeDirectory` reading is `tests/common/codesign.rs`
// already, so only `LC_MAIN` is parsed here. It parses by hand, like the
// writers above, and never panics: a malformed input yields `None`, so a test
// says what it expected with its own `expect`.

/// `LC_SEGMENT_64`.
const READ_LC_SEGMENT_64: u32 = 0x19;
/// `LC_MAIN`, the load command naming a modern Mach-O's entry point as a file
/// offset into `__TEXT`.
const READ_LC_MAIN: u32 = 0x8000_0028;

/// The little-endian `u32` at `at`, or `None` past the end.
fn read_u32(bytes: &[u8], at: usize) -> Option<u32> {
    let slice = bytes.get(at..at.checked_add(4)?)?;
    Some(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

/// The little-endian `u64` at `at`, or `None` past the end.
fn read_u64(bytes: &[u8], at: usize) -> Option<u64> {
    let slice = bytes.get(at..at.checked_add(8)?)?;
    let mut buf = [0u8; 8];
    buf.copy_from_slice(slice);
    Some(u64::from_le_bytes(buf))
}

/// Calls `visit` with `(cmd, offset)` for each load command of a thin 64-bit
/// Mach-O, stopping the moment a command runs past the buffer. `None` only when
/// the header itself cannot be read.
fn each_load_command(bytes: &[u8], mut visit: impl FnMut(u32, usize)) -> Option<()> {
    let ncmds = read_u32(bytes, 16)?;
    let mut offset = HEADER_LEN as usize;
    for _ in 0..ncmds {
        let cmd = read_u32(bytes, offset)?;
        let cmdsize = read_u32(bytes, offset + 4)? as usize;
        if cmdsize < 8 || offset.checked_add(cmdsize)? > bytes.len() {
            return Some(());
        }
        visit(cmd, offset);
        offset = offset.checked_add(cmdsize)?;
    }
    Some(())
}

/// The `fileoff` of the segment named `name`, or `None`.
fn segment_fileoff(bytes: &[u8], name: &str) -> Option<u64> {
    let mut found = None;
    let _ = each_load_command(bytes, |cmd, at| {
        if cmd != READ_LC_SEGMENT_64 {
            return;
        }
        let seg = bytes.get(at + 8..at + 24).unwrap_or(&[]);
        let end = seg.iter().position(|&b| b == 0).unwrap_or(seg.len());
        if String::from_utf8_lossy(&seg[..end]) == name {
            found = read_u64(bytes, at + 40);
        }
    });
    found
}

/// The absolute file offset the entry point resolves to, and `take` bytes of
/// the machine code that sits there, read from a finished thin 64-bit Mach-O.
pub struct EntryPoint {
    /// The absolute file offset the entry point resolves to.
    pub file_offset: u64,
    /// The bytes found there.
    pub bytes: Vec<u8>,
}

/// Reads the entry point of a finished thin 64-bit Mach-O, taking `take` bytes
/// of code, or `None` when the file carries no `LC_MAIN` or no `__TEXT`.
///
/// The entry point is `LC_MAIN`'s `entryoff`, a file offset into `__TEXT`
/// (`fileoff` `0` in an executable), so the absolute offset is
/// `__TEXT.fileoff + entryoff`. The instrument reads the *bytes at the mapped
/// entry*, not the `entryoff` field: an artifact runs its stub's first
/// instructions iff those bytes survive the rewrite wherever the entry lands —
/// whether a writer moved `__TEXT` and shifted `entryoff` to match, or left
/// both untouched. See `docs/dev/log/E9.md`.
pub fn entry_point(bytes: &[u8], take: usize) -> Option<EntryPoint> {
    let text_fileoff = segment_fileoff(bytes, "__TEXT")?;
    let mut entryoff = None;
    let _ = each_load_command(bytes, |cmd, at| {
        if cmd == READ_LC_MAIN {
            entryoff = read_u64(bytes, at + 8);
        }
    });
    let file_offset = text_fileoff.checked_add(entryoff?)?;
    let start = usize::try_from(file_offset).ok()?;
    let slice = bytes.get(start..start.checked_add(take)?)?;
    Some(EntryPoint {
        file_offset,
        bytes: slice.to_vec(),
    })
}

// ------------------------------------- a stub shaped like a linker's own --
//
// `with_section` above is a one-segment file: enough for a section lookup, and
// not enough for the writer in `src/sign_macos.rs`, which reads `__TEXT`,
// `__LINKEDIT`, `LC_MAIN` and the slack a linker leaves between the last load
// command and the first section. `stub_like` builds that shape, with the two
// knobs the missing-`LC_CODE_SIGNATURE` case turns on: whether the file
// carries one at all, and how many spare bytes sit before the first section.

/// `LC_MAIN`.
const LC_MAIN: u32 = 0x8000_0028;
/// The length of an `entry_point_command`.
const MAIN_CMD_LEN: u64 = 24;
/// The `vmaddr` `__TEXT` is given, as a real `MH_EXECUTE` carries.
const TEXT_VMADDR: u64 = 0x1_0000_0000;

/// How [`stub_like`] lays a stub out.
pub struct StubSpec<'a> {
    /// The `cputype` the header names.
    pub cpu_type: u32,
    /// Whether the file carries an `LC_CODE_SIGNATURE` over a small blob at
    /// the end of `__LINKEDIT`, as the linker leaves on every arm64 image.
    pub code_signature: bool,
    /// How many spare bytes lie between the end of the load commands and the
    /// first section's file offset. A real arm64 stub has forty.
    pub slack: u64,
    /// `__TEXT`'s one section: the bytes the entry point names.
    pub text: &'a [u8],
    /// `__LINKEDIT`'s content, before any signature.
    pub linkedit: &'a [u8],
}

/// What [`stub_like`] built, with every offset a test would otherwise have to
/// re-derive from the bytes.
pub struct BuiltStub {
    /// The whole file.
    pub bytes: Vec<u8>,
    /// How many load commands the header names.
    pub ncmds: u32,
    /// The header's `sizeofcmds`.
    pub sizeofcmds: u32,
    /// Where the load commands end: `32 + sizeofcmds`.
    pub commands_end: u64,
    /// The first section's file offset, which is where content begins.
    pub first_content_offset: u64,
    /// The spare bytes between the two, as [`StubSpec::slack`] asked for.
    pub slack: u64,
    /// `__LINKEDIT`'s `fileoff`.
    pub linkedit_fileoff: u64,
    /// The `LC_CODE_SIGNATURE` blob's offset and size, when one was asked for.
    pub codesig: Option<(u64, u64)>,
}

/// A thin, 64-bit `MH_EXECUTE` for `cpu_type` with `__TEXT`, `__LINKEDIT`,
/// `LC_MAIN` and — per [`StubSpec::code_signature`] — an `LC_CODE_SIGNATURE`,
/// laid out the way a linker lays one out: the commands first, then
/// [`StubSpec::slack`] spare bytes, then the first section, then
/// `__LINKEDIT`, then the signature blob if there is one.
///
/// # Panics
///
/// If the spec's sizes do not fit in the header fields they are written into,
/// which would mean the fixture itself is malformed rather than the code under
/// test.
pub fn stub_like(spec: &StubSpec<'_>) -> BuiltStub {
    let codesig_blob: &[u8] = b"a linker's own ad-hoc signature blob, stood in for";
    let ncmds: u32 = if spec.code_signature { 4 } else { 3 };
    let sizeofcmds = SEGMENT_CMD_LEN
        + SECTION_LEN
        + SEGMENT_CMD_LEN
        + MAIN_CMD_LEN
        + if spec.code_signature {
            CODESIG_CMD_LEN
        } else {
            0
        };
    let commands_end = HEADER_LEN + sizeofcmds;
    let text_offset = commands_end + spec.slack;
    let text_end = text_offset + spec.text.len() as u64;
    let linkedit_fileoff = text_end;
    let linkedit_len = spec.linkedit.len() as u64;
    let codesig_offset = linkedit_fileoff + linkedit_len;
    let codesig_size = codesig_blob.len() as u64;
    let linkedit_filesize = linkedit_len + if spec.code_signature { codesig_size } else { 0 };

    let mut bytes = Vec::new();
    bytes.extend_from_slice(&MH_MAGIC_64.to_le_bytes());
    bytes.extend_from_slice(&spec.cpu_type.to_le_bytes());
    bytes.extend_from_slice(&0i32.to_le_bytes()); // cpusubtype
    bytes.extend_from_slice(&MH_EXECUTE.to_le_bytes());
    bytes.extend_from_slice(&ncmds.to_le_bytes());
    bytes.extend_from_slice(&u32::try_from(sizeofcmds).expect("fits").to_le_bytes());
    bytes.extend_from_slice(&0x0020_0085u32.to_le_bytes()); // flags: PIE, TWOLEVEL, DYLDLINK
    bytes.extend_from_slice(&0u32.to_le_bytes()); // reserved
    assert_eq!(bytes.len() as u64, HEADER_LEN, "the header is 32 bytes");

    // __TEXT, mapping the header and the commands as a real one does, with one
    // section whose file offset is where the slack ends.
    segment_command(&mut bytes, "__TEXT", TEXT_VMADDR, text_end, 0, text_end, 1);
    bytes.extend_from_slice(&name16("__text"));
    bytes.extend_from_slice(&name16("__TEXT"));
    bytes.extend_from_slice(&(TEXT_VMADDR + text_offset).to_le_bytes()); // addr
    bytes.extend_from_slice(&(spec.text.len() as u64).to_le_bytes()); // size
    bytes.extend_from_slice(&u32::try_from(text_offset).expect("fits").to_le_bytes());
    bytes.extend_from_slice(&4u32.to_le_bytes()); // align
    bytes.extend_from_slice(&0u32.to_le_bytes()); // reloff
    bytes.extend_from_slice(&0u32.to_le_bytes()); // nreloc
    bytes.extend_from_slice(&0x8000_0400u32.to_le_bytes()); // PURE_INSTRUCTIONS
    bytes.extend_from_slice(&0u32.to_le_bytes()); // reserved1
    bytes.extend_from_slice(&0u32.to_le_bytes()); // reserved2
    bytes.extend_from_slice(&0u32.to_le_bytes()); // reserved3

    segment_command(
        &mut bytes,
        "__LINKEDIT",
        TEXT_VMADDR + text_end,
        linkedit_filesize,
        linkedit_fileoff,
        linkedit_filesize,
        0,
    );

    // LC_MAIN: the entry point is a file offset into __TEXT.
    bytes.extend_from_slice(&LC_MAIN.to_le_bytes());
    bytes.extend_from_slice(&u32::try_from(MAIN_CMD_LEN).expect("fits").to_le_bytes());
    bytes.extend_from_slice(&text_offset.to_le_bytes()); // entryoff
    bytes.extend_from_slice(&0u64.to_le_bytes()); // stacksize

    if spec.code_signature {
        bytes.extend_from_slice(&LC_CODE_SIGNATURE.to_le_bytes());
        bytes.extend_from_slice(&u32::try_from(CODESIG_CMD_LEN).expect("fits").to_le_bytes());
        bytes.extend_from_slice(&u32::try_from(codesig_offset).expect("fits").to_le_bytes());
        bytes.extend_from_slice(&u32::try_from(codesig_size).expect("fits").to_le_bytes());
    }

    assert_eq!(
        bytes.len() as u64,
        commands_end,
        "the load commands must end exactly `sizeofcmds` after the header"
    );
    bytes.extend(std::iter::repeat_n(
        0u8,
        usize::try_from(spec.slack).expect("fits"),
    ));
    bytes.extend_from_slice(spec.text);
    bytes.extend_from_slice(spec.linkedit);
    let codesig = if spec.code_signature {
        bytes.extend_from_slice(codesig_blob);
        Some((codesig_offset, codesig_size))
    } else {
        None
    };

    BuiltStub {
        bytes,
        ncmds,
        sizeofcmds: u32::try_from(sizeofcmds).expect("fits"),
        commands_end,
        first_content_offset: text_offset,
        slack: spec.slack,
        linkedit_fileoff,
        codesig,
    }
}

/// Writes one `segment_command_64` header, without its sections.
fn segment_command(
    bytes: &mut Vec<u8>,
    name: &str,
    vmaddr: u64,
    vmsize: u64,
    fileoff: u64,
    filesize: u64,
    nsects: u32,
) {
    let cmdsize = SEGMENT_CMD_LEN + SECTION_LEN * u64::from(nsects);
    bytes.extend_from_slice(&LC_SEGMENT_64.to_le_bytes());
    bytes.extend_from_slice(&u32::try_from(cmdsize).expect("fits").to_le_bytes());
    bytes.extend_from_slice(&name16(name));
    bytes.extend_from_slice(&vmaddr.to_le_bytes());
    bytes.extend_from_slice(&vmsize.to_le_bytes());
    bytes.extend_from_slice(&fileoff.to_le_bytes());
    bytes.extend_from_slice(&filesize.to_le_bytes());
    bytes.extend_from_slice(&7i32.to_le_bytes()); // maxprot
    bytes.extend_from_slice(&5i32.to_le_bytes()); // initprot
    bytes.extend_from_slice(&nsects.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes()); // flags
}

/// The same Mach-O with its `LC_CODE_SIGNATURE` taken away: the command
/// dropped, `ncmds` and `sizeofcmds` reduced by its sixteen bytes, the bytes
/// it occupied zeroed so they become slack, `__LINKEDIT` shrunk back to what
/// it held before the signature, and the file truncated at `dataoff`.
///
/// This is what a linker that never signed its output would have left, and it
/// is derived from a real Mach-O rather than fabricated: an x86_64 darwin stub
/// cannot be built on this machine, and the one property that matters here —
/// a whole, ordinary image with no signature command in it — survives the
/// removal exactly.
///
/// # Panics
///
/// If `bytes` is not a thin 64-bit Mach-O whose *last* load command is an
/// `LC_CODE_SIGNATURE` covering the tail of the file.
pub fn without_code_signature(bytes: &[u8]) -> Vec<u8> {
    let ncmds = read_u32(bytes, 16).expect("a Mach-O header");
    let sizeofcmds = read_u32(bytes, 20).expect("a Mach-O header");
    let commands_end = HEADER_LEN as usize + sizeofcmds as usize;
    let command_at = commands_end - CODESIG_CMD_LEN as usize;
    assert_eq!(
        read_u32(bytes, command_at),
        Some(LC_CODE_SIGNATURE),
        "the last load command must be the LC_CODE_SIGNATURE this removes"
    );
    let dataoff = u64::from(read_u32(bytes, command_at + 8).expect("dataoff"));
    let datasize = u64::from(read_u32(bytes, command_at + 12).expect("datasize"));
    assert_eq!(
        dataoff + datasize,
        bytes.len() as u64,
        "the signature must be the last thing in the file"
    );

    let mut out = bytes[..usize::try_from(dataoff).expect("fits")].to_vec();
    out[16..20].copy_from_slice(&(ncmds - 1).to_le_bytes());
    out[20..24].copy_from_slice(&(sizeofcmds - CODESIG_CMD_LEN as u32).to_le_bytes());
    out[command_at..commands_end].fill(0);

    let linkedit_at = segment_command_offset(&out, "__LINKEDIT").expect("__LINKEDIT is there");
    let linkedit_fileoff = read_u64(&out, linkedit_at + 40).expect("fileoff");
    let filesize = dataoff - linkedit_fileoff;
    out[linkedit_at + 48..linkedit_at + 56].copy_from_slice(&filesize.to_le_bytes());
    out
}

/// The file offset of the `LC_SEGMENT_64` load command naming `name`.
pub fn segment_command_offset(bytes: &[u8], name: &str) -> Option<usize> {
    let mut found = None;
    let _ = each_load_command(bytes, |cmd, at| {
        if cmd != READ_LC_SEGMENT_64 {
            return;
        }
        let seg = bytes.get(at + 8..at + 24).unwrap_or(&[]);
        let end = seg.iter().position(|&b| b == 0).unwrap_or(seg.len());
        if String::from_utf8_lossy(&seg[..end]) == name {
            found = Some(at);
        }
    });
    found
}

/// Every load command of a thin 64-bit Mach-O, as `(cmd, offset, cmdsize)`.
pub fn load_commands(bytes: &[u8]) -> Vec<(u32, usize, usize)> {
    let mut out = Vec::new();
    let _ = each_load_command(bytes, |cmd, at| {
        let size = read_u32(bytes, at + 4).unwrap_or(0) as usize;
        out.push((cmd, at, size));
    });
    out
}

/// The header's `ncmds` and `sizeofcmds`.
///
/// # Panics
///
/// If `bytes` is shorter than a Mach-O header.
pub fn command_counts(bytes: &[u8]) -> (u32, u32) {
    (
        read_u32(bytes, 16).expect("a Mach-O header"),
        read_u32(bytes, 20).expect("a Mach-O header"),
    )
}

/// The lowest file offset any section in any segment begins at, ignoring the
/// zero offset a `__bss`-style section carries.
pub fn first_section_offset(bytes: &[u8]) -> Option<u64> {
    let mut lowest: Option<u64> = None;
    let _ = each_load_command(bytes, |cmd, at| {
        if cmd != READ_LC_SEGMENT_64 {
            return;
        }
        let nsects = read_u32(bytes, at + 64).unwrap_or(0);
        for index in 0..nsects as usize {
            let section = at + SEGMENT_CMD_LEN as usize + index * SECTION_LEN as usize;
            let offset = u64::from(read_u32(bytes, section + 48).unwrap_or(0));
            if offset > 0 {
                lowest = Some(lowest.map_or(offset, |current: u64| current.min(offset)));
            }
        }
    });
    lowest
}
