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

use std::collections::BTreeMap;
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

// -------------------------------------------------- the Rust toolchains --

/// The action every Rust toolchain in this repository is installed with.
pub const RUST_TOOLCHAIN_ACTION: &str = "dtolnay/rust-toolchain";

/// One `dtolnay/rust-toolchain` step, and the toolchain it installs.
///
/// Which toolchain CI builds with is not a detail: a workflow that installs
/// the MSRV in every job never once compiles the crate on current stable, so
/// a lint, a compile error or a behaviour change introduced by any Rust past
/// the floor reaches a contributor's machine before it reaches CI. Holding
/// that to a rule needs the toolchain of every job at once, which is what this
/// is.
///
/// Read out of the parsed workflow rather than grepped: the word `toolchain`
/// appears in comments, in `GINARY_REQUIRE_TOOLCHAIN` and in the name of the
/// test job, and none of those installs anything.
///
/// This covers the one mechanism the repository uses and only that: a step
/// whose `uses:` is [`RUST_TOOLCHAIN_ACTION`]. A `run: rustup toolchain
/// install`, a `cargo +1.88.0` or a committed `rust-toolchain.toml` would pin
/// a numbered release without ever appearing here, so the other half of the
/// rule is asserted separately, by
/// `no_workflow_reaches_around_the_toolchain_action_and_no_override_is_committed`
/// in `tests/ci_matrix.rs`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ToolchainSite {
    /// The workflow or action file, repository-relative.
    pub workflow: String,
    /// The job id the step belongs to, or `runs` for a composite action.
    pub job: String,
    /// The `with: toolchain:` value, or `<unset>` when the step names none.
    pub toolchain: String,
}

impl std::fmt::Display for ToolchainSite {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{}: job `{}` installs `{}`",
            self.workflow, self.job, self.toolchain
        )
    }
}

/// Every Rust toolchain installation under `.github/`, sorted.
///
/// # Panics
///
/// If a workflow or composite action is not valid YAML. A file GitHub cannot
/// parse is a job that never runs, and reading it as text would hide that.
pub fn rust_toolchain_sites() -> Vec<ToolchainSite> {
    let mut files = yaml_files_under(".github/workflows");
    files.extend(yaml_files_under(".github/actions"));
    files.sort();

    let mut out = Vec::new();
    for relative in files {
        let text = read(&relative);
        let parsed = parse_yaml(&text)
            .unwrap_or_else(|error| panic!("{relative} is not valid YAML: {error}"));
        if let Some(jobs) = parsed
            .as_mapping_get("jobs")
            .and_then(YamlOwned::as_mapping)
        {
            for (id, job) in jobs {
                let name = id.as_str().unwrap_or("<a job id that is not a string>");
                collect_toolchains(&relative, name, job.as_mapping_get("steps"), &mut out);
            }
        }
        if let Some(runs) = parsed.as_mapping_get("runs") {
            collect_toolchains(&relative, "runs", runs.as_mapping_get("steps"), &mut out);
        }
    }
    out.sort();
    out
}

/// Appends every rust-toolchain step of one `steps:` sequence.
fn collect_toolchains(
    workflow: &str,
    job: &str,
    steps: Option<&YamlOwned>,
    out: &mut Vec<ToolchainSite>,
) {
    let Some(steps) = steps.and_then(YamlOwned::as_vec) else {
        return;
    };
    for step in steps {
        let Some(uses) = step.as_mapping_get("uses").and_then(YamlOwned::as_str) else {
            continue;
        };
        if !uses.starts_with(RUST_TOOLCHAIN_ACTION) {
            continue;
        }
        let toolchain = step
            .as_mapping_get("with")
            .and_then(|with| with.as_mapping_get("toolchain"))
            .map_or_else(|| "<unset>".to_owned(), scalar_text);
        out.push(ToolchainSite {
            workflow: workflow.to_owned(),
            job: job.to_owned(),
            toolchain,
        });
    }
}

