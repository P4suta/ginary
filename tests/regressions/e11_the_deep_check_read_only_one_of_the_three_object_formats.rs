// SPDX-License-Identifier: MIT OR Apache-2.0
//! `ginary verify` and the `needs:` line decide what is an object by looking
//! for `\x7fELF`, so on Windows they reported nothing at all rather than
//! reporting that they had read nothing.
//!
//! **What went wrong.** Three call sites ask "is this file an object I should
//! read", and each of them spells the question as `elf::is_elf`:
//! `verify::read_entry`, `strip::starts_like_an_elf` and `report::measure`. On
//! a platform whose objects are PE, all three answer no for every file, and
//! each then reports the *absence* of native code rather than the absence of a
//! reader for it:
//!
//! ```text
//! ---- a_real_artifact_verifies_clean ----
//! not one of the artifact's objects was found in the installation at
//! d:/a/_temp/.setup-beam/otp, so the expectation below is empty because
//! nothing was read rather than because nothing is wrong. The objects are []
//!
//! ---- the_needs_line_lists_the_libraries_the_runtime_loads ----
//! `libc.so.6` is what beam.smp loads, and an artifact that does not say so is
//! a trap:
//! needs: (none)
//! ```
//!
//! (`Windows build and exit-code propagation`
//! <https://github.com/P4suta/ginary/actions/runs/33751715516/job/100636537290>,
//! `tests/verify.rs:773` and `tests/stage_run.rs:453`.)
//!
//! This is the last `tests/verify.rs` failure, and it is exactly one thing:
//! `report.objects` is empty because the deep check reads one container format
//! and the artifact is written in another. Its own anti-vacuity guard — added
//! by `e7_a_real_artifact_had_to_verify_on_the_hosts_own_erlang` — is what
//! caught it, which is the guard working.
//!
//! **The input.** Any artifact whose objects are PE or Mach-O.
//!
//! **The correct behaviour.** Which format a file's first bytes name is one
//! rule, `ginary::platform::object_format_of`, and the three call sites use
//! it. Answering `None` for a PE is what makes the silence possible: a caller
//! told "this is a PE, which I do not read" can say so, and a caller told
//! "this is not an object" has nothing to report. What each call site then
//! does with a format it cannot read is its own decision — a reported skip for
//! `strip`, a stated `needs:` line, a named object for `verify` — but none of
//! them may go on saying there was nothing there.

use ginary::platform::{ObjectFormat, object_format_of};

use crate::common::native::{
    MACHO_CPU_ARM64, MACHO_CPU_X86_64, MACHO_TYPE_DYLIB, MACHO_TYPE_EXECUTE, dos_stub, elf_bytes,
    macho_bytes,
};
use crate::common::stubfile::{PE_MACHINE_AMD64, PE_MACHINE_ARM64, pe_bytes};

#[test]
fn each_of_the_three_magics_names_its_own_format() {
    assert_eq!(
        object_format_of(&elf_bytes(
            crate::common::repack::EM_X86_64,
            crate::common::native::ET_DYN,
            None
        )),
        Some(ObjectFormat::Elf),
    );
    assert_eq!(
        object_format_of(&pe_bytes(PE_MACHINE_AMD64, &[0u8; 128])),
        Some(ObjectFormat::Pe),
        "`MZ` is a PE, and answering `None` for it is what let a Windows artifact verify as \
         one holding no objects at all"
    );
    assert_eq!(
        object_format_of(&pe_bytes(PE_MACHINE_ARM64, &[0u8; 128])),
        Some(ObjectFormat::Pe),
    );
    assert_eq!(
        object_format_of(&macho_bytes(MACHO_CPU_ARM64, MACHO_TYPE_DYLIB)),
        Some(ObjectFormat::MachO),
    );
    assert_eq!(
        object_format_of(&macho_bytes(MACHO_CPU_X86_64, MACHO_TYPE_EXECUTE)),
        Some(ObjectFormat::MachO),
    );
}

#[test]
fn a_file_that_is_not_an_object_is_still_not_one() {
    // The rule may not widen into "anything might be an object": a `.so` that
    // is really a shell wrapper is data, and so is an empty file.
    for bytes in [
        crate::common::native::SHELL_WRAPPER,
        b"".as_slice(),
        b"FOR1\x00\x00\x00\x28BEAM".as_slice(),
        b"M".as_slice(),
    ] {
        assert_eq!(
            object_format_of(bytes),
            None,
            "{:?} names no container format",
            String::from_utf8_lossy(&bytes[..bytes.len().min(8)])
        );
    }
}

#[test]
fn a_dos_program_is_read_as_a_pe_by_its_magic_and_refused_later() {
    // The magic is the whole of this rule's job. A file that begins `MZ` and
    // carries no `PE\0\0` is a *broken* PE, which a caller reports; deciding
    // that here would be the second reader this milestone is removing.
    assert_eq!(object_format_of(&dos_stub()), Some(ObjectFormat::Pe));
}

#[test]
fn the_rule_reads_only_the_head_it_is_given() {
    // `verify::read_entry` hands it the first bytes of a payload entry rather
    // than the whole file, so the answer may not depend on anything further
    // in.
    let whole = pe_bytes(PE_MACHINE_AMD64, &[0u8; 128]);
    assert_eq!(object_format_of(&whole[..8]), Some(ObjectFormat::Pe));

    let elf = elf_bytes(
        crate::common::repack::EM_X86_64,
        crate::common::native::ET_DYN,
        None,
    );
    assert_eq!(object_format_of(&elf[..4]), Some(ObjectFormat::Elf));
}
