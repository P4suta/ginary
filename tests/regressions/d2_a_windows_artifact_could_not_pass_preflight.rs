// SPDX-License-Identifier: MIT OR Apache-2.0
//! `launch::preflight` demanded the three unix ERTS port programs on every
//! platform, so no Windows artifact could ever start.
//!
//! `REQUIRED_BINARIES` is `beam.smp`, `erl_child_setup` and `inet_gethost`,
//! and `preflight` walked it whatever the manifest's target said. A Windows
//! artifact's bindir holds what `assemble::windows_required_bins` stages —
//! `erl.exe`, the DLLs beside it and the resolver — and holds none of those
//! three names, so `check_program(bindir/beam.smp)` answered
//! [`PreflightIssue::Missing`], `launcher::repair_once` threw the entry away,
//! extracted the whole payload a second time, failed again and exited 124.
//! The Windows arm of `check_program` was split and the list it is called with
//! was not, which is what made the gap invisible.
//!
//! The right behaviour: which names must be under the bindir is a property of
//! the artifact's target, the same way `assemble::stage_erts_bins` reads the
//! flavour off the tree. `preflight` is a pure function over a directory, so
//! the whole rule is checkable on Linux.
//!
//! [`PreflightIssue::Missing`]: ginary::launch::PreflightIssue::Missing

use std::path::{Path, PathBuf};

use ginary::launch::{self, PreflightIssue};
use ginary::manifest::Manifest;
use ginary::target::{Arch, Libc, Os, Target};

use crate::common::artifact::canonical_manifest;
use crate::common::stubfile::write_executable;

/// The manifest a Windows build writes: the unix one with the target and the
/// launch program a Windows target implies, and nothing else changed.
fn windows_manifest() -> Manifest {
    let mut m = canonical_manifest();
    m.target = Target::new(Os::Windows, Arch::X86_64, Libc::None);
    m.launch.program = m.target.launch_program().to_owned();
    m
}

/// Writes an extracted root holding exactly `bins` under the manifest's
/// bindir, plus the boot file every artifact carries.
fn root_with(dir: &Path, m: &Manifest, bins: &[&str]) -> PathBuf {
    let root = dir.join("root");
    let bindir = root.join(&m.launch.bindir);
    for name in bins {
        write_executable(&bindir, name, b"a fake runtime file\n");
    }
    let boot = root.join(format!("{}.boot", m.launch.boot));
    if let Some(parent) = boot.parent() {
        std::fs::create_dir_all(parent).expect("the boot directory");
    }
    std::fs::write(&boot, b"boot").expect("the boot file");
    root
}

#[test]
fn a_windows_bindir_holds_what_a_windows_runtime_needs_and_passes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let m = windows_manifest();
    // Exactly what `assemble::windows_required_bins` stages for the fixture
    // tree: the launch program, the emulator it loads, the resolver, and the
    // other DLLs that came with them.
    let root = root_with(
        dir.path(),
        &m,
        &["erl.exe", "beam.smp.dll", "inet_gethost.exe", "ei.dll"],
    );

    assert_eq!(
        launch::preflight(&root, &m),
        Ok(()),
        "a Windows artifact has no `beam.smp`, no `erl_child_setup` and no \
         `erlexec`; demanding them means no Windows artifact can ever start"
    );
}

#[test]
fn a_windows_artifact_without_the_emulator_dll_is_still_refused_by_name() {
    let dir = tempfile::tempdir().expect("tempdir");
    let m = windows_manifest();
    let root = root_with(dir.path(), &m, &["erl.exe", "inet_gethost.exe"]);

    assert_eq!(
        launch::preflight(&root, &m),
        Err(PreflightIssue::Missing {
            path: root.join(&m.launch.bindir).join("beam.smp.dll"),
        }),
        "`erl.exe` loads its emulator as a DLL, so a tree without one starts \
         nothing and preflight is where that is found out"
    );
}

#[test]
fn a_windows_artifact_without_its_resolver_is_refused_by_name() {
    let dir = tempfile::tempdir().expect("tempdir");
    let m = windows_manifest();
    let root = root_with(dir.path(), &m, &["erl.exe", "beam.smp.dll"]);

    assert_eq!(
        launch::preflight(&root, &m),
        Err(PreflightIssue::Missing {
            path: root.join(&m.launch.bindir).join("inet_gethost.exe"),
        }),
        "`inet_gethost` is required on unix for the reason it is required \
         here: without it the runtime resolves no host name"
    );
}

#[test]
fn a_unix_artifact_is_held_to_the_unix_list_it_always_was() {
    let dir = tempfile::tempdir().expect("tempdir");
    let m = canonical_manifest();
    // The Windows names, on a unix manifest: every one of them is beside the
    // point, and the first unix name that is missing is what is reported.
    let root = root_with(
        dir.path(),
        &m,
        &["erlexec", "erl.exe", "beam.smp.dll", "inet_gethost.exe"],
    );

    assert_eq!(
        launch::preflight(&root, &m),
        Err(PreflightIssue::Missing {
            path: root.join(&m.launch.bindir).join("beam.smp"),
        }),
        "the fix must not have loosened the unix rule: a unix artifact still \
         needs the emulator it execs"
    );
}
