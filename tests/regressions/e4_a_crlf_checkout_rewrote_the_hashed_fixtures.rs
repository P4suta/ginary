// SPDX-License-Identifier: MIT OR Apache-2.0
//! The digest vectors over the committed `hello_ffi` fixture were pinned to
//! bytes a checkout is allowed to rewrite.
//!
//! **What went wrong.** E4 added `tests/digest.rs`, whose
//! `the_committed_hello_ffi_fixture_hashes_to_its_recorded_digests` snapshots
//! the size and the SHA-256 of five committed *text* files —
//! `tests/snapshots/digest__hello_ffi_fixture_digests.snap` pins
//! `priv/greeting.txt 16 7b01fa9f...`, and four more beside it. The repository
//! carried no `.gitattributes`, so nothing forced the working-tree line
//! endings: on any checkout with `core.autocrlf=true` — the Git-for-Windows
//! default, and the setting the hosted `windows-2022` runner that ci.yml's
//! `windows` job runs `cargo test --locked` on inherits — git rewrites every
//! `\n` in those five files to `\r\n`. `priv/greeting.txt` is then 17 bytes
//! with a different digest, and so are `gleam.toml` (572), `manifest.toml`
//! (321), `src/hello_ffi.gleam` (614) and `src/hello_ffi_ffi.erl` (1523).
//!
//! The failure is the worst shape a digest test has: an opaque snapshot diff,
//! on one platform only, over a file nobody edited, reported by the one target
//! whose whole purpose is to prove that a *digest* did not move. The vectors
//! would look wrong when what actually changed was the checkout.
//!
//! **The input.** The repository itself, checked out with `core.autocrlf`
//! left at its platform default.
//!
//! **The correct behaviour.** A committed `.gitattributes` makes the
//! working-tree bytes of every hashed fixture the bytes in the object, so a
//! checkout setting cannot invalidate a published digest. This file holds the
//! record to that: the rules are committed and they cover
//! `tests/fixtures/`, and no file `tests/digest.rs` hashes carries a carriage
//! return in the working tree the suite is reading.

use crate::common::repo::{read, read_opt};

/// The committed fixture files `tests/digest.rs` hashes into
/// `tests/snapshots/digest__hello_ffi_fixture_digests.snap`.
///
/// Named rather than walked, for the reason `tests/digest.rs` names them:
/// `tests/fixtures/*/build/` is git-ignored, so a walk would pick up whatever
/// the last `gleam export erlang-shipment` left behind.
const HASHED_FIXTURES: [&str; 5] = [
    "tests/fixtures/hello_ffi/gleam.toml",
    "tests/fixtures/hello_ffi/manifest.toml",
    "tests/fixtures/hello_ffi/priv/greeting.txt",
    "tests/fixtures/hello_ffi/src/hello_ffi.gleam",
    "tests/fixtures/hello_ffi/src/hello_ffi_ffi.erl",
];

/// One `.gitattributes` rule: the pattern, and the attributes it sets.
struct Rule {
    pattern: String,
    attributes: Vec<String>,
}

/// Parses `.gitattributes` into its rules, dropping blank and comment lines.
fn rules(text: &str) -> Vec<Rule> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let pattern = fields.next()?.to_owned();
            Some(Rule {
                pattern,
                attributes: fields.map(str::to_owned).collect(),
            })
        })
        .collect()
}

/// Whether a `.gitattributes` pattern covers a repository-relative path.
///
/// Deliberately narrow: `*` and `**` match everything, a pattern ending in
/// `/*` or `/**` matches everything below its prefix, and anything else has to
/// be the path itself. A pattern this does not understand is reported as not
/// covering, which fails the test rather than passing it — the safe direction
/// for a guard whose whole job is to notice that nothing pins the bytes.
fn covers(pattern: &str, path: &str) -> bool {
    if pattern == "*" || pattern == "**" {
        return true;
    }
    for suffix in ["/**", "/*"] {
        if let Some(prefix) = pattern.strip_suffix(suffix) {
            return path.starts_with(&format!("{prefix}/"));
        }
    }
    pattern == path
}

/// Whether an attribute list stops git rewriting a checked-out file's line
/// endings: `-text` (never convert) or an explicit `eol=lf`.
fn pins_line_endings(attributes: &[String]) -> bool {
    attributes
        .iter()
        .any(|attribute| attribute == "-text" || attribute == "eol=lf")
}

#[test]
fn a_committed_gitattributes_pins_the_bytes_of_every_hashed_fixture() {
    let text = read_opt(".gitattributes").unwrap_or_else(|| {
        panic!(
            "the repository has no `.gitattributes`, so the working-tree bytes of every file \
             `tests/digest.rs` hashes are whatever the checkout's `core.autocrlf` makes them. On \
             a Windows checkout — the default there, and what ci.yml's `windows` job runs \
             `cargo test --locked` on — the five hello_ffi fixtures come out with CRLF, a byte \
             longer per line and a different SHA-256, and the published digest snapshot fails \
             for a reason that has nothing to do with hashing."
        )
    });
    let parsed = rules(&text);
    for path in HASHED_FIXTURES {
        assert!(
            parsed
                .iter()
                .any(|rule| covers(&rule.pattern, path) && pins_line_endings(&rule.attributes)),
            "`.gitattributes` has no rule that pins the line endings of `{path}`, a file whose \
             SHA-256 is published in tests/snapshots/digest__hello_ffi_fixture_digests.snap. A \
             rule needs `-text` or `eol=lf` and a pattern that covers the path; the file holds:\n\
             {text}"
        );
    }
}

#[test]
fn no_hashed_fixture_carries_a_carriage_return_in_this_working_tree() {
    for path in HASHED_FIXTURES {
        let bytes = read(path).into_bytes();
        assert!(
            !bytes.windows(2).any(|pair| pair == b"\r\n"),
            "`{path}` holds CRLF in this working tree, so its size and SHA-256 are not the ones \
             tests/snapshots/digest__hello_ffi_fixture_digests.snap publishes. The file is \
             committed with LF; a checkout rewrote it. Set `core.autocrlf=false` (or `input`) \
             and check it out again — `.gitattributes` pins this, so a checkout that ignored it \
             predates the rule."
        );
    }
}
