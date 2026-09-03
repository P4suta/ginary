// SPDX-License-Identifier: MIT OR Apache-2.0
//! The shared test helpers reached into the unix-only standard library at file
//! scope, so `cargo test` on Windows never compiled a single test.
//!
//! **What went wrong.** E5 taught `build.rs` the `x86_64-pc-windows-msvc`
//! triple, so the crate itself finally built on a Windows runner — both
//! flavors, release, `--locked`, all green. The step after it did not:
//!
//! ```text
//! error[E0433]: cannot find `unix` in `os`
//!   --> tests\common\artifact.rs:32:14
//!    |
//! 32 | use std::os::unix::ffi::{OsStrExt, OsStringExt};
//!    |              ^^^^ could not find `unix` in `os`
//! ...
//! error: could not compile `ginary` (test "stubid") due to 15 previous errors
//! ```
//!
//! (`Windows build and exit-code propagation`
//! <https://github.com/P4suta/ginary/actions/runs/33681144884/job/100417745912>).
//! Fifteen errors across four helper modules — `artifact.rs`, `payload.rs`,
//! `repack.rs`, `stubfile.rs` — and cargo abandoned the build with the other
//! test targets still queued, so the count is a floor and not a total. The
//! `Assert exit-code propagation` step, which is the whole reason the job
//! exists and the last unclosed hole D2 left, was skipped: nothing about the
//! Windows launcher was proved by this run.
//!
//! Every one of the fifteen is the same shape. `std::os::unix` does not exist
//! on Windows, and `OsStringExt::from_vec`, `PermissionsExt::from_mode`,
//! `Permissions::mode` and `OsStrExt::as_bytes` are the methods those imports
//! bring in. On Linux they compile and nothing says they are unix-only; on
//! Windows they are the entire error list.
//!
//! **The input.** `cargo test` on any Windows host. There is no other way to
//! see it, which is exactly why this is a scan and not a compile: no Windows
//! toolchain is reachable from the developer machine or from the Linux jobs,
//! and `cargo check --target x86_64-pc-windows-gnu` cannot stand in for one
//! because the C sources of `zstd-sys` want a mingw compiler that is not there
//! either.
//!
//! **The correct behaviour.** Every mention of the unix-only standard library
//! in the test tree sits under a `cfg(unix)` gate — an inner attribute on a
//! file that is wholly about unix, an outer attribute on the item, or an
//! attribute on the enclosing block, whichever fits. `tests/common/script.rs`
//! already does it the third way and is the model. Gating an import drags its
//! call sites under the same gate, because a trait method is only callable
//! where the trait is in scope, so the file stops compiling on Linux the
//! moment the gate and the uses disagree — which is what makes a scan over
//! the import sites worth running.
//!
//! It does not make it sufficient, and the limit is written down rather than
//! argued away: a call to something *already* gated names no `os::unix`, so
//! two ungated calls of the `cfg(unix)` `cache::prepare` were invisible to this
//! test and fatal on Windows. A real cross-compile catches both and needs only
//! docker and `mingw-w64`; see `tests/common/portability.rs` for the recipe.
//! This scan is the half that runs in every suite, on every machine, in
//! milliseconds.
//!
//! The scanner is [`crate::common::portability::unix_sites`], a pure function
//! over one file's text, asserted below against source it is handed before it
//! is turned loose on the tree.

use crate::common::portability::{collect_tracked_sources, tracked_test_sources, unix_sites};

/// A file-scope reach, a block-gated one and an item-gated one.
const MIXED: &str = r##"
use std::os::unix::ffi::OsStrExt;

fn mode(path: &Path) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        return path.metadata().expect("stat").permissions().mode();
    }
    0
}

#[cfg(unix)]
fn link(from: &Path, to: &Path) {
    std::os::unix::fs::symlink(from, to).expect("the symlink");
}
"##;

/// A whole file gated by an inner attribute, the spelling a wholly unix test
/// file wants.
const INNER: &str = r##"
#![cfg(all(unix, feature = "cli"))]

use std::os::unix::fs::PermissionsExt;
"##;

