// SPDX-License-Identifier: MIT OR Apache-2.0
//! Finding the unix-only code in the test tree that a Windows compiler cannot
//! read.
//!
//! The suite is compiled on three operating systems and executed on all of
//! them. Everything under `src/` already carries that: `cache_lock`, `launch`
//! and `launch_windows` are split by `cfg`, and `cargo check
//! --no-default-features` for the msvc triple has been green since E5. The
//! *tests* never were, because until that build script fix nothing on Windows
//! ever got as far as compiling them. The first run that did found seven
//! ungated reaches into the unix-only half of the standard library in
//! `tests/common` alone, and cargo stopped there:
//!
//! ```text
//! error[E0433]: cannot find `unix` in `os`
//!   --> tests\common\artifact.rs:32:14
//! ```
//!
//! A reach into that module is invisible on Linux and fatal on Windows, which
//! is exactly the shape of defect a scan is for. The rule this module states
//! is the one the tree already follows where it follows it at all — see
//! `tests/common/script.rs`, which imports `PermissionsExt` inside a
//! `cfg(unix)` block rather than at file scope:
//!
//! > every mention of the unix-only standard library must sit under a
//! > `cfg(unix)` gate, whether that gate is an inner attribute on the file, an
//! > outer attribute on the item, or an attribute on an enclosing block.
//!
//! Gating the *import* is not enough on its own and the rule does not stop
//! there: a trait method is only callable where the trait is in scope, so an
//! import that moves inside a gate takes its call sites with it or the file
//! stops compiling on Linux too. That is what makes a scan over the import
//! sites worth running at all.
//!
//! It is not, however, sufficient, and E6 measured exactly where it stops. A
//! *call* to something already gated mentions no `os::unix` for this scan to
//! find: `cache::prepare` is `#[cfg(unix)]` in `src/cache.rs`, and two ungated
//! call sites of it in `tests/cache.rs` were invisible here and fatal on
//! Windows. The check that catches both is a real compile, and a Linux machine
//! with docker can run one — `mingw-w64` is the only thing the `zstd-sys` C
//! sources were missing:
//!
//! ```console
//! $ mise run check:windows
//! ```
//!
//! That task builds `scripts/ci/wincheck.Dockerfile` and runs
//! `cargo check --all-targets --target x86_64-pc-windows-gnu` inside it; the
//! recipe is a committed file rather than prose so it can be run rather than
//! transcribed, and `docs/dev/testing.md` spells out the two commands it is.
//! Run it before changing a shared test helper; this scan is the cheap half
//! that runs in every suite.
//!
//! [`unix_sites`] is a pure function over one file's text, so the scanner
//! itself is tested against source it is handed rather than against whatever
//! happens to be in the tree; the rule is asserted over the real tree by
//! `tests/regressions/e6_the_test_helpers_did_not_compile_on_windows.rs`.

use std::path::PathBuf;

use crate::common::repo::root;

/// One mention of the unix-only standard library, and whether a gate covers it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnixSite {
    /// The file, as it was named to [`unix_sites`].
    pub file: String,
    /// The line it is on, counting from one.
    pub line: usize,
    /// The line's text, trimmed, for a failure that can be read without the
    /// file open beside it.
    pub text: String,
    /// Whether a `cfg(unix)` gate encloses it.
    pub gated: bool,
}

impl std::fmt::Display for UnixSite {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}:{}: {}", self.file, self.line, self.text)
    }
}

