// SPDX-License-Identifier: MIT OR Apache-2.0
//! Writing a macOS artifact: the `__GINARY,__payload` section, and the
//! ad-hoc signature over it.
//!
//! There is no macOS toolchain on this host and no way to launch what this
//! produces here — see `docs/dev/log/D3.md` for exactly what only a macOS CI
//! runner can confirm. What *is* checkable on Linux, and what this file
//! checks, is structural: the section lands at the offset and size
//! `src/macho.rs` itself reports back, an `LC_CODE_SIGNATURE` load command
//! is present exactly when signing was asked for, and
//! [`ginary::payload::locate`] round-trips the exact bytes and digest that
//! went in. Per the plan, the committed real Mach-O fixture
//! (`tests/fixtures/macho/`) stands in for a darwin stub here: this is the
//! real coverage for `inject_and_sign`, since no genuine darwin stub can be
//! built on this machine.
// The command line half of the suite: `sign_macos` is a `cli`-gated module.
#![cfg(feature = "cli")]

mod common;

use common::codesign;
use common::codesign::{CS_ADHOC, CS_LINKER_SIGNED};
use common::macho::{
    CPU_TYPE_X86_64, MH_EXECUTE, StubSpec, command_counts, entry_point, fat_header,
    first_section_offset, load_commands, real_fixture_bytes, segment_command_offset, stub_like,
    thin_header, with_payload_section, without_code_signature,
};
use ginary::macho::{self, MachoError};
use ginary::payload::{PayloadVia, locate};
use ginary::sign_macos::{
    CODE_SIGNATURE_COMMAND_LEN, CodeSign, CodeSignatureSlot, LoadCommandSlack, MacSignCfg,
    SignMacosError, inject_and_sign, load_command_slack,
};
use ginary::trailer::{TRAILER_LEN, Trailer};

/// A digest whose bytes are all different, so a slice taken in the wrong
/// place is visible rather than accidentally right.
const DIGEST: [u8; 32] = [
    0x2a, 0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee,
    0xff, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
];

/// What `bundle.rs`'s darwin arm hands `inject_and_sign`: the 64-byte
/// trailer, `payload_offset` relative to the section it will end up inside,
/// immediately followed by the payload bytes.
fn payload_with_trailer(payload: &[u8], sha256: [u8; 32]) -> Vec<u8> {
    let mut bytes = Trailer {
        payload_offset: TRAILER_LEN,
        payload_len: payload.len() as u64,
        payload_sha256: sha256,
    }
    .to_bytes()
    .to_vec();
    bytes.extend_from_slice(payload);
    bytes
}

/// Opens `bytes` as a real file, for [`locate`], which reads a [`std::fs::File`].
fn open(bytes: &[u8]) -> (std::fs::File, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("out");
    std::fs::write(&path, bytes).expect("write");
    (std::fs::File::open(&path).expect("open"), dir)
}

#[test]
fn inject_and_sign_appends_a_payload_that_locate_round_trips_unsigned() {
    let stub = real_fixture_bytes();
    let payload = b"the payload bytes a macOS artifact carries, unsigned run";
    let with_trailer = payload_with_trailer(payload, DIGEST);
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().join("artifact");

    let report = inject_and_sign(
        &stub,
        &with_trailer,
        &out,
        &MacSignCfg {
            codesign: CodeSign::None,
        },
    )
    .expect("injecting into a real, valid thin Mach-O stub succeeds");

    assert_eq!(report.cputype, "arm64");
    assert!(!report.signed, "CodeSign::None must not sign");
    assert_eq!(report.payload_size, TRAILER_LEN + payload.len() as u64);

    let written = std::fs::read(&out).expect("the output was written");
    let facts = macho::read(&written).expect("the output is still a whole Mach-O");
    assert!(
        !facts.has_code_signature,
        "an unsigned build drops the stale LC_CODE_SIGNATURE, so there is none"
    );

    // Unsigned, so nothing sits after the payload: its trailer is the last 64
    // bytes of the file, exactly the shape every ELF and PE artifact has.
    let (file, _dir2) = open(&written);
    let found = locate(&file)
        .expect("the written artifact locates")
        .expect("its payload is there");
    assert_eq!(found.via, PayloadVia::EofTrailer);
    assert_eq!(found.offset, report.payload_offset);
    assert_eq!(found.len, payload.len() as u64);
    assert_eq!(found.sha256, DIGEST);
    assert_eq!(
        &written[found.offset as usize..(found.offset + found.len) as usize],
        payload,
        "locate's offset must point at the exact payload bytes that were injected"
    );
}

