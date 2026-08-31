// SPDX-License-Identifier: MIT OR Apache-2.0
//! Read-only ELF inspection: `ginary::elf`.
//!
//! The test binary this file is compiled into is the fixture. It is a real ELF
//! executable, it is dynamically linked against the same libc every artifact
//! will be, it is not stripped, and it is on disk at a path
//! `std::env::current_exe` hands over — which makes it the one real binary a
//! test can rely on without a toolchain, an installed runtime or a checked-in
//! blob. What it *cannot* show is a second architecture or a shared object, and
//! the gated tests at the end use the host `beam.smp` for the rest.
//!
//! Everything that reads `current_exe` is `cfg(target_os = "linux")`: on macOS
//! that file is a Mach-O and the assertions would be about the wrong format.
//! The rest — the magic check, the version comparison, the never-panic
//! properties — is portable and always runs.
// The command line half of the suite: every claim in this file is about a
// module the `cli` feature carries, so a `--no-default-features` build has
// nothing here to run. See `docs/dev/log/C2.md`.
#![cfg(feature = "cli")]

mod common;

use ginary::elf::{self, ELF_MAGIC, ElfError};
use proptest::prelude::*;

use crate::common::tools::require_tools;

/// Writes `bytes` into a fresh temporary file and hands back both.
fn temp_file(name: &str, bytes: &[u8]) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join(name);
    std::fs::write(&path, bytes).expect("a temporary file");
    (dir, path)
}

#[test]
fn the_elf_magic_is_the_four_bytes_every_elf_file_starts_with() {
    assert_eq!(ELF_MAGIC, [0x7f, b'E', b'L', b'F']);
}

#[test]
fn is_elf_rejects_text_and_short_input() {
    assert!(!elf::is_elf(b"#!/bin/sh\nexit 0\n"));
    assert!(!elf::is_elf(b"\x7fEL"), "three bytes is not the magic");
    assert!(!elf::is_elf(b""));
    assert!(!elf::is_elf(b"FOR1\x00\x00\x00\x04BEAM"));
}

#[test]
fn is_elf_accepts_the_magic_and_nothing_else_about_the_file() {
    // Detection is the magic and only the magic: staging has to decide whether
    // to hand a file to `strip` before anything has parsed it.
    assert!(elf::is_elf(b"\x7fELF"));
    assert!(elf::is_elf(b"\x7fELF and then nothing that parses"));
}

#[test]
fn the_highest_glibc_version_is_found_numerically_and_not_lexically() {
    // The whole reason this is a function of its own: sorting the strings puts
    // `2.9` above `2.38`, and every artifact would report a floor two hundred
    // releases too low.
    assert_eq!(
        elf::max_glibc_version(["GLIBC_2.9", "GLIBC_2.38", "GLIBC_2.4"]),
        Some("2.38".to_owned())
    );
    assert_eq!(
        elf::max_glibc_version(["GLIBC_2.2.5", "GLIBC_2.10"]),
        Some("2.10".to_owned())
    );
}

#[test]
fn a_version_that_is_not_glibcs_is_not_a_glibc_floor() {
    assert_eq!(elf::max_glibc_version(["OPENSSL_1_1_0"]), None);
    assert_eq!(elf::max_glibc_version(Vec::<&str>::new()), None);
    assert_eq!(
        elf::max_glibc_version(["OPENSSL_1_1_0", "GLIBC_2.17"]),
        Some("2.17".to_owned())
    );
}

#[test]
fn a_file_that_is_not_an_elf_is_not_elf_rather_than_a_parse_failure() {
    let (_dir, path) = temp_file("greeting.txt", b"hello from priv\n");

    match elf::inspect(&path) {
        Err(ElfError::NotElf) => {}
        other => panic!("expected NotElf, got {other:?}"),
    }
}

#[test]
fn a_missing_file_is_an_io_error_naming_it() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let missing = dir.path().join("not-here");

    match elf::inspect(&missing) {
        Err(ElfError::Io { path, source }) => {
            assert_eq!(path, missing);
            assert_eq!(source.kind(), std::io::ErrorKind::NotFound);
        }
        other => panic!("expected Io, got {other:?}"),
    }
}

#[test]
fn four_bytes_of_magic_and_nothing_else_is_a_parse_error() {
    match elf::inspect_bytes(&ELF_MAGIC) {
        Err(ElfError::Parse { message }) => assert!(!message.is_empty(), "the parser said nothing"),
        other => panic!("expected Parse, got {other:?}"),
    }
}

