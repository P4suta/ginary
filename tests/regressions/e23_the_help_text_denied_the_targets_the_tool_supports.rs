// SPDX-License-Identifier: MIT OR Apache-2.0
//! `ginary --help` told every user that cross-target builds are not
//! implemented, three phases after they were.
//!
//! **What went wrong.** The top-level clap `long_about` in `src/cli.rs` ended
//! with
//!
//! ```text
//! Only Linux x86_64 host packaging is implemented; cross-target builds are not.
//! ```
//!
//! Both clauses were true in A0, which is where the sentence comes from —
//! `docs/dev/log/A0.md` records the same shape, then saying `build` itself was
//! not implemented. Neither has been true since:
//! `docs/dev/v1-readiness.md` marks multi-target plumbing done at C1, the
//! local-first OTP catalog and cross-Linux artifacts at C3, Windows packaging
//! at D2 and Mach-O packaging at D3, and `README.md`'s status matrix carries a
//! row for all seven targets.
//!
//! **The input.** `ginary --help`. It is the most-read sentence the tool
//! emits and it survived every phase that falsified it, because nothing in the
//! suite reads help text: `tests/docs.rs` scans `src/` for *missing* doc
//! comments, which is a different question, and no test had ever asked what
//! the binary tells a user about itself.
//!
//! **The correct behaviour.** A tool does not deny what it does. The rule is
//! derived rather than pinned to a wording: no target the readiness document
//! marks done may be described by the help text as unimplemented. That way the
//! next phase to add a target cannot leave the sentence behind — and a
//! rewording that keeps the meaning does not fail a test for no reason.

// The command line half: `--help` is clap's, and a launcher-only build has no
// clap in it.
#![cfg(feature = "cli")]

use assert_cmd::Command;

use crate::common::repo::read;

/// The readiness document, which is where "this target is done" is recorded.
const READINESS: &str = "docs/dev/v1-readiness.md";

/// The phrases a help text uses to say a thing is not there.
///
/// Small and closed on purpose: this is a scan for a *denial*, and a sentence
/// that merely mentions a target is not one.
const DENIALS: [&str; 4] = [
    "is not implemented",
    "are not implemented",
    "cross-target builds are not",
    "Only Linux",
];

fn help() -> String {
    let output = Command::cargo_bin("ginary")
        .expect("the `ginary` binary is built for tests")
        .arg("--help")
        .output()
        .expect("`ginary --help` runs");
    assert!(
        output.status.success(),
        "`ginary --help` exits zero: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("help is UTF-8")
}

#[test]
fn the_help_text_denies_nothing_the_readiness_document_marks_done() {
    let help = help();
    let denials: Vec<&str> = DENIALS
        .iter()
        .copied()
        .filter(|phrase| help.contains(phrase))
        .collect();
    assert!(
        denials.is_empty(),
        "`ginary --help` tells a user something is not implemented — {denials:?} — while \
         {READINESS} records the whole pipeline for seven targets as delivered. The help text \
         is the most-read sentence this tool emits:\n{help}"
    );
}

#[test]
fn the_readiness_document_still_records_the_targets_the_help_text_may_not_deny() {
    // The other half of the rule, so that the test above cannot be satisfied
    // by a readiness document that stopped claiming anything.
    let readiness = read(READINESS);
    for target in ["linux-x86_64-gnu", "windows-x86_64", "macos-aarch64"] {
        assert!(
            readiness.contains(target) || readiness.contains("seven targets"),
            "{READINESS} no longer records `{target}`, so `--help` denying it would be honest \
             and this rule has lost its premise"
        );
    }
}