#[test]
fn inject_and_sign_applies_an_adhoc_signature_when_asked() {
    let stub = real_fixture_bytes();
    let payload = b"the payload bytes a macOS artifact carries, signed run";
    let with_trailer = payload_with_trailer(payload, DIGEST);
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().join("artifact");

    let report = inject_and_sign(
        &stub,
        &with_trailer,
        &out,
        &MacSignCfg {
            codesign: CodeSign::Adhoc,
        },
    )
    .expect("injecting and ad-hoc signing a real, valid thin Mach-O stub succeeds");

    assert!(report.signed, "CodeSign::Adhoc must sign");

    let written = std::fs::read(&out).expect("the output was written");
    let facts = macho::read(&written).expect("the signed output is still a whole Mach-O");
    assert!(
        facts.has_code_signature,
        "an ad-hoc signature adds an LC_CODE_SIGNATURE load command"
    );

    let (file, _dir2) = open(&written);
    let found = locate(&file)
        .expect("the signed artifact still locates")
        .expect("its payload is there");
    // The signed artifact ends with its ad-hoc signature, so its payload is
    // located through `LC_CODE_SIGNATURE`, not the end of the file — the
    // production path a real signed macOS artifact takes. Pin the
    // discriminant and the geometry to the same strength as the unsigned
    // twin, so this branch is asserted rather than merely digested.
    assert_eq!(found.via, PayloadVia::MachOAppended);
    assert_eq!(found.offset, report.payload_offset);
    assert_eq!(found.len, payload.len() as u64);
    assert_eq!(found.sha256, DIGEST);
    assert_eq!(
        &written[found.offset as usize..(found.offset + found.len) as usize],
        payload,
        "locate's offset must point at the exact payload bytes that were injected"
    );
}

#[test]
fn inject_and_sign_refuses_a_fat_stub() {
    let stub = fat_header(&[(0x0100_0007, 0), (0x0100_000c, 0)]);
    let payload = payload_with_trailer(b"anything", DIGEST);
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().join("artifact");

    let error = inject_and_sign(
        &stub,
        &payload,
        &out,
        &MacSignCfg {
            codesign: CodeSign::None,
        },
    )
    .expect_err("a fat Mach-O has no single architecture to inject into");

    assert!(
        matches!(error, SignMacosError::Fat),
        "expected SignMacosError::Fat, got {error:?}"
    );
}

#[test]
fn inject_and_sign_refuses_a_stub_that_is_not_a_macho_at_all() {
    let stub = b"this is not a Mach-O file of any kind".to_vec();
    let payload = payload_with_trailer(b"anything", DIGEST);
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().join("artifact");

    let error = inject_and_sign(
        &stub,
        &payload,
        &out,
        &MacSignCfg {
            codesign: CodeSign::None,
        },
    )
    .expect_err("not a Mach-O");

    assert!(
        matches!(error, SignMacosError::NotAMachO { .. }),
        "expected SignMacosError::NotAMachO, got {error:?}"
    );
}

#[test]
fn inject_and_sign_refuses_a_stub_that_already_carries_the_section() {
    let already = with_payload_section(CPU_TYPE_X86_64, b"already here", DIGEST);
    let payload = payload_with_trailer(b"a second payload", DIGEST);
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().join("artifact");

    let error = inject_and_sign(
        &already.bytes,
        &payload,
        &out,
        &MacSignCfg {
            codesign: CodeSign::None,
        },
    )
    .expect_err("a payload may not be injected twice");

    assert!(
        matches!(error, SignMacosError::AlreadySectioned),
        "expected SignMacosError::AlreadySectioned, got {error:?}"
    );
}

#[test]
fn inject_and_sign_reports_the_cputype_of_a_fabricated_x86_64_stub() {
    let stub = thin_header(CPU_TYPE_X86_64, MH_EXECUTE);
    let payload = payload_with_trailer(b"an x86_64 payload", DIGEST);
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().join("artifact");

    let report = inject_and_sign(
        &stub,
        &payload,
        &out,
        &MacSignCfg {
            codesign: CodeSign::None,
        },
    )
    .expect("injecting into a minimal, valid x86_64 thin Mach-O succeeds");

    assert_eq!(report.cputype, "x86_64");
}

