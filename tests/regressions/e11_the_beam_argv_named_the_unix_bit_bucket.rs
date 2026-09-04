// SPDX-License-Identifier: MIT OR Apache-2.0
//! Two expectations spelled the crash-dump sink `/dev/null`, which is not a
//! file on the host the beam step was running on.
//!
//! **What went wrong.** `src/strip.rs` starts the OTP installation's own `erl`
//! with `-env ERL_CRASH_DUMP <sink>` so that a runtime dying inside
//! `beam_lib:strip_files/1` does not drop an `erl_crash.dump` into whatever
//! directory the developer was standing in. The production side already sends
//! the platform's own name for the sink; both tests that assert on the
//! argument vector had the unix one written into them:
//!
//! ```text
//! ---- the_beam_step_runs_the_otp_roots_own_erl_with_the_beam_lib_one_liner ----
//! assertion `left == right` failed: the runtime is run by absolute path, every
//! module arrives after -extra as a path of its own, and the crash dump goes to
//! the bit bucket
//!   left: ["-noshell", "-env", "ERL_CRASH_DUMP", "nul",       "-eval", ...]
//!  right: ["-noshell", "-env", "ERL_CRASH_DUMP", "/dev/null", "-eval", ...]
//! ```
//!
//! (`Windows build and exit-code propagation`
//! <https://github.com/P4suta/ginary/actions/runs/33751715516/job/100636537290>;
//! `tests/strip.rs:227` and the same four arguments in `tests/cli.rs:1113`.)
//!
//! **The input.** Any host that is not unix. `nul` on Windows is a reserved
//! *device name* rather than a path: it resolves in every directory, it is the
//! only spelling there is — a `\\?\` verbatim path cannot name it — and
//! `/dev/null` there is an ordinary relative path naming a directory that does
//! not exist.
//!
//! **The correct behaviour.** Which name the sink has is a fact about an
//! operating system, so it is `ginary::platform::null_device` and both the
//! production constant and the two expectations compose it. Neither test may
//! carry a literal.

use ginary::platform::null_device;
use ginary::target::Os;

#[test]
fn the_bit_bucket_is_named_the_way_the_platform_names_it() {
    assert_eq!(
        [
            null_device(Os::Linux),
            null_device(Os::Macos),
            null_device(Os::Windows),
        ],
        ["/dev/null", "/dev/null", "nul"],
        "`nul` is a reserved device name on Windows and `/dev/null` is a path that is not there"
    );
}

#[test]
fn the_sink_the_beam_step_passes_is_the_one_this_platform_has() {
    // The other half: the production constant and the rule may not drift
    // apart, or the two expectations would compose a name nothing sends.
    assert_eq!(
        ginary::process::null_device_here(),
        null_device(ginary::platform::HOST),
        "`src/process.rs` sends what `platform::null_device` names"
    );
}
