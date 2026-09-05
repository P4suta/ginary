// SPDX-License-Identifier: MIT OR Apache-2.0
//! The three records of a version, and what each of them actually means.
//!
//! `Cargo.toml` holds the version *being prepared*. `.release-please-manifest.json`
//! holds the version that was *last released* — release-please reads it as
//! nothing else, and it is what the next proposal is derived from.
//! `docs/RELEASE.md` says, in one sentence a maintainer deletes when they cut
//! the first release, whether anything has been released at all.
//!
//! Those are three different questions, and E20 exists because the tree
//! answered the second one with the first one's value: a manifest of `0.1.0`
//! for a repository whose `git tag` and `gh release list` are both empty. This
//! module is the one place the suite reads them from, so the rule that relates
//! them is written once.
//!
//! [`VersionRoot`] is the other half: a throwaway tree carrying just those two
//! files, so `scripts/ci/version-consistency.sh` can be driven over every state
//! — never released, released, and drifted — rather than only over whichever
//! state this checkout happens to be in.

use std::path::{Path, PathBuf};

use tempfile::TempDir;

use crate::common::repo::{read, root};

/// The manifest value release-please reads as "this package has never been
/// released".
///
/// Not a convention of ours: `Manifest.buildPullRequests` in release-please
/// synthesises a latest release from the manifest entry only when
/// `this.releasedVersions[path].toString() !== '0.0.0'`, so `0.0.0` is the one
/// spelling that leaves a package with no previous release at all. See
/// `docs/dev/log/E20.md` for the quoted source.
pub const NOTHING_RELEASED: &str = "0.0.0";

/// The manifest's single key: this repository releases one package, at the root.
pub const MANIFEST_KEY: &str = ".";

/// `.release-please-manifest.json`, relative to the repository root.
pub const MANIFEST_FILE: &str = ".release-please-manifest.json";

/// `release-please-config.json`, relative to the repository root.
pub const CONFIG_FILE: &str = "release-please-config.json";

/// The document that says whether a release has been cut.
pub const RELEASE_DOC: &str = "docs/RELEASE.md";

/// The sentence `docs/RELEASE.md` carries while nothing has been released.
///
/// The tree's own answer to a question no committed file can derive: a tag and
/// a GitHub release live on the server, a shallow CI checkout fetches no tags,
/// and `git tag` in a `cargo mutants` copy of the tree answers about nothing.
/// So the fact is written down once, in the document a maintainer edits when
/// they cut the release, and every test that needs it reads it from there.
pub const NO_RELEASE_YET: &str = "No release has been cut yet.";

/// The environment variable that points the version-consistency script at a
/// tree other than this one.
///
/// The script reads `Cargo.toml` and `.release-please-manifest.json` from the
/// repository it lives in, which is exactly one state of the world. The seam
/// is what lets the suite hold it to its contract in the states this checkout
/// is not in.
pub const ROOT_VAR: &str = "GINARY_VERSION_ROOT";

/// The version `Cargo.toml` carries: the version being prepared.
pub fn cargo_version() -> String {
    cargo_version_in(&root())
}

/// The version the `Cargo.toml` under `tree` carries.
///
/// # Panics
///
/// If the file is not there or carries no `version = "..."` line.
pub fn cargo_version_in(tree: &Path) -> String {
    let path = tree.join("Cargo.toml");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    text.lines()
        .find_map(|line| line.strip_prefix("version = \"")?.strip_suffix('"'))
        .unwrap_or_else(|| panic!("{} carries no version = \"...\" line", path.display()))
        .to_owned()
}

/// The version `.release-please-manifest.json` records as last released.
pub fn manifest_version() -> String {
    manifest_version_in(&root())
}

/// The version the manifest under `tree` records as last released.
///
/// # Panics
///
/// If the file is not there, is not JSON, or carries no `"."` entry: each of
/// those is a manifest release-please would not read either.
pub fn manifest_version_in(tree: &Path) -> String {
    let path = tree.join(MANIFEST_FILE);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    let parsed: serde_json::Value = serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("{} is not JSON: {error}", path.display()));
    parsed
        .get(MANIFEST_KEY)
        .and_then(serde_json::Value::as_str)
        .unwrap_or_else(|| panic!("{} carries no \"{MANIFEST_KEY}\" entry", path.display()))
        .to_owned()
}

/// The last released version, or `None` while nothing has been released.
pub fn last_released_version() -> Option<String> {
    let recorded = manifest_version();
    (recorded != NOTHING_RELEASED).then_some(recorded)
}

/// Whether `docs/RELEASE.md` still says no release has been cut.
pub fn nothing_has_been_released() -> bool {
    read(RELEASE_DOC).contains(NO_RELEASE_YET)
}

