// SPDX-License-Identifier: MIT OR Apache-2.0
//! Nothing asserted that an artifact was made out of the stub that was located.
//!
//! **What went wrong.** C2's headline claim is that a build for a target other
//! than this machine locates a stub, proves it, and then *uses its bytes* in
//! place of the running executable. `stub::locate` and `stub::verify` were
//! pinned thoroughly; the line that spends their answer was not. A host build
//! uses the self executable anyway, and the one cross build that got past
//! `verify` was asserted to fail at the runtime, so the assembler was never
//! reached with a foreign file. Reverting `assemble_and_write` to the self
//! executable therefore broke no test: the artifact would have been this
//! machine's `ginary` with somebody else's target in its manifest, and the
//! suite would have been green.
//!
//! **The input.** A real build of `hello_ffi` with `--stub` naming a
//! *cross-built* stub for this machine — the one `mise run stubs:build` writes
//! into `target/stubs`. It is byte-for-byte a different file from the `ginary`
//! that runs the build (a different size, a different flavor in its marker),
//! and it is a valid host stub, so the build must succeed with it.
//!
//! **The correct behaviour.** The artifact's leading bytes are the stub file,
//! exactly; the payload starts where the stub ends; and what comes out runs.
//!
//! Gated twice, on the toolchain and on `target/stubs` being populated, for
//! the reason `docs/dev/testing.md` gives: a claim about a real cross-built
//! ELF cannot be made on a machine that has not got one, and a skip says so.

#![cfg(unix)]
#![cfg(feature = "cli")]

use ginary::target::Target;

use crate::common::built::BuiltProject;
use crate::common::stubfile;
use crate::common::tools::require_tools;

/// The fixture this file builds.
const APP: &str = "hello_ffi";

#[test]
fn the_artifact_begins_with_the_stub_the_build_was_given() {
    let Some(_tools) = require_tools(&["gleam", "erl", "strip"]) else {
        return;
    };
    let host = Target::host();
    let Some(stub) = stubfile::cross_stub(&host) else {
        return;
    };
    let stub_bytes = std::fs::read(&stub).expect("the cross-built stub is readable");

    let project = BuiltProject::copy(APP);
    let output = project.build_with(&["--stub", &stub.display().to_string()], &[]);
    assert!(
        output.status.success(),
        "a valid host stub is a stub: {}\n--- stderr ---\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr),
    );

    let artifact = project.artifact();
    let artifact_bytes = std::fs::read(&artifact).expect("the artifact is readable");
    assert!(
        artifact_bytes.len() > stub_bytes.len(),
        "the artifact is the stub plus a payload and a trailer: {} against {}",
        artifact_bytes.len(),
        stub_bytes.len()
    );
    assert!(
        artifact_bytes[..stub_bytes.len()] == stub_bytes[..],
        "the artifact's first {} bytes are the stub that was named, and they are not; the \
         build used some other file",
        stub_bytes.len()
    );

    // The other half of the claim: bytes that match are not enough, because a
    // stub the build copied and then wrote the payload at the wrong offset of
    // would match too. What comes out has to start.
    let run = project.run("stub-bytes").output();
    assert_eq!(run.code(), 0, "{}", run.stderr());
    assert!(
        run.stdout().contains("hello from priv"),
        "the artifact built on the located stub runs: {}",
        run.stdout()
    );
}
