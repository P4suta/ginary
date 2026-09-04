// SPDX-License-Identifier: MIT OR Apache-2.0
//! `GINARY_CMD=extract-only` printed the `\\?\` spelling of the cache entry,
//! and `GINARY_CMD=directory` printed the ordinary one, for the same
//! directory.
//!
//! **What went wrong.** `ginary::winpath` states the rule in one sentence:
//! ginary opens the verbatim spelling and hands `erl.exe` the ordinary one. A
//! path stops being something ginary opens when it reaches a person, and both
//! maintenance commands print onto standard output, where a shell's `$(…)`
//! reads it. `directory` derives its answer from `CacheDirs::key_dir` and
//! prints it plainly; `extract-only` prints what `cache::ensure_extracted`
//! answered, which deliberately carries the prefix:
//!
//! ```text
//! ---- ginary_cmd_extract_only_prints_the_cache_path_of_the_built_artifact ----
//! the printed entry must be under this run's own cache:
//! \\?\C:\Windows\Temp\ginary-unknown\hello_ffi\5e6351992db98538
//!
//! ---- a_cold_cache_extracts_into_the_key_directory ----
//! assertion `left == right` failed
//!   left: "\\\\?\\C:\\Users\\RUNNER~1\\...\\cache\\hello\\1179d51043100e24"
//!  right: "C:\\Users\\RUNNER~1\\...\\cache\\hello\\1179d51043100e24"
//! ```
//!
//! (`Windows build and exit-code propagation`
//! <https://github.com/P4suta/ginary/actions/runs/33751715516/job/100636537290>,
//! `tests/e2e_hello.rs:469`, `tests/cache.rs:199` and `:288`.)
//!
//! **The input.** Any Windows run. The two commands answered about one
//! directory in two spellings, which is a difference a caller has to paper
//! over and which no documentation promises.
//!
//! **The correct behaviour.** Two halves, and they are separate claims. The
//! printed spelling is the plain one, from `launcher::printed_path`, for both
//! commands. And a test comparing a path `ensure_extracted` answered with one
//! it built by hand compares the directories rather than the spellings, which
//! is `common::hostpath::same_path` — the answer keeps its prefix on purpose,
//! because the launcher goes on to open it.

use std::path::{Path, PathBuf};

use crate::common::hostpath::same_path;
use ginary::launcher::printed_path;

#[test]
fn a_printed_path_is_the_spelling_a_person_and_a_shell_read() {
    assert_eq!(
        printed_path(Path::new(
            r"\\?\C:\Users\ada\AppData\Local\ginary\hello\abc"
        )),
        r"C:\Users\ada\AppData\Local\ginary\hello\abc",
        "standard output is not something ginary opens"
    );
    assert_eq!(
        printed_path(Path::new(r"\\?\UNC\srv\share\ginary\hello\abc")),
        r"\\srv\share\ginary\hello\abc",
        "and a verbatim UNC path has an ordinary spelling too"
    );
}

#[test]
fn a_path_with_no_prefix_and_a_unix_one_are_printed_unchanged() {
    for path in [
        r"C:\Users\ada\AppData\Local\ginary\hello\abc",
        "/home/ada/.cache/ginary/hello/abc",
        "relative/entry",
    ] {
        assert_eq!(printed_path(Path::new(path)), path);
    }
}

#[test]
fn a_verbatim_device_path_keeps_its_prefix_because_it_has_no_other_name() {
    // `winpath::plain_path`'s own rule, restated at the call site that
    // matters: removing the prefix from `\\?\Volume{…}` would change which
    // object the path names rather than shorten it.
    let volume = r"\\?\Volume{9c1b0d5e-0000-0000-0000-100000000000}\ginary";
    assert_eq!(printed_path(Path::new(volume)), volume);
}

#[test]
fn the_two_spellings_of_one_entry_are_one_directory() {
    let verbatim = PathBuf::from(r"\\?\C:\Users\ada\AppData\Local\ginary\hello\abc");
    let plain = PathBuf::from(r"C:\Users\ada\AppData\Local\ginary\hello\abc");

    assert!(
        same_path(&verbatim, &plain),
        "a test that built the entry by hand and one that read it back from \
         `ensure_extracted` are talking about the same directory"
    );
    assert!(
        !same_path(
            &plain,
            Path::new(r"C:\Users\ada\AppData\Local\ginary\other\abc")
        ),
        "and the comparison still tells two directories apart"
    );
    assert!(same_path(
        Path::new("/home/ada/.cache/ginary/hello/abc"),
        Path::new("/home/ada/.cache/ginary/hello/abc")
    ));
}
