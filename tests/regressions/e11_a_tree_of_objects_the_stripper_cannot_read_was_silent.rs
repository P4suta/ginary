// SPDX-License-Identifier: MIT OR Apache-2.0
//! A staged tree whose native code is not ELF was reported as a tree with no
//! native code in it, which is the silent skip `CLAUDE.md` forbids.
//!
//! **What went wrong.** `strip_elf` walks the staged tree, keeps every file
//! that begins with `\x7fELF`, and answers `ElfOutcome::NothingToStrip` when
//! that list is empty. On a Windows host the emulator, the resolver, the port
//! programs and every NIF are PE files, so the list is always empty and the
//! report says the tree held nothing:
//!
//! ```text
//! elf:   nothing to strip
//! beams: 205 files, 10164300 -> 1889457 bytes, 8274843 saved
//!
//! category      files  before    after     saved
//! erts_binary   5      15567872  15567872  0
//! ```
//!
//! (`Windows build and exit-code propagation`
//! <https://github.com/P4suta/ginary/actions/runs/33751715516/job/100636537290>,
//! `tests/stage_run.rs:478`: "the ERTS binaries are where most of the saving
//! is: CategorySize { files: 5, bytes_before: 15567872, bytes_after:
//! 15567872 }".)
//!
//! Fifteen megabytes of native code was left untouched and the report said
//! there was none. C3 already decided what the honest answer is for the
//! neighbouring case — a tree whose ELF files are for another machine is a
//! *reported* skip and not a silent `NothingToStrip`, see
//! `tests/regressions/c3_a_foreign_native_left_the_strip_report_silent.rs` —
//! and the same reasoning applies one step further out: a tree whose objects
//! are in a container ginary's stripper does not read at all.
//!
//! **The input.** Any staged tree holding PE or Mach-O objects and no ELF.
//! That is every Windows tree and every macOS tree, and it is reachable on
//! this machine by staging fabricated objects, which is what this test does.
//!
//! **The correct behaviour.** `NothingToStrip` means the tree holds no native
//! code. A tree that holds native code the ELF stripper cannot read is
//! `ElfOutcome::Skipped`, naming the format and how many files, so the size
//! table's zero has a sentence beside it.
#![cfg(feature = "cli")]

use std::path::{Path, PathBuf};

use ginary::platform::ObjectFormat;
use ginary::strip::{self, ElfOutcome, StripOptions};

use crate::common::fake_otp::FakeOtp;
use crate::common::native::{
    MACHO_CPU_ARM64, MACHO_TYPE_DYLIB, host_writes_elf, macho_bytes, pe_bytes,
};
use crate::common::stubfile::PE_MACHINE_AMD64;

/// The ERTS version the fixture runtime carries.
const ERTS_VSN: &str = "17.0.5";

/// A runtime root holding one object per entry of `natives`.
///
/// The bytes are fabricated here rather than through
/// `common::native::object_for`, so that what this file asserts is what the
/// strip report says about a tree of PE and Mach-O objects and not what some
/// other rule decided the tree should hold.
fn runtime_with(dir: &Path, natives: &[(&str, Vec<u8>)]) -> PathBuf {
    let root = dir.join("otp");
    FakeOtp::new().erts_vsn(ERTS_VSN).build_in(&root);
    for (relative, bytes) in natives {
        let path = root.join(relative);
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("a native directory");
        std::fs::write(&path, bytes).expect("a native file");
    }
    root
}

/// A PE shared library for the machine a Windows runtime is built for.
fn windows_object() -> Vec<u8> {
    pe_bytes(PE_MACHINE_AMD64, true)
}

/// The ELF half of a strip over `root`, with the beam half turned off.
fn strip_elf_only(root: &Path) -> strip::StripReport {
    let otp = ginary::otp::inspect_root(root).expect("the fixture is a usable runtime root");
    strip::strip(
        root,
        &otp,
        &StripOptions {
            elf: true,
            beams: false,
        },
    )
    .expect("the ELF phase never fails over a file it cannot read")
}

