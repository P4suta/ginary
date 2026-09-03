// SPDX-License-Identifier: MIT OR Apache-2.0
//! A staging report quoted a path with the platform's own separator, so the
//! junk table named files that appear nowhere in the listing it belongs to.
//!
//! **What went wrong.** On the first Windows runner the junk table and the
//! `stage --explain` snapshot came back with the separator the operating
//! system spells paths with, spliced onto the `/`-separated application
//! directory the listing already held:
//!
//! ```text
//! ---- the_junk_files_are_removed_and_recorded_with_their_sizes stdout ----
//!   left: [("lib/crypto-5.9.2\\priv\\lib\\libcrypto_static.a", 21), ...]
//!  right: [("lib/crypto-5.9.2/priv/lib/libcrypto_static.a", 21), ...]
//! ```
//!
//! (`Windows build and exit-code propagation`
//! <https://github.com/P4suta/ginary/actions/runs/33712111530/job/100513644404>.)
//!
//! **The input.** Any build on Windows. The path was built by joining an
//! `OsStr` walked off the filesystem onto a listing path instead of being
//! respelled the way [`ginary::assemble`] respells every path it writes into
//! `ginary.stage.json`.
//!
//! **The correct behaviour.** Every path a document or a report carries is
//! `/`-separated, because it is matched against `ginary.stage.json` and
//! `ginary.index.json` and those are read on every platform.
//! [`ginary::winpath::slash_path_str`] is that respelling, and it lives beside
//! [`ginary::winpath::long_path_str`] for the same reason: it is Windows path
//! syntax rather than a system call, so it is checkable here. It is applied
//! only to a Windows path — `\` is an ordinary character in a unix file name,
//! and rewriting one would rename the file rather than respell it.

use ginary::winpath::slash_path_str;

#[test]
fn a_windows_relative_path_is_respelled_the_way_the_listing_carries_it() {
    assert_eq!(
        slash_path_str(r"lib/crypto-5.9.2\priv\lib\libcrypto_static.a"),
        "lib/crypto-5.9.2/priv/lib/libcrypto_static.a",
        "the mixed spelling the runner produced is one path, and the listing has one way of \
         writing it"
    );
    assert_eq!(
        slash_path_str(r"lib\hello\ebin\hello.beam"),
        "lib/hello/ebin/hello.beam"
    );
}

#[test]
fn a_path_that_is_already_a_listing_path_is_handed_back_unchanged() {
    assert_eq!(
        slash_path_str("lib/hello/ebin/hello.beam"),
        "lib/hello/ebin/hello.beam"
    );
    assert_eq!(slash_path_str(""), "");
    assert_eq!(slash_path_str("hello.beam"), "hello.beam");
}
