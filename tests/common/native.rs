// SPDX-License-Identifier: MIT OR Apache-2.0
//! Fabricated object files, and the shipment trees they are planted in.
//!
//! `src/native.rs` reads three container formats and there is no cross
//! toolchain on this machine, so every fixture below is a header written by
//! hand — the rule `tests/common/stubfile.rs` already follows for PE and
//! `tests/common/payload.rs` follows for tar, extended to the two shapes this
//! milestone needs and neither of them had: an ELF whose `PT_INTERP` names any
//! loader at all, musl's included, and a Mach-O, which nothing in the crate
//! had a fixture for.
//!
//! Written rather than rewritten, and that is the point: `repack::patch_elf_machine`
//! can turn a copy of this test run's own binary into another machine's, but
//! it cannot give it musl's loader, and every copy of it costs thirteen
//! megabytes on a disk the whole suite shares. A whole ELF64 with one program
//! header is a hundred and forty-eight bytes.
//!
//! Nothing here writes a file that could be loaded. What the fixtures carry is
//! the header, which is all `scan_shipment` and `reconcile` read.

use std::path::{Path, PathBuf};

use crate::common::repack::{EM_AARCH64, EM_X86_64};

/// The `cpu_type` a 64-bit x86 Mach-O names.
pub const MACHO_CPU_X86_64: u32 = 0x0100_0007;

/// The `cpu_type` an arm64 Mach-O names.
pub const MACHO_CPU_ARM64: u32 = 0x0100_000C;

/// `MH_MAGIC_64`, the magic a little-endian 64-bit Mach-O starts with.
pub const MACHO_MAGIC_64: u32 = 0xfeed_facf;

/// `MH_DYLIB`, the `filetype` of a Mach-O shared library.
pub const MACHO_TYPE_DYLIB: u32 = 6;

/// `MH_EXECUTE`, the `filetype` of a Mach-O program.
pub const MACHO_TYPE_EXECUTE: u32 = 2;

/// `IMAGE_FILE_DLL`, the COFF characteristic that makes a PE a library.
pub const PE_CHARACTERISTICS_DLL: u16 = 0x2000;

/// Where the COFF `Characteristics` field sits in [`stubfile::pe_bytes`]'s
/// output: the DOS stub is `0x40` bytes, then `PE\0\0`, then eighteen bytes of
/// COFF header.
///
/// [`stubfile::pe_bytes`]: crate::common::stubfile::pe_bytes
const PE_CHARACTERISTICS_OFFSET: usize = 0x40 + 4 + 18;

/// The value that helper writes there, asserted rather than assumed so that a
/// change to it fails here instead of silently producing a program where a
/// library was asked for.
const PE_CHARACTERISTICS_EXECUTABLE: u16 = 0x0022;

/// The `e_type` of a shared object, which is also a position-independent
/// executable.
pub const ET_DYN: u16 = 3;

/// The `e_type` of a plain program.
pub const ET_EXEC: u16 = 2;

/// The `p_type` of a `PT_INTERP` program header.
const PT_INTERP: u32 = 3;

/// The length of a 64-bit ELF header.
const EHSIZE: usize = 64;

/// The length of one 64-bit program header.
const PHENTSIZE: usize = 56;

