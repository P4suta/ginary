// SPDX-License-Identifier: MIT OR Apache-2.0
//! Both macOS jobs died resolving the runtime, because the only unix emulator
//! reader ginary has is the ELF one.
//!
//! **What went wrong.** E6 removed the phantom `--erts` flag, so for the first
//! time both macOS jobs got past argument parsing and into `ginary build`.
//! They then stopped at the trust anchor, on both architectures, identically:
//!
//! ```text
//! error: cannot resolve the runtime to bundle
//!   caused by: the emulator at /Users/runner/work/_temp/.setup-beam/otp/
//!   erts-17.0.5/bin/beam.smp is not an ELF binary (not an ELF file); a
//!   runtime for another target has to be a real cross-built tree, not a
//!   stand-in
//! ```
//!
//! (`macOS (macos-14, macos-aarch64)`
//! <https://github.com/P4suta/ginary/actions/runs/33702776627/job/100485421745>
//! and `macOS (macos-15-intel, macos-x86_64)`
//! <https://github.com/P4suta/ginary/actions/runs/33702776627/job/100485421566>.)
//!
//! [`ginary::erts_source::resolve_with`] chooses its reader from the *layout*
//! of `erts-<vsn>/bin`: a Windows tree spells its emulator `beam.smp.dll` and
//! keeps an `erl.ini` beside it, so it gets the PE reader, and everything else
//! gets the ELF reader. That is a complete dichotomy on the two platforms D2
//! and C2 built for, and it is wrong the moment a third exists: a macOS tree
//! has the unix layout exactly, and the only difference a build can observe is
//! that its `beam.smp` is a Mach-O. D3 could not see this — there was no Mac,
//! and `macho.rs` was written and proved against fixtures rather than against
//! a toolchain-produced runtime.
//!
//! **The input.** `[tools.ginary.target."macos-*"] erts = "dir:<an OTP
//! installation>"` on any macOS host, which is what the CI job appends and
//! what any user packaging on their own Mac would write. `host` is the same
//! path.
//!
//! **The correct behaviour.** The reader is chosen by the emulator's own
//! magic, which is the trust anchor the rest of the module already uses:
//! nothing the configuration spelled is believed about a runtime.
//! [`ginary::erts_source::EmulatorFormat`] is that classification, and it is
//! pure, so the dispatch is pinned from Linux even though the resolution it
//! guards can only be exercised with a real Mach-O — which the suite has, both
//! hand-built and committed.
//!
//! A macOS runtime resolves like a Windows one and for the same reasons: there
//! is one system C library, so the target's libc is [`Libc::None`] and there is
//! no version floor to record; the emulator is dynamically linked and NIFs
//! load. The `cputype` is the whole of the target, exactly as
//! [`ginary::macho::MachoFacts::target`] already documents.
#![cfg(feature = "cli")]

use ginary::erts_source::{self, EmulatorFormat, ErtsError, ErtsSourceSpec, emulator_format};
use ginary::target::{Arch, Libc, Os, Target};

use crate::common::fake_otp::FakeOtp;
use crate::common::macho::{CPU_TYPE_ARM64, CPU_TYPE_X86_64, MH_EXECUTE, fat_header, thin_header};

/// The two macOS targets ginary packages for.
fn macos(arch: Arch) -> Target {
    Target::new(Os::Macos, arch, Libc::None)
}

/// A temporary directory the test owns.
fn tempdir() -> tempfile::TempDir {
    tempfile::tempdir().expect("a temporary directory")
}

#[test]
fn an_emulators_own_magic_decides_which_reader_reads_it() {
    assert_eq!(
        emulator_format(&ginary::elf::ELF_MAGIC),
        Some(EmulatorFormat::Elf),
        "a Linux runtime is read by the ELF reader"
    );
    assert_eq!(
        emulator_format(&thin_header(CPU_TYPE_ARM64, MH_EXECUTE)),
        Some(EmulatorFormat::MachO),
        "a thin 64-bit Mach-O is a macOS runtime"
    );
    assert_eq!(
        emulator_format(&thin_header(CPU_TYPE_X86_64, MH_EXECUTE)),
        Some(EmulatorFormat::MachO),
        "on either architecture: the magic is the same and the cputype is the target"
    );
    assert_eq!(
        emulator_format(&fat_header(&[(CPU_TYPE_ARM64, 0), (CPU_TYPE_X86_64, 0)])),
        Some(EmulatorFormat::MachO),
        "a universal binary is still a Mach-O; which architecture it holds is the reader's \
         question and not this one's"
    );
    assert_eq!(
        emulator_format(b"MZ\x90\x00"),
        Some(EmulatorFormat::Pe),
        "a Windows runtime is read straight from `object`"
    );
    assert_eq!(
        emulator_format(b"#!/bin/sh\n"),
        None,
        "a shell script where the emulator belongs is a runtime nobody can read, reported \
         rather than guessed at"
    );
    assert_eq!(
        emulator_format(b"\x7fE"),
        None,
        "and so is a file too short to carry any magic at all"
    );
}

#[test]
fn a_macos_runtime_resolves_to_the_macos_target() {
    let dir = tempdir();
    let root = dir.path().join("otp");
    FakeOtp::new()
        .macos()
        .macho_cpu_type(CPU_TYPE_ARM64)
        .build_in(&root);

    let resolved = erts_source::resolve(&ErtsSourceSpec::Dir(root.clone()), &macos(Arch::Aarch64))
        .expect("a macOS runtime root resolves");

    assert_eq!(
        resolved.target,
        macos(Arch::Aarch64),
        "the cputype is the whole of the target"
    );
    assert_eq!(
        resolved.linkage,
        ginary::target::Linkage::Dynamic,
        "macOS has one system C library and the emulator resolves it at load time"
    );
    assert!(
        resolved.nif_loading,
        "a dynamically linked runtime loads NIFs"
    );
    assert_eq!(
        resolved.libc_min, None,
        "there is no glibc symbol-version floor to record on macOS"
    );
    assert_eq!(resolved.otp.root, root);
    assert_eq!(
        resolved.provenance,
        format!("dir:{}", root.display()),
        "the provenance is the spelling, and the spelling names the directory"
    );
    assert!(
        resolved.warnings.is_empty(),
        "`dir:` makes no claim a guard could disagree with: {:?}",
        resolved.warnings
    );
}

#[test]
fn a_macos_runtime_for_the_other_architecture_is_a_target_mismatch() {
    let dir = tempdir();
    let root = dir.path().join("otp");
    FakeOtp::new()
        .macos()
        .macho_cpu_type(CPU_TYPE_X86_64)
        .build_in(&root);

    let error = erts_source::resolve(&ErtsSourceSpec::Dir(root.clone()), &macos(Arch::Aarch64))
        .expect_err("an x86-64 runtime is not an aarch64 one");

    assert!(
        matches!(
            &error,
            ErtsError::TargetMismatch { path, requested, actual }
                if *path == root
                    && *requested == macos(Arch::Aarch64)
                    && *actual == macos(Arch::X86_64)
        ),
        "the machine comes off the emulator's own header rather than off the spelling that named \
         the tree, so the wrong architecture is caught here and not by a loader on somebody \
         else's machine: {error:?}"
    );
}