/// Every mention of `os::unix` in one source file, with the gate over each.
///
/// The needle is `os::unix` rather than the full `std::os::unix`, because
/// `use std::os::unix::fs::PermissionsExt` and a bare
/// `os::unix::fs::symlink(..)` after `use std::os` are the same reach and the
/// second spelling is the one a later edit is likely to introduce.
///
/// Comments and string literals are removed before the search, so a doc
/// comment that quotes the rule and a test that embeds a sample of source are
/// not sites. Both of those exist — this module is one and the regression
/// file is the other — and a scan that reported them would have to carry a
/// list of exceptions, which is the thing that rots.
pub fn unix_sites(file: &str, source: &str) -> Vec<UnixSite> {
    let mut sites = Vec::new();
    let mut lex = Lex::default();
    // One entry per open brace: whether a `cfg(unix)` gate encloses it.
    let mut stack: Vec<bool> = Vec::new();
    let mut file_gated = false;
    // An outer `#[cfg(unix)]` seen and not yet applied to an item.
    let mut pending = false;

    for (index, raw) in source.lines().enumerate() {
        let code = lex.strip(raw);
        let trimmed = raw.trim_start();

        // An inner attribute gates the rest of the file, wherever in the
        // prelude it stands.
        if trimmed.starts_with("#![cfg") {
            file_gated = file_gated || names_unix(trimmed);
            continue;
        }
        // Any attribute line, not only a `cfg` one: `#[cfg(unix)]` is almost
        // always followed by `#[test]`, and treating that second line as code
        // would drop the gate before the item it belongs to and report every
        // gated test in the tree as naked.
        let attribute = trimmed.starts_with("#[") || trimmed.starts_with("#![");
        if trimmed.starts_with("#[cfg") && names_unix(trimmed) {
            pending = true;
        }

        let enclosing = file_gated || stack.last().copied().unwrap_or(false);
        if code.contains("os::unix") {
            sites.push(UnixSite {
                file: file.to_owned(),
                line: index + 1,
                text: raw.trim().to_owned(),
                gated: enclosing || pending,
            });
        }

        for byte in code.bytes() {
            match byte {
                b'{' => {
                    let inherited = file_gated || stack.last().copied().unwrap_or(false);
                    stack.push(inherited || pending);
                }
                b'}' => {
                    stack.pop();
                }
                _ => {}
            }
        }

        // An attribute line, a blank line and a line that is nothing but a
        // comment all leave the pending gate alone: `#[cfg(unix)]` followed by
        // `#[test]`, by `#[allow(..)]`, by a blank line or by an explanatory
        // comment still gates the item underneath. Anything else consumes it.
        if !attribute && !code.trim().is_empty() {
            pending = false;
        }
    }

    sites
}

/// The tracked `.rs` files under `tests/`, and the ones the scan could not
/// read.
///
/// Two lists rather than one, because a file that is dropped is a file the
/// scan has no answer for, and reporting it as clean is the silent skip
/// `CLAUDE.md` forbids.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrackedSources {
    /// Each readable file, repository-relative, with its text, sorted by name.
    pub files: Vec<(String, String)>,
    /// Each file the scan could not read, named as `git` spelled it (lossily,
    /// when the name is not UTF-8), sorted.
    pub unreadable: Vec<String>,
}

