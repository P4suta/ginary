// SPDX-License-Identifier: MIT OR Apache-2.0
//! Reading the committed dependency record: `Cargo.toml` and `Cargo.lock`.
//!
//! Two facts about this repository are written down in the manifest and
//! nowhere else, and both of them are load-bearing for a test rather than for
//! the compiler.
//!
//! The first is `rust-version`, the crate's declared MSRV. It is a number CI
//! has to install somewhere or the floor is a claim nobody checks, and it is a
//! number that must appear in exactly one job or it drifts. `tests/ci_matrix.rs`
//! reads it from here and holds the workflow to it.
//!
//! The second is the version requirement of a dependency, and the version the
//! lockfile actually resolved for it. The local `pre-push` gate refuses a push
//! that leaves a dependency it touched on a stale release; a test that reads
//! the same two files is the part of that gate the suite can run offline.
//!
//! Both are parsed by hand rather than with a TOML reader on purpose: `toml`
//! is an optional dependency behind the `cli` feature, and these helpers have
//! to work under `cargo test --no-default-features` too. The scanner
//! understands exactly the two shapes this manifest is written in — `name =
//! "req"` and `name = { version = "req", ... }`, the latter possibly spread
//! over several lines — and says so by returning `None` for anything else
//! rather than guessing.

use crate::common::repo::read;

/// A `major.minor.patch` version, with the parts a requirement may leave out
/// filled in with zero.
///
/// `rust-version = "1.88"` and `toolchain: 1.88.0` are the same floor written
/// two ways, and the drift test between them has to compare the floor rather
/// than the spelling.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    /// The major component.
    pub major: u64,
    /// The minor component, zero when the text omitted it.
    pub minor: u64,
    /// The patch component, zero when the text omitted it.
    pub patch: u64,
}

impl Version {
    /// Parses `[op]major[.minor[.patch]][-pre][+build]`.
    ///
    /// A leading requirement operator (`^`, `~`, `=`, `>`, `<`, whitespace) is
    /// dropped, and anything from the first `-` or `+` on is ignored: what the
    /// callers compare is the release, and a pre-release suffix on a
    /// dependency requirement is not a thing this manifest does.
    ///
    /// `None` when the text holds no leading integer at all, which is what a
    /// wildcard (`*`) or a git requirement is.
    pub fn parse(text: &str) -> Option<Self> {
        let trimmed = text.trim_start_matches(['^', '~', '=', '>', '<', ' ']);
        let release = trimmed
            .split(['-', '+'])
            .next()
            .unwrap_or(trimmed)
            .trim_end();
        let mut parts = release.split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next().map_or(Some(0), |p| p.parse().ok())?;
        let patch = parts.next().map_or(Some(0), |p| p.parse().ok())?;
        Some(Self {
            major,
            minor,
            patch,
        })
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// The `rust-version` of `[package]`, exactly as `Cargo.toml` spells it.
///
/// # Panics
///
/// If `[package]` states no `rust-version`. A crate whose MSRV is not written
/// down has no floor for CI to check, which is the defect E4 exists to fix.
pub fn rust_version() -> String {
    package_field("rust-version").unwrap_or_else(|| {
        panic!("Cargo.toml `[package]` states no `rust-version`; the MSRV has no single source")
    })
}

/// The value of `key = "value"` in the `[package]` table.
fn package_field(key: &str) -> Option<String> {
    let manifest = read("Cargo.toml");
    let mut in_package = false;
    for line in manifest.lines() {
        let line = line.trim();
        if let Some(section) = section_of(line) {
            in_package = section == "package";
            continue;
        }
        if !in_package {
            continue;
        }
        if let Some(value) = assignment(line, key) {
            return quoted(value);
        }
    }
    None
}

/// The version requirement `Cargo.toml` states for one dependency.
///
/// Every table whose name ends in `dependencies` is searched, so a
/// target-specific or a dev dependency answers as readily as a plain one.
/// `None` when no such table names it.
pub fn dependency_requirement(name: &str) -> Option<String> {
    let manifest = read("Cargo.toml");
    let lines: Vec<&str> = manifest.lines().collect();
    let mut in_dependencies = false;
    for (index, raw) in lines.iter().enumerate() {
        let line = raw.trim();
        if let Some(section) = section_of(line) {
            in_dependencies = section.ends_with("dependencies");
            continue;
        }
        if !in_dependencies {
            continue;
        }
        let Some(value) = assignment(line, name) else {
            continue;
        };
        if let Some(literal) = quoted(value) {
            return Some(literal);
        }
        // The table form. It may be one line or many, so the braces are
        // balanced before `version` is looked for: stopping at the first
        // newline would read `object = {` as a dependency with no version.
        let table = inline_table(&lines, index, value)?;
        return version_in_table(&table);
    }
    None
}

/// Every version `Cargo.lock` resolves for a package name, sorted.
///
/// More than one means the graph carries two majors of it, which for the
/// RustCrypto stack is the thing a `sha2` bump has to be checked for: two
/// `digest` majors compile, and produce two incompatible `Digest` traits.
pub fn locked_versions(name: &str) -> Vec<String> {
    let lock = read("Cargo.lock");
    let mut out = Vec::new();
    let mut current: Option<String> = None;
    for line in lock.lines() {
        let line = line.trim();
        if line == "[[package]]" {
            current = None;
            continue;
        }
        if let Some(value) = assignment(line, "name") {
            current = quoted(value);
            continue;
        }
        if let Some(value) = assignment(line, "version")
            && current.as_deref() == Some(name)
            && let Some(version) = quoted(value)
        {
            out.push(version);
        }
    }
    out.sort();
    out
}

/// The table name of a `[section]` line, or `None` for anything else.
fn section_of(line: &str) -> Option<&str> {
    let inner = line.strip_prefix('[')?.strip_suffix(']')?;
    Some(inner.trim_start_matches('[').trim_end_matches(']'))
}

/// The right-hand side of `key = ...`, or `None` when the line assigns
/// something else.
///
/// The key is matched whole: `sha2` must not answer for `sha2_extra`.
fn assignment<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(key)?.trim_start();
    Some(rest.strip_prefix('=')?.trim())
}

/// The contents of a `"..."` literal at the start of `value`.
fn quoted(value: &str) -> Option<String> {
    let rest = value.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(rest[..end].to_owned())
}

/// The whole `{ ... }` of an inline table that starts on `lines[index]`,
/// joined into one string.
fn inline_table(lines: &[&str], index: usize, first: &str) -> Option<String> {
    if !first.starts_with('{') {
        return None;
    }
    let mut joined = String::new();
    for line in &lines[index..] {
        let piece = if joined.is_empty() {
            first
        } else {
            line.trim()
        };
        joined.push(' ');
        joined.push_str(piece);
        if joined.matches('}').count() >= joined.matches('{').count() {
            return Some(joined);
        }
    }
    None
}

/// The `version = "..."` of an inline dependency table.
fn version_in_table(table: &str) -> Option<String> {
    let at = table.find("version")?;
    quoted(assignment(table[at..].trim(), "version")?)
}
