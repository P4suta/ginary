// SPDX-License-Identifier: MIT OR Apache-2.0
//! A test read `doctor`'s output and asked whether a path began with `/`, so a
//! perfectly good Windows answer was reported as not absolute.
//!
//! **What went wrong.** `doctor_text_names_the_otp_root_and_version` failed on
//! the Windows runner against output that is entirely correct:
//!
//! ```text
//! no absolute `otp root:` line in:
//! ...
//! otp root: d:/a/_temp/.setup-beam/otp
//! ```
//!
//! (`Windows build and exit-code propagation`
//! <https://github.com/P4suta/ginary/actions/runs/33739517757/job/100597889388>.)
//!
//! **The input.** Any absolute path that is not a POSIX one. The assertion was
//! `line.starts_with("otp root: /")`, which is the POSIX rule asked of every
//! host — the same defect E7 fixed inside `src/cache.rs`, in a test this time.
//! Note the shape the runtime actually prints: a lower-case drive letter and
//! forward slashes, `d:/a/_temp/.setup-beam/otp`, which is neither the POSIX
//! spelling nor the one `cmd` would echo.
//!
//! **The correct behaviour.** "Absolute" is decided by the platform whose path
//! it is, and the rule is a pure function so it can be stated once and checked
//! on any host. See
//! `tests/regressions/e7_the_xdg_rule_used_the_hosts_idea_of_an_absolute_path.rs`
//! for the same rule on the other side of the tree.

use crate::common::hostpath::is_absolute_for;
use ginary::target::Os;

#[test]
fn absoluteness_is_decided_by_the_platform_the_path_belongs_to() {
    for path in ["/opt/otp", "/", "/usr/local/lib/erlang"] {
        for os in [Os::Linux, Os::Macos] {
            assert!(is_absolute_for(os, path), "{path:?} is absolute on {os:?}");
        }
        assert!(
            !is_absolute_for(Os::Windows, path),
            "{path:?} names no drive, so Windows resolves it against the current one"
        );
    }

    for path in [
        r"C:\Program Files\erl",
        "d:/a/_temp/.setup-beam/otp",
        r"\\srv\share\otp",
        r"\\?\C:\Users\RUNNER~1\otp",
    ] {
        assert!(
            is_absolute_for(Os::Windows, path),
            "{path:?} is absolute on Windows"
        );
        assert!(
            !is_absolute_for(Os::Linux, path),
            "{path:?} is a relative name on unix, where `\\` and `:` are ordinary characters"
        );
    }

    for path in ["otp", "otp/bin", "", "d:", "erl.exe"] {
        for os in [Os::Linux, Os::Macos, Os::Windows] {
            assert!(
                !is_absolute_for(os, path),
                "{path:?} is absolute on no platform"
            );
        }
    }
}
