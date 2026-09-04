// SPDX-License-Identifier: MIT OR Apache-2.0
//! Five tests copied `std::env::current_exe()` somewhere and read it back with
//! `ginary::elf`, which is a claim about the host's object format rather than
//! about anything under test.
//!
//! **What went wrong.** "The running test binary is the only real, unstripped,
//! dynamically linked ELF a test can count on" is a comment that appears, in
//! nearly those words, in `tests/strip.rs`, in
//! `c3_a_foreign_native_left_the_strip_report_silent` and in the module
//! documentation of `tests/e2e_native.rs`. It is true on a Linux host and on
//! no other. The Windows runner read the same file and got what a PE is:
//!
//! ```text
//! ---- a_native_binary_in_the_staged_tree_is_stripped_and_stays_the_same_machine ----
//! the copy is an ELF file: NotElf
//!
//! ---- a_host_build_records_the_native_code_it_shipped ----
//! error: cannot ship the native code this shipment carries
//!   caused by: native code in the shipment does not match target windows-x86_64
//! package    artifact                   object
//! hello_ffi  hello_ffi/priv/lib/nif.so  ELF x86_64 glibc (linux-x86_64-gnu)
//!
//! ---- a_runtime_that_cannot_load_a_nif_does_not_refuse_a_port_program ----
//! a static runtime never has to open a program: Mismatch {
//!   target: Target { os: Windows, arch: X86_64, libc: None },
//!   rows: [MismatchRow { rel_path: "tooling/priv/bin/helper",
//!                        facts: "ELF x86_64 glibc (linux-x86_64-gnu)" }] }
//! ```
//!
//! (`Windows build and exit-code propagation`
//! <https://github.com/P4suta/ginary/actions/runs/33751715516/job/100636537290>,
//! `tests/strip.rs:718`, `tests/e2e_native.rs:125`,
//! `tests/regressions/c4_a_position_independent_program_was_a_shared_object.rs:124`
//! and both tests of
//! `tests/regressions/c3_a_foreign_native_left_the_strip_report_silent.rs`.)
//!
//! Every one of those refusals is ginary being right. An ELF cannot travel to
//! a Windows target, and `ginary::elf` reading a PE as `NotElf` is the whole
//! of its job. What is wrong is the fixture calling an ELF "the host's own
//! native code" on a host that writes something else.
//!
//! **The input.** Any host whose executables are not ELF.
//!
//! **The correct behaviour.** Which container format an operating system
//! writes is `ginary::platform::object_format`, and a fixture that wants "an
//! object for this target" gets one in that format from
//! `common::native::object_for`. `ObjectFormat` moves to `platform` for the
//! same reason: it is a fact about a platform before it is a fact about a
//! file, and the launcher-side half of the suite has to be able to ask.

#![cfg(feature = "cli")]

use ginary::platform::{ObjectFormat, object_format};
use ginary::target::{Arch, Libc, Os, Target};

use crate::common::native::object_for;

#[test]
fn each_operating_system_writes_the_format_it_writes() {
    assert_eq!(
        [
            object_format(Os::Linux),
            object_format(Os::Macos),
            object_format(Os::Windows),
        ],
        [ObjectFormat::Elf, ObjectFormat::MachO, ObjectFormat::Pe],
    );
}

#[test]
fn the_format_is_the_same_value_the_scanner_names() {
    // `native::ObjectFormat` is the re-export, so a table column and a
    // platform rule cannot come to mean different things.
    assert_eq!(
        object_format(Os::Windows).as_str(),
        ginary::native::ObjectFormat::Pe.as_str()
    );
    assert_eq!(
        [
            ObjectFormat::Elf.as_str(),
            ObjectFormat::Pe.as_str(),
            ObjectFormat::MachO.as_str()
        ],
        ["elf", "pe", "macho"]
    );
}

/// Writes `bytes` into `dir` and answers what the scanner makes of the file.
fn described(dir: &std::path::Path, name: &str, bytes: &[u8]) -> ginary::native::ObjectDescription {
    let path = dir.join(name);
    std::fs::write(&path, bytes).expect("a fixture object");
    ginary::native::describe_object(&path)
        .expect("the scanner reads the file")
        .unwrap_or_else(|| panic!("{name} is not an object at all"))
}

#[test]
fn an_object_for_a_target_is_readable_as_that_targets_own_kind() {
    let dir = tempfile::tempdir().expect("a temporary directory");

    let windows = Target::new(Os::Windows, Arch::X86_64, Libc::None);
    assert_eq!(
        described(dir.path(), "windows.dll", &object_for(&windows)).format,
        ObjectFormat::Pe,
        "an object planted as this target's own native code has to be in this target's format"
    );

    let macos = Target::new(Os::Macos, Arch::Aarch64, Libc::None);
    assert_eq!(
        described(dir.path(), "macos.so", &object_for(&macos)).format,
        ObjectFormat::MachO
    );

    let linux = Target::new(Os::Linux, Arch::X86_64, Libc::Gnu);
    assert_eq!(
        described(dir.path(), "linux.so", &object_for(&linux)).format,
        ObjectFormat::Elf
    );
}

#[test]
fn a_shipment_planted_with_it_is_one_the_host_build_can_ship() {
    // The claim `tests/e2e_native.rs` and the C4 regression are really
    // making: whatever this machine is, the object standing in for its own
    // native code is one a build for `Target::host()` does not refuse.
    let dir = tempfile::tempdir().expect("a temporary directory");
    let host = Target::host();
    assert_eq!(
        described(dir.path(), "host.so", &object_for(&host)).format,
        object_format(host.os),
        "the object a fixture plants as the host's own has to be in the host's own format"
    );
}
