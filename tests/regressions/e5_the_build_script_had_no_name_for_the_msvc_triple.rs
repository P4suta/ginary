// SPDX-License-Identifier: MIT OR Apache-2.0
//! `build.rs` had no target name for `x86_64-pc-windows-msvc`, so ginary could
//! not be built on a Windows machine at all.
//!
//! **What went wrong.** `build.rs` maps Cargo's `TARGET` to the canonical
//! `<os>-<arch>` name that goes into the embedded identity marker, and a triple
//! it does not know is a deliberate `panic!` rather than a guess. Its table
//! held the seven triples ginary *distributes* for, and the Windows one of
//! those is `x86_64-pc-windows-gnu`, because the released Windows stub is
//! cross-compiled from Linux. The default host triple on a Windows machine —
//! and on the `windows-2022` runner — is `x86_64-pc-windows-msvc`, which is not
//! in the table, so the very first step of the `windows` job died in the build
//! script:
//!
//! ```text
//! ginary has no target name for the Rust triple `x86_64-pc-windows-msvc`
//! ```
//!
//! (run <https://github.com/P4suta/ginary/actions/runs/33658759531>). Every
//! later step of that job — both flavors, the native launcher unit tests, the
//! exit-code propagation proof that closes the D2 wine gap — was skipped, so
//! the one thing the job exists for has still never run.
//!
//! **The input.** `cargo build` on any Windows host with the default MSVC
//! toolchain.
//!
//! **The correct behaviour.** The marker names the *platform* an artifact runs
//! on, not the ABI it was linked with, so both Windows triples map to
//! `windows-x86_64`. The mapping stops being one-to-one, which is a fact about
//! Windows rather than a hole: `Target::rust_triple` keeps naming the `-gnu`
//! triple as the one releases are built with, and `build.rs` accepts either as
//! an input. A triple that is still unknown stays a `panic!`.

/// The build script, read as text.
///
/// A test cannot invoke a build script, and re-running one under `cargo test`
/// would answer for this machine's triple only. The table is a committed
/// constant, so the table is what is held. `tests/target.rs` and
/// `src/target.rs` hold the other direction.
const BUILD_SCRIPT: &str = include_str!("../../build.rs");

/// The `(rust triple, ginary target)` rows of `build.rs`'s `TARGETS` table.
fn rows() -> Vec<(String, String)> {
    let body = BUILD_SCRIPT
        .split_once("const TARGETS")
        .expect("build.rs declares a `TARGETS` table")
        .1
        .split_once('[')
        .expect("the table is a literal array")
        .1
        .split_once("];")
        .expect("the array is closed")
        .0;
    body.lines()
        .filter_map(|line| {
            let mut quoted = line.split('"');
            let _before = quoted.next()?;
            let triple = quoted.next()?;
            let _between = quoted.next()?;
            let name = quoted.next()?;
            Some((triple.to_owned(), name.to_owned()))
        })
        .collect()
}

#[test]
fn the_build_script_names_the_triple_a_windows_host_actually_builds() {
    let rows = rows();
    assert!(
        rows.len() >= 7,
        "the TARGETS table did not parse; rows found: {rows:?}"
    );
    let msvc: Vec<&(String, String)> = rows
        .iter()
        .filter(|(triple, _)| triple == "x86_64-pc-windows-msvc")
        .collect();
    assert_eq!(
        msvc,
        [&(
            "x86_64-pc-windows-msvc".to_owned(),
            "windows-x86_64".to_owned()
        )],
        "`cargo build` on a Windows host passes TARGET=x86_64-pc-windows-msvc, and build.rs \
         panics on a triple it has no name for. The table holds: {rows:?}"
    );
    let declared = BUILD_SCRIPT
        .split_once("const TARGETS: [(&str, &str); ")
        .expect("the table declares its own length")
        .1
        .split_once(']')
        .expect("a closed length")
        .0;
    assert_eq!(
        declared,
        rows.len().to_string(),
        "the declared length of TARGETS and the number of rows in it disagree"
    );
}