/// A gated block whose braces are outnumbered by braces that are not braces.
const LITERALS: &str = r##"
#[cfg(unix)]
fn gated() {
    // a line comment with a } in it
    let message = "an opening { and a quote \" and a closing }";
    let raw = r#"a raw } string holding a " of its own"#;
    let brace = '}';
    use std::os::unix::fs::PermissionsExt as _;
}

use std::os::unix::ffi::OsStrExt;
"##;

/// The shape almost every gated test in this suite has: the gate, then
/// `#[test]`, then the item.
const GATE_THEN_TEST: &str = r##"
#[cfg(unix)]
#[test]
fn a_symlink_is_copied_as_a_file() {
    std::os::unix::fs::symlink("greeting.txt", link).expect("the symlink");
}

#[test]
fn an_ordinary_test() {
    use std::os::unix::fs::PermissionsExt as _;
}
"##;

/// A block gated the other way round.
const WINDOWS_GATE: &str = r##"
#[cfg(windows)]
fn only_on_windows() {
    use std::os::unix::fs::PermissionsExt as _;
}

#[cfg(not(unix))]
fn not_on_unix() {
    use std::os::unix::ffi::OsStrExt;
}
"##;

#[test]
fn a_reach_at_file_scope_is_ungated_and_one_inside_a_cfg_unix_item_is_not() {
    let sites = unix_sites("mixed.rs", MIXED);
    let seen: Vec<(usize, bool)> = sites.iter().map(|site| (site.line, site.gated)).collect();
    assert_eq!(
        seen,
        vec![(2, false), (7, true), (15, true)],
        "the import at file scope is the one Windows cannot read; the block attribute and the \
         item attribute both cover theirs:\n{}",
        render(&sites)
    );
}

#[test]
fn an_inner_attribute_gates_the_whole_file_it_stands_in() {
    let sites = unix_sites("inner.rs", INNER);
    let seen: Vec<(usize, bool)> = sites.iter().map(|site| (site.line, site.gated)).collect();
    assert_eq!(
        seen,
        vec![(4, true)],
        "`#![cfg(all(unix, ..))]` is how a wholly unix test file says so, and it covers every \
         line under it:\n{}",
        render(&sites)
    );
}

#[test]
fn a_brace_in_a_comment_a_string_or_a_character_literal_does_not_close_a_gate() {
    // The rule below is enforced by counting braces, and this suite writes
    // braces inside `assert!` messages, inside raw strings and as character
    // literals on almost every page. A scanner that counted those would report
    // the gate closed three lines early and call a covered import naked — or,
    // worse, report an uncovered one as gated and let the next Windows job
    // fail the same way.
    let sites = unix_sites("literals.rs", LITERALS);
    let seen: Vec<(usize, bool)> = sites.iter().map(|site| (site.line, site.gated)).collect();
    assert_eq!(
        seen,
        vec![(8, true), (11, false)],
        "the import inside the gated function is covered and the one after the closing brace is \
         not:\n{}",
        render(&sites)
    );
}

#[test]
fn a_second_attribute_between_the_gate_and_the_item_does_not_drop_the_gate() {
    // `#[cfg(unix)]` is followed by `#[test]` on almost every gated test in
    // this suite, and a scanner that treated that second attribute as the
    // start of the item would report all of them as naked — seven of the
    // forty-three this scan first counted were exactly that, and "fixing"
    // them would have meant gating what was already gated.
    let sites = unix_sites("gate_then_test.rs", GATE_THEN_TEST);
    let seen: Vec<(usize, bool)> = sites.iter().map(|site| (site.line, site.gated)).collect();
    assert_eq!(
        seen,
        vec![(5, true), (10, false)],
        "the gate reaches over `#[test]` to the function under it, and stops at the next item \
         that has no gate of its own:\n{}",
        render(&sites)
    );
}

#[test]
fn a_windows_gate_and_a_negated_unix_gate_cover_nothing() {
    let sites = unix_sites("windows.rs", WINDOWS_GATE);
    let seen: Vec<(usize, bool)> = sites.iter().map(|site| (site.line, site.gated)).collect();
    assert_eq!(
        seen,
        vec![(4, false), (9, false)],
        "`cfg(windows)` and `cfg(not(unix))` are where the unix-only standard library is least \
         available, not most:\n{}",
        render(&sites)
    );
}

