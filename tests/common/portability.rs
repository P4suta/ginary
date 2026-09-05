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

use crate::common::bounded::run_bounded;
use crate::common::repo::root;
use crate::common::tools::{LS_FILES_BUDGET, git_command, require_git_work_tree};

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
    // The attribute being read, joined, and how many `[` of it are still open.
    // `rustfmt` breaks a `cfg` that does not fit on a line, and a scan that
    // read one line at a time saw `#[cfg(all(` — an `all` of no options, which
    // guarantees no unix — and then read the next line as ordinary code and
    // dropped the gate, reporting the covered reach under it as naked. The
    // same bracket count `gnu_gate_sites` keeps, for the same reason.
    let mut attribute_text = String::new();
    let mut attribute_depth = 0usize;

    for (index, raw) in source.lines().enumerate() {
        let code = lex.strip(raw);
        // Whether a line *is* an attribute is decided on the stripped code, so
        // that a `#[cfg(unix)]` inside a block comment or inside a raw string
        // arms nothing — a compiler reads neither, and this file and the
        // regression beside it are both full of both spellings. Whether that
        // attribute *names unix* is decided on the raw line, because
        // `target_family = "unix"` puts the word inside a string literal the
        // stripper has already blanked out.
        let trimmed = raw.trim_start();
        let stripped = code.trim_start();

        // Any attribute line, not only a `cfg` one: `#[cfg(unix)]` is almost
        // always followed by `#[test]`, and treating that second line as code
        // would drop the gate before the item it belongs to and report every
        // gated test in the tree as naked. A continuation of an attribute
        // already open counts as one too, however it is indented.
        let attribute =
            attribute_depth > 0 || stripped.starts_with("#[") || stripped.starts_with("#![");
        if attribute {
            attribute_text.push_str(trimmed);
            attribute_text.push(' ');
            // Counted on the stripped code, so a bracket inside a string
            // literal or a trailing comment closes nothing.
            attribute_depth = (attribute_depth + code.matches('[').count())
                .saturating_sub(code.matches(']').count());
            if attribute_depth == 0 {
                let whole = std::mem::take(&mut attribute_text);
                let whole = whole.trim();
                // An inner attribute gates the rest of the file, wherever in
                // the prelude it stands; an outer one gates the item under it.
                if whole.starts_with("#![cfg") {
                    file_gated = file_gated || names_unix(whole);
                } else if whole.starts_with("#[cfg") && names_unix(whole) {
                    pending = true;
                }
            }
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
/// than quietly falling back to a different question. "Cannot answer" is
/// [`require_git_work_tree`]'s question rather than this function's own: a
/// `git` that is not installed and a directory that is not a checkout are two
/// different reasons, they are reported differently, and they were answered
/// separately in three places until E19 gave them one gate.
pub fn tracked_test_sources() -> Option<TrackedSources> {
    let tools = require_git_work_tree(&root())?;
    // Through `git_command` and under a deadline, for the reason
    // `homepath::tracked_code_files` gives.
    let output = run_bounded(
        git_command(tools.path("git"))
            .arg("-C")
            .arg(root())
            .args(["ls-files", "-z", "--", "tests"]),
        LS_FILES_BUDGET,
        "git ls-files",
    );
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

/// Whether a `cfg` attribute *guarantees* that the item under it is compiled
/// only on unix.
///
/// Mentioning `unix` is not the question and never was. `#[cfg(any(unix,
/// windows))]` names it and is true on Windows; `#[cfg(not(any(unix, feature =
/// "cli")))]` names it and is true on Windows whenever the feature is off. An
/// item under either reaches the one platform with no `std::os::unix`, which
/// is the opposite of what a gate is for, so a scan that accepted them would
/// report a naked reach as covered. The predicate is therefore parsed and
/// evaluated rather than searched: [`CfgExpr::guarantees_unix`] answers
/// whether every configuration the expression admits is a unix one.
///
/// `#[cfg(unix)]`, `#[cfg(all(unix, feature = "cli"))]` and
/// `#[cfg(target_family = "unix")]` all count; `#[cfg(not(unix))]`,
/// `#[cfg(windows)]` and the two above do not.
///
/// The check is over the raw attribute text rather than over the stripped
/// code, because `target_family = "unix"` puts the word inside a string
/// literal. Whether the line is an attribute at all is the stripped code's
/// question, and [`unix_sites`] asks it there.
fn names_unix(attribute: &str) -> bool {
    cfg_predicate(attribute).is_some_and(|predicate| CfgExpr::parse(predicate).guarantees_unix())
}

/// The text inside the outermost `cfg(..)` of one attribute line, or [`None`]
/// when the line is not a `cfg` attribute.
///
/// `cfg_attr` is deliberately not one: `#[cfg_attr(unix, path = "..")]`
/// applies an *attribute* conditionally and gates no item, so an item under it
/// is compiled everywhere.
fn cfg_predicate(attribute: &str) -> Option<&str> {
    let rest = attribute.trim_start();
    let rest = rest
        .strip_prefix("#![")
        .or_else(|| rest.strip_prefix("#["))?;
    let rest = rest.trim_start().strip_prefix("cfg")?;
    let inner = rest.trim_start().strip_prefix('(')?;
    // Up to the paren that closes that first one, so a trailing `)]` and
    // anything after it are not read as part of the predicate.
    let mut depth = 1usize;
    for (at, byte) in inner.bytes().enumerate() {
        match byte {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&inner[..at]);
                }
            }
            _ => {}
        }
    }
    // A predicate whose parentheses never close. The callers join an attribute
    // run to its closing bracket before asking, so this is a malformed
    // attribute rather than a wrapped one; whatever is there is answered on,
    // and an expression that does not parse guarantees no unix.
    Some(inner)
}

