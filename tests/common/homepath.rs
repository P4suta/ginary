// SPDX-License-Identifier: MIT OR Apache-2.0
//! Finding an absolute home directory in code that has to run anywhere.
//!
//! The rule this module measures is not "no file names *this* machine's
//! `$HOME`". That rule is only meaningful on the machine that wrote the file:
//! it passes on every other one, and on a hosted runner — where `$HOME` is
//! `/home/runner` — it fails on prose that quotes a CI transcript. The rule
//! here is the machine-independent one: **no file under [`CODE_ROOTS`] may
//! carry an absolute home path belonging to a person**, whatever machine the
//! scan runs on and whoever is running it.
//!
//! Three decisions make that a rule rather than a grep.
//!
//! - **Only executable and configuration paths are scanned.** `docs/` is not
//!   in [`CODE_ROOTS`], because a milestone log is *supposed* to reproduce the
//!   failing path a runner printed, verbatim, and a scan that policed prose
//!   would make the record of a bug into a bug.
//! - **Prose inside code is prose, and only where it is prose.** A regression
//!   test's module documentation quoting the transcript that identified it is
//!   the same document as the milestone log, so a comment line is not
//!   policed — but what opens a comment depends on the language, and
//!   [`Syntax`] is what says so. `//` opens one in Rust and `#` does not:
//!   `#` opens an *attribute* there, and `#[path = "/home/<a person>/x.rs"]`
//!   is code that names one machine. Reading `#` as a comment everywhere
//!   would exempt exactly the shape the rule exists to catch, in the language
//!   the scanned tree is almost entirely written in.
//! - **A fictional account is not a person.** This suite's unit tests are full
//!   of `/home/u/.cache` and `/Users/ada/AppData`: hand-written inputs to pure
//!   functions, naming accounts that exist nowhere. [`FICTIONAL_ACCOUNTS`]
//!   names them, and it is short and argued on purpose — putting a real
//!   developer's account on it would be visible in the diff and obviously
//!   wrong.
//!
//! [`home_path_sites`] is pure: bytes in, line numbers out. It reads *bytes*
//! rather than a decoded string because the class of file most likely to
//! embed an absolute path is the one that is not UTF-8 at all — three tracked
//! `.beam` fixtures carry the compiler's own `-o` path in their `Dbgi` chunk.

use std::path::PathBuf;

use crate::common::repo::root;

/// The directories whose tracked files must never name a person's home.
///
/// `docs/` is deliberately absent; see the module documentation.
pub const CODE_ROOTS: [&str; 4] = ["src", "tests", "scripts", ".github"];

/// The account names this suite's own fixtures use, which belong to nobody.
///
/// `u` is the placeholder every `cache` and `cache_dir` unit test spells
/// `HOME` as, and `ada` is the one `tests/windows.rs` and `src/cache.rs` give
/// the fictional user whose profile the Windows rules are asserted against.
/// Neither is an account on any machine, so neither is a path that exists on
/// one machine and not another — which is the whole of what this scan is for.
pub const FICTIONAL_ACCOUNTS: [&str; 2] = ["ada", "u"];

/// The bytes that end the account segment of an absolute home path.
///
/// A path may be quoted, be the last thing on the line, or continue into the
/// rest of itself, so the segment ends at the first separator, quote, bracket,
/// space or angle bracket. `<` is in the list because `/home/<user>/` is a
/// documentation placeholder and the empty segment it produces is not a hit.
const DELIMITERS: &[u8] = b"/\"'`,;:)]}\\ \t\r\n<>";

/// What opens a comment in a file, and so what the scan may skip.
///
/// Chosen from the file's name by [`Syntax::of`]. Anything the repository does
/// not write comments in — a `.beam`, a snapshot, a fixture — is
/// [`Syntax::Opaque`], where no line is prose: guessing wrong in that
/// direction reports a path that turns out to be in a comment, and guessing
/// wrong in the other hides one that is not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Syntax {
    /// Rust, where `//` opens a comment and `#` opens an attribute.
    Rust,
    /// YAML, TOML, shell and the rest of the `#` family.
    Hash,
    /// Everything else: no line is prose.
    Opaque,
}

