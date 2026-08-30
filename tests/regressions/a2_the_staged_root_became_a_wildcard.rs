// SPDX-License-Identifier: MIT OR Apache-2.0
//! The staged root was spliced into a `filelib:wildcard` pattern.
//!
//! **What went wrong.** The beam half of stripping ran
//! `beam_lib:strip_release(Root)`, and `strip_rel/2` in `stdlib` is
//! `strip_fils(filelib:wildcard(filename:join(Root, "lib/*/ebin/*.beam")))`.
//! The root is not a path there, it is the *prefix of a glob*, and
//! `assert_directory/1` passes first because the literal directory does exist.
//! A staged root whose own name holds `*`, `?`, `[`, `]`, `{` or `}` therefore
//! sent the runtime looking somewhere else: `--out '/tmp/build[1]/staged'`
//! matched nothing, ginary's own verification then found the `Dbgi` chunk it
//! had just reported removing and blamed the runtime for it, and
//! `--out '/tmp/build*'` rewrote every `.beam` under every sibling directory
//! whose name began `build` — files ginary was never asked to build.
//!
//! The module documentation reasoned only about Erlang *source* injection —
//! the root travels after `-extra` so a quote in a directory name cannot become
//! an expression — and missed that the value is used as a pattern once it
//! arrives.
//!
//! **The input.** Two trees of one module each, side by side: a root named
//! `out[1]` next to a sibling named `out1`, and a root named `out*` next to a
//! sibling named `outer`. Every module is the same unstripped fixture, so
//! "was it stripped" and "was it left alone" are both answerable byte for byte.
//!
//! **The correct behaviour.** The modules inside the root ginary was given are
//! stripped, whatever the root is called, and no file outside it is touched.

use std::path::{Path, PathBuf};

use ginary::beam::{CODE_CHUNK, DEBUG_INFO_CHUNK};
use ginary::strip::{self, StripOptions};

use crate::common::tools::require_tools;

/// The unstripped module every tree in this file is built from.
fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/beam/gleam@list.beam")
}

/// Writes `<root>/lib/notify/ebin/gleam@list.beam` and answers its path.
fn tree(root: &Path) -> PathBuf {
    let ebin = root.join("lib/notify/ebin");
    std::fs::create_dir_all(&ebin).expect("an ebin directory");
    let module = ebin.join("gleam@list.beam");
    std::fs::copy(fixture(), &module).expect("the fixture copies");
    module
}

/// Whether the module at `path` still carries its debug information.
fn has_debug_info(path: &Path) -> bool {
    let bytes = std::fs::read(path).expect("a readable module");
    assert!(
        ginary::beam::has_chunk(&bytes, &CODE_CHUNK),
        "{} lost its Code chunk",
        path.display()
    );
    ginary::beam::has_chunk(&bytes, &DEBUG_INFO_CHUNK)
}

#[test]
fn a_root_named_with_a_bracket_still_has_its_own_modules_stripped() {
    let Some(_tools) = require_tools(&["erl"]) else {
        return;
    };
    let otp = ginary::otp::discover(None).expect("the host OTP installation");
    let dir = tempfile::tempdir().expect("a temporary directory");
    let staged = tree(&dir.path().join("out[1]"));
    let neighbour = tree(&dir.path().join("out1"));
    let neighbour_before = std::fs::read(&neighbour).expect("a readable module");

    let report = strip::strip(
        &dir.path().join("out[1]"),
        &otp,
        &StripOptions {
            elf: false,
            beams: true,
        },
    )
    .unwrap_or_else(|error| panic!("stripping a root named `out[1]` should succeed: {error}"));

    assert!(
        !has_debug_info(&staged),
        "the module inside the root ginary was given was not stripped: {report}"
    );
    assert_eq!(
        std::fs::read(&neighbour).expect("a readable module"),
        neighbour_before,
        "a directory outside the staged root was rewritten"
    );
}

#[test]
fn a_root_named_with_a_star_leaves_its_neighbours_alone() {
    let Some(_tools) = require_tools(&["erl"]) else {
        return;
    };
    let otp = ginary::otp::discover(None).expect("the host OTP installation");
    let dir = tempfile::tempdir().expect("a temporary directory");
    let staged = tree(&dir.path().join("out*"));
    let neighbour = tree(&dir.path().join("outer"));
    let neighbour_before = std::fs::read(&neighbour).expect("a readable module");

    let report = strip::strip(
        &dir.path().join("out*"),
        &otp,
        &StripOptions {
            elf: false,
            beams: true,
        },
    )
    .unwrap_or_else(|error| panic!("stripping a root named `out*` should succeed: {error}"));

    assert!(
        !has_debug_info(&staged),
        "the module inside the root ginary was given was not stripped: {report}"
    );
    assert_eq!(
        std::fs::read(&neighbour).expect("a readable module"),
        neighbour_before,
        "`out*` reached `outer`, which ginary was never asked to build"
    );
}