/// `release-please-config.json`, parsed.
///
/// # Panics
///
/// If it is not there or is not JSON.
pub fn release_please_config() -> serde_json::Value {
    let text = read(CONFIG_FILE);
    serde_json::from_str(&text).unwrap_or_else(|error| panic!("{CONFIG_FILE} is not JSON: {error}"))
}

/// One setting of the root package's release-please configuration.
///
/// The config nests the settings under `packages` keyed by path, so a test
/// that reached for a key at the top level would silently find nothing.
pub fn package_setting(key: &str) -> Option<serde_json::Value> {
    release_please_config()
        .get("packages")?
        .get(MANIFEST_KEY)?
        .get(key)
        .cloned()
}

/// Every line of a changelog that release-please would take as the point it
/// inserts a generated release section *above*.
///
/// Not our regex: `src/updaters/changelog.ts` searches the file with
/// `DEFAULT_VERSION_HEADER_REGEX = '\n###? v?[0-9[]'` and splices the generated
/// section in before the first match, leaving everything from that line down
/// untouched. `## [Unreleased]` matches it — the `[` is inside the character
/// class — so the insertion point of this repository's changelog is the
/// `[Unreleased]` heading itself. See `docs/dev/log/E20.md` for the quoted
/// source.
pub fn version_header_lines(text: &str) -> Vec<String> {
    text.lines()
        .filter(|line| is_version_header(line))
        .map(|line| line.trim_end().to_owned())
        .collect()
}

/// Whether `line` is what release-please's `versionHeaderRegex` matches: `##`
/// or `###`, a space, an optional `v`, then a digit or a `[`.
fn is_version_header(line: &str) -> bool {
    let Some(rest) = line.strip_prefix("##") else {
        return false;
    };
    let rest = rest.strip_prefix('#').unwrap_or(rest);
    let Some(rest) = rest.strip_prefix(' ') else {
        return false;
    };
    let rest = rest.strip_prefix('v').unwrap_or(rest);
    matches!(rest.chars().next(), Some(first) if first.is_ascii_digit() || first == '[')
}

/// Every heading of a changelog that claims a released version.
///
/// The same shape release-please's own insertion regex uses, minus
/// `[Unreleased]`: any `##`/`###` heading whose first character is a digit or a
/// `[`. Matching only `## [` would miss a hand-written `## 0.1.0 - 2026-09-02`,
/// which is the same false claim in a different spelling.
pub fn released_section_headings(text: &str) -> Vec<String> {
    version_header_lines(text)
        .into_iter()
        .filter(|line| !names_the_unreleased_section(line))
        .collect()
}

/// Whether a version heading is the living section rather than a release.
///
/// Stated over the same shape [`is_version_header`] matches, at both heading
/// levels. `version_header_lines` recognises `##` and `###` because
/// release-please's own insertion regex does, and an exclusion written for one
/// of the two levels reports the other as a released version — a heading that
/// claims nothing at all read as the loudest claim there is.
fn names_the_unreleased_section(line: &str) -> bool {
    line.trim_start_matches('#')
        .trim_start()
        .starts_with("[Unreleased]")
}

