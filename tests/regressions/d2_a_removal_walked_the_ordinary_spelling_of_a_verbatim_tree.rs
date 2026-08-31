// SPDX-License-Identifier: MIT OR Apache-2.0
//! Every removal walked the ordinary spelling of a tree the extraction had
//! written under `\\?\`, so on Windows a past-`MAX_PATH` entry could be listed
//! but never removed.
//!
//! D2 put the verbatim prefix on the directory an extraction hangs off —
//! `CacheDirs::extraction_dir` — and on what `ensure_extracted` answers with,
//! so that everything the launcher *opens* afterwards is the path that is
//! actually there. The removals were not part of that sweep.
//! `GINARY_CMD=uninstall` was handed `CacheDirs::app_dir`, `ginary cache prune`
//! and `ginary cache clean` were handed `CacheDirs::root`, and each of them
//! then walked the tree in a spelling Rust's `std` resolves through the
//! `MAX_PATH`-limited normalisation.
//!
//! The consequence is not a failed command, which is what makes it worth a
//! regression test: `read_dir` on the application directory succeeds, so the
//! entry is found, its age is read and its lock is taken — and the `rename`
//! aside then fails on the length, so the entry is reported `Unremovable` and
//! left on disk. An uninstall would print `kept … (unremovable)` for every
//! entry it was asked to remove, forever, on exactly the deep
//! `%LOCALAPPDATA%` path the prefix was added for.
//!
//! The rule now lives in the walker rather than at the call site: `sweep`,
//! `discard_incomplete`, `prune_app`, `uninstall`, `prune` and `clean` each
//! put the directory they were given into the verbatim spelling themselves,
//! and put the ordinary spelling back on every path they report. Keeping two
//! spellings at the call site is what let them drift apart in the first place,
//! so the launcher now hands the prune the same `app_dir` it hands the crash
//! dump.
//!
//! What a Linux machine can check honestly is the path rule on both sides —
//! it is pure syntax — and that the reports a caller reads are unchanged. The
//! `rename` that fails past `MAX_PATH` is a Windows fact; `docs/dev/log/D3.md`
//! records that rather than claiming this machine proved it.

use std::path::{Path, PathBuf};

use ginary::cache::{self, KeptReason};
use ginary::winpath::{self, LONG_PATH_PREFIX};

/// A cache root where the limit is reached: `%LOCALAPPDATA%\ginary`.
const ROOT: &str = r"C:\Users\ada\AppData\Local\ginary";

/// One entry under it, as `read_dir` would name it.
const ENTRY: &str = r"C:\Users\ada\AppData\Local\ginary\hello\0123456789abcdef";

#[test]
fn the_spelling_a_removal_opens_is_not_the_one_its_caller_holds() {
    let opened = winpath::long_path_str(ROOT);

    assert_eq!(
        opened,
        format!("{LONG_PATH_PREFIX}{ROOT}"),
        "a removal walks the tree the extraction wrote, and the extraction wrote it verbatim"
    );
    assert_ne!(
        opened, ROOT,
        "so handing a walker the ordinary spelling is handing it a different path — the one \
         Windows resolves through the MAX_PATH-limited normalisation"
    );
    assert_eq!(
        winpath::long_path_str(&opened),
        opened,
        "and a caller that already holds the verbatim spelling is not prefixed twice, which is \
         what lets the rule live in the walker rather than at the call site"
    );
}

#[test]
fn what_a_removal_reports_is_the_spelling_its_caller_asked_about() {
    let found = format!("{}\\hello\\0123456789abcdef", winpath::long_path_str(ROOT));
    assert!(
        found.starts_with(LONG_PATH_PREFIX),
        "every path read_dir hands back under a verbatim root inherits the prefix"
    );
    assert_eq!(
        winpath::plain_path_str(&found),
        ENTRY,
        "and a prune table, an uninstall report and the path inside a cache error are read by \
         a person, so the prefix comes back off before any of them names it"
    );
}

#[test]
fn an_uninstall_still_reports_the_paths_the_caller_gave_it() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let app_dir = dir.path().join("hello");
    let entry = plant(&app_dir, "0123456789abcdef");
    let residue = plant_residue(&app_dir, ".0123456789abcdef.tmp-4000000000");

    let report = cache::uninstall(&app_dir);

    assert_eq!(
        report.removed,
        vec![residue, entry],
        "the report names what went, in the spelling the caller handed in"
    );
    assert_eq!(report.kept, Vec::<(PathBuf, KeptReason)>::new());
    assert!(
        !app_dir.exists(),
        "and an application directory with nothing left in it goes too"
    );
}

/// A complete entry: a `<key>` directory with a `ginary.json` in it.
fn plant(app_dir: &Path, key: &str) -> PathBuf {
    let entry = app_dir.join(key);
    std::fs::create_dir_all(&entry).expect("create the entry");
    std::fs::write(entry.join("ginary.json"), b"{}").expect("write the manifest");
    entry
}

/// Residue: a tree an interrupted extraction left behind.
fn plant_residue(app_dir: &Path, name: &str) -> PathBuf {
    let path = app_dir.join(name);
    std::fs::create_dir_all(&path).expect("create the residue");
    path
}
