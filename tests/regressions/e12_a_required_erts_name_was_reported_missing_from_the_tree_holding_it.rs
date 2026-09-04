// SPDX-License-Identifier: MIT OR Apache-2.0
//! A Windows tree spelling one of the three required names in another case was
//! refused by a message naming a file that was in the directory.
//!
//! **What went wrong.** E12 taught `assemble::windows_required_bins` to
//! decline `beam.debug.smp.dll` case-insensitively, because the `.dll` rule
//! beside the decline compares its suffix that way and a zip spelling the
//! debug emulator `BEAM.DEBUG.SMP.DLL` would otherwise have been admitted as a
//! library. The argument the decline rests on — a Windows filesystem does not
//! distinguish the two spellings and an upstream zip is under no obligation to
//! pick one — applies word for word to the three names checked above it, and
//! those were, and stay, compared exactly:
//!
//! ```text
//! the ERTS binary `beam.smp.dll` is missing; looked for it at
//! `…/erts-17.0.5/bin/beam.smp.dll`
//! ```
//!
//! for a directory that holds `BEAM.SMP.DLL`. A user reading that is told a
//! file is missing that they can see, with nothing to do about it.
//!
//! **The input.** Any Windows `erts-<vsn>/bin` — a real zip, or one
//! reassembled by a tool that upper-cased its names — spelling one of
//! `assemble::WINDOWS_REQUIRED_BINS` in a case other than the one ginary
//! names.
//!
//! **The correct behaviour.** The three names stay exact, and the reason is
//! not an oversight: every other gate that names these files names them
//! literally — `assemble::is_windows_erts_bin` decides a tree's flavour by
//! `erts_bin.join("erl.exe").is_file()`, `launch::WINDOWS_REQUIRED_BINARIES`
//! checks the extracted tree by name, and the index a Linux host verifies a
//! Windows artifact against carries the name the tree was staged under. A
//! tree staged under a different spelling would satisfy this one rule and fail
//! the others on a case-sensitive host, so the tree is refused rather than
//! staged. What is owed to the user is the diagnosis: the refusal names the
//! spelling the tree really uses beside the spelling ginary needs, so the
//! answer is "rename it", not "it is not there".

#![cfg(feature = "cli")]

use std::path::Path;

use ginary::assemble::{
    AssembleError, WINDOWS_EMULATOR_DLL, WINDOWS_REQUIRED_BINS, windows_required_bins,
};

/// The emulator DLL, spelled the way a filesystem that does not care would.
const SHOUTED_EMULATOR: &str = "BEAM.SMP.DLL";

/// Writes a Windows `erts-<vsn>/bin` holding `names`, as empty files.
///
/// The rule under test decides by name and reads no header, so the files need
/// no content.
fn erts_bin(dir: &Path, names: &[&str]) -> std::path::PathBuf {
    let bin = dir.join("erts-17.0.5").join("bin");
    std::fs::create_dir_all(&bin).expect("the fixture bin directory");
    for name in names {
        std::fs::write(bin.join(name), b"").expect("a fixture file");
    }
    bin
}

/// The required names, with `WINDOWS_EMULATOR_DLL` replaced by `spelling`.
fn required_but_for_the_emulator(spelling: &str) -> Vec<String> {
    WINDOWS_REQUIRED_BINS
        .iter()
        .map(|name| {
            if *name == WINDOWS_EMULATOR_DLL {
                spelling.to_owned()
            } else {
                (*name).to_owned()
            }
        })
        .collect()
}

#[test]
fn the_refusal_names_the_spelling_the_tree_uses() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let names = required_but_for_the_emulator(SHOUTED_EMULATOR);
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let bin = erts_bin(dir.path(), &names);

    let error = windows_required_bins(&bin).expect_err("ginary names the emulator exactly");
    let message = error.to_string();

    assert!(
        message.contains(WINDOWS_EMULATOR_DLL),
        "the refusal names the spelling ginary needs: {message}"
    );
    assert!(
        message.contains(SHOUTED_EMULATOR),
        "and the spelling the tree really uses, because `{SHOUTED_EMULATOR}` is in the directory \
         the message says `{WINDOWS_EMULATOR_DLL}` is missing from, and a user told a file they \
         can see is not there has nothing to do next: {message}"
    );
}

/// `message` up to the path it ends with.
///
/// Both refusals end in `looked for it at `<path>``, and the path is a
/// temporary directory that differs between two fixtures, so a comparison of
/// whole messages would pass whatever the two said. What is being compared is
/// the diagnosis.
fn diagnosis(message: &str) -> &str {
    message
        .split("looked for it at")
        .next()
        .expect("a split yields at least one part")
}

#[test]
fn a_file_that_is_not_there_at_all_is_still_diagnosed_as_missing() {
    // The other half, and the reason the first is a distinct answer rather
    // than a longer message: a tree that never had the emulator is a
    // different problem from one that spells it differently, and the two are
    // not told the same thing.
    let dir = tempfile::tempdir().expect("a temporary directory");
    let names: Vec<&str> = WINDOWS_REQUIRED_BINS
        .iter()
        .copied()
        .filter(|name| *name != WINDOWS_EMULATOR_DLL)
        .collect();
    let bin = erts_bin(dir.path(), &names);

    let error = windows_required_bins(&bin).expect_err("the emulator is not there");
    let absent = error.to_string();

    assert!(
        matches!(error, AssembleError::MissingErtsBinary { .. }),
        "a file that is not in the tree at all is missing: {error:?}"
    );

    let dir = tempfile::tempdir().expect("a temporary directory");
    let names = required_but_for_the_emulator(SHOUTED_EMULATOR);
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let bin = erts_bin(dir.path(), &names);
    let mismatch = windows_required_bins(&bin)
        .expect_err("ginary names the emulator exactly")
        .to_string();

    assert_ne!(
        diagnosis(&absent),
        diagnosis(&mismatch),
        "the two refusals are two diagnoses: one file is nowhere and the other is right there \
         under another name"
    );
}

#[test]
fn a_name_that_is_kept_is_the_name_the_rule_asked_for() {
    // The rule the two tests above rest on, stated once: what a Windows tree
    // contributes for a required name is that name, exactly, because
    // `launch::WINDOWS_REQUIRED_BINARIES` and the index a Linux host verifies
    // the artifact against both spell it literally. A rule that accepted
    // another spelling here would have to carry it through both of those.
    let dir = tempfile::tempdir().expect("a temporary directory");
    let mut names: Vec<&str> = WINDOWS_REQUIRED_BINS.to_vec();
    names.push("erlexec.dll");
    let bin = erts_bin(dir.path(), &names);

    let carried = windows_required_bins(&bin).expect("the tree holds every required name");

    for required in WINDOWS_REQUIRED_BINS {
        assert!(
            carried.iter().any(|name| name == required),
            "`{required}` travels under the name every other gate names it by: {carried:?}"
        );
    }
}
