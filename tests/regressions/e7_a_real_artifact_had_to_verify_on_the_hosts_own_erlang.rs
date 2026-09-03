// SPDX-License-Identifier: MIT OR Apache-2.0
//! `a_real_artifact_verifies_clean` asserted a property of the machine's
//! Erlang build, not of ginary.
//!
//! **What went wrong.** The test builds a real artifact from whatever OTP the
//! host installed and asserts `ginary verify` reports nothing at all. On the
//! `test` runner, `setup-beam`'s `beam.smp` is linked against `libz.so.1`,
//! which is not on [`ginary::verify::NEEDED_ALLOWLIST`] and is not part of a
//! glibc system's own runtime, so the verifier reported it — correctly:
//!
//! ```text
//! ---- a_real_artifact_verifies_clean stdout ----
//! thread 'a_real_artifact_verifies_clean' panicked at tests/verify.rs:736:5:
//! assertion `left == right` failed
//!   left: ["erts-17.0.5/bin/beam.smp: needs `libz.so.1`, which the artifact does not carry"]
//!  right: []
//! ```
//!
//! (`Test (both flavors, stable)`
//! <https://github.com/P4suta/ginary/actions/runs/33702776627/job/100485421869>;
//! the `Coverage` job never reached it, having failed earlier in the same
//! run.) The developer machine's `beam.smp` names five libraries and every one
//! of them is on the allowlist, so the assertion held here and nowhere it
//! mattered.
//!
//! **The input.** Any machine whose Erlang was built against a library outside
//! the allowlist. `libz` is the common one; a distribution that links
//! `libsystemd` or `libcrypto` into the emulator would do it too.
//!
//! **The correct behaviour.** The verifier is right and the expectation is
//! wrong. An artifact is only as portable as the emulator it was built from,
//! and `needs \`libz.so.1\`` is exactly the finding ginary exists to produce.
//! What the test may assert is that the findings are *the ones the installed
//! runtime accounts for and no others* — computed from the OTP installation,
//! which is a different file from the artifact the report was read out of, so
//! the assertion still has two independent sides. Neither the allowlist nor
//! the assertion is weakened: adding `libz.so.1` to
//! [`ginary::verify::NEEDED_ALLOWLIST`] would be promising a stranger's
//! machine has zlib, which ginary does not know.
//!
//! [`crate::common::portability::unmet_needs`] is that computation, and it is
//! calibrated below before the rule about the test is asserted.
#![cfg(feature = "cli")]

use crate::common::portability::unmet_needs;
use crate::common::repo::read;

/// The file holding the test whose expectation was one machine's.
const VERIFY_TESTS: &str = "tests/verify.rs";

#[test]
fn the_real_artifact_test_derives_its_expectation_from_the_runtime_the_host_installed() {
    // The instrument: what a `DT_NEEDED` set costs against the shipped
    // allowlist, including the loader rule an exact-match filter would get
    // wrong.
    let needed: Vec<String> = [
        "libc.so.6",
        "libz.so.1",
        "ld-linux-x86-64.so.2",
        "libtinfo.so.6",
        "libz.so.1",
        "libsystemd.so.0",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    assert_eq!(
        unmet_needs(&needed, &ginary::verify::NEEDED_ALLOWLIST),
        vec!["libsystemd.so.0".to_owned(), "libz.so.1".to_owned()],
        "sorted, without repeats, and the glibc loader is admitted by the companion rule rather \
         than by its own name"
    );
    assert_eq!(
        unmet_needs(&needed, &[]),
        vec![
            "ld-linux-x86-64.so.2".to_owned(),
            "libc.so.6".to_owned(),
            "libsystemd.so.0".to_owned(),
            "libtinfo.so.6".to_owned(),
            "libz.so.1".to_owned(),
        ],
        "an empty allowlist assumes nothing about the target machine, the loader included"
    );

    let source = read(VERIFY_TESTS);
    assert!(
        !source.contains("assert_eq!(sentences(&report), Vec::<String>::new());"),
        "{VERIFY_TESTS} still asserts that a real artifact has no findings at all, which is a \
         claim about the host's Erlang build and not about ginary"
    );
    assert!(
        source.contains("unmet_needs"),
        "{VERIFY_TESTS} has to compare the report against what the *installed runtime* accounts \
         for, so that a finding is still a failure and only the host's own libraries are not"
    );
}
