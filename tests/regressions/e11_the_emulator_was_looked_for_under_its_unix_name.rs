// SPDX-License-Identifier: MIT OR Apache-2.0
//! A test measured the staged emulator by opening `beam.smp`, which is not the
//! file a Windows runtime carries.
//!
//! **What went wrong.** On unix the emulator is a program `erlexec` execs,
//! `erts-<vsn>/bin/beam.smp`. On Windows it is a *library* `erl.exe` loads into
//! its own process, `erts-<vsn>/bin/beam.smp.dll` — `ginary::target` already
//! names both, as `LAUNCH_PROGRAM`/`WINDOWS_LAUNCH_PROGRAM` and
//! `WINDOWS_EMULATOR_DLL`, and `ginary::launch::preflight` already looks for
//! the right one. The size-budget test opened the unix name on every host:
//!
//! ```text
//! ---- a_stripped_hello_ffi_fits_in_the_size_budget ----
//! the staged beam.smp: Os { code: 2, kind: NotFound,
//!   message: "The system cannot find the file specified." }
//! ```
//!
//! (`Windows build and exit-code propagation`
//! <https://github.com/P4suta/ginary/actions/runs/33751715516/job/100636537290>,
//! `tests/stage_run.rs:335`.)
//!
//! The same file also runs a staged root by starting `erts-<vsn>/bin/erlexec`,
//! which a Windows tree does not carry either, and six tests failed on it:
//!
//! ```text
//! cannot run the staged `hello_ffi` under
//!   C:\Users\RUNNER~1\AppData\Local\Temp\.tmpmVnjuM\staged\erts-17.0.5\bin:
//!   The system cannot find the file specified. (os error 2)
//! ```
//!
//! **The input.** Any Windows tree.
//!
//! **The correct behaviour.** Both names are properties of the target, so both
//! are `ginary::target::Target` methods: `launch_program` already existed and
//! `emulator_program` joins it. A test names the file the tree it is looking
//! at actually ships.

use ginary::target::{
    Arch, LAUNCH_PROGRAM, Libc, Os, Target, WINDOWS_EMULATOR_DLL, WINDOWS_LAUNCH_PROGRAM,
};

/// The three targets whose runtimes this suite stages.
fn targets() -> [Target; 3] {
    [
        Target::new(Os::Linux, Arch::X86_64, Libc::Gnu),
        Target::new(Os::Macos, Arch::Aarch64, Libc::None),
        Target::new(Os::Windows, Arch::X86_64, Libc::None),
    ]
}

#[test]
fn every_target_names_the_emulator_its_runtime_carries() {
    let names: Vec<&str> = targets()
        .iter()
        .map(|target| target.emulator_program())
        .collect();
    assert_eq!(
        names,
        ["beam.smp", "beam.smp", WINDOWS_EMULATOR_DLL],
        "the unix emulator is a program and the Windows one is a DLL `erl.exe` loads"
    );
}

#[test]
fn the_emulator_and_the_launch_program_are_two_files_on_every_target() {
    for target in targets() {
        assert_ne!(
            target.emulator_program(),
            target.launch_program(),
            "{target}: the program the launcher starts is not the emulator it reaches"
        );
    }
    assert_eq!(
        targets()
            .iter()
            .map(|target| target.launch_program())
            .collect::<Vec<_>>(),
        [LAUNCH_PROGRAM, LAUNCH_PROGRAM, WINDOWS_LAUNCH_PROGRAM],
    );
}

#[test]
fn the_windows_emulator_is_the_name_the_preflight_already_requires() {
    // `launch::preflight` refuses a Windows entry that has no `beam.smp.dll`,
    // so a test measuring the emulator has to be looking at that same file.
    assert_eq!(
        Target::new(Os::Windows, Arch::X86_64, Libc::None).emulator_program(),
        WINDOWS_EMULATOR_DLL
    );
}