/// One `cfg` predicate, as far as a unix gate is concerned.
///
/// Every option that is not `unix` or `target_family = "unix"` is
/// [`CfgExpr::Other`], because no other option can *make* a target unix and
/// this scan asks only whether one is guaranteed.
#[derive(Clone, Debug, PartialEq, Eq)]
enum CfgExpr {
    /// `unix`, or `target_family = "unix"`.
    Unix,
    /// Any other option: a feature, a `target_os`, a `target_env`.
    Other,
    /// `all(..)`.
    All(Vec<CfgExpr>),
    /// `any(..)`.
    Any(Vec<CfgExpr>),
    /// `not(..)`.
    Not(Box<CfgExpr>),
}

impl CfgExpr {
    /// Parses one predicate — the text between the parentheses of `cfg(..)`.
    fn parse(text: &str) -> Self {
        let text = text.trim();
        if let Some(inner) = call_arguments(text, "all") {
            return Self::All(
                split_top_level(inner)
                    .iter()
                    .map(|a| Self::parse(a))
                    .collect(),
            );
        }
        if let Some(inner) = call_arguments(text, "any") {
            return Self::Any(
                split_top_level(inner)
                    .iter()
                    .map(|a| Self::parse(a))
                    .collect(),
            );
        }
        if let Some(inner) = call_arguments(text, "not") {
            return Self::Not(Box::new(Self::parse(inner)));
        }
        if is_unix_option(text) {
            Self::Unix
        } else {
            Self::Other
        }
    }

    /// Whether every configuration this predicate admits is a unix one.
    ///
    /// `all` guarantees unix as soon as one of its operands does, because all
    /// of them hold together. `any` guarantees it only when every operand
    /// does, because any one of them alone is enough to include the item — so
    /// `any(unix, windows)` guarantees nothing.
    ///
    /// `not` guarantees nothing, deliberately. `not(windows)` is true on unix
    /// *and* on every target that is neither, and deciding which of those the
    /// standard library gives an `os::unix` to is a question about the target
    /// list rather than about this file. Answering "no" costs a false report
    /// on a spelling nothing in this tree uses; answering "yes" would hide a
    /// naked reach, which is the failure this scan exists to prevent.
    fn guarantees_unix(&self) -> bool {
        match self {
            Self::Unix => true,
            Self::Other | Self::Not(_) => false,
            Self::All(operands) => operands.iter().any(Self::guarantees_unix),
            Self::Any(operands) => {
                !operands.is_empty() && operands.iter().all(Self::guarantees_unix)
            }
        }
    }
}

/// The arguments of `name(..)` when `text` is exactly that call.
fn call_arguments<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    let rest = text.strip_prefix(name)?.trim_start();
    let inner = rest.strip_prefix('(')?;
    let inner = inner.strip_suffix(')')?;
    // The stripped suffix has to be the paren that closes the stripped prefix,
    // or `all(a), any(b)` would parse as an `all` of `a), any(b`.
    let mut depth = 1usize;
    for byte in inner.bytes() {
        match byte {
            b'(' => depth += 1,
            b')' if depth == 0 => return None,
            b')' => depth -= 1,
            _ => {}
        }
    }
    Some(inner)
}

