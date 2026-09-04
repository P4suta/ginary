// SPDX-License-Identifier: MIT OR Apache-2.0
//! Every Windows artifact carried `beam.debug.smp.dll`, a debug build of the
//! emulator nothing loads and no user's machine can.
//!
//! **What went wrong.** A unix `erts-<vsn>/bin` contributes exactly
//! `otp::REQUIRED_ERTS_BINARIES` — `beam.smp` and three others — so the debug
//! emulator beside it, `beam.debug.smp`, is left behind by name. A Windows
//! `bin` cannot be a fixed list, because the set of DLLs moves between
//! releases, so `assemble::windows_required_bins` takes the three required
//! names *and every `.dll` beside them*. That rule takes the debug emulator
//! too. `ginary verify` on a real Windows artifact read it back out:
//!
//! ```text
//! ---- a_real_artifact_verifies_clean ----
//! ObjectInfo { path: "erts-17.0.5/bin/beam.debug.smp.dll", …
//!   needed: [… "MSVCP140D.dll", "VCRUNTIME140D.dll", "VCRUNTIME140_1D.dll", "ucrtbased.dll"],
//!   issues: [UnexpectedNeeded { needed: "MSVCP140D.dll" },
//!            UnexpectedNeeded { needed: "VCRUNTIME140D.dll" },
//!            UnexpectedNeeded { needed: "VCRUNTIME140_1D.dll" },
//!            UnexpectedNeeded { needed: "ucrtbased.dll" }] }
//! ```
//!
//! (`Windows build and exit-code propagation`
//! <https://github.com/P4suta/ginary/actions/runs/33823103540/job/100869848230>.)
//! The four libraries it needs are the *debug* C runtime, which is not
//! redistributable and exists only where Visual Studio is installed, so the
//! file could not load on a user's machine even if something tried. Nothing
//! does: `erl.exe` loads `beam.smp.dll` unless it is asked for the debug
//! emulator, which a packaged artifact never is. It is dead weight in every
//! artifact and four findings in every report.
//!
//! **The input.** Any upstream `otp_win64_<version>.zip`; the debug emulator
//! has shipped in them for years.
//!
//! **The correct behaviour.** The Windows rule leaves the debug emulator
//! behind for the same reason the unix rule does, and says so: an excluded
//! program is recorded with a reason rather than dropped silently.

#![cfg(feature = "cli")]

use std::path::Path;

use ginary::assemble::{
    self, StageOptions, WINDOWS_EMULATOR_DLL, WINDOWS_ERL_INI, WINDOWS_REQUIRED_BINS,
    excluded_reason, windows_required_bins,
};
use ginary::otp::OtpInfo;

use crate::common::fake_otp::{
    DEFAULT_ERTS_VSN, DEFAULT_OTP_VERSION, DEFAULT_RELEASE, FakeOtp, FakeShipment,
};

/// The debug build of the emulator that ships beside the real one.
const DEBUG_EMULATOR: &str = "beam.debug.smp.dll";

/// Writes a Windows `erts-<vsn>/bin` holding `names`, as empty files.
///
/// `windows_required_bins` decides by name and reads no header, so the files
/// need no content; a fixture that wrote real PE images would be asserting on
/// a second thing.
fn erts_bin(dir: &Path, names: &[&str]) -> std::path::PathBuf {
    let bin = dir.join("erts-17.0.5").join("bin");
    std::fs::create_dir_all(&bin).expect("the fixture bin directory");
    for name in names {
        std::fs::write(bin.join(name), b"").expect("a fixture file");
    }
    bin
}

/// The tree an upstream Windows zip really lays down, in miniature.
fn upstream_names() -> Vec<&'static str> {
    let mut names: Vec<&'static str> = WINDOWS_REQUIRED_BINS.to_vec();
    names.extend([
        DEBUG_EMULATOR,
        "erlexec.dll",
        "beam.smp.pdb",
        "werl.exe",
        "erlsrv.exe",
        WINDOWS_ERL_INI,
    ]);
    names
}

#[test]
fn the_debug_emulator_does_not_travel() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let bin = erts_bin(dir.path(), &upstream_names());

    let carried = windows_required_bins(&bin).expect("the tree holds every required name");

    assert!(
        !carried.iter().any(|name| name == DEBUG_EMULATOR),
        "`{DEBUG_EMULATOR}` needs the debug C runtime, which is not redistributable, and nothing \
         in a packaged artifact ever loads it: {carried:?}"
    );
}

#[test]
fn the_emulator_that_is_loaded_still_travels() {
    // The guard: the rule that drops the debug emulator may not drop the
    // emulator, and it may not turn a data-driven list into a fixed one.
    let dir = tempfile::tempdir().expect("a temporary directory");
    let bin = erts_bin(dir.path(), &upstream_names());

    let carried = windows_required_bins(&bin).expect("the tree holds every required name");

    for required in WINDOWS_REQUIRED_BINS {
        assert!(
            carried.iter().any(|name| name == required),
            "`{required}` is one of the three names a Windows runtime must hold: {carried:?}"
        );
    }
    assert!(
        carried.iter().any(|name| name == "erlexec.dll"),
        "a DLL beside the emulator is part of the runtime: {carried:?}"
    );
    assert!(
        carried.iter().any(|name| name == WINDOWS_EMULATOR_DLL),
        "and the emulator itself is: {carried:?}"
    );
}

