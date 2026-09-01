// SPDX-License-Identifier: MIT OR Apache-2.0
//! Read-only inspection of Mach-O binaries: the magic, the `cputype`, the
//! section table, whether a code signature is present, and whether the file
//! is fat.
//!
//! Not gated behind the `cli` feature: `src/macho.rs` is on the launcher
//! path, so this test binary has to build and pass under
//! `--no-default-features` (`mise run test:stub`) as well as the default
//! build. Everything under test is either hand-fabricated by
//! `tests/common/macho.rs` or the one real Mach-O committed at
//! `tests/fixtures/macho/` — see that directory's `README.md` for where it
//! came from.

mod common;

use common::macho::{
    CPU_TYPE_ARM64, CPU_TYPE_X86_64, MH_EXECUTE, fat_header, magic_only, real_fixture_bytes,
    real_fixture_path, thin_header, with_section,
};
use ginary::macho::{MachoError, inspect, is_macho, read, section};
use ginary::target::{Arch, Libc, Os, Target};
use proptest::prelude::*;

/// `arm64`'s target: no libc distinction on macOS, only one architecture at
/// a time here.
fn macos_aarch64() -> Target {
    Target::new(Os::Macos, Arch::Aarch64, Libc::None)
}

/// `x86_64`'s target.
fn macos_x86_64() -> Target {
    Target::new(Os::Macos, Arch::X86_64, Libc::None)
}

#[test]
fn is_macho_recognizes_all_four_thin_magics_and_both_fat_magics() {
    let magics: [u32; 6] = [
        0xfeed_face, // MH_MAGIC
        0xcefa_edfe, // MH_CIGAM
        0xfeed_facf, // MH_MAGIC_64
        0xcffa_edfe, // MH_CIGAM_64
        0xcafe_babe, // FAT_MAGIC
        0xbeba_feca, // FAT_CIGAM
    ];
    for magic in magics {
        let bytes = magic.to_le_bytes();
        assert!(
            is_macho(&bytes),
            "0x{magic:08x} as the first four bytes should be recognized as Mach-O"
        );
    }
}

#[test]
fn is_macho_is_false_for_bytes_that_hold_no_macho_magic() {
    assert!(!is_macho(&[]));
    assert!(!is_macho(b"\x7fELF"));
    assert!(!is_macho(b"MZ\0\0"));
    assert!(!is_macho(b"not an object at all"));
}

#[test]
fn read_reports_the_x86_64_cputype_from_a_hand_fabricated_header() {
    let bytes = thin_header(CPU_TYPE_X86_64, MH_EXECUTE);

    let facts = read(&bytes).expect("a whole, empty thin header parses");

    assert_eq!(facts.cputype, "x86_64");
    assert_eq!(facts.target, Some(macos_x86_64()));
    assert!(!facts.is_fat);
    assert!(!facts.has_code_signature);
    assert_eq!(facts.sections, Vec::new());
}

#[test]
fn read_reports_the_arm64_cputype_from_a_hand_fabricated_header() {
    let bytes = thin_header(CPU_TYPE_ARM64, MH_EXECUTE);

    let facts = read(&bytes).expect("a whole, empty thin header parses");

    assert_eq!(facts.cputype, "arm64");
    assert_eq!(facts.target, Some(macos_aarch64()));
    assert!(!facts.is_fat);
    assert!(!facts.has_code_signature);
    assert_eq!(facts.sections, Vec::new());
}

#[test]
fn read_reports_the_arm64_cputype_from_the_committed_real_fixture() {
    let bytes = real_fixture_bytes();

    let facts = read(&bytes).expect("the committed fixture is a real, whole Mach-O");

    assert_eq!(facts.cputype, "arm64");
    assert_eq!(facts.target, Some(macos_aarch64()));
    assert!(!facts.is_fat, "the fixture is thin, not fat");
}

#[test]
fn read_lists_a_known_section_from_the_committed_real_fixture() {
    let bytes = real_fixture_bytes();

    let facts = read(&bytes).expect("the committed fixture is a real, whole Mach-O");

    assert!(
        facts
            .sections
            .contains(&("__TEXT".to_owned(), "__text".to_owned(), 1728, 22324)),
        "expected (__TEXT, __text, 1728, 22324) among {:?}",
        facts.sections
    );
}

#[test]
fn read_reports_the_code_signature_load_command_on_the_committed_real_fixture() {
    let bytes = real_fixture_bytes();

    let facts = read(&bytes).expect("the committed fixture is a real, whole Mach-O");

    assert!(
        facts.has_code_signature,
        "erlef's own build already ad-hoc signs arm64 binaries; see \
         tests/fixtures/macho/README.md"
    );
}

#[test]
fn read_reports_no_code_signature_on_a_header_with_no_load_commands() {
    let bytes = thin_header(CPU_TYPE_ARM64, MH_EXECUTE);

    let facts = read(&bytes).expect("a whole, empty thin header parses");

    assert!(!facts.has_code_signature);
}