/// One argument list, split on the commas that are not inside a nested call or
/// a string literal.
fn split_top_level(arguments: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for character in arguments.chars() {
        if in_string {
            current.push(character);
            match character {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match character {
            '"' => {
                in_string = true;
                current.push(character);
            }
            '(' => {
                depth += 1;
                current.push(character);
            }
            ')' => {
                depth = depth.saturating_sub(1);
                current.push(character);
            }
            ',' if depth == 0 => out.push(std::mem::take(&mut current)),
            _ => current.push(character),
        }
    }
    out.push(current);
    // A trailing comma leaves an empty argument, and `#[cfg(all(\n unix,\n))]`
    // spelled over three lines is ordinary rustfmt output.
    out.into_iter()
        .map(|argument| argument.trim().to_owned())
        .filter(|argument| !argument.is_empty())
        .collect()
}

/// Whether one `cfg` option is true only on unix.
///
/// `unix` itself, and `target_family = "unix"`, which is what `unix` is
/// shorthand for.
fn is_unix_option(option: &str) -> bool {
    let option = option.trim();
    if option == "unix" {
        return true;
    }
    let Some((name, value)) = option.split_once('=') else {
        return false;
    };
    name.trim() == "target_family" && value.trim().trim_matches('"') == "unix"
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

/// The spellings that make a test's claim one only glibc can satisfy.
///
/// `libc.so.6` is glibc's own `SONAME`, `glibc_max` is the symbol-version
/// floor read out of `.gnu.version_r`, and `ld-linux` names the glibc dynamic
/// loader. A musl Linux binary has none of the three: it needs
/// `libc.musl-<arch>.so.1`, carries no symbol versions to derive a floor from,
/// and a fully static one names no interpreter at all.
pub const GLIBC_CLAIMS: [&str; 3] = ["libc.so.6", "glibc_max", "ld-linux"];

/// One `#[test]` gated on Linux that asserts something only glibc provides.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GnuGateSite {
    /// The file, as it was named to [`gnu_gate_sites`].
    pub file: String,
    /// The line the test's `fn` is on, counting from one.
    pub line: usize,
    /// The test's name.
    pub name: String,
    /// The first of [`GLIBC_CLAIMS`] its body makes.
    pub claim: String,
}

impl std::fmt::Display for GnuGateSite {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{}:{}: `{}` asserts `{}` under a gate that does not name gnu",
            self.file, self.line, self.name, self.claim
        )
    }
}

/// Every `#[test]` in one file that `target_os = "linux"` admits, that asserts
/// one of [`GLIBC_CLAIMS`], and whose gate does not also require
/// `target_env = "gnu"`.
///
/// `target_os = "linux"` is true on musl Linux as well as on gnu Linux, and
/// the two differ in exactly the facts [`GLIBC_CLAIMS`] names. A test gated on
/// the operating system alone therefore asserts glibc's shape against a
/// runtime that never promised it — and the assertion that is wrong is the one
/// nobody runs, which is what makes this a scan rather than a compile. The
/// same argument [`unix_sites`] is written from.
///
/// Deliberately syntactic, and calibrated on
/// `tests/fixtures/portability/gnu_gated_tests.rs.txt` before it is turned
/// loose on the tree. It reads the shape `rustfmt` produces: a test is a `fn`
/// at column zero under a run of column-zero attributes, and its body ends at
/// the first line that is exactly `}`. A needle inside a `//` comment is
/// prose, not a claim.
///
/// **What it cannot see, stated rather than left to be discovered.** The rule
/// is about a `cfg` *attribute*, so a test that picks its expectation at run
/// time is outside it however wrong that choice is — a `match
/// platform::object_format(platform::HOST)` whose ELF arm names `libc.so.6`
/// carries no attribute for this scan to read, and
/// `tests/stage_run.rs::the_needs_line_lists_the_libraries_the_runtime_loads`
/// was exactly that. Widening the rule to report every *ungated* test whose
/// body holds a [`GLIBC_CLAIMS`] needle would report a pure unit test over
/// literal input, which asserts nothing about the host and is not a defect; so
/// the runtime shape is answered where it belongs instead, by
/// [`host_needs_expectation`], which keys the expectation on the C library as
/// well as the object format. See
/// `tests/regressions/e16_a_glibc_only_expectation_was_asserted_on_any_elf_host.rs`.
pub fn gnu_gate_sites(file: &str, source: &str) -> Vec<GnuGateSite> {
    let lines: Vec<&str> = source.lines().collect();
    let mut sites = Vec::new();
    // The attribute run above the item being read, and how many `[` of it are
    // still open, so a `#[cfg(all(..))]` wrapped over four lines is one
    // attribute rather than four unrelated ones.
    let mut attributes = String::new();
    let mut depth = 0usize;
    let mut index = 0usize;

    while index < lines.len() {
        let line = lines[index];
        if depth > 0 || line.starts_with("#[") || line.starts_with("#![") {
            attributes.push_str(line);
            attributes.push('\n');
            depth = (depth + line.matches('[').count()).saturating_sub(line.matches(']').count());
            index += 1;
            continue;
        }

        let signature = line.strip_prefix("pub ").unwrap_or(line);
        let signature = signature.strip_prefix("async ").unwrap_or(signature);
        if let Some(rest) = signature.strip_prefix("fn ") {
            if attributes.contains("#[test]") {
                let name = rest
                    .split(['(', '<'])
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .to_owned();
                let mut end = index + 1;
                while end < lines.len() && lines[end] != "}" {
                    end += 1;
                }
                let body = &lines[index + 1..end.min(lines.len())];
                if attributes.contains("target_os = \"linux\"")
                    && !attributes.contains("target_env = \"gnu\"")
                    && let Some(claim) = GLIBC_CLAIMS.iter().find(|claim| {
                        body.iter().any(|text| {
                            !text.trim_start().starts_with("//") && text.contains(**claim)
                        })
                    })
                {
                    sites.push(GnuGateSite {
                        file: file.to_owned(),
                        line: index + 1,
                        name,
                        claim: (*claim).to_owned(),
                    });
                }
                attributes.clear();
                index = end + 1;
                continue;
            }
            attributes.clear();
            index += 1;
            continue;
        }

        // A blank line and a comment both leave the attribute run alone: a
        // `#[cfg(..)]` followed by a doc comment still gates the item under it.
        // Anything else is an item of its own and consumes the run.
        if !line.trim().is_empty() && !line.starts_with("//") {
            attributes.clear();
        }
        index += 1;
    }

    sites
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

/// What one host's own emulator has to say in a `needs:` line.
///
/// Three fields rather than a list, because the two questions a `needs:`
/// assertion asks are not the same question: *which libraries* the emulator
/// loads, and *whether a glibc symbol-version floor stands beside them*.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NeedsExpectation {
    /// The sonames the line must hold.
    pub libraries: Vec<String>,
    /// Whether the search folds case: a PE import table spells one file name
    /// both ways, and an ELF or Mach-O one does not.
    pub fold_case: bool,
    /// Whether the line carries a `(GLIBC_<version>)` floor. Asserted in both
    /// directions by the caller, so a floor appearing where there can be none
    /// is a failure rather than a silence.
    pub glibc_floor: bool,
}