/// Every `.rs` file `git` tracks under `tests/`, with its text.
///
/// `git ls-files` rather than a directory walk, for the reason
/// `tests/regressions/e5_a_gated_test_defaulted_to_one_developers_machine.rs`
/// gives: the claim is about the repository, and a contributor's `gleam build`
/// leaves files under `tests/fixtures/` that belong to nobody's tree.
///
/// `None` when `git` cannot answer, which the caller reports as a skip rather
/// than quietly falling back to a different question.
pub fn tracked_test_sources() -> Option<TrackedSources> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(root())
        .args(["ls-files", "-z", "--", "tests"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(collect_tracked_sources(&output.stdout, &|name: &str| {
        let path: PathBuf = root().join(name);
        std::fs::read_to_string(path).ok()
    }))
}

/// The pure half of [`tracked_test_sources`]: one `git ls-files -z` listing
/// and a reader, in; the readable files and the unreadable names, out.
///
/// `read` answers `None` for a file that is not there or whose bytes are not
/// UTF-8. Passed in so the collector can be handed a name no filesystem has.
pub fn collect_tracked_sources(
    listing: &[u8],
    read: &dyn Fn(&str) -> Option<String>,
) -> TrackedSources {
    let mut out = TrackedSources::default();
    for name in listing.split(|byte| *byte == 0) {
        if name.is_empty() {
            continue;
        }
        // A name whose bytes are not UTF-8 is reported rather than dropped: it
        // is a tracked source this scan has no answer for, and the answer it
        // would otherwise give is silence. This repository writes non-UTF-8
        // paths on purpose elsewhere — see
        // `tests/regressions/a4_a_non_utf8_output_path_failed_the_json_report.rs`
        // — so the shape is not hypothetical.
        let Ok(name) = std::str::from_utf8(name) else {
            let lossy = String::from_utf8_lossy(name).into_owned();
            if lossy.ends_with(".rs") {
                out.unreadable.push(lossy);
            }
            continue;
        };
        if !name.ends_with(".rs") {
            continue;
        }
        match read(name) {
            Some(text) => out.files.push((name.to_owned(), text)),
            None => out.unreadable.push(name.to_owned()),
        }
    }
    out.files.sort();
    out.unreadable.sort();
    out
}

/// Whether a `cfg` attribute names `unix`.
///
/// `#[cfg(unix)]`, `#[cfg(all(unix, feature = "cli"))]` and
/// `#[cfg(target_family = "unix")]` all count; `#[cfg(not(unix))]` does not,
/// and neither does `#[cfg(windows)]`. The check is over the raw attribute
/// text rather than over the stripped code, because `target_family = "unix"`
/// puts the word inside a string literal.
fn names_unix(attribute: &str) -> bool {
    if attribute.contains("not(unix") || attribute.contains("not(target_family") {
        return false;
    }
    attribute
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .any(|word| word == "unix")
}

/// The comment and literal state that carries from one line to the next.
///
/// A block comment, a string with a `\` line continuation and a raw string all
/// span lines, and a brace inside any of them is not a brace. Without this the
/// gate stack drifts the moment a test writes `assert!(.., "…{…}…")` across
/// two lines, and the answer stops being about the code.
#[derive(Default)]
struct Lex {
    /// Inside a `/* .. */`.
    block_comment: bool,
    /// Inside an ordinary `".."`.
    plain_string: bool,
    /// Inside an `r#".."#`, holding the number of hashes that closes it.
    raw_hashes: Option<usize>,
}

impl Lex {
    /// One line with its comments and literals blanked out.
    fn strip(&mut self, line: &str) -> String {
        let chars: Vec<char> = line.chars().collect();
        let mut out = String::new();
        let mut index = 0;

        while index < chars.len() {
            if self.block_comment {
                if chars[index] == '*' && chars.get(index + 1) == Some(&'/') {
                    self.block_comment = false;
                    index += 2;
                } else {
                    index += 1;
                }
                continue;
            }
            if self.plain_string {
                index = self.close_plain(&chars, index);
                continue;
            }
            if let Some(hashes) = self.raw_hashes {
                index = self.close_raw(&chars, index, hashes);
                continue;
            }

            let current = chars[index];
            match current {
                '/' if chars.get(index + 1) == Some(&'/') => break,
                '/' if chars.get(index + 1) == Some(&'*') => {
                    self.block_comment = true;
                    index += 2;
                }
                'r' if starts_raw(&chars, index) => {
                    let mut hashes = 0;
                    let mut cursor = index + 1;
                    while chars.get(cursor) == Some(&'#') {
                        hashes += 1;
                        cursor += 1;
                    }
                    self.raw_hashes = Some(hashes);
                    out.push(' ');
                    index = self.close_raw(&chars, cursor + 1, hashes);
                }
                '"' => {
                    self.plain_string = true;
                    out.push(' ');
                    index = self.close_plain(&chars, index + 1);
                }
                '\'' => {
                    out.push(' ');
                    index = skip_char_literal(&chars, index);
                }
                _ => {
                    out.push(current);
                    index += 1;
                }
            }
        }

        // A line comment and the end of a line both end an ordinary string:
        // the only way one may continue is a `\` at the very end, which the
        // scan above leaves `plain_string` set for.
        out
    }

    /// Advances past the rest of an ordinary string on this line.
    fn close_plain(&mut self, chars: &[char], start: usize) -> usize {
        let mut index = start;
        while index < chars.len() {
            match chars[index] {
                '\\' => index += 2,
                '"' => {
                    self.plain_string = false;
                    return index + 1;
                }
                _ => index += 1,
            }
        }
        // Ran off the end with the string still open. Rust lets an ordinary
        // string hold a newline, with or without a `\` continuation, and this
        // suite is full of both, so the string carries to the next line. A
        // string the file never closes is a compile error somebody else
        // reports.
        self.plain_string = true;
        chars.len()
    }

    /// Advances past the rest of a raw string on this line.
    fn close_raw(&mut self, chars: &[char], start: usize, hashes: usize) -> usize {
        let mut index = start;
        while index < chars.len() {
            if chars[index] == '"' {
                let mut seen = 0;
                let mut cursor = index + 1;
                while seen < hashes && chars.get(cursor) == Some(&'#') {
                    seen += 1;
                    cursor += 1;
                }
                if seen == hashes {
                    self.raw_hashes = None;
                    return cursor;
                }
            }
            index += 1;
        }
        chars.len()
    }
}

/// Whether an `r` at `index` opens a raw string rather than an identifier.
fn starts_raw(chars: &[char], index: usize) -> bool {
    if index > 0 {
        let previous = chars[index - 1];
        if previous.is_ascii_alphanumeric() || previous == '_' {
            return false;
        }
    }
    let mut cursor = index + 1;
    while chars.get(cursor) == Some(&'#') {
        cursor += 1;
    }
    chars.get(cursor) == Some(&'"')
}

/// Advances past a character literal, or past the tick of a lifetime.
fn skip_char_literal(chars: &[char], index: usize) -> usize {
    if chars.get(index + 1) == Some(&'\\') {
        // The escaped character comes first and may itself be a tick: `'\''`
        // is four characters, and a search for the terminator that started at
        // `index + 2` would stop on that escaped tick and return the index of
        // the *closing* one. The lexer would then re-open a character literal
        // there and swallow whatever follows — including a brace the gate
        // stack is counting, which turns an ungated reach into a reported-gated
        // one. So the search starts past the escape.
        let mut cursor = index + 3;
        while cursor < chars.len() && chars[cursor] != '\'' {
            cursor += 1;
        }
        return cursor + 1;
    }
    if chars.get(index + 2) == Some(&'\'') {
        return index + 3;
    }
    index + 1
}

/// The `DT_NEEDED` names in `needed` that `allowlist` does not admit, sorted
/// and without repeats.
///
/// The portability promise a packaged application makes is about the *host's*
/// runtime and not about ginary: an artifact is only as portable as the
/// emulator whoever built it had installed, and a `beam.smp` linked against a
/// library outside [`ginary::verify::NEEDED_ALLOWLIST`] is reported by
/// `ginary verify` exactly as it should be. A test that asserted a real
/// artifact verifies with no findings at all was therefore asserting a
/// property of one machine's Erlang build; this is what it has to compare
/// against instead, computed from the installation rather than from the
/// artifact, so that the two sides of the assertion are two different files.
#[cfg(feature = "cli")]
pub fn unmet_needs(needed: &[String], allowlist: &[&str]) -> Vec<String> {
    let mut names: Vec<String> = needed
        .iter()
        .filter(|name| !ginary::verify::needed_is_allowed(name, allowlist))
        .cloned()
        .collect();
    names.sort_unstable();
    names.dedup();
    names
}