#[test]
fn inject_and_sign_never_panics_on_a_truncated_thin_magic() {
    let stub = ginary::macho::MH_MAGIC_64.to_le_bytes().to_vec();
    let payload = payload_with_trailer(b"anything", DIGEST);
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().join("artifact");

    let error = inject_and_sign(
        &stub,
        &payload,
        &out,
        &MacSignCfg {
            codesign: CodeSign::None,
        },
    )
    .expect_err("four magic bytes and nothing else do not parse");

    assert!(
        matches!(
            &error,
            SignMacosError::NotAMachO { source } if matches!(source, MachoError::Parse { .. })
        ),
        "expected NotAMachO(Parse), got {error:?}"
    );
}

// ------------------------------------------- the signature the kernel reads --

// Everything below is about the *validity* of the ad-hoc signature rather
// than about its presence. `inject_and_sign_applies_an_adhoc_signature_when_
// asked` above asks whether an `LC_CODE_SIGNATURE` command is there; a kernel
// asks something stricter, and on 2026-09-03 both macOS runners answered it
// for the first time:
//
// ```text
// /Users/runner/work/_temp/84fd6172-....sh: line 10:  7695 Killed: 9  "$artifact" 0 hello world
// ##[error]Process completed with exit code 137.
// ```
//
// Line 10 is the artifact's own run, not `codesign` — which never got to
// start. 137 is 128+9, and `Killed: 9` is what the kernel does to a Mach-O
// whose `CodeDirectory` does not match the pages it is mapping. See
// `docs/dev/log/E8.md`.
//
// (`macOS build, launch and signature (macos-14, macos-aarch64)`
// <https://github.com/P4suta/ginary/actions/runs/33712111530/job/100513644659>.)

/// The signed artifact the tests below read, and the bytes it was written
/// from.
fn signed_artifact() -> (Vec<u8>, tempfile::TempDir) {
    let stub = real_fixture_bytes();
    let payload = b"the payload bytes a macOS artifact carries, and the signature must cover";
    let with_trailer = payload_with_trailer(payload, DIGEST);
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().join("artifact");

    let report = inject_and_sign(
        &stub,
        &with_trailer,
        &out,
        &MacSignCfg {
            codesign: CodeSign::Adhoc,
        },
    )
    .expect("injecting and ad-hoc signing a real, valid thin Mach-O stub succeeds");
    assert!(report.signed, "CodeSign::Adhoc must sign");

    (std::fs::read(&out).expect("the output was written"), dir)
}

#[test]
fn the_ad_hoc_code_directory_hashes_the_bytes_that_were_finally_written() {
    let (written, _dir) = signed_artifact();
    let signature = codesign::signature(&written).expect("a signed artifact carries a signature");
    let directory = &signature.code_directory;

    assert_eq!(
        codesign::first_bad_slot(&written, directory),
        None,
        "every code slot is the SHA-256 of the page it stands for. A slot that is not is a page \
         the kernel refuses to map, and it kills the process rather than reporting it: the \
         signature has to be computed over the *finished* file, after the last field written \
         into it"
    );
}

#[test]
fn the_ad_hoc_signature_starts_where_the_loader_expects_to_find_it() {
    let (written, _dir) = signed_artifact();
    let signature = codesign::signature(&written).expect("a signed artifact carries a signature");

    assert_eq!(
        signature.data_offset % codesign::SIGNATURE_ALIGNMENT,
        0,
        "the code signature begins on a {}-byte boundary, as every linker-produced one does; it \
         began at {}",
        codesign::SIGNATURE_ALIGNMENT,
        signature.data_offset
    );
    assert_eq!(
        signature.data_offset + signature.data_size,
        written.len() as u64,
        "the signature is the last thing in the file"
    );
}

#[test]
fn the_ad_hoc_code_directory_describes_the_file_it_is_attached_to() {
    let (written, _dir) = signed_artifact();
    let signature = codesign::signature(&written).expect("a signed artifact carries a signature");
    let directory = &signature.code_directory;

    assert_eq!(signature.magic, codesign::CSMAGIC_EMBEDDED_SIGNATURE);
    assert_eq!(signature.blob_count, 1, "one blob: the CodeDirectory");
    assert_eq!(signature.first_slot, codesign::CSSLOT_CODEDIRECTORY);
    assert_eq!(
        directory.flags & codesign::CS_ADHOC,
        codesign::CS_ADHOC,
        "the signature asserts no identity, and says so"
    );
    assert_eq!(directory.hash_type, 2, "SHA-256");
    assert_eq!(directory.hash_size, 32);
    assert_eq!(directory.page_size_log2, 12, "4096-byte pages");
    assert_eq!(directory.n_special_slots, 0);

    assert_eq!(
        directory.code_limit, signature.data_offset,
        "the hashes cover everything below the signature and nothing else"
    );
    assert_eq!(
        directory.n_code_slots as usize,
        usize::try_from(directory.code_limit)
            .expect("a code limit that fits")
            .div_ceil(directory.page_size()),
        "one slot per page of what is covered"
    );

    let text = codesign::segment(&written, "__TEXT").expect("a Mach-O program carries __TEXT");
    assert_eq!(
        (directory.exec_seg_base, directory.exec_seg_limit),
        (text.fileoff, text.filesize),
        "the executable segment the CodeDirectory names is __TEXT as the finished file lays it \
         out, not as the stub did"
    );
}

