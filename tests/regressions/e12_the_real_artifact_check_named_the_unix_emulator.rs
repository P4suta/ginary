// SPDX-License-Identifier: MIT OR Apache-2.0
//! The check that a real artifact carries a real runtime looked for
//! `beam.smp`, which a Windows runtime does not ship.
//!
//! **What went wrong.** E11 gave `ginary::target::Target` an
//! `emulator_program` method precisely because the emulator is a program
//! `erlexec` execs on unix and a library `erl.exe` loads on Windows, and
//! `e11_the_emulator_was_looked_for_under_its_unix_name` pins the two answers.
//! One call site was not moved with it — the last assertion of
//! `a_real_artifact_verifies_clean`, which reads a real host runtime out of a
//! real artifact and then asks for the unix name:
//!
//! ```rust,ignore
//! .any(|object| object.path.ends_with("/beam.smp"))
//! ```
//!
//! ```text
//! ---- a_real_artifact_verifies_clean ----
//! panicked at tests\verify.rs:820:5:
//! [ObjectInfo { path: "erts-17.0.5/bin/beam.debug.smp.dll", … },
//!  ObjectInfo { path: "erts-17.0.5/bin/beam.smp.dll", … },
//!  ObjectInfo { path: "erts-17.0.5/bin/erl.exe", … },
//!  ObjectInfo { path: "erts-17.0.5/bin/erlexec.dll", … },
//!  ObjectInfo { path: "erts-17.0.5/bin/inet_gethost.exe", … }]
//! ```
//!
//! (`Windows build and exit-code propagation`
//! <https://github.com/P4suta/ginary/actions/runs/33823103540/job/100869848230>.)
//! The report is right: the emulator is there, under the name that platform
//! gives it, and the object list the failure prints is the evidence.
//!
//! **The input.** Any artifact built for a platform whose emulator is not
//! `beam.smp`.
//!
//! **The correct behaviour.** A test reading a real runtime names the file
//! that runtime ships, by asking the rule rather than by writing one
//! platform's answer down. Both halves are asserted here: the rule itself, and
//! that the call site consults it.

#![cfg(feature = "cli")]

use ginary::target::{Os, Target};

use crate::common::hostpath::emulator_suffix;
use crate::common::repo::read;
use crate::common::srcscan::literal_sites;

#[test]
fn the_suffix_a_report_is_searched_for_is_the_targets_own_emulator() {
    assert_eq!(emulator_suffix(Os::Linux), "/beam.smp");
    assert_eq!(emulator_suffix(Os::Macos), "/beam.smp");
    assert_eq!(
        emulator_suffix(Os::Windows),
        "/beam.smp.dll",
        "the Windows emulator is the library `erl.exe` loads, and a report path that ends \
         `/beam.smp` is not it"
    );
    assert_eq!(
        emulator_suffix(ginary::platform::HOST),
        format!("/{}", Target::host().emulator_program()),
        "and the helper is the `Target` rule rather than a second copy of it"
    );
}

#[test]
fn the_scanner_reads_code_and_not_the_prose_that_describes_it() {
    let planted = "\
// the emulator, `erts-<vsn>/bin/beam.smp` on unix\n\
    .any(|object| object.path.ends_with(\"/beam.smp\"))\n\
    .any(|object| object.path.ends_with(&emulator_suffix(HOST)))\n";

    assert_eq!(
        literal_sites(planted, r#""/beam.smp""#),
        vec![2],
        "a comment may name the unix emulator; an assertion about a real host runtime may not"
    );
}

#[test]
fn the_real_artifact_check_does_not_write_one_platforms_emulator_down() {
    assert_eq!(
        literal_sites(&read("tests/verify.rs"), r#""/beam.smp""#),
        Vec::<usize>::new(),
        "`a_real_artifact_verifies_clean` reads whatever runtime this host installed, so the \
         emulator it looks for is `Target::emulator_program` and not the name one platform \
         happens to use"
    );
}
