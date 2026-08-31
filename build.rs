// SPDX-License-Identifier: MIT OR Apache-2.0
//! The two facts a ginary binary cannot read off itself at run time.
//!
//! `src/stubid.rs` embeds an identity marker naming the version, the target,
//! the payload format and the flavor of the build. Three of those four are
//! knowable inside the crate — `CARGO_PKG_VERSION` is an environment variable
//! Cargo already sets, and `manifest::FORMAT_VERSION` is a constant, which is
//! why this script deliberately does *not* emit it: one source of truth per
//! fact. The other two are known only here:
//!
//! - `GINARY_TARGET`, the canonical `<os>-<arch>[-<libc>]` name for the Rust
//!   triple this build is for. `std::env::consts` would answer for the host,
//!   and a cross-compiled stub is exactly the case where the host is the wrong
//!   answer, so the mapping is done from Cargo's own `TARGET`.
//! - `GINARY_FLAVOR`, `full` when the `cli` feature is on and `stub` when it
//!   is not, from `CARGO_FEATURE_CLI`.
//!
//! A triple ginary has no target name for is a compile error rather than a
//! guess: a stub whose marker named the wrong target would be located,
//! verified and packaged, and the artifact would fail on the machine it was
//! built for.

use std::env;

/// The seven Rust triples ginary builds for, and the names it calls them.
///
/// The same mapping as `Target::rust_triple`, read the other way round. It is
/// duplicated here rather than shared because a build script cannot use the
/// crate it is building, and `tests/target.rs` holds the mapping this table is
/// the inverse of.
const TARGETS: [(&str, &str); 7] = [
    ("x86_64-unknown-linux-gnu", "linux-x86_64-gnu"),
    ("x86_64-unknown-linux-musl", "linux-x86_64-musl"),
    ("aarch64-unknown-linux-gnu", "linux-aarch64-gnu"),
    ("aarch64-unknown-linux-musl", "linux-aarch64-musl"),
    ("x86_64-apple-darwin", "macos-x86_64"),
    ("aarch64-apple-darwin", "macos-aarch64"),
    ("x86_64-pc-windows-gnu", "windows-x86_64"),
];

fn main() {
    // Nothing but this file and the two variables below decides the output, so
    // a rebuild is needed only when the file itself changes; without this line
    // Cargo reruns the script whenever any source does.
    println!("cargo:rerun-if-changed=build.rs");

    let triple = env::var("TARGET").expect("cargo sets TARGET for every build script");
    let name = TARGETS
        .iter()
        .find(|(rust, _)| *rust == triple)
        .map(|(_, name)| *name)
        .unwrap_or_else(|| {
            panic!(
                "ginary has no target name for the Rust triple `{triple}`; it builds for {}. \
                 Add the triple to `target::Target::rust_triple` and to the table in build.rs \
                 before building for it.",
                TARGETS
                    .iter()
                    .map(|(rust, _)| *rust)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        });
    println!("cargo:rustc-env=GINARY_TARGET={name}");

    // Cargo sets `CARGO_FEATURE_<NAME>` for every enabled feature, so the
    // presence of the variable is the whole question.
    let flavor = if env::var_os("CARGO_FEATURE_CLI").is_some() {
        "full"
    } else {
        "stub"
    };
    println!("cargo:rustc-env=GINARY_FLAVOR={flavor}");
}