#[test]
fn the_payload_is_inside_what_the_ad_hoc_signature_covers() {
    let stub = real_fixture_bytes();
    let payload = b"a payload the signature has to be taken over, not around";
    let with_trailer = payload_with_trailer(payload, DIGEST);
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().join("artifact");

    let report = inject_and_sign(
        &stub,
        &with_trailer,
        &out,
        &MacSignCfg {
            codesign: CodeSign::Adhoc,
        },
    )
    .expect("injecting and ad-hoc signing a real, valid thin Mach-O stub succeeds");
    let written = std::fs::read(&out).expect("the output was written");
    let signature = codesign::signature(&written).expect("a signed artifact carries a signature");

    assert!(
        report.payload_offset + report.payload_size <= signature.code_directory.code_limit,
        "ADR 0016 grows __LINKEDIT over the payload so the signature covers it: the payload is \
         {}..{} and the hashes stop at {}",
        report.payload_offset,
        report.payload_offset + report.payload_size,
        signature.code_directory.code_limit
    );

    let linkedit =
        codesign::segment(&written, "__LINKEDIT").expect("a Mach-O program carries __LINKEDIT");
    assert!(
        linkedit.fileoff <= report.payload_offset,
        "the payload lives inside __LINKEDIT, which starts at {}",
        linkedit.fileoff
    );
    assert!(
        linkedit.fileoff <= signature.data_offset,
        "the signature lives inside __LINKEDIT, which starts at {}",
        linkedit.fileoff
    );
    assert_eq!(
        linkedit.fileoff + linkedit.filesize,
        written.len() as u64,
        "__LINKEDIT's filesize is grown to hold the payload and the signature appended to it"
    );
}

// ------------------------------------------------- E9: verifies AND runs --
//
// On both macOS runners of CI run 33724862229 the artifact's signature was
// valid (`codesign --verify --strict --verbose=4` exited 0) and the binary
// then segfaulted on exec (exit 139). A valid signature over a structurally
// broken image is exactly what these tests catch on Linux: they hold the
// output of the real `inject_and_sign` to two invariants a runnable Mach-O has
// that `codesign` does not check. See docs/dev/log/E9.md.

/// The finished artifact must run the stub's own first instructions.
///
/// `inject_and_sign` inserts the payload segment where `__LINKEDIT` began and
/// shifts every following byte of file content forward by a whole page, but it
/// does not move `LC_MAIN`'s `entryoff` with them: the entry point still names
/// the file offset the code used to sit at, which now holds load-command bytes.
/// The kernel jumps there and the process faults. The invariant is expressed
/// against the *bytes at the mapped entry*, so it holds whichever way the fix
/// keeps them there.
#[test]
fn an_injected_artifact_runs_the_stubs_own_entry_instructions() {
    let stub = real_fixture_bytes();
    let stub_entry = entry_point(&stub, 32).expect("the stub is a Mach-O with an LC_MAIN entry");

    let payload = b"a macOS artifact that must run, not merely verify";
    let with_trailer = payload_with_trailer(payload, DIGEST);
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().join("artifact");
    inject_and_sign(
        &stub,
        &with_trailer,
        &out,
        &MacSignCfg {
            codesign: CodeSign::Adhoc,
        },
    )
    .expect("injecting and signing a real thin Mach-O stub succeeds");
    let written = std::fs::read(&out).expect("the output was written");

    let out_entry = entry_point(&written, 32).expect("the finished artifact still has an entry");
    assert_eq!(
        out_entry.bytes, stub_entry.bytes,
        "the finished artifact's entry point (file offset {}) must hold the stub's own first \
         instructions (from file offset {}); a validly signed image that jumps into its own load \
         commands is the segfault CI saw",
        out_entry.file_offset, stub_entry.file_offset
    );
}

