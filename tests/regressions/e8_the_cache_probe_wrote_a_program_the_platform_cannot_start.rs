// SPDX-License-Identifier: MIT OR Apache-2.0
//! `ginary doctor` reported every healthy Windows cache as one no program can
//! be run from, because the program it wrote there was a `/bin/sh` script.
//!
//! **What went wrong.** The Windows runner's `doctor` row read:
//!
//! ```text
//! CacheProbe { writable: true, executable: false,
//!              detail: Some("%1 is not a valid Win32 application.") }
//! ```
//!
//! (`Windows build and exit-code propagation`
//! <https://github.com/P4suta/ginary/actions/runs/33712111530/job/100513644404>.)
//!
//! Nothing was wrong with the directory. `doctor::probe_cache_dir` answers
//! "can a program be run out of here?" the only honest way there is — it
//! writes one and starts it — and the program it wrote was
//! `#!/bin/sh\nexit 0\n` under an extensionless name, on every platform.
//! Windows decides what it will start from the file's extension: nothing there
//! reads a `#!` line, `CreateProcessW` refuses the file with
//! `ERROR_BAD_EXE_FORMAT`, and `doctor` reported the refusal as a property of
//! the cache. Every Windows user would have been sent to
//! `GINARY_CACHE_DIR` for a directory that was never at fault.
//!
//! This is the one member of the Windows log's "shell-script fixtures" class
//! that is a *product* defect rather than a test assumption, and it is the
//! reason the class was re-audited in Fix round 2 rather than left recorded as
//! open.
//!
//! **The correct behaviour.** Which bytes are a program, and what a program's
//! file has to be called, are properties of the platform. They are asked of a
//! named [`Os`] — [`ginary::platform::probe_program`] and
//! [`ginary::platform::probe_suffix`] — so both answers are asserted on the
//! machine ginary is developed on rather than only on a runner. The unix
//! answer is unchanged, byte for byte.

use ginary::doctor;
use ginary::platform::{probe_program, probe_suffix};
use ginary::target::Os;

#[test]
fn the_probe_writes_a_program_each_platform_will_actually_start() {
    assert_eq!(
        [
            probe_program(Os::Linux),
            probe_program(Os::Macos),
            probe_program(Os::Windows),
        ],
        [
            b"#!/bin/sh\nexit 0\n".as_slice(),
            b"#!/bin/sh\nexit 0\n".as_slice(),
            b"@exit /b 0\r\n".as_slice(),
        ],
        "a `#!` line is a program where a kernel reads one, and data everywhere \
         else; Windows starts a batch file",
    );
}

#[test]
fn the_probe_file_carries_the_suffix_its_platform_needs_to_start_it() {
    assert_eq!(
        [
            probe_suffix(Os::Linux),
            probe_suffix(Os::Macos),
            probe_suffix(Os::Windows),
        ],
        ["", "", ".cmd"],
        "on Windows the extension is the whole of the decision, so the probe \
         file cannot be the extensionless dot-file unix wants",
    );
}

#[test]
fn a_writable_directory_on_this_host_probes_as_one_a_program_runs_from() {
    // The invariant the two rules above exist to keep true on every platform,
    // asserted here on the one this suite runs on: a fresh, ordinary directory
    // is writable and a program can be started from it.
    let dir = tempfile::tempdir().expect("a temporary directory");

    let probe = doctor::probe_cache_dir(&dir.path().join("cache"));

    assert_eq!(
        (probe.writable, probe.executable),
        (true, true),
        "a directory nothing is wrong with: {probe:?}",
    );
}
