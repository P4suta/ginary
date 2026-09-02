// SPDX-License-Identifier: MIT OR Apache-2.0
//! Reading committed repository files, for the tests that hold the CI, the
//! release workflows and the v1 documentation against the tree.
//!
//! None of the E1 product is code the suite can execute: a workflow runs only
//! on GitHub, and a document is prose. What every one of those artifacts shares
//! is that it can rot silently, and a claim nobody checks reads as evidence.
//! These helpers are the same shape [`tests/formal.rs`](../formal.rs) and
//! [`tests/smoke_matrix.rs`](../smoke_matrix.rs) grew their own copies of; this
//! module is the one place the E1 targets share them from.

use std::path::PathBuf;

use saphyr::{LoadableYamlNode, YamlOwned};

/// The repository root, the directory holding `Cargo.toml`.
pub fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Reads a repository file as text.
///
/// # Panics
///
/// If the file is not there. For the E1 targets that *is* the assertion: a
/// workflow or a document the milestone promised and did not write is a failed
/// test, named by the path it was looked for at.
pub fn read(relative: &str) -> String {
    let path = root().join(relative);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()))
}

/// Reads a repository file as text, or `None` when it is not there.
///
/// Unlike [`read`], a missing file is not a panic here, so a test can assert
/// on its absence or make its own message.
pub fn read_opt(relative: &str) -> Option<String> {
    std::fs::read_to_string(root().join(relative)).ok()
}

/// Whether a repository path exists at all, file or directory.
pub fn exists(relative: &str) -> bool {
    root().join(relative).exists()
}

/// Reads a repository file as text, or a one-line `(missing <path>)` marker.
///
/// [`read`] panics on a file that is not there, which is the right answer for
/// a plain assertion. A snapshot test wants the other one: rendering the marker
/// makes the failure a diff between the record the milestone promised and the
/// empty tree, so one run names both the path and the whole expected content.
pub fn read_or_missing(relative: &str) -> String {
    read_opt(relative).unwrap_or_else(|| format!("(missing {relative})"))
}

/// Parses one YAML document, or returns the parser's own message.
///
/// GitHub reads several of this repository's records as YAML — the issue
/// forms, `dependabot.yml`, every workflow — and none of them is executed by
/// the suite. A substring assertion is happy with a file YAML cannot load at
/// all, which is how a plain scalar carrying `": "` reached the tree once
/// already; see `tests/regressions/e3_an_issue_form_was_not_valid_yaml.rs`.
/// Parsing first makes that failure a test failure.
///
/// An empty document parses to [`YamlOwned::BadValue`] rather than to an
/// error, which is what a file holding nothing but comments is.
pub fn parse_yaml(text: &str) -> Result<YamlOwned, String> {
    let mut documents = YamlOwned::load_from_str(text).map_err(|error| error.to_string())?;
    if documents.len() > 1 {
        return Err(format!("{} documents, expected one", documents.len()));
    }
    Ok(documents.pop().unwrap_or(YamlOwned::BadValue))
}

/// Reads a repository file and parses it as one YAML document.
///
/// # Panics
///
/// If the file is not there, or if YAML cannot load it. Both are the
/// assertion: a record GitHub cannot parse is a record GitHub ignores.
pub fn yaml(relative: &str) -> YamlOwned {
    parse_yaml(&read(relative))
        .unwrap_or_else(|error| panic!("{relative} is not valid YAML: {error}"))
}

/// Every `.yml`/`.yaml` file under a repository directory, recursively,
/// as repository-relative paths, sorted.
///
/// The order is the sorted one rather than the filesystem's, so a failure
/// names the same file on every machine.
pub fn yaml_files_under(relative: &str) -> Vec<String> {
    let mut out = Vec::new();
    collect_yaml(&root().join(relative), relative, &mut out);
    out.sort();
    out
}

/// The recursive half of [`yaml_files_under`].
fn collect_yaml(directory: &std::path::Path, prefix: &str, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let relative = format!("{prefix}/{name}");
        if path.is_dir() {
            collect_yaml(&path, &relative, out);
        } else if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("yml") | Some("yaml")
        ) {
            out.push(relative);
        }
    }
}