/// The finished artifact must not claim to have been linker-signed.
///
/// The ad-hoc `CodeDirectory` `inject_and_sign` writes sets `flags` to
/// `CS_ADHOC | CS_LINKER_SIGNED` (`0x20002`), which `codesign --display`
/// renders as `flags=0x20002(adhoc,linker-signed)`. ginary rewrote and
/// re-signed this binary; it did not come out of a linker, so the
/// `CS_LINKER_SIGNED` bit asserts a provenance that is no longer true. Only
/// `CS_ADHOC` may stand.
#[test]
fn an_injected_artifact_does_not_claim_to_be_linker_signed() {
    let stub = real_fixture_bytes();
    let payload = b"a macOS artifact ginary rewrote and signed itself";
    let with_trailer = payload_with_trailer(payload, DIGEST);
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().join("artifact");
    inject_and_sign(
        &stub,
        &with_trailer,
        &out,
        &MacSignCfg {
            codesign: CodeSign::Adhoc,
        },
    )
    .expect("injecting and signing a real thin Mach-O stub succeeds");
    let written = std::fs::read(&out).expect("the output was written");

    let signature =
        codesign::signature(&written).expect("a signed artifact carries an embedded signature");
    let flags = signature.code_directory.flags;
    assert_eq!(
        flags & CS_LINKER_SIGNED,
        0,
        "a binary ginary rewrote must not claim CS_LINKER_SIGNED; flags were {flags:#x}"
    );
    assert_eq!(
        flags & CS_ADHOC,
        CS_ADHOC,
        "the signature is still ad-hoc; flags were {flags:#x}"
    );
}

// ---------------------------------- E10: a stub with no signature to reuse --
//
// The arm64 job is green and the x86_64 job is not, on the same code, because
// of one difference the platform linker makes: it ad-hoc signs every arm64
// Mach-O it produces and does not always sign an x86_64 one. E9's writer reuses
// the `LC_CODE_SIGNATURE` the linker left; with none to reuse it refused,
// honestly, rather than corrupting the image:
//
// ```text
// error: cannot write the macOS payload section
//   caused by: cannot ad-hoc sign a Mach-O with no LC_CODE_SIGNATURE to reuse;
//              its load-command area cannot grow without relocating code
// ```
//
// (`macOS (macos-15-intel, macos-x86_64)`
// <https://github.com/P4suta/ginary/actions/runs/33739517757/job/100597889308>.)
//
// The way out follows from E9's own measurements. An `LC_CODE_SIGNATURE` is a
// `linkedit_data_command`: sixteen bytes. The segment-plus-section command E9
// proved impossible is a hundred and fifty-two, and the slack before the first
// section is forty. Sixteen fits in forty, and it fits *without moving
// anything*: the bytes it is written into belong to no command and no section.

/// A real, whole Mach-O carrying no `LC_CODE_SIGNATURE`: the committed arm64
/// fixture with the one its linker left taken away again.
///
/// Derived from a real binary rather than fabricated, because the claim under
/// test is about what a writer does to a *linker's* layout — the slack, the
/// segment geometry, the fixups — and no x86_64 darwin binary can be produced
/// on this machine. The `cputype` is left alone: it is not what selects the
/// branch, the absence of the command is, and restamping the header would make
/// the fixture claim to be a file it is not.
fn unsigned_real_stub() -> Vec<u8> {
    without_code_signature(&real_fixture_bytes())
}

/// The page size a segment's `vmsize` is a whole number of, matching Apple
/// Silicon's, which is what `src/sign_macos.rs` rounds a grown `__LINKEDIT` up
/// to.
const SEGMENT_PAGE_ALIGN: u64 = 0x4000;

/// The first offset at which `left` and `right` differ, or [`None`].
///
/// A byte-for-byte `assert_eq!` over sixty kilobytes prints sixty kilobytes;
/// the offset is the whole of what a reader needs.
fn first_difference(left: &[u8], right: &[u8]) -> Option<usize> {
    if left.len() != right.len() {
        return Some(left.len().min(right.len()));
    }
    left.iter().zip(right).position(|(a, b)| a != b)
}

/// Injects `payload` into `stub` and returns the finished bytes and the report.
fn signed(stub: &[u8], payload: &[u8]) -> (Vec<u8>, ginary::sign_macos::InjectReport) {
    let with_trailer = payload_with_trailer(payload, DIGEST);
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().join("artifact");
    let report = inject_and_sign(
        stub,
        &with_trailer,
        &out,
        &MacSignCfg {
            codesign: CodeSign::Adhoc,
        },
    )
    .expect("a thin Mach-O stub is injected into and signed");
    (std::fs::read(&out).expect("the output was written"), report)
}