#[test]
fn a_tree_whose_only_emulator_is_the_debug_one_is_not_a_runtime() {
    // The other direction, and the reason this is a rule about a name rather
    // than a filter over the answer: a zip that shipped only the debug build
    // would otherwise stage as a runtime and fail on somebody else's machine.
    let dir = tempfile::tempdir().expect("a temporary directory");
    let names: Vec<&str> = WINDOWS_REQUIRED_BINS
        .iter()
        .copied()
        .filter(|name| *name != WINDOWS_EMULATOR_DLL)
        .chain([DEBUG_EMULATOR])
        .collect();
    let bin = erts_bin(dir.path(), &names);

    let error = windows_required_bins(&bin).expect_err("the emulator is missing");

    assert!(
        error.to_string().contains(WINDOWS_EMULATOR_DLL),
        "the refusal names the file that is not there: {error}"
    );
}

/// The spellings an upstream zip is free to use for the same file.
///
/// A Windows filesystem does not distinguish them, and the rule beside the
/// decline — [`ginary::assemble::windows_required_bins`]'s `.dll` test —
/// already says so and compares its suffix case-insensitively. The decline
/// did not, so a zip spelling the emulator either of these ways passed it,
/// was admitted as a library by the rule on the next line, and put the four
/// findings back.
const DEBUG_EMULATOR_SPELLINGS: [&str; 2] = ["BEAM.DEBUG.SMP.DLL", "beam.debug.smp.DLL"];

#[test]
fn the_case_the_zip_spelled_it_in_does_not_change_the_answer() {
    for spelling in DEBUG_EMULATOR_SPELLINGS {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let mut names: Vec<&str> = WINDOWS_REQUIRED_BINS.to_vec();
        names.extend(["erlexec.dll", spelling]);
        let bin = erts_bin(dir.path(), &names);

        let carried = windows_required_bins(&bin).expect("the tree holds every required name");

        assert!(
            !carried.iter().any(|name| *name == spelling),
            "`{spelling}` is `{DEBUG_EMULATOR}` on the filesystem this tree came off, and it \
             needs the same debug C runtime whichever way the zip spelled it: {carried:?}"
        );
        assert!(
            carried.iter().any(|name| name == "erlexec.dll"),
            "and the DLL beside it still travels: {carried:?}"
        );
    }
}

#[test]
fn the_reason_the_debug_emulator_is_given_is_its_own_whatever_the_case() {
    // The decline and the reason are one policy read from two places, so a
    // spelling one of them accepts and the other does not is a file reported
    // as "not on the launcher's allowlist" when the answer is that nothing
    // could load it.
    let generic = excluded_reason("werl.exe");
    assert_ne!(
        excluded_reason(DEBUG_EMULATOR),
        generic,
        "the debug emulator is declined for a reason of its own"
    );
    for spelling in DEBUG_EMULATOR_SPELLINGS {
        assert_eq!(
            excluded_reason(spelling),
            excluded_reason(DEBUG_EMULATOR),
            "`{spelling}` names the same file and is declined for the same reason"
        );
    }
}

#[test]
fn the_declined_emulator_is_reported_with_that_reason_by_a_staged_tree() {
    // The claim `windows_required_bins` makes in its own documentation —
    // "what is declined here is reported" — held against a staged tree rather
    // than argued. `CLAUDE.md`: a skip is a reported decision or an error,
    // never a default.
    let dir = tempfile::tempdir().expect("a temporary directory");
    let shipment = dir.path().join("shipment");
    FakeShipment::new().build_in(&shipment);
    let otp = FakeOtp::new()
        .windows()
        .extra_erts_bins(&[DEBUG_EMULATOR])
        .build_in(dir.path().join("otp"));
    let set =
        ginary::closure::app_dependency_closure(&shipment, &otp.lib(), &["stdlib".to_owned()], &[])
            .expect("the seeded root resolves");
    let info = OtpInfo {
        root: otp.root.clone(),
        release: DEFAULT_RELEASE,
        erts_vsn: DEFAULT_ERTS_VSN.to_owned(),
        otp_version: DEFAULT_OTP_VERSION.to_owned(),
        erts_bin: otp.erts_bin(),
        lib: otp.lib(),
    };

    let staged = assemble::stage(
        &set,
        &info,
        &StageOptions {
            remove_junk: true,
            force: true,
            ..StageOptions::default()
        },
        &dir.path().join("out"),
    )
    .expect("a whole Windows tree stages");

    let excluded = staged
        .excluded_erts_bins()
        .iter()
        .find(|bin| bin.name == DEBUG_EMULATOR)
        .unwrap_or_else(|| {
            panic!(
                "a program left behind is a reported decision: {:?}",
                staged.excluded_erts_bins()
            )
        });
    assert_eq!(
        excluded.reason,
        excluded_reason(DEBUG_EMULATOR),
        "and the reason is the one the policy gives"
    );
}
