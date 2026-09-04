// SPDX-License-Identifier: MIT OR Apache-2.0
//! The two branches C3 added to `strip_elf` — a tree whose native files are
//! all for another machine, and one where only some of them are — shipped
//! with no test at all.
//!
//! The bug they fix is real: `strip` from this machine's binutils answers
//! "Unable to recognise the format of the input file" for an aarch64 object
//! on an x86-64 host, so a cross build that ran it failed over files upstream
//! had already stripped. The fix reports the skip instead. But nothing held
//! either branch to anything, so the whole-tree branch could have returned
//! `NothingToStrip` — the silent skip `CLAUDE.md` forbids — and the partial
//! branch could have dropped the host's own files along with the foreign ones,
//! and the suite would have stayed green.
//!
//! Both sentences are asserted as rendered text, because that is what a
//! developer reads and because the whole-tree one shipped with a run of
//! eighteen literal spaces in the middle of it.
#![cfg(feature = "cli")]

use std::path::{Path, PathBuf};

use ginary::strip::{self, ElfOutcome, StripOptions};

use crate::common::fake_otp::FakeOtp;
use crate::common::native::host_writes_elf;
use crate::common::repack::{foreign_machine, patch_elf_machine};

/// The ERTS version the fixture runtime carries.
const ERTS_VSN: &str = "17.0.5";

/// A runtime root with `count` foreign natives and `host_natives` host ones.
///
/// The host's native code is the running test binary, which is the only real,
/// dynamically linked, unstripped ELF a test can count on; the foreign one is
/// the same bytes with `e_machine` rewritten, which is a file `elf::inspect`
/// reads as another architecture and `strip` here genuinely cannot work on.
fn runtime_with(dir: &Path, foreign: &[&str], host: &[&str]) -> PathBuf {
    let root = dir.join("otp");
    FakeOtp::new().erts_vsn(ERTS_VSN).build_in(&root);
    let exe = std::env::current_exe().expect("the running test binary");
    let bytes = std::fs::read(&exe).expect("the test binary is readable");
    let patched = patch_elf_machine(&bytes, foreign_machine());

    for relative in foreign {
        let path = root.join(relative);
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("a native directory");
        std::fs::write(&path, &patched).expect("a foreign native file");
    }
    for relative in host {
        let path = root.join(relative);
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("a native directory");
        std::fs::write(&path, &bytes).expect("a host native file");
    }
    root
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
fn a_tree_whose_natives_are_all_for_another_machine_is_a_reported_skip() {
    // The fixture is the running test binary, patched: a real ELF a linker
    // wrote, which is what `strip` and `elf::inspect` are being held to here.
    // It is an ELF only where the host writes one, and on a Windows runner
    // the tree it built held two PE files, which `strip_elf` reports as a
    // container it cannot read rather than as another machine — a different
    // rule, pinned by
    // `e11_a_tree_of_objects_the_stripper_cannot_read_was_silent`.
    if !host_writes_elf() {
        eprintln!(
            "skipping: this host's own objects are not ELF, so a tree built from its own \
             binary cannot exercise the cross-machine branch of the ELF stripper"
        );
        return;
    }
    let dir = tempfile::tempdir().expect("a temporary directory");
    let root = runtime_with(
        dir.path(),
        &["lib/nif/priv/one.so", "lib/nif/priv/two.so"],
        &[],
    );

    let report = strip_elf_only(&root);

    let reason = match &report.elf {
        ElfOutcome::Skipped { reason } => reason.clone(),
        other => panic!(
            "a tree `strip` here cannot read is a reported skip, not {other:?}; a silent \
             `NothingToStrip` would say the tree held no native code at all"
        ),
    };
    assert!(
        reason.contains("another machine"),
        "the reason says why: {reason}"
    );
    assert!(
        reason.contains(&foreign_machine_name()) && reason.contains(host_machine()),
        "and names both machines, so a reader can tell which end is wrong: {reason}"
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
fn a_tree_with_some_foreign_natives_warns_and_still_strips_the_hosts_own() {
    // The fixture is the running test binary, patched: a real ELF a linker
    // wrote, which is what `strip` and `elf::inspect` are being held to here.
    // It is an ELF only where the host writes one, and on a Windows runner
    // the tree it built held two PE files, which `strip_elf` reports as a
    // container it cannot read rather than as another machine — a different
    // rule, pinned by
    // `e11_a_tree_of_objects_the_stripper_cannot_read_was_silent`.
    if !host_writes_elf() {
        eprintln!(
            "skipping: this host's own objects are not ELF, so a tree built from its own \
             binary cannot exercise the cross-machine branch of the ELF stripper"
        );
        return;
    }
    let dir = tempfile::tempdir().expect("a temporary directory");
    let root = runtime_with(
        dir.path(),
        &["lib/nif/priv/foreign.so"],
        &[&format!("erts-{ERTS_VSN}/bin/beam.smp")],
    );

    let report = strip_elf_only(&root);

    let warning = report
        .warnings
        .iter()
        .find(|warning| warning.contains("another machine"))
        .unwrap_or_else(|| {
            panic!(
                "the file that was left alone is said out loud: {:?}",
                report.warnings
            )
        });
    assert!(
        warning.contains(host_machine()),
        "the warning names the machine `strip` here reads: {warning}"
    );
    assert!(
        !warning.contains("  "),
        "and is a sentence rather than a wrapped source line: {warning}"
    );

    // Whether `strip` is on this machine or not, exactly one file was still a
    // candidate: the foreign one was taken out of the list, and the host's own
    // was not.
    match &report.elf {
        ElfOutcome::Stripped { files, .. } => assert_eq!(
            *files, 1,
            "the host's own native was stripped and the foreign one was not"
        ),
        ElfOutcome::Skipped { reason } => assert!(
            reason.contains("is not on PATH") && reason.contains("1 native file"),
            "with no `strip` on the machine the count is still the host's own one: {reason}"
        ),
        other => panic!("expected the ELF phase to have one candidate, got {other:?}"),
    }
}

/// The machine name `elf::inspect` gives the patched files.
fn foreign_machine_name() -> String {
    match ginary::target::Target::host().arch {
        ginary::target::Arch::X86_64 => "aarch64".to_owned(),
        ginary::target::Arch::Aarch64 => "x86_64".to_owned(),
    }
}

/// The machine name this host reads.
fn host_machine() -> &'static str {
    ginary::target::Target::host().arch.as_str()
}