/// A whole ELF64 object, written field by field.
///
/// Every field `src/elf.rs` reads and nothing else: the class, the endianness,
/// `e_type`, `e_machine`, and — when `interp` is given — one `PT_INTERP`
/// program header pointing at the string that follows the table. That is a
/// hundred and fifty bytes rather than the thirteen megabytes a copy of this
/// test run's own binary costs, and it is the only way to produce the one
/// shape no rewriting of a host binary can: an object whose interpreter names
/// *musl*, on a machine that has no musl toolchain on it.
///
/// `tests/common/repack.rs` keeps the other technique — a real binary with two
/// header bytes rewritten — because its claims are about a file a linker
/// actually wrote. These claims are about header fields, so the header is what
/// the fixture is.
pub fn elf_bytes(machine: u16, e_type: u16, interp: Option<&str>) -> Vec<u8> {
    let phnum = usize::from(interp.is_some());
    let interp_at = EHSIZE + PHENTSIZE * phnum;
    let mut bytes = Vec::with_capacity(interp_at + 64);

    bytes.extend_from_slice(&[0x7f, b'E', b'L', b'F', 2, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    bytes.extend_from_slice(&e_type.to_le_bytes());
    bytes.extend_from_slice(&machine.to_le_bytes());
    bytes.extend_from_slice(&1u32.to_le_bytes()); // e_version
    bytes.extend_from_slice(&0u64.to_le_bytes()); // e_entry
    let phoff = if phnum == 0 { 0 } else { EHSIZE as u64 };
    bytes.extend_from_slice(&phoff.to_le_bytes());
    bytes.extend_from_slice(&0u64.to_le_bytes()); // e_shoff
    bytes.extend_from_slice(&0u32.to_le_bytes()); // e_flags
    bytes.extend_from_slice(&(EHSIZE as u16).to_le_bytes());
    bytes.extend_from_slice(&(PHENTSIZE as u16).to_le_bytes());
    bytes.extend_from_slice(
        &u16::try_from(phnum)
            .expect("one header at most")
            .to_le_bytes(),
    );
    for _ in 0..3 {
        bytes.extend_from_slice(&0u16.to_le_bytes()); // shentsize, shnum, shstrndx
    }
    assert_eq!(bytes.len(), EHSIZE, "the ELF header is 64 bytes");

    if let Some(path) = interp {
        let len = path.len() as u64 + 1;
        let at = interp_at as u64;
        bytes.extend_from_slice(&PT_INTERP.to_le_bytes());
        bytes.extend_from_slice(&4u32.to_le_bytes()); // PF_R
        for value in [at, at, at, len, len, 1] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        assert_eq!(bytes.len(), interp_at, "the program header is 56 bytes");
        bytes.extend_from_slice(path.as_bytes());
        bytes.push(0);
    }
    bytes
}

/// A shared object for `machine`, with the given interpreter.
pub fn shared_object(machine: u16, interp: Option<&str>) -> Vec<u8> {
    elf_bytes(machine, ET_DYN, interp)
}

/// A program for `machine`, with the given interpreter.
pub fn program(machine: u16, interp: Option<&str>) -> Vec<u8> {
    elf_bytes(machine, ET_EXEC, interp)
}

/// The `e_machine` of the architecture this host runs.
pub fn host_machine() -> u16 {
    match ginary::target::Target::host().arch {
        ginary::target::Arch::X86_64 => EM_X86_64,
        ginary::target::Arch::Aarch64 => EM_AARCH64,
    }
}

/// glibc's loader for `machine`, as a `PT_INTERP` names it.
pub fn gnu_interp(machine: u16) -> String {
    if machine == EM_AARCH64 {
        "/lib/ld-linux-aarch64.so.1".to_owned()
    } else {
        "/lib64/ld-linux-x86-64.so.2".to_owned()
    }
}

/// musl's loader for `machine`, as a `PT_INTERP` names it.
pub fn musl_interp(machine: u16) -> String {
    let arch = if machine == EM_AARCH64 {
        "aarch64"
    } else {
        "x86_64"
    };
    format!("/lib/ld-musl-{arch}.so.1")
}

/// The target an object built with [`host_machine`] and [`host_interp`]
/// describes.
///
/// Not [`ginary::target::Target::host`]: the fixtures here are ELF files with
/// a glibc or musl `PT_INTERP`, which is a *Linux* object whatever machine
/// wrote them. On a Linux host the two are the same value, which is why the
/// difference went unnoticed until a Windows runner read
/// `target: Some(Target { os: Linux, .. })` out of a fixture the test claimed
/// was the host's — see `docs/dev/log/E8.md` section 14. The libc follows the
/// host's, because [`host_interp`] names musl's loader on a musl machine.
pub fn host_object_target() -> ginary::target::Target {
    use ginary::target::{Libc, Os, Target};
    let libc = match Target::host().libc {
        Libc::Musl => Libc::Musl,
        _ => Libc::Gnu,
    };
    Target::new(Os::Linux, Target::host().arch, libc)
}

/// The loader an object built on *this* machine would name.
pub fn host_interp() -> String {
    match ginary::target::Target::host().libc {
        ginary::target::Libc::Musl => musl_interp(host_machine()),
        _ => gnu_interp(host_machine()),
    }
}

/// A minimal PE32+ for `machine`, marked as a library or as a program.
///
/// Built on [`crate::common::stubfile::pe_bytes`], whose COFF header is
/// already the smallest one `object` will parse; the only thing rewritten is
/// the `Characteristics` field, which is what tells a `.dll` from a `.exe`.
pub fn pe_bytes(machine: u16, dll: bool) -> Vec<u8> {
    let mut bytes = crate::common::stubfile::pe_bytes(machine, &[0u8; 128]);
    let at = PE_CHARACTERISTICS_OFFSET;
    assert_eq!(
        u16::from_le_bytes(bytes[at..at + 2].try_into().expect("two bytes")),
        PE_CHARACTERISTICS_EXECUTABLE,
        "the PE helper's characteristics field has moved"
    );
    if dll {
        let value = PE_CHARACTERISTICS_EXECUTABLE | PE_CHARACTERISTICS_DLL;
        bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
    }
    bytes
}

/// A 64-bit Mach-O header, and nothing after it.
///
/// Eight fields, no load commands, no sections: `ncmds` is zero, so the header
/// describes a whole — if empty — object rather than a truncated one. Written
/// by hand for the reason the PE helper is: there is no macOS toolchain here,
/// and the only fields this milestone reads are the magic, the `cpu_type` and
/// the `filetype`.
pub fn macho_bytes(cpu_type: u32, filetype: u32) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(32);
    bytes.extend_from_slice(&MACHO_MAGIC_64.to_le_bytes());
    bytes.extend_from_slice(&cpu_type.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes()); // cpu_subtype
    bytes.extend_from_slice(&filetype.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes()); // ncmds
    bytes.extend_from_slice(&0u32.to_le_bytes()); // sizeofcmds
    bytes.extend_from_slice(&0u32.to_le_bytes()); // flags
    bytes.extend_from_slice(&0u32.to_le_bytes()); // reserved
    bytes
}

/// A file that begins like a Mach-O and holds nothing else.
///
/// The magic and four bytes of rubbish: the scan has to notice it and has
/// nothing it can say about it, which is [`NativeKind::Unknown`] and a
/// warning.
///
/// [`NativeKind::Unknown`]: ginary::native::NativeKind::Unknown
pub fn macho_magic_only() -> Vec<u8> {
    let mut bytes = MACHO_MAGIC_64.to_le_bytes().to_vec();
    bytes.extend_from_slice(b"junk");
    bytes
}

/// A file that begins like an ELF and holds nothing else.
pub fn elf_magic_only() -> Vec<u8> {
    let mut bytes = ginary::elf::ELF_MAGIC.to_vec();
    bytes.extend_from_slice(b"not really an object");
    bytes
}

/// A DOS program: the `MZ` magic a PE begins with, and no `PE\0\0` behind it.
///
/// Long enough to hold the `e_lfanew` field at `0x3c` — a file too short for
/// it would prove the bounds check rather than the signature check — and the
/// offset it names points at rubbish.
pub fn dos_stub() -> Vec<u8> {
    let mut bytes = vec![0u8; 0x40];
    bytes[0] = b'M';
    bytes[1] = b'Z';
    bytes[0x3c..0x40].copy_from_slice(&0x40u32.to_le_bytes());
    bytes.extend_from_slice(b"this is a DOS program, not a PE\n");
    bytes
}

/// A shell script, which is what a `.so` under `priv` is often enough.
pub const SHELL_WRAPPER: &[u8] = b"#!/bin/sh\nexec /usr/bin/env true\n";

/// Writes `bytes` at `<root>/<relative>`, creating the directories.
///
/// # Panics
///
/// If the file cannot be written.
pub fn plant(root: &Path, relative: &str, bytes: &[u8]) -> PathBuf {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .unwrap_or_else(|error| panic!("cannot create {}: {error}", parent.display()));
    }
    std::fs::write(&path, bytes)
        .unwrap_or_else(|error| panic!("cannot write {}: {error}", path.display()));
    path
}