#[test]
fn a_tree_whose_objects_are_pe_is_a_reported_skip_and_not_nothing_to_strip() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let root = runtime_with(
        dir.path(),
        &[
            (
                &format!("erts-{ERTS_VSN}/bin/beam.smp.dll"),
                windows_object(),
            ),
            ("lib/crypto-5.9.2/priv/lib/crypto.dll", windows_object()),
        ],
    );

    let report = strip_elf_only(&root);

    let reason = match &report.elf {
        ElfOutcome::Skipped { reason } => reason.clone(),
        other => panic!(
            "a tree holding two PE objects is a reported skip, not {other:?}; \
             `NothingToStrip` says the tree held no native code at all, and fifteen megabytes \
             of untouched emulator is not none"
        ),
    };
    assert!(
        reason.contains(ObjectFormat::Pe.as_str()),
        "the reason names the format `strip` was handed: {reason}"
    );
    assert!(
        reason.contains('2'),
        "and how many files it covers, so the size table's zero has a sentence beside it: \
         {reason}"
    );
    assert!(
        !reason.contains("  "),
        "the sentence is a sentence, not a wrapped source line: {reason}"
    );
    assert!(
        report.per_file.is_empty(),
        "nothing was rewritten: {:?}",
        report.per_file
    );
}

#[test]
fn a_tree_whose_objects_are_mach_o_is_the_same_reported_skip() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let root = runtime_with(
        dir.path(),
        &[(
            &format!("erts-{ERTS_VSN}/bin/beam.smp"),
            macho_bytes(MACHO_CPU_ARM64, MACHO_TYPE_DYLIB),
        )],
    );

    let report = strip_elf_only(&root);

    match &report.elf {
        ElfOutcome::Skipped { reason } => assert!(
            reason.contains(ObjectFormat::MachO.as_str()),
            "the reason names the format: {reason}"
        ),
        other => panic!("a tree holding a Mach-O is a reported skip, not {other:?}"),
    }
}

#[test]
fn a_tree_that_really_holds_no_native_code_still_says_nothing_to_strip() {
    // The other side of the rule, or the change above would turn every fake
    // runtime's honest `NothingToStrip` into a skip nobody needs to read.
    let dir = tempfile::tempdir().expect("a temporary directory");
    let root = runtime_with(dir.path(), &[]);

    assert_eq!(strip_elf_only(&root).elf, ElfOutcome::NothingToStrip);
}

#[test]
fn a_tree_that_holds_both_says_out_loud_what_it_could_not_read() {
    // The mixed tree, which is the case the reported skip above does not
    // reach: `strip_elf` only consulted its tally of unreadable containers
    // when there was no readable ELF at all, so one ELF beside twenty PE
    // objects stripped the one and said nothing about the twenty. That is the
    // same silence this file exists to remove, one branch over.
    //
    // The ELF is the running test binary, which is the only real, dynamically
    // linked, unstripped ELF a test can count on — and it is an ELF only where
    // the host writes one.
    if !host_writes_elf() {
        eprintln!(
            "skipping: this host's own objects are not ELF, so a tree built from its own \
             binary cannot hold the readable half of a mixed tree"
        );
        return;
    }
    let dir = tempfile::tempdir().expect("a temporary directory");
    let exe = std::env::current_exe().expect("the running test binary");
    let root = runtime_with(
        dir.path(),
        &[
            (
                &format!("erts-{ERTS_VSN}/bin/beam.smp"),
                std::fs::read(&exe).expect("the test binary is readable"),
            ),
            ("lib/crypto-5.9.2/priv/lib/crypto.dll", windows_object()),
            ("lib/asn1-5.4/priv/lib/asn1rt_nif.dll", windows_object()),
        ],
    );

    let report = strip_elf_only(&root);

    let warning = report
        .warnings
        .iter()
        .find(|warning| warning.contains(ObjectFormat::Pe.as_str()))
        .unwrap_or_else(|| {
            panic!(
                "the two objects that were left alone are said out loud, the way the \
                 cross-machine rule beside this one says so: {:?}",
                report.warnings
            )
        });
    assert!(
        warning.contains('2'),
        "the warning names how many files it covers: {warning}"
    );
    assert!(
        !warning.contains("  "),
        "and is a sentence rather than a wrapped source line: {warning}"
    );
}

/// A tree whose only object is ELF has nothing to report about containers.
#[test]
fn a_tree_the_stripper_reads_whole_warns_about_no_container_at_all() {
    if !host_writes_elf() {
        eprintln!("skipping: this host's own objects are not ELF");
        return;
    }
    let dir = tempfile::tempdir().expect("a temporary directory");
    let exe = std::env::current_exe().expect("the running test binary");
    let root = runtime_with(
        dir.path(),
        &[(
            &format!("erts-{ERTS_VSN}/bin/beam.smp"),
            std::fs::read(&exe).expect("the test binary is readable"),
        )],
    );

    let report = strip_elf_only(&root);

    assert!(
        !report
            .warnings
            .iter()
            .any(|warning| warning.contains("does not read")),
        "nothing was skipped, so nothing is reported as skipped: {:?}",
        report.warnings
    );
}