impl Syntax {
    /// The comment syntax of a file, from its name.
    ///
    /// Pure, and total: an extension nobody listed is [`Syntax::Opaque`],
    /// which is the answer that hides nothing.
    #[must_use]
    pub fn of(name: &str) -> Self {
        let extension = name.rsplit_once('.').map_or("", |(_, after)| after);
        match extension {
            "rs" => Self::Rust,
            "yml" | "yaml" | "toml" | "sh" | "gitignore" => Self::Hash,
            _ => Self::Opaque,
        }
    }

    /// Whether `line` is a comment in this syntax, and so prose.
    fn is_prose(self, line: &[u8]) -> bool {
        let trimmed = line
            .iter()
            .position(|byte| !byte.is_ascii_whitespace())
            .map_or(&line[line.len()..], |first| &line[first..]);
        match self {
            Self::Rust => trimmed.starts_with(b"//"),
            Self::Hash => trimmed.starts_with(b"#"),
            Self::Opaque => false,
        }
    }
}

/// One absolute home path found in a file that has to run anywhere.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HomePathSite {
    /// The 1-based line the path was found on.
    pub line: usize,
    /// The root and account that were matched, as `home/<name>` or
    /// `Users/<name>` — the part of the path that names a machine, without
    /// whatever followed it.
    pub account: String,
}

/// Every absolute home path in `source` that names a person.
///
/// Pure, and byte-oriented: `source` is a file's contents, not a decoded
/// string. A hit is `/home/<name>` or `/Users/<name>` on a line that is not a
/// comment *in `syntax`*, where `<name>` is neither in [`FICTIONAL_ACCOUNTS`],
/// nor a documentation placeholder (`<user>`), nor an elision (`...`), nor
/// bytes that are not UTF-8.
pub fn home_path_sites(source: &[u8], syntax: Syntax) -> Vec<HomePathSite> {
    let mut sites = Vec::new();
    for (index, line) in source.split(|byte| *byte == b'\n').enumerate() {
        if syntax.is_prose(line) {
            continue;
        }
        for (offset, root) in home_roots(line) {
            let start = offset + root.len();
            let mut end = start;
            while end < line.len() && !DELIMITERS.contains(&line[end]) {
                end += 1;
            }
            let Ok(name) = std::str::from_utf8(&line[start..end]) else {
                continue;
            };
            if name.is_empty()
                || FICTIONAL_ACCOUNTS.contains(&name)
                || name.bytes().all(|byte| byte == b'.')
            {
                continue;
            }
            sites.push(HomePathSite {
                line: index + 1,
                account: format!("{}{name}", root.trim_start_matches('/')),
            });
        }
    }
    sites
}

/// Every `(offset, "/home/" | "/Users/")` in `line`, in order.
fn home_roots(line: &[u8]) -> Vec<(usize, &'static str)> {
    let mut found = Vec::new();
    for root in ["/home/", "/Users/"] {
        let needle = root.as_bytes();
        let mut from = 0;
        while from + needle.len() <= line.len() {
            let Some(at) = line[from..]
                .windows(needle.len())
                .position(|window| window == needle)
            else {
                break;
            };
            found.push((from + at, root));
            from += at + 1;
        }
    }
    found.sort_unstable();
    found
}

/// Every file `git` tracks under [`CODE_ROOTS`], repository-relative.
///
/// `git ls-files` rather than a directory walk, for the reason the rule this
/// scan replaced already gave: a contributor who has run `gleam build` has
/// `tests/fixtures/hello_ffi/build/` full of absolute paths that belong to no
/// repository, and a gitignored artifact naming a machine is not a defect in
/// the tree.
///
/// `None` when `git` cannot answer at all — no `git` on `PATH`, or a source
/// tree unpacked from a tarball — which is a reported skip and never a quiet
/// fallback to the directory read.
pub fn tracked_code_files() -> Option<Vec<String>> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(root())
        .args(["ls-files", "-z", "--"])
        .args(CODE_ROOTS)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(
        output
            .stdout
            .split(|byte| *byte == 0)
            .filter_map(|name| std::str::from_utf8(name).ok())
            .filter(|name| !name.is_empty())
            .map(str::to_owned)
            .collect(),
    )
}

/// The absolute path of a repository-relative name.
pub fn in_repo(relative: &str) -> PathBuf {
    root().join(relative)
}