#[cfg(target_os = "linux")]
#[test]
fn the_running_test_binary_needs_libc_and_names_its_loader() {
    let exe = std::env::current_exe().expect("the running test binary");
    let info = elf::inspect(&exe).expect("a cargo test binary is an ELF file");

    assert_eq!(info.class, 64, "ginary supports 64-bit hosts only");
    assert_eq!(
        info.machine,
        std::env::consts::ARCH,
        "the machine is spelled the way `target.rs` spells an architecture"
    );
    assert!(
        info.needed.iter().any(|name| name == "libc.so.6"),
        "a dynamically linked Rust binary needs libc: {:?}",
        info.needed
    );
    let interp = info.interp.expect("a dynamic executable names its loader");
    assert!(
        interp.contains("ld-linux"),
        "the interpreter is the dynamic loader: {interp}"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn the_running_test_binarys_glibc_floor_is_a_version_number() {
    let exe = std::env::current_exe().expect("the running test binary");
    let info = elf::inspect(&exe).expect("a cargo test binary is an ELF file");

    let floor = info
        .glibc_max
        .expect("a binary that needs libc.so.6 requires a GLIBC_x.y from it");
    let parts: Vec<&str> = floor.split('.').collect();
    assert!(
        parts.len() >= 2 && parts.iter().all(|part| part.parse::<u32>().is_ok()),
        "the floor is the number alone, without the GLIBC_ prefix: {floor}"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn a_cargo_test_binary_is_a_pie_and_is_not_stripped() {
    let exe = std::env::current_exe().expect("the running test binary");
    let info = elf::inspect(&exe).expect("a cargo test binary is an ELF file");

    assert!(info.is_pie, "cargo links a position-independent executable");
    assert!(
        !info.stripped,
        "a debug build keeps its .symtab; a `stripped` that is always true says nothing"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn a_truncated_binary_is_an_error_rather_than_a_panic() {
    let exe = std::env::current_exe().expect("the running test binary");
    let bytes = std::fs::read(&exe).expect("the running test binary is readable");

    // The section header table lives at the end of the file, so every one of
    // these cuts it off. A parser that trusted `e_shoff` would index past the
    // end of the buffer at each.
    for end in [4, 16, 64, 1024, bytes.len() / 2] {
        let (_dir, path) = temp_file("truncated", &bytes[..end]);
        assert!(
            elf::inspect(&path).is_err(),
            "a binary cut to {end} bytes must be a reported error"
        );
        assert!(elf::inspect_bytes(&bytes[..end]).is_err());
    }
}

#[test]
fn the_host_beam_smp_needs_the_three_libraries_a_packaged_runtime_carries_nothing_for() {
    let Some(_tools) = require_tools(&["erl"]) else {
        return;
    };
    let otp = ginary::otp::discover(None).expect("the host OTP installation");
    let beam_smp = otp.erts_bin.join("beam.smp");

    let info = elf::inspect(&beam_smp).expect("beam.smp is an ELF file");

    for library in [
        "libc.so.6",
        "libtinfo.so.6",
        "libstdc++.so.6",
        "libgcc_s.so.1",
    ] {
        assert!(
            info.needed.iter().any(|name| name == library),
            "beam.smp needs `{library}`, and an artifact that does not say so is a trap: {:?}",
            info.needed
        );
    }
    assert_eq!(info.machine, std::env::consts::ARCH);

    // The number docs/dev/log/A2.md records. Printed rather than asserted:
    // it is a property of the machine's OTP build, not of ginary.
    eprintln!(
        "beam.smp glibc_max = {:?}, interp = {:?}, stripped = {}",
        info.glibc_max, info.interp, info.stripped
    );
    assert!(
        info.glibc_max.is_some(),
        "a binary linked against glibc requires a version of it"
    );
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Arbitrary bytes are a typed error, never a panic.
    #[test]
    fn inspect_bytes_never_panics_on_arbitrary_bytes(
        bytes in prop::collection::vec(any::<u8>(), 0..1024),
    ) {
        let _ = elf::inspect_bytes(&bytes);
    }

    /// The same, over bytes that start with the magic and then do not.
    #[test]
    fn inspect_bytes_never_panics_on_almost_an_elf(
        tail in prop::collection::vec(any::<u8>(), 0..512),
    ) {
        let mut bytes = ELF_MAGIC.to_vec();
        bytes.extend_from_slice(&tail);
        let _ = elf::inspect_bytes(&bytes);
    }

    /// `is_elf` answers rather than raising, whatever it is given.
    #[test]
    fn is_elf_never_panics_on_arbitrary_bytes(
        bytes in prop::collection::vec(any::<u8>(), 0..64),
    ) {
        prop_assert_eq!(elf::is_elf(&bytes), bytes.starts_with(&ELF_MAGIC));
    }
}
