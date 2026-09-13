// SPDX-License-Identifier: MIT OR Apache-2.0
//! `ginary doctor` said an ELF matched a host no ELF can run on.
//!
//! **What went wrong.** The native-code table of `doctor::project_context`
//! reads every object under a shipment's `priv` and answers one column with
//! `info.machine == host`, where `host` is `Target::host().arch`. The scan
//! itself is ELF-only — a file whose first bytes are not the ELF magic is not
//! listed at all — so the comparison decides `matches_host` for an ELF using
//! nothing but the CPU.
//!
//! On a Linux host that is the whole question. On an `aarch64` macOS host it
//! is half of it: an `aarch64` Linux shared object and the machine agree, the
//! column said `yes`, and the object cannot be loaded there by any means. The
//! first macOS run of the suite reported it as a test failure —
//!
//! ```text
//! ---- an_object_for_another_machine_is_flagged stdout ----
//! an object for aarch64 on a aarch64 host: NativeObject { path:
//! "notify/priv/lib/nif.so", machine: "aarch64", …, matches_host: true, … }
//! ```
//!
//! — and it is not a test failure. It is the defect `docs/dev/log/F1-*` fixed
//! for `ginary verify`, which "checks object format against OS as well as
//! CPU", left standing in the diagnostic that a user reaches for *first*.
//!
//! **The input.** A shipment carrying an ELF built for this host's own
//! architecture, read by `doctor` on a host whose own objects are Mach-O or
//! PE.
//!
//! **The correct behaviour.** The column answers whether this host can run the
//! object, so both halves are asked: the CPU, and whether this host's own
//! objects are ELF at all. On a Linux host nothing changes; on every other
//! host an ELF matches nothing.
#![cfg(feature = "cli")]

use std::time::SystemTime;

use crate::common::project::TempProject;
use crate::common::repack::{EM_AARCH64, EM_X86_64, patch_elf_machine, test_binary};
use ginary::doctor;
use ginary::platform::{HOST, ObjectFormat, object_format};
use ginary::target::{Arch, Target};

/// The `e_machine` this host's own architecture is spelled with.
fn host_machine() -> u16 {
    match Target::host().arch {
        Arch::X86_64 => EM_X86_64,
        Arch::Aarch64 => EM_AARCH64,
    }
}

/// Whether this host's own objects are ELF, which is the half that was missing.
fn host_runs_elf() -> bool {
    object_format(HOST) == ObjectFormat::Elf
}

/// The one object `doctor` reports for a shipment carrying `bytes`.
fn only_object(bytes: &[u8]) -> ginary::doctor::NativeObject {
    let project = TempProject::named("notify");
    let shipment = project.empty_shipment();
    let path = shipment.join("notify/priv/lib/nif.so");
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("the parent directory");
    std::fs::write(&path, bytes).expect("the planted object is written");
    let report =
        doctor::project_context(project.root(), SystemTime::now()).expect("a project context");
    let mut native = report.native;
    assert_eq!(native.len(), 1, "one planted object, one row: {native:?}");
    native.remove(0)
}

#[test]
fn an_elf_for_this_cpu_matches_only_where_the_host_runs_elf() {
    let object = only_object(&patch_elf_machine(&test_binary(), host_machine()));
    assert_eq!(
        object.machine,
        Target::host().arch.as_str(),
        "the fixture was rewritten to this host's own architecture"
    );
    assert_eq!(
        object.matches_host,
        host_runs_elf(),
        "the CPU agrees, so this column is deciding the other half: an ELF runs on a host whose \
         own objects are ELF and on no other, whatever its `e_machine` says. Row: {object:?}"
    );
}

#[test]
fn an_elf_for_another_cpu_never_matches() {
    let other = match Target::host().arch {
        Arch::X86_64 => EM_AARCH64,
        Arch::Aarch64 => EM_X86_64,
    };
    let object = only_object(&patch_elf_machine(&test_binary(), other));
    assert!(
        !object.matches_host,
        "a different CPU is enough on its own, and stays enough: {object:?}"
    );
}