/// A YAML scalar as text, or a message naming what is wrong with it.
///
/// `toolchain: stable` and `toolchain: 1.88.0` are both strings. Anything else
/// is a `toolchain:` a YAML reader resolved to a number or a boolean, and
/// re-rendering the typed value would name a toolchain the file does not
/// contain: `toolchain: 1.10` parses as the float 1.1 and prints as `1.1`, so
/// a failure would send the reader looking for a release nobody wrote down.
/// The message is the accurate answer and it is also the fix — quote it.
fn scalar_text(node: &YamlOwned) -> String {
    node.as_str().map_or_else(
        || {
            "<unquoted: `toolchain:` has to be a quoted string. YAML resolved this one to a              number or a boolean, and the value it resolved to is not the text the file holds              — `1.10` becomes the float 1.1>"
                .to_owned()
        },
        str::to_owned,
    )
}

// ------------------------------------------------------- workflow steps --

/// One step of one workflow job, with the environment it runs under.
///
/// The tests that came out of the first live CI runs are all about *order and
/// environment* rather than about the presence of a word: which build wrote
/// `target/release/ginary` last, whether two `cross` invocations share one
/// `CARGO_TARGET_DIR`. A substring search over the file cannot answer either,
/// so the steps are read out of the parsed document with their env merged.
#[derive(Clone, Debug)]
pub struct WorkflowStep {
    /// The workflow file, repository-relative.
    pub workflow: String,
    /// The job id the step belongs to.
    pub job: String,
    /// The step's position within its job, counting from one.
    pub position: usize,
    /// The step's `name:`, or its `uses:`, or `<a run step>`.
    pub name: String,
    /// The step's `run:` script, empty for a step that only `uses:` an action.
    pub run: String,
    /// The job's `env:` overlaid with the step's own, values as written.
    pub env: BTreeMap<String, String>,
}

impl WorkflowStep {
    /// The step's script as one command per line, with backslash
    /// continuations joined and each line trimmed.
    ///
    /// Every rule a test writes over a `run:` block reads one command at a
    /// time, and a shell command wrapped over three lines for width is still
    /// one command. Without joining, a purely cosmetic reflow of a workflow
    /// silently changes what such a rule asserts — in both directions: a
    /// `--target-dir` moved onto a continuation makes the first line look like
    /// a build that writes to the default path, and a flag moved off one hides
    /// the build the rule was watching.
    pub fn commands(&self) -> Vec<String> {
        self.run
            .replace("\\\n", " ")
            .lines()
            .map(|line| line.trim().to_owned())
            .collect()
    }
}

impl std::fmt::Display for WorkflowStep {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{}: job `{}` step {} (`{}`)",
            self.workflow, self.job, self.position, self.name
        )
    }
}

/// Every step of every job of one workflow, in file order.
///
/// # Panics
///
/// If the file is not there or is not valid YAML: a workflow GitHub cannot
/// parse is a workflow that never runs.
pub fn workflow_steps(relative: &str) -> Vec<WorkflowStep> {
    let parsed = yaml(relative);
    let mut out = Vec::new();
    let Some(jobs) = parsed
        .as_mapping_get("jobs")
        .and_then(YamlOwned::as_mapping)
    else {
        return out;
    };
    for (id, job) in jobs {
        let name = id.as_str().unwrap_or("<a job id that is not a string>");
        let job_env = env_map(job.as_mapping_get("env"));
        let Some(steps) = job.as_mapping_get("steps").and_then(YamlOwned::as_vec) else {
            continue;
        };
        for (index, step) in steps.iter().enumerate() {
            let run = step
                .as_mapping_get("run")
                .and_then(YamlOwned::as_str)
                .unwrap_or_default()
                .to_owned();
            let label = step
                .as_mapping_get("name")
                .and_then(YamlOwned::as_str)
                .or_else(|| step.as_mapping_get("uses").and_then(YamlOwned::as_str))
                .unwrap_or("<a run step>")
                .to_owned();
            let mut env = job_env.clone();
            env.extend(env_map(step.as_mapping_get("env")));
            out.push(WorkflowStep {
                workflow: relative.to_owned(),
                job: name.to_owned(),
                position: index + 1,
                name: label,
                run,
                env,
            });
        }
    }
    out
}

/// One `env:` mapping as name to value, dropping anything that is not a pair
/// of strings — a numeric or boolean value cannot be an environment variable
/// GitHub passes through unchanged anyway.
fn env_map(node: Option<&YamlOwned>) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Some(mapping) = node.and_then(YamlOwned::as_mapping) else {
        return out;
    };
    for (key, value) in mapping {
        if let (Some(key), Some(value)) = (key.as_str(), value.as_str()) {
            out.insert(key.to_owned(), value.to_owned());
        }
    }
    out
}