#[test]
fn no_tracked_test_source_reaches_the_unix_standard_library_outside_a_unix_gate() {
    let Some(sources) = tracked_test_sources() else {
        eprintln!("skipping: `git ls-files` did not answer, so `tracked` would be a guess");
        return;
    };
    assert!(
        sources.unreadable.is_empty(),
        "the scan could not read {:?}. A file it never read is a file it has no answer for, and \
         reporting it among the clean ones is the silent skip this scan exists to replace",
        sources.unreadable
    );
    let files = &sources.files;
    assert!(
        files.len() > 50,
        "only {} tracked `.rs` files under `tests/` were found; this scan has lost its subject",
        files.len()
    );

    let mut offenders = Vec::new();
    let mut gated = 0usize;
    for (relative, text) in files {
        for site in unix_sites(relative, text) {
            if site.gated {
                gated += 1;
            } else {
                offenders.push(site.to_string());
            }
        }
    }
    assert!(
        gated > 0,
        "not one reach into the unix-only standard library is gated anywhere under `tests/`, \
         which means the scanner found nothing rather than that the tree is clean"
    );
    assert!(
        offenders.is_empty(),
        "{} reach{} into `std::os::unix` under `tests/` are not covered by a `cfg(unix)` gate. \
         None of them compiles on Windows, and `cargo test` there stops at the first file that \
         holds one:\n{}",
        offenders.len(),
        if offenders.len() == 1 { "" } else { "es" },
        offenders.join("\n")
    );
}

/// An escaped-quote character literal, then a brace that closes a gate.
///
/// `'\''` is four characters, and a lexer that returns the index of its
/// closing tick rather than the index after it re-enters character-literal
/// handling on that tick and swallows whatever follows — including the brace
/// the gate stack is counting.
const ESCAPED_TICK: &str = r##"
#[cfg(unix)]
fn gated() {
    let quote = matches!(c, '\''|'{');
    use std::os::unix::fs::PermissionsExt as _;
}

use std::os::unix::ffi::OsStrExt;
"##;

#[test]
fn an_escaped_quote_character_literal_does_not_swallow_the_brace_after_it() {
    let sites = unix_sites("escaped.rs", ESCAPED_TICK);
    let seen: Vec<(usize, bool)> = sites.iter().map(|site| (site.line, site.gated)).collect();
    assert_eq!(
        seen,
        vec![(5, true), (8, false)],
        "`'\\''` is a character literal and `'{{'` is another; a lexer that counts the `{{` \
         between them pushes a gate that is never popped, and every later import in the file \
         reads as covered when it is not:\n{}",
        render(&sites)
    );
}

#[test]
fn a_tracked_file_the_scan_cannot_read_is_reported_rather_than_dropped() {
    // The two shapes `git ls-files -z` can hand a scanner that no `String`
    // holds: a name whose bytes are not UTF-8 — this repository writes such
    // paths deliberately, see
    // `tests/regressions/a4_a_non_utf8_output_path_failed_the_json_report.rs`
    // — and a name that is fine but whose contents cannot be read as text.
    let mut listing: Vec<u8> = Vec::new();
    listing.extend_from_slice(b"tests/readable.rs\0");
    listing.extend_from_slice(b"tests/inval\xffid.rs\0");
    listing.extend_from_slice(b"tests/vanished.rs\0");
    listing.extend_from_slice(b"tests/fixtures/not_rust.txt\0");
    let collected = collect_tracked_sources(&listing, &|name| {
        (name == "tests/readable.rs").then(|| "fn main() {}\n".to_owned())
    });
    assert_eq!(
        collected.files,
        vec![("tests/readable.rs".to_owned(), "fn main() {}\n".to_owned())]
    );
    assert_eq!(
        collected.unreadable,
        vec![
            "tests/inval\u{fffd}id.rs".to_owned(),
            "tests/vanished.rs".to_owned(),
        ],
        "a tracked source the scan never read must reach the caller as unreadable; dropped, it \
         is indistinguishable from a file that was scanned and found clean"
    );
}

/// Every site of one scan, one per line, for a failure that can be read.
fn render(sites: &[crate::common::portability::UnixSite]) -> String {
    sites
        .iter()
        .map(|site| format!("  line {} gated={}: {}", site.line, site.gated, site.text))
        .collect::<Vec<_>>()
        .join("\n")
}
