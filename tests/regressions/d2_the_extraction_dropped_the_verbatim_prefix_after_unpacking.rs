// SPDX-License-Identifier: MIT OR Apache-2.0
//! `long_path` was applied to the extraction destination alone, so the flush,
//! the rename and the cleanup that follow it all re-derived un-prefixed paths.
//!
//! `winpath`'s own module documentation states the contract: the `\\?\` prefix
//! is applied to every path the launcher *writes* under. The code applied it
//! at exactly one call site — the directory handed to the unpacker — and the
//! next four operations took the plain temporary path: `chmod_tree` over the
//! bindir, `sync_tree`'s per-file `File::open`, `rename_into_place`, and the
//! `remove_dir_all` of both the failure paths. Each re-derives a non-verbatim
//! path, so a tree the verbatim prefix let the unpacker create is one the
//! per-file flush cannot open: a deep `%LOCALAPPDATA%` still failed, one step
//! later and with a worse message.
//!
//! The fix is to prefix once at the source. The directory an application's
//! entries live in is put in its verbatim form before anything is derived from
//! it, so the temporary tree, the entry, the bindir and every file under them
//! inherit the prefix through `Path::join` rather than through a second call
//! somebody has to remember to make.
//!
//! **This test does not run on Linux.** `long_path` is the identity here by
//! design, so there is no length limit to hit and nothing to observe; it
//! compiles for `x86_64-pc-windows-gnu` and `docs/dev/log/D2.md` lists the
//! claim among the ones the GitHub Actions milestone has to check.
#![cfg(windows)]

use std::path::{Path, PathBuf};

use ginary::cache::{CacheDirs, Origin};
use ginary::winpath::LONG_PATH_PREFIX;

/// A cache root as `%LOCALAPPDATA%` spells it.
fn dirs() -> CacheDirs {
    CacheDirs {
        root: PathBuf::from(r"C:\Users\ada\AppData\Local\ginary"),
        origin: Origin::LocalAppData,
        is_fallback: false,
    }
}

#[test]
fn everything_one_extraction_writes_hangs_off_a_verbatim_directory() {
    let extraction = dirs().extraction_dir("hello");
    let text = extraction.to_str().expect("the fixture is Unicode");

    assert!(
        text.starts_with(LONG_PATH_PREFIX),
        "the prefix goes on once, where every path an extraction writes is \
         derived from: {text}"
    );

    // The property the single call site has to have, and the one the old code
    // did not: a path joined onto a verbatim path is verbatim too, so the
    // temporary tree, the entry and every file under them are covered by the
    // one call.
    let tmp = extraction.join(".0123456789abcdef.tmp-4242");
    let file = tmp.join("erts-17.0.5").join("bin").join("erl.exe");
    for derived in [&tmp, &file] {
        let derived = derived.to_str().expect("the fixture is Unicode");
        assert!(
            derived.starts_with(LONG_PATH_PREFIX),
            "a path the extraction flushes, renames or removes must carry the \
             prefix the unpacker was given: {derived}"
        );
    }
    assert_eq!(
        Path::new(&extraction).file_name(),
        Some(std::ffi::OsStr::new("hello")),
        "and it is still the application's own directory"
    );
}
