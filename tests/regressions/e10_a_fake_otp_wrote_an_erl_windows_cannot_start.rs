// SPDX-License-Identifier: MIT OR Apache-2.0
//! The fake OTP root a test builds wrote its `erl` as a `/bin/sh` script, and
//! Windows refused to start it, so every `stage`, `strip` and `stage_run`
//! target that reaches the beam step failed.
//!
//! **What went wrong.** Thirty-nine targets across `cli`, `strip` and
//! `stage_run` failed with one error:
//!
//! ```text
//! cannot exec C:\Users\RUNNER~1\AppData\Local\Temp\.tmpnCZUIT\otp\bin\erl:
//! %1 is not a valid Win32 application. (os error 193)
//! ```
//!
//! (`Windows build and exit-code propagation`
//! <https://github.com/P4suta/ginary/actions/runs/33739517757/job/100597889388>.)
//!
//! **The input.** Any test that plants a stub `erl` under a fake OTP root.
//! `src/strip.rs` looks for `bin/<platform::erl_program>` — `erl.exe` on
//! Windows, which E8 already got right — and the fixture wrote a shebang file
//! called `erl`. Two things are wrong with it there: the name, and the form.
//! `CreateProcess` starts a program by reading its image header, so a shebang
//! is not a program however it is named, and a `.cmd` is not one either.
//!
//! **The correct behaviour.** The fixture asks the platform what a program
//! looks like, exactly as `src/` already does. On unix that is a shell script
//! called `erl`; on Windows it is a compiled image called `erl.exe`. Nothing
//! in `src/` moves: the production rule is right and the fixture was not
//! keeping it.

use crate::common::script::{ShimForm, shim_file_name, shim_form};
use ginary::platform::erl_program;
use ginary::target::Os;

#[test]
fn a_planted_program_takes_the_form_the_platform_can_start() {
    assert_eq!(
        [
            shim_form(Os::Linux),
            shim_form(Os::Macos),
            shim_form(Os::Windows),
        ],
        [
            ShimForm::ShellScript,
            ShimForm::ShellScript,
            ShimForm::CompiledProgram,
        ],
        "a shebang is a program on unix and a data file on Windows"
    );
}

#[test]
fn a_planted_program_is_named_the_way_the_platform_names_one() {
    assert_eq!(
        [
            shim_file_name("erl", Os::Linux),
            shim_file_name("erl", Os::Macos),
            shim_file_name("erl", Os::Windows),
        ],
        ["erl", "erl", "erl.exe"],
    );
    for os in [Os::Linux, Os::Macos, Os::Windows] {
        assert_eq!(
            shim_file_name("erl", os),
            erl_program(os),
            "the fixture writes the file `src/strip.rs` goes looking for"
        );
    }
    assert_eq!(shim_file_name("strip", Os::Windows), "strip.exe");
}
