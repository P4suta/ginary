// SPDX-License-Identifier: MIT OR Apache-2.0
//! A `.beam` outside `lib/<app>/ebin/` was verified and never stripped.
//!
//! **What went wrong.** The two halves of the beam step disagreed about which
//! modules they were talking about. ginary counted, measured and verified every
//! `.beam` in the staged tree, while `beam_lib:strip_release/1` rewrites only
//! what `filelib:wildcard(filename:join(Root, "lib/*/ebin/*.beam"))` matches. A
//! shipment that ships a module under its `priv` — a helper an application
//! loads by path, which is a shape OTP allows — was therefore never handed to
//! the runtime, kept its `Dbgi`, and was then held against the runtime:
//! `ginary stage` aborted with "still holds the `Dbgi` chunk after
//! beam_lib:strip_release/1 reported success", sending the reader after a
//! runtime bug that does not exist. `BeamOutcome::Stripped { files }` also
//! counted modules the tool was never asked about.
//!
//! **The input.** A tree holding one module under `lib/notify/ebin` and one
//! under `lib/notify/priv`, both the same unstripped fixture.
//!
//! **The correct behaviour.** The set ginary verifies is the set the runtime
//! was asked to rewrite. Both modules are stripped and both are counted.

use std::path::{Path, PathBuf};

use ginary::beam::{CODE_CHUNK, DEBUG_INFO_CHUNK, DOCS_CHUNK};
use ginary::strip::{self, BeamOutcome, StripOptions};

use crate::common::tools::require_tools;

/// The unstripped module both copies are made from.
fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/beam/gleam@list.beam")
}

/// Copies the fixture to `<root>/<relative>`.
fn copy_to(root: &Path, relative: &str) -> PathBuf {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("a directory");
    std::fs::copy(fixture(), &path).expect("the fixture copies");
    path
}

/// Asserts that `path` is a module that kept its byte code and lost its debug
/// information.
fn assert_stripped(path: &Path) {
    let bytes = std::fs::read(path).expect("a readable module");
    assert!(
        ginary::beam::has_chunk(&bytes, &CODE_CHUNK),
        "{} lost its Code chunk",
        path.display()
    );
    assert!(
        !ginary::beam::has_chunk(&bytes, &DEBUG_INFO_CHUNK),
        "{} still holds its Dbgi chunk",
        path.display()
    );
    assert!(
        !ginary::beam::has_chunk(&bytes, &DOCS_CHUNK),
        "{} still holds its Docs chunk",
        path.display()
    );
}

#[test]
fn a_module_under_priv_is_stripped_like_one_under_ebin() {
    let Some(_tools) = require_tools(&["erl"]) else {
        return;
    };
    let otp = ginary::otp::discover(None).expect("the host OTP installation");
    let dir = tempfile::tempdir().expect("a temporary directory");
    let root = dir.path().join("out");
    let in_ebin = copy_to(&root, "lib/notify/ebin/gleam@list.beam");
    let in_priv = copy_to(&root, "lib/notify/priv/helper.beam");

    let report = strip::strip(
        &root,
        &otp,
        &StripOptions {
            elf: false,
            beams: true,
        },
    )
    .unwrap_or_else(|error| panic!("stripping should succeed: {error}"));

    assert_stripped(&in_ebin);
    assert_stripped(&in_priv);
    match report.beams {
        BeamOutcome::Stripped { files, .. } => assert_eq!(
            files, 2,
            "the count has to name modules the runtime was actually given"
        ),
        other => panic!("expected the beam step to run, got {other:?}"),
    }
}
