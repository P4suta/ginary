// SPDX-License-Identifier: MIT OR Apache-2.0
//! A Windows machine could not build a Windows artifact from the Erlang it
//! already had, because the refusal that guards a Windows build was written
//! about the target and reasoned about the host.
//!
//! **What went wrong.** Twenty-three of the twenty-five `tests/e2e_hello.rs`
//! targets, the real-artifact half of `tests/verify.rs` and every
//! `tests/sbom.rs` build failed on the first Windows runner with one
//! sentence:
//!
//! ```text
//! ---- a_real_artifact_verifies_clean stdout ----
//! the build failed:
//! error: cannot bundle a runtime for windows-x86_64 from `host`: windows ERTS
//! trees arrive with the windows catalog entry, or from a `dir:` source holding one
//! ```
//!
//! (`Windows build and exit-code propagation`
//! <https://github.com/P4suta/ginary/actions/runs/33712111530/job/100513644404>.)
//!
//! **The input.** `erts = "host"`, or the default, on a Windows machine
//! building for `windows-x86_64`. The check's own documentation gives the
//! reason it was written — "nothing on a Linux build machine produces one:
//! there is no host runtime to fall back to" — which is true of a Linux
//! machine and false of the machine that hit it. The host runtime on Windows
//! *is* a Windows ERTS tree.
//!
//! **The correct behaviour.** The rule turns on the host as well as the
//! target, so [`ginary::bundle::check_windows_erts`] is given it. A Windows
//! target built on a Windows host may take the host runtime; every other
//! source, and every host that is not Windows, is refused exactly as before
//! and with the same sentence.

use std::path::PathBuf;

use ginary::bundle::{self, BundleError};
use ginary::erts_source::ErtsSourceSpec;
use ginary::target::{Arch, Libc, Os, Target};

/// The Windows target every build in this file asks for.
fn windows() -> Target {
    Target::new(Os::Windows, Arch::X86_64, Libc::None)
}

#[test]
fn a_windows_host_may_bundle_its_own_runtime_into_a_windows_artifact() {
    assert!(
        bundle::check_windows_erts(windows(), &ErtsSourceSpec::Host, Os::Windows).is_ok(),
        "the host runtime on a Windows machine is a Windows ERTS tree, and the default source \
         has to reach it"
    );
}

#[test]
fn a_host_that_is_not_windows_still_has_no_windows_runtime_to_offer() {
    for host in [Os::Linux, Os::Macos] {
        match bundle::check_windows_erts(windows(), &ErtsSourceSpec::Host, host) {
            Err(BundleError::WindowsErtsUnavailable { target, spec }) => {
                assert_eq!(target, windows());
                assert_eq!(spec, ErtsSourceSpec::Host.label());
            }
            other => panic!("a {host} host holds no Windows runtime, and this answered {other:?}"),
        }
    }
}

#[test]
fn no_host_turns_a_linux_tarball_into_a_windows_runtime() {
    let tarball =
        ErtsSourceSpec::Tarball(PathBuf::from("/srv/otp-29.0.5-linux-x86_64-gnu.tar.zst"));
    for host in [Os::Linux, Os::Macos, Os::Windows] {
        assert!(
            bundle::check_windows_erts(windows(), &tarball, host).is_err(),
            "a {host} host does not make a Linux tarball run on Windows"
        );
    }
}