#[test]
fn load_command_slack_measures_the_room_between_the_commands_and_the_first_section() {
    let stub = real_fixture_bytes();

    let slack = load_command_slack(&stub).expect("the committed fixture is a thin 64-bit Mach-O");

    assert_eq!(
        slack,
        LoadCommandSlack {
            commands_end: 1688,
            first_content_offset: 1728,
            free: 40,
        },
        "the measurement E9 took by hand, taken by the code that has to act on it"
    );
    let (_, sizeofcmds) = command_counts(&stub);
    assert_eq!(
        slack.commands_end,
        32 + u64::from(sizeofcmds),
        "the commands end where the header says they do"
    );
    assert_eq!(
        Some(slack.first_content_offset),
        first_section_offset(&stub),
        "content begins at the lowest section file offset in the image"
    );
    assert!(
        slack.free >= CODE_SIGNATURE_COMMAND_LEN,
        "sixteen bytes of load command fit in the forty this linker left; {} were free",
        slack.free
    );
}

#[test]
fn removing_the_code_signature_leaves_room_for_one_and_a_stub_that_still_reads() {
    let stub = unsigned_real_stub();

    let facts = macho::read(&stub).expect("the derived fixture is still a whole Mach-O");
    let slack = load_command_slack(&stub).expect("and its slack is still measurable");

    assert!(!facts.has_code_signature, "the command was taken away");
    assert_eq!(
        slack,
        LoadCommandSlack {
            commands_end: 1672,
            first_content_offset: 1728,
            free: 56,
        },
        "the sixteen bytes the removed command occupied are slack again"
    );
}

#[test]
fn a_stub_with_no_code_signature_gains_one_in_the_load_command_slack() {
    let stub = unsigned_real_stub();
    let (before_ncmds, before_sizeofcmds) = command_counts(&stub);

    let (written, report) = signed(&stub, b"a payload for a stub the linker never signed");

    assert_eq!(
        report.code_signature,
        Some(CodeSignatureSlot::Added),
        "there was no command to reuse, so one was added"
    );
    assert!(report.signed);
    let (after_ncmds, after_sizeofcmds) = command_counts(&written);
    assert_eq!(
        (after_ncmds, after_sizeofcmds),
        (
            before_ncmds + 1,
            before_sizeofcmds + u32::try_from(CODE_SIGNATURE_COMMAND_LEN).expect("16 fits"),
        ),
        "one command more, sixteen bytes longer"
    );
    let commands = load_commands(&written);
    assert_eq!(
        commands.len(),
        after_ncmds as usize,
        "`ncmds` counts the commands that are actually there"
    );
    assert_eq!(
        commands.iter().map(|(_, _, size)| *size).sum::<usize>(),
        after_sizeofcmds as usize,
        "`sizeofcmds` is the sum of the command sizes and nothing else"
    );
    assert_eq!(
        commands.last().map(|(cmd, at, size)| (*cmd, *at, *size)),
        Some((
            0x1d,
            32 + before_sizeofcmds as usize,
            CODE_SIGNATURE_COMMAND_LEN as usize
        )),
        "the added LC_CODE_SIGNATURE is written into the slack, immediately after the commands \
         that were already there"
    );
}