#[test]
fn read_reports_a_code_signature_load_command_on_a_hand_fabricated_section_with_signing() {
    let built = with_section(
        CPU_TYPE_ARM64,
        "__GINARY",
        "__payload",
        b"payload bytes",
        true,
    );

    let facts = read(&built.bytes).expect("the hand-fabricated Mach-O parses");

    assert!(facts.has_code_signature);
}

#[test]
fn read_reports_is_fat_true_and_no_single_cputype_for_a_fat_header() {
    let bytes = fat_header(&[(CPU_TYPE_X86_64, 0), (CPU_TYPE_ARM64, 0)]);

    let facts = read(&bytes).expect("a fat header is not an error, only ambiguous");

    assert!(facts.is_fat);
    assert_eq!(facts.cputype, "", "a fat binary has no single cputype");
    assert_eq!(facts.target, None);
    assert_eq!(facts.sections, Vec::new());
}

#[test]
fn read_refuses_bytes_that_are_not_a_macho_at_all() {
    let error = read(b"this is plainly not a Mach-O file").expect_err("not a Mach-O");

    assert!(
        matches!(error, MachoError::NotMachO),
        "expected MachoError::NotMachO, got {error:?}"
    );
}

#[test]
fn read_refuses_a_truncated_thin_header() {
    let error = read(&magic_only()).expect_err("four magic bytes and junk do not parse");

    assert!(
        matches!(error, MachoError::Parse { .. }),
        "expected MachoError::Parse, got {error:?}"
    );
}

#[test]
fn section_finds_a_planted_section_at_its_file_offset() {
    let built = with_section(
        CPU_TYPE_ARM64,
        "__GINARY",
        "__payload",
        b"payload bytes",
        false,
    );

    let found = section(&built.bytes, "__GINARY", "__payload");

    assert_eq!(found, Some((built.section_offset, built.section_size)));
}

#[test]
fn section_is_none_for_a_segment_the_file_does_not_carry() {
    let built = with_section(
        CPU_TYPE_ARM64,
        "__GINARY",
        "__payload",
        b"payload bytes",
        false,
    );

    assert_eq!(section(&built.bytes, "__GINARY", "__nope"), None);
    assert_eq!(section(&built.bytes, "__TEXT", "__payload"), None);
}

#[test]
fn section_is_none_for_bytes_that_are_not_a_macho_at_all() {
    assert_eq!(section(b"not a macho", "__GINARY", "__payload"), None);
}

#[test]
fn section_finds_the_known_section_in_the_committed_real_fixture() {
    let bytes = real_fixture_bytes();

    assert_eq!(section(&bytes, "__TEXT", "__text"), Some((1728, 22324)));
}

#[test]
fn inspect_reads_the_committed_fixture_from_disk() {
    let facts = inspect(&real_fixture_path()).expect("the committed fixture is readable");

    assert_eq!(facts.cputype, "arm64");
}

#[test]
fn inspect_refuses_a_file_larger_than_the_cap_without_reading_it_whole() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("oversized");
    let file = std::fs::File::create(&path).expect("create the sparse file");
    file.set_len(ginary::macho::MAX_MACHO_BYTES + 1)
        .expect("extend it past the cap");
    drop(file);

    let error = inspect(&path).expect_err("a file past the cap is refused");

    assert!(
        matches!(error, MachoError::TooLarge { len, .. } if len == ginary::macho::MAX_MACHO_BYTES + 1),
        "expected MachoError::TooLarge, got {error:?}"
    );
}

#[test]
fn inspect_reports_io_for_a_file_that_does_not_exist() {
    let error = inspect(std::path::Path::new("/nonexistent/does/not/exist/at/all"))
        .expect_err("no such file");

    assert!(
        matches!(error, MachoError::Io { .. }),
        "expected MachoError::Io, got {error:?}"
    );
}

proptest! {
    /// Every byte string is either not a Mach-O, a fat one, or a thin one
    /// that parses or does not — but `is_macho` and `read` never panic.
    #[test]
    fn read_and_is_macho_never_panic_on_arbitrary_bytes(bytes in prop::collection::vec(any::<u8>(), 0..256)) {
        let _ = is_macho(&bytes);
        let _ = read(&bytes);
        let _ = section(&bytes, "__TEXT", "__text");
    }

    /// A buffer that starts with a real thin magic still never panics,
    /// whatever nonsense follows it — this is the shape `is_macho` says yes
    /// to and `read` has to be the most careful about.
    #[test]
    fn read_never_panics_on_bytes_that_begin_with_a_thin_magic(
        rest in prop::collection::vec(any::<u8>(), 0..256)
    ) {
        let mut bytes = ginary::macho::MH_MAGIC_64.to_le_bytes().to_vec();
        bytes.extend_from_slice(&rest);
        let _ = read(&bytes);
    }
}
