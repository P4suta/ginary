// SPDX-License-Identifier: MIT OR Apache-2.0
//! No Windows artifact could be built, because nothing could read a Windows
//! runtime tree.
//!
//! `bundle::check_windows_erts` accepts a `dir:` source, the README says a
//! tree unpacked from `otp_win64_<version>.zip` is the one accepted source,
//! and `assemble::stage` has a whole Windows arm. None of it was reachable:
//! `otp::inspect_root` demanded the four *unix* programs of every tree it was
//! given, and `erts_source::resolve` then read `erts-<vsn>/bin/beam.smp` as an
//! ELF. A real Windows tree holds `erl.exe`, `beam.smp.dll`, `erlexec.dll` and
//! `inet_gethost.exe` and none of the four, so a Windows build stopped at
//! `MissingErtsBinary` long before the staging arm the tests exercised by hand.
//!
//! The seam is `erts_source::resolve` over a Windows root: the point where the
//! build stops trusting a spelling and reads the tree. This test drives that,
//! not `assemble::stage`.
#![cfg(feature = "cli")]

use ginary::erts_source::{self, ErtsError, ErtsSourceSpec};
use ginary::target::{Arch, Libc, Linkage, Os, Target};

use crate::common::fake_otp::{DEFAULT_ERTS_VSN, FakeOtp, FakeOtpRoot, PE_MACHINE_ARM64};

/// The target a Windows runtime is for.
fn windows() -> Target {
    Target::new(Os::Windows, Arch::X86_64, Libc::None)
}

/// The target the same build machine would otherwise be asked for.
fn linux() -> Target {
    Target::new(Os::Linux, Arch::X86_64, Libc::Gnu)
}

/// A Windows runtime root, written into a fresh temporary directory.
fn windows_root() -> (tempfile::TempDir, FakeOtpRoot) {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let root = FakeOtp::new().windows().build_in(dir.path().join("otp"));
    (dir, root)
}

#[test]
fn a_windows_runtime_tree_is_an_installation_ginary_can_read() {
    let (_dir, root) = windows_root();
    match ginary::otp::inspect_root(&root.root) {
        Ok(otp) => {
            assert_eq!(otp.erts_vsn, DEFAULT_ERTS_VSN);
            assert_eq!(otp.erts_bin, root.erts_bin());
        }
        Err(error) => panic!(
            "a tree unpacked from otp_win64_<version>.zip is the one source a Windows build \
             may take its runtime from, and inspecting it said: {error}"
        ),
    }
}

#[test]
fn a_windows_runtime_root_resolves_to_the_windows_target() {
    let (_dir, root) = windows_root();
    let spec = ErtsSourceSpec::Dir(root.root.clone());

    match erts_source::resolve(&spec, &windows()) {
        Ok(resolved) => {
            assert_eq!(
                resolved.target,
                windows(),
                "the machine comes off the runtime's own object file, not off the request"
            );
            assert_eq!(
                resolved.linkage,
                Linkage::Dynamic,
                "a Windows runtime is a set of DLLs the emulator loads; there is no static one"
            );
            assert!(
                resolved.nif_loading,
                "and a runtime that loads DLLs loads NIFs"
            );
            assert_eq!(resolved.libc_min, None, "Windows has no glibc floor");
            assert_eq!(resolved.provenance, spec.label());
        }
        Err(error) => panic!(
            "a Windows runtime root has to resolve, or no Windows artifact can be built \
             at all; it said: {error}"
        ),
    }
}

#[test]
fn a_windows_runtime_asked_for_by_a_linux_build_is_the_mismatch_it_is() {
    let (_dir, root) = windows_root();
    match erts_source::resolve(&ErtsSourceSpec::Dir(root.root.clone()), &linux()) {
        Err(ErtsError::TargetMismatch {
            requested, actual, ..
        }) => {
            assert_eq!(requested, linux());
            assert_eq!(actual, windows());
        }
        other => panic!(
            "the flavour is read off the tree, so a Windows tree in a Linux build is a \
             target mismatch and not a missing `beam.smp`; this answered {other:?}"
        ),
    }
}

#[test]
fn a_windows_tree_missing_its_launch_program_is_refused_as_a_unix_tree_would_be() {
    let (_dir, root) = windows_root();
    // The launch program is what says "this is a Windows tree", so removing it
    // leaves a directory that is neither flavour, and the unix list is what
    // such a tree is measured against.
    std::fs::remove_file(root.erts_bin().join("erl.exe")).expect("remove erl.exe");

    match ginary::otp::inspect_root(&root.root) {
        Err(ginary::otp::OtpError::MissingErtsBinary { path }) => {
            assert_eq!(
                path.file_name().and_then(|name| name.to_str()),
                Some("beam.smp"),
                "a tree with no `erl.exe` is read as the unix tree it is not"
            );
        }
        other => panic!("a tree of neither flavour is not an installation: {other:?}"),
    }
}

#[test]
fn a_windows_tree_missing_the_emulator_dll_is_named_by_the_file_it_lacks() {
    let (_dir, root) = windows_root();
    std::fs::remove_file(root.erts_bin().join("beam.smp.dll")).expect("remove beam.smp.dll");

    match ginary::otp::inspect_root(&root.root) {
        Err(ginary::otp::OtpError::MissingErtsBinary { path }) => {
            assert_eq!(
                path,
                root.erts_bin().join("beam.smp.dll"),
                "the refusal names the file that is not there"
            );
        }
        other => panic!("a Windows tree without its emulator is not an installation: {other:?}"),
    }
}

#[test]
fn a_windows_runtime_for_another_machine_is_read_off_its_own_header() {
    // The trust anchor, on this arm too: the spelling said `dir:`, the tree is
    // a Windows one, and the architecture came out of the emulator's PE header
    // rather than out of either.
    let dir = tempfile::tempdir().expect("a temporary directory");
    let root = FakeOtp::new()
        .windows()
        .pe_machine(PE_MACHINE_ARM64)
        .build_in(dir.path().join("otp"));

    match erts_source::resolve(&ErtsSourceSpec::Dir(root.root.clone()), &windows()) {
        Err(ErtsError::TargetMismatch {
            requested, actual, ..
        }) => {
            assert_eq!(requested, windows());
            assert_eq!(actual, Target::new(Os::Windows, Arch::Aarch64, Libc::None));
        }
        other => panic!(
            "an aarch64 runtime in an x86-64 build is a mismatch a user must be told about \
             at build time; this answered {other:?}"
        ),
    }
}

#[test]
fn a_windows_tree_whose_emulator_is_not_a_pe_is_refused_by_name() {
    let (_dir, root) = windows_root();
    std::fs::write(root.erts_bin().join("beam.smp.dll"), b"#!/bin/sh\nexit 0\n")
        .expect("replace the emulator");

    match erts_source::resolve(&ErtsSourceSpec::Dir(root.root.clone()), &windows()) {
        Err(ErtsError::NotAPeRuntime { path, .. }) => {
            assert_eq!(
                path,
                root.erts_bin().join("beam.smp.dll"),
                "a stand-in where the emulator belongs is refused with the file that was read"
            );
        }
        other => panic!(
            "a runtime for another target has to be a real cross-built tree; this answered \
             {other:?}"
        ),
    }
}