#[test]
fn nothing_before_linkedit_moves_when_a_code_signature_is_added() {
    let stub = unsigned_real_stub();
    let (ncmds, sizeofcmds) = command_counts(&stub);
    let commands_end = 32 + sizeofcmds as usize;
    let linkedit_at = segment_command_offset(&stub, "__LINKEDIT").expect("__LINKEDIT is there");
    let linkedit_fileoff = usize::try_from(u64::from_le_bytes(
        stub[linkedit_at + 40..linkedit_at + 48]
            .try_into()
            .expect("eight bytes"),
    ))
    .expect("an offset that fits");

    let (written, _report) = signed(&stub, b"a payload that must not push a single byte along");

    let signature = codesign::signature(&written).expect("the artifact is signed");
    let mut expected = stub[..linkedit_fileoff].to_vec();
    expected[16..20].copy_from_slice(&(ncmds + 1).to_le_bytes());
    expected[20..24].copy_from_slice(
        &(sizeofcmds + u32::try_from(CODE_SIGNATURE_COMMAND_LEN).expect("16 fits")).to_le_bytes(),
    );
    let mut command = 0x1du32.to_le_bytes().to_vec();
    command.extend_from_slice(
        &u32::try_from(CODE_SIGNATURE_COMMAND_LEN)
            .unwrap()
            .to_le_bytes(),
    );
    command.extend_from_slice(
        &u32::try_from(signature.data_offset)
            .expect("a signature offset that fits")
            .to_le_bytes(),
    );
    command.extend_from_slice(
        &u32::try_from(signature.data_size)
            .expect("a signature size that fits")
            .to_le_bytes(),
    );
    expected[commands_end..commands_end + command.len()].copy_from_slice(&command);
    // `__LINKEDIT` grows over the payload and the signature — that is the whole
    // design, and its `filesize`/`vmsize` are fields, not moved bytes. The two
    // values are derived here from the finished file's own geometry rather than
    // copied out of it, so the expectation still states what they must be.
    let linkedit_filesize = signature.data_offset + signature.data_size - linkedit_fileoff as u64;
    expected[linkedit_at + 32..linkedit_at + 40].copy_from_slice(
        &linkedit_filesize
            .next_multiple_of(SEGMENT_PAGE_ALIGN)
            .to_le_bytes(),
    );
    expected[linkedit_at + 48..linkedit_at + 56].copy_from_slice(&linkedit_filesize.to_le_bytes());

    assert_eq!(
        first_difference(&written[..linkedit_fileoff], &expected),
        None,
        "every byte before __LINKEDIT is the stub's own, except `ncmds`, `sizeofcmds`, the \
         sixteen slack bytes the new command was written into and __LINKEDIT's own two size \
         fields; a byte that moved is a relocated entry point and a segfault at exec"
    );
    let entry = entry_point(&written, 32).expect("the artifact still has an entry point");
    let stub_entry = entry_point(&stub, 32).expect("so does the stub");
    assert_eq!(
        (entry.file_offset, entry.bytes),
        (stub_entry.file_offset, stub_entry.bytes),
        "the entry point names the same offset and the same instructions"
    );
}

#[test]
fn the_added_code_signature_points_at_the_signature_after_the_grown_linkedit() {
    let stub = unsigned_real_stub();

    let (written, report) = signed(&stub, b"a payload the added command has to account for");

    let signature = codesign::signature(&written).expect("the artifact is signed");
    assert_eq!(
        signature.data_offset % codesign::SIGNATURE_ALIGNMENT,
        0,
        "the signature begins on a 16-byte boundary"
    );
    assert_eq!(
        signature.data_offset + signature.data_size,
        written.len() as u64,
        "the signature is the last thing in the file"
    );
    let linkedit = codesign::segment(&written, "__LINKEDIT").expect("__LINKEDIT is there");
    assert_eq!(
        linkedit.fileoff + linkedit.filesize,
        written.len() as u64,
        "__LINKEDIT grew over the payload and the signature, and still ends the file"
    );
    assert!(
        linkedit.fileoff <= report.payload_offset,
        "the payload is inside __LINKEDIT, which starts at {}",
        linkedit.fileoff
    );
    assert_eq!(
        report.payload_offset + report.payload_size,
        signature.data_offset,
        "the payload's trailer ends exactly where the signature begins, which is where `locate` \
         reads it from"
    );
}

#[test]
fn the_code_directory_of_a_stub_that_had_no_signature_covers_the_finished_file() {
    let stub = unsigned_real_stub();

    let (written, _report) = signed(&stub, b"bytes the CodeDirectory has to be taken over");

    let signature = codesign::signature(&written).expect("the artifact is signed");
    let directory = &signature.code_directory;
    assert_eq!(
        codesign::first_bad_slot(&written, directory),
        None,
        "every code slot is the SHA-256 of the page it stands for"
    );
    assert_eq!(
        directory.code_limit, signature.data_offset,
        "the hashes cover everything below the signature and nothing else"
    );
    assert_eq!(
        directory.flags & CS_ADHOC,
        CS_ADHOC,
        "the signature asserts no identity, and says so"
    );
    assert_eq!(
        directory.flags & CS_LINKER_SIGNED,
        0,
        "no linker produced this file"
    );
}