/// Every reference in a changelog to a git tag, whatever version it names.
///
/// A dangling link is a 404 in the project's own release notes, and the
/// version it names is not knowable in advance: pinning the two spellings of
/// `v0.1.0` would say nothing about a re-added `v0.2.0`.
pub fn tag_references(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in text.lines() {
        for needle in ["releases/tag/", "compare/"] {
            // Every occurrence, not the first. `str::find` answers once, and a
            // line carrying two references — a reflow, a hand edit, a sentence
            // naming two comparisons — hid everything after it. It hid it in
            // the direction that passes, and worst for `compare/`, whose first
            // occurrence on the changelog's link-reference line is the
            // `[Unreleased]` branch range this check deliberately ignores.
            for (at, _) in line.match_indices(needle) {
                let reference: String = line[at..]
                    .chars()
                    .take_while(|character| !character.is_whitespace() && *character != ')')
                    .collect();
                // `compare/` also spells a branch range, which names no tag. Only a
                // `v` followed by a digit is a reference to one.
                let names_a_tag = needle == "releases/tag/"
                    || reference.split(['/', '.']).any(|part| {
                        part.starts_with('v') && part[1..].starts_with(|c: char| c.is_ascii_digit())
                    });
                if names_a_tag {
                    out.push(reference);
                }
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Whether a file with no permission bits set is unreadable on this machine.
///
/// It is not, for root: the mode bits are advisory to a process that holds
/// `CAP_DAC_OVERRIDE`, so a suite run as root would drive
/// [`VersionRoot::unreadable`] over a file it can still read and assert the
/// wrong thing. Measured rather than assumed — the answer is a `geteuid` this
/// crate has no dependency to ask — by writing a file, clearing its bits and
/// trying it.
#[cfg(unix)]
pub fn files_can_be_made_unreadable() -> bool {
    use std::os::unix::fs::PermissionsExt as _;

    let Ok(dir) = TempDir::new() else {
        return false;
    };
    let path = dir.path().join("probe");
    if std::fs::write(&path, b"probe").is_err() {
        return false;
    }
    if std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).is_err() {
        return false;
    }
    std::fs::read(&path).is_err()
}

/// A throwaway tree carrying the two files the version check reads.
///
/// Owns a [`TempDir`], so the tree lives exactly as long as the value does.
pub struct VersionRoot {
    dir: TempDir,
}

impl VersionRoot {
    /// Writes a tree whose `Cargo.toml` carries `cargo` and whose manifest
    /// records `manifest` as the last released version.
    ///
    /// # Panics
    ///
    /// If the temporary directory or either file cannot be written.
    pub fn new(cargo: &str, manifest: &str) -> Self {
        let dir = TempDir::new().expect("temporary directory for a version fixture");
        let cargo_toml =
            format!("[package]\nname = \"ginary\"\nversion = \"{cargo}\"\nedition = \"2024\"\n");
        std::fs::write(dir.path().join("Cargo.toml"), cargo_toml).expect("write Cargo.toml");
        std::fs::write(
            dir.path().join(MANIFEST_FILE),
            format!("{{\n  \"{MANIFEST_KEY}\": \"{manifest}\"\n}}\n"),
        )
        .expect("write the release-please manifest");
        Self { dir }
    }

    /// The same tree, with the manifest written the way `JSON.stringify` and
    /// `jq -c` write it: one line, no indentation.
    ///
    /// A manifest is JSON, not a line-oriented record, and nothing stops a
    /// `prettier` pass or a hand edit from producing this shape. The check has
    /// to read it as release-please would; see
    /// `tests/regressions/e20_a_compact_manifest_read_as_one_with_no_entry.rs`.
    ///
    /// # Panics
    ///
    /// If the file cannot be written.
    pub fn compact(cargo: &str, manifest: &str) -> Self {
        let tree = Self::new(cargo, manifest);
        std::fs::write(
            tree.path().join(MANIFEST_FILE),
            format!("{{\"{MANIFEST_KEY}\": \"{manifest}\"}}\n"),
        )
        .expect("write a compact release-please manifest");
        tree
    }

    /// The same tree with one of its files deleted, for the states where a
    /// record cannot be read at all.
    ///
    /// # Panics
    ///
    /// If the file is not there to remove: a fixture asking to drop a file it
    /// never wrote is a test that has stopped describing what it runs.
    pub fn without(self, relative: &str) -> Self {
        std::fs::remove_file(self.path().join(relative))
            .unwrap_or_else(|error| panic!("remove {relative} from the fixture tree: {error}"));
        self
    }

    /// The same tree with one of its files left in place and made unreadable.
    ///
    /// The other half of [`VersionRoot::without`]. A record that is *there* and
    /// cannot be opened is a different state from a record that is gone, and it
    /// is the state a `-f` test answers `true` for: the guard passes, the read
    /// fails, and whatever the reading tool says in the runner's locale becomes
    /// the whole diagnostic. Only the owner's bits are cleared, so the fixture
    /// can still be removed when the [`TempDir`] goes.
    ///
    /// # Panics
    ///
    /// If the file is not there to change, or if the mode cannot be set.
    /// Callers guard with [`files_can_be_made_unreadable`], because a suite run
    /// as root can read a file whatever its mode says.
    #[cfg(unix)]
    pub fn unreadable(self, relative: &str) -> Self {
        use std::os::unix::fs::PermissionsExt as _;

        let path = self.path().join(relative);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000))
            .unwrap_or_else(|error| panic!("make {relative} unreadable in the fixture: {error}"));
        self
    }

    /// A tree that has released nothing: the manifest records [`NOTHING_RELEASED`].
    pub fn never_released(cargo: &str) -> Self {
        Self::new(cargo, NOTHING_RELEASED)
    }

    /// A tree whose last release is the version it is now preparing.
    pub fn released(version: &str) -> Self {
        Self::new(version, version)
    }

    /// The directory the two files are in.
    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    /// The directory as an owned path, for handing to a child process.
    pub fn to_path_buf(&self) -> PathBuf {
        self.dir.path().to_path_buf()
    }
}