/// Writes `bytes` at `<root>/<relative>` with the execute bit set.
///
/// # Panics
///
/// If the file cannot be written or its mode cannot be set.
pub fn plant_executable(root: &Path, relative: &str, bytes: &[u8]) -> PathBuf {
    let path = plant(root, relative, bytes);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .unwrap_or_else(|error| panic!("cannot chmod {}: {error}", path.display()));
    }
    path
}

// ------------------------------------------------------- the real ELF --

/// The committed real ELF fixture: `erts-17.0.5/bin/inet_gethost` from the
/// Erlang/OTP 29.0.5 toolchain, stripped — a genuine `x86_64` Linux ELF a
/// linker wrote, with a real `PT_INTERP`, real `DT_NEEDED` (`libm.so.6`,
/// `libc.so.6`, both on `verify`'s allowlist) and `e_machine` `EM_X86_64`.
///
/// This is the file the "plant a real ELF in the payload" tests are meant to
/// carry, in place of this test run's own binary: a PE on Windows, an ELF on
/// Linux, and so a native object whose machine depends on the host rather than
/// on the file. See `tests/fixtures/elf/README.md` and `docs/dev/log/E9.md`.
pub fn real_elf_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/elf/inet_gethost-x86_64-linux-gnu")
}

/// The bytes of [`real_elf_path`].
///
/// # Panics
///
/// If the fixture cannot be read, which would mean the repository itself is
/// incomplete rather than that a test input was malformed.
pub fn real_elf_bytes() -> Vec<u8> {
    std::fs::read(real_elf_path()).expect("tests/fixtures/elf/ is committed to the repository")
}