/// The `needs:` line [`NeedsExpectation`] a runtime built for `host` produces.
///
/// **The C library is part of the question and the object format is not all of
/// it.** `ObjectFormat::Elf` is what a Linux host writes whichever C library it
/// links, and the two differ in exactly the names this expectation holds:
/// glibc publishes `libc.so.6` and symbol versions to derive a floor from,
/// musl publishes one library named after the architecture and no symbol
/// versions at all. A rule keyed on the format alone therefore asserted
/// glibc's shape against a musl host and failed a healthy machine — the same
/// defect the `cfg(target_os = "linux")` gates carried, in the one shape
/// [`gnu_gate_sites`] cannot see, because a runtime branch is not an attribute.
///
/// The musl row names the C library and stops there. Which of `libstdc++`,
/// `libgcc_s` and a terminal library an Alpine `erl` links is a packaging
/// decision that varies between builds, and a test that pinned it would be
/// asserting a property of one distribution rather than of the runtime.
pub fn host_needs_expectation(host: ginary::target::Target) -> NeedsExpectation {
    use ginary::platform::ObjectFormat;
    use ginary::target::Libc;

    match ginary::platform::object_format(host.os) {
        ObjectFormat::Elf => match host.libc {
            // glibc's own soname and the three libraries every OTP build this
            // project has measured links beside it. `Libc::None` shares the arm
            // for exhaustiveness only: `Target::host()` reports `Musl` or `Gnu`
            // on Linux and never `None`, so no host reaches it, and keeping the
            // pre-E16 answer there changes nothing any test can observe.
            Libc::Gnu | Libc::None => NeedsExpectation {
                libraries: [
                    "libc.so.6",
                    "libtinfo.so.6",
                    "libstdc++.so.6",
                    "libgcc_s.so.1",
                ]
                .iter()
                .map(|name| (*name).to_owned())
                .collect(),
                fold_case: false,
                glibc_floor: true,
            },
            Libc::Musl => NeedsExpectation {
                libraries: vec![format!("libc.musl-{}.so.1", host.arch)],
                fold_case: false,
                glibc_floor: false,
            },
        },
        // `KERNEL32.dll` is the kernel interface every Windows process has; an
        // import table that names none of it is one that was not read.
        ObjectFormat::Pe => NeedsExpectation {
            libraries: vec!["kernel32.dll".to_owned()],
            fold_case: true,
            glibc_floor: false,
        },
        // macOS re-exports `libm`, `libpthread` and the rest from one umbrella
        // dylib, so there is one name to look for.
        ObjectFormat::MachO => NeedsExpectation {
            libraries: vec!["/usr/lib/libSystem.B.dylib".to_owned()],
            fold_case: false,
            glibc_floor: false,
        },
    }
}