#[test]
fn locate_round_trips_a_payload_written_into_a_stub_that_had_no_code_signature() {
    let stub = unsigned_real_stub();
    let payload = b"the payload a launcher has to find again through the command that was added";

    let (written, report) = signed(&stub, payload);

    let (file, _dir) = open(&written);
    let found = locate(&file)
        .expect("the finished artifact reads")
        .expect("and it carries a payload");
    assert_eq!(
        found.via,
        PayloadVia::MachOAppended,
        "the trailer sits immediately before the signature the added command points at"
    );
    assert_eq!(
        (found.offset, found.len, found.sha256),
        (report.payload_offset, payload.len() as u64, DIGEST)
    );
    let start = usize::try_from(found.offset).expect("an offset that fits");
    let end = start + usize::try_from(found.len).expect("a length that fits");
    assert_eq!(&written[start..end], payload, "byte for byte");
}

#[test]
fn a_fabricated_x86_64_stub_with_no_code_signature_is_signed_like_any_other() {
    let text = b"\x55\x48\x89\xe5\x31\xc0\x5d\xc3 the entry point's own instructions";
    let built = stub_like(&StubSpec {
        cpu_type: CPU_TYPE_X86_64,
        code_signature: false,
        slack: 40,
        text,
        linkedit: b"__LINKEDIT: a symbol table, a string table, and nothing else",
    });

    let slack = load_command_slack(&built.bytes).expect("a thin 64-bit Mach-O");
    assert_eq!(
        slack,
        LoadCommandSlack {
            commands_end: built.commands_end,
            first_content_offset: built.first_content_offset,
            free: 40,
        }
    );

    let (written, report) = signed(&built.bytes, b"a payload for an x86_64 stub");

    assert_eq!(report.cputype, "x86_64");
    assert_eq!(report.code_signature, Some(CodeSignatureSlot::Added));
    assert_eq!(
        codesign::first_bad_slot(
            &written,
            &codesign::signature(&written)
                .expect("the artifact is signed")
                .code_directory,
        ),
        None
    );
}

#[test]
fn the_committed_arm64_fixture_still_reuses_the_code_signature_its_linker_left() {
    let stub = real_fixture_bytes();
    let (ncmds, sizeofcmds) = command_counts(&stub);

    let (written, report) = signed(&stub, b"a payload for a stub that arrived already signed");

    assert_eq!(
        report.code_signature,
        Some(CodeSignatureSlot::Reused),
        "an arm64 image always carries a command to reuse, and reusing it adds nothing"
    );
    assert_eq!(
        command_counts(&written),
        (ncmds, sizeofcmds),
        "not one load command is added when there is one to reuse"
    );
}

#[test]
fn a_stub_with_too_little_slack_says_how_many_bytes_it_needed_and_how_many_were_free() {
    let built = stub_like(&StubSpec {
        cpu_type: CPU_TYPE_X86_64,
        code_signature: false,
        slack: 8,
        text: b"eight spare bytes are not sixteen",
        linkedit: b"__LINKEDIT",
    });
    let with_trailer = payload_with_trailer(b"a payload with nowhere to be described", DIGEST);
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().join("artifact");

    let error = inject_and_sign(
        &built.bytes,
        &with_trailer,
        &out,
        &MacSignCfg {
            codesign: CodeSign::Adhoc,
        },
    )
    .expect_err("eight bytes of slack cannot hold a sixteen-byte load command");

    assert!(
        matches!(
            error,
            SignMacosError::NoRoomForCodeSignature {
                needed: 16,
                free: 8
            }
        ),
        "the refusal names both numbers, got {error:?}"
    );
    assert_eq!(
        error.to_string(),
        "cannot ad-hoc sign a Mach-O with no LC_CODE_SIGNATURE to reuse: adding one needs 16 \
         bytes of load command and only 8 are free before the first section, and the \
         load-command area cannot grow without relocating code"
    );
    assert!(
        !out.exists(),
        "nothing is written when nothing can be signed"
    );
}

#[test]
fn the_adr_and_the_format_document_state_both_signature_cases() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let adr = std::fs::read_to_string(
        root.join("docs/adr/0016-macho-section-payload-and-adhoc-signing.md"),
    )
    .expect("ADR 0016 is committed");
    let format = std::fs::read_to_string(root.join("docs/format.md")).expect("docs/format.md");

    for (name, text) in [("ADR 0016", &adr), ("docs/format.md", &format)] {
        assert!(
            text.contains("no LC_CODE_SIGNATURE"),
            "{name} must state the case where the stub carries no LC_CODE_SIGNATURE"
        );
        assert!(
            text.contains("16 bytes") || text.contains("sixteen bytes"),
            "{name} must say how large the load command that is added is"
        );
        assert!(
            text.contains("reuse") || text.contains("reuses"),
            "{name} must state the other case: the command the stub already carries is reused"
        );
    }
}
