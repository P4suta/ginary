// SPDX-License-Identifier: MIT OR Apache-2.0
//! The beam-stripping step skipped every module on Windows, because it looked
//! for a program under the OTP root by its unix name.
//!
//! **What went wrong.** `ginary stage` reported the whole beam half as
//! skipped, and said why:
//!
//! ```text
//! beams: skipped: the OTP installation has no `d:/a/_temp/.setup-beam/otp\bin\erl`,
//! and `beam_lib:strip_files/1` can only be run by the runtime the modules came
//! from; 1 module kept their debug information
//! ```
//!
//! The installation is there and so is the program: it is spelled `erl.exe`.
//!
//! (`Windows build and exit-code propagation`
//! <https://github.com/P4suta/ginary/actions/runs/33712111530/job/100513644404>.)
//!
//! **The input.** Any `ginary stage` or `ginary build` on Windows.
//! `src/strip.rs` joined the constant `"erl"` onto `<otp root>/bin`, and the
//! skip it produced was honest about the path it looked at and wrong about
//! what to look for.
//!
//! **The correct behaviour.** The name is a property of the machine the OTP
//! installation is on, so it is asked of that machine.
//! [`ginary::platform::erl_program`] answers it for any platform, which is
//! what makes it checkable here rather than only on a Windows runner. It is
//! deliberately not [`ginary::target::Target::launch_program`]: that names
//! what a packaged *artifact* execs, which is `erlexec` on unix and a
//! different program entirely.

use ginary::platform::erl_program;
use ginary::target::Os;

#[test]
fn the_otp_launcher_is_named_the_way_the_host_platform_spells_it() {
    assert_eq!(
        [
            erl_program(Os::Linux),
            erl_program(Os::Macos),
            erl_program(Os::Windows),
        ],
        ["erl", "erl", "erl.exe"],
        "the program under `<otp root>/bin` that `beam_lib:strip_files/1` is reached through"
    );
}
