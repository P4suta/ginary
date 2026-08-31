// SPDX-License-Identifier: MIT OR Apache-2.0
//! The `\\?\` prefix covered the extraction and stopped there, so a cache
//! entry deep enough to need it was unpacked and then never usable.
//!
//! `ensure_extracted` kept two spellings of one directory — `writing`, the
//! verbatim one everything it *writes* hangs off, and `target`, the ordinary
//! one it answered with — and answered with the second. Every read that
//! follows takes that answer: the cache-hit check, the `.lock` open, the
//! manifest probe and `launch::preflight`. Rust's `std` opens an ordinary
//! Windows path through the `MAX_PATH`-limited normalisation, so on exactly
//! the deep `%LOCALAPPDATA%` path the prefix was added for, the payload
//! extracted, the hit check then missed on every launch, the rename took its
//! "somebody finished first" branch and `preflight` reported `Missing` — a
//! second whole extraction, then exit 124, every run.
//!
//! The rule is now one sentence: **ginary opens the verbatim spelling and
//! hands `erl.exe` the ordinary one.** The conversion happens once, in
//! `launch::plan`, at the only place a cache path leaves this process.
//!
//! What a Linux machine can check honestly is that conversion, because it is
//! pure path syntax: a verbatim root reaches `ROOTDIR`, `BINDIR` and the
//! argument vector without its prefix, and the program the launcher spawns
//! itself keeps it. The extraction half is asserted as the shape it must have
//! — `extraction_dir(app).join(key)` — which on unix is the same path either
//! way; `docs/dev/log/D2.md` records that rather than claiming this machine
//! proved it.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use crate::common::artifact::{APP, SyntheticArtifact, canonical_manifest};

use ginary::cache::{self, CacheDirs, Env, Origin};
use ginary::diag::Diag;
use ginary::launch::{self, LaunchPlan};
use ginary::winpath::LONG_PATH_PREFIX;

/// A cache entry as `%LOCALAPPDATA%` spells it, verbatim.
const VERBATIM_ENTRY: &str =
    r"\\?\C:\Users\ada\AppData\Local\ginary\hello\0123456789abcdef0123456789abcdef";

/// The same entry as `erl.exe` has to be handed it.
const PLAIN_ENTRY: &str =
    r"C:\Users\ada\AppData\Local\ginary\hello\0123456789abcdef0123456789abcdef";

/// The plan for one launch out of `root`.
fn plan_from(root: &Path) -> LaunchPlan {
    let manifest = canonical_manifest();
    let env = Env::from_pairs(std::iter::empty());
    let dumps = Path::new(r"C:\Users\ada\AppData\Local\ginary\hello");
    let exe = Path::new(r"C:\Program Files\hello\hello.exe");
    match launch::plan(root, &manifest, &[], &env, dumps, exe) {
        Ok(plan) => plan,
        Err(error) => panic!("the canonical manifest must produce a plan: {error}"),
    }
}

/// The value of one variable the plan sets.
fn set_value(plan: &LaunchPlan, name: &str) -> OsString {
    plan.set
        .iter()
        .find(|(key, _)| key == OsStr::new(name))
        .map(|(_, value)| value.clone())
        .unwrap_or_else(|| panic!("the plan must set {name}"))
}

/// The argument after `flag`.
fn arg_after(plan: &LaunchPlan, flag: &str) -> OsString {
    let position = plan
        .args
        .iter()
        .position(|arg| arg == OsStr::new(flag))
        .unwrap_or_else(|| panic!("the plan must pass {flag}"));
    plan.args
        .get(position + 1)
        .cloned()
        .unwrap_or_else(|| panic!("{flag} must take a value"))
}

#[test]
fn a_verbatim_entry_reaches_the_runtime_in_its_ordinary_spelling() {
    let plan = plan_from(Path::new(VERBATIM_ENTRY));

    assert_eq!(
        set_value(&plan, "ROOTDIR"),
        OsString::from(PLAIN_ENTRY),
        "`erl.exe` takes ROOTDIR apart and puts it back together; a `\\\\?\\` path is a shape \
         it does not understand"
    );
    assert!(
        !set_value(&plan, "BINDIR")
            .to_string_lossy()
            .starts_with(LONG_PATH_PREFIX),
        "BINDIR is joined onto the same root and leaves this process the same way"
    );
    assert!(
        !arg_after(&plan, "-boot")
            .to_string_lossy()
            .starts_with(LONG_PATH_PREFIX),
        "and so does every path in the argument vector"
    );
}

#[test]
fn the_program_the_launcher_starts_itself_keeps_the_prefix() {
    let plan = plan_from(Path::new(VERBATIM_ENTRY));

    assert!(
        plan.program.to_string_lossy().starts_with(LONG_PATH_PREFIX),
        "the launcher opens this one itself, so it is spelled the way the extraction wrote it: \
         {}",
        plan.program.display()
    );
}

#[test]
fn an_ordinary_entry_is_left_exactly_as_it_was_given() {
    // The other half of the rule, and the one every unix launch takes: a path
    // with no prefix on it is handed on untouched, prefix-stripping being a
    // Windows-syntax rule rather than a normalisation.
    let plan = plan_from(Path::new(PLAIN_ENTRY));

    assert_eq!(set_value(&plan, "ROOTDIR"), OsString::from(PLAIN_ENTRY));
    assert_eq!(
        plan.program,
        Path::new(PLAIN_ENTRY)
            .join(&canonical_manifest().launch.bindir)
            .join(&canonical_manifest().launch.program)
    );
}

#[test]
fn the_entry_the_extraction_answers_with_is_the_one_it_wrote_under() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let artifact = SyntheticArtifact::build(dir.path());
    let file = std::fs::File::open(artifact.path()).expect("open the artifact");
    let trailer = *artifact.trailer();
    let dirs = CacheDirs {
        root: dir.path().join("cache"),
        origin: Origin::GinaryCacheDir,
        is_fallback: false,
    };

    let entry = cache::ensure_extracted(&file, &trailer, APP, &dirs, &Diag::disabled())
        .expect("a cold cache must extract");

    let written: PathBuf = dirs.extraction_dir(APP).join(artifact.key());
    assert_eq!(
        entry, written,
        "the answer is the spelling the extraction wrote under, so the hit check, the lock, \
         the manifest probe and the preflight all open the file that is there"
    );
}
