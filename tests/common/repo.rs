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
    collect_files(&root().join(relative), relative, &["yml", "yaml"], &mut out);
    out.sort();
    out
}

/// Every `.sh` file under a repository directory, recursively, as
/// repository-relative paths, sorted.
///
/// The committed scripts are the other half of what CI executes: a `run:`
/// step that calls `scripts/smoke-matrix.sh` runs every command in it, and a
/// scan that reads only the workflow reads only the call.
pub fn shell_scripts_under(relative: &str) -> Vec<String> {
    let mut out = Vec::new();
    collect_files(&root().join(relative), relative, &["sh"], &mut out);
    out.sort();
    out
}

/// The recursive half of [`yaml_files_under`] and [`shell_scripts_under`].
fn collect_files(
    directory: &std::path::Path,
    prefix: &str,
    extensions: &[&str],
    out: &mut Vec<String>,
) {
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
            collect_files(&path, &relative, extensions, out);
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|extension| extensions.contains(&extension))
        {
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
    /// The step's `uses:`, empty for a step that only `run:`s a script.
    pub uses: String,
    /// The step's `shell:`, empty for a step that does not name one.
    ///
    /// Which shell a script runs under decides what its last line means. A
    /// `shell: pwsh` step is run as `pwsh -command ". '<file>'"` with
    /// `if ((Test-Path -LiteralPath variable:\LASTEXITCODE)) { exit
    /// $LASTEXITCODE }` appended, so a step that ends with a non-zero
    /// `$LASTEXITCODE` fails whatever its own assertions concluded — which is
    /// exactly what happened to the Windows exit-code probe. A rule about
    /// that cannot be written from the script alone. See
    /// `tests/regressions/e15_a_pwsh_step_ended_with_the_code_it_asserted.rs`.
    pub shell: String,
    /// The step's `with:` mapping, string pairs only.
    ///
    /// Which *tool* an install step installs is a `with:` key and not part of
    /// the action reference, so a rule about the programs a job has on `PATH`
    /// cannot be written without it. See
    /// `tests/regressions/e7_actionlint_was_required_of_every_toolchain_job.rs`.
    pub with: BTreeMap<String, String>,
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
        push_steps(relative, name, steps, &job_env, &mut out);
    }
    out
}

/// Every step of one composite action, in file order.
///
/// A composite action's steps are `run:` scripts with a `shell:` of their own,
/// run in the caller's job by the same runner that runs a workflow step, so a
/// rule about what CI executes that reads `.github/workflows` alone reads half
/// the tree. The `job` of every step it returns is `<composite>`, because a
/// composite action does not have one — it borrows whichever job used it.
///
/// # Panics
///
/// As [`workflow_steps`].
pub fn composite_action_steps(relative: &str) -> Vec<WorkflowStep> {
    let parsed = yaml(relative);
    let mut out = Vec::new();
    let Some(steps) = parsed
        .as_mapping_get("runs")
        .and_then(|runs| runs.as_mapping_get("steps"))
        .and_then(YamlOwned::as_vec)
    else {
        return out;
    };
    let action_env = env_map(parsed.as_mapping_get("env"));
    push_steps(relative, "<composite>", steps, &action_env, &mut out);
    out
}

/// The half of [`workflow_steps`] and [`composite_action_steps`] that reads a
/// `steps:` sequence, whichever document it came out of.
fn push_steps(
    relative: &str,
    job: &str,
    steps: &[YamlOwned],
    outer_env: &BTreeMap<String, String>,
    out: &mut Vec<WorkflowStep>,
) {
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
        let mut env = outer_env.clone();
        env.extend(env_map(step.as_mapping_get("env")));
        out.push(WorkflowStep {
            workflow: relative.to_owned(),
            job: job.to_owned(),
            position: index + 1,
            name: label,
            run,
            uses: step
                .as_mapping_get("uses")
                .and_then(YamlOwned::as_str)
                .unwrap_or_default()
                .to_owned(),
            shell: step
                .as_mapping_get("shell")
                .and_then(YamlOwned::as_str)
                .unwrap_or_default()
                .to_owned(),
            with: env_map(step.as_mapping_get("with")),
            env,
        });
    }
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

/// One job of one workflow: what it waits on, and the environment every one
/// of its steps inherits.
///
/// [`workflow_steps`] flattens a workflow to its steps and merges the job's
/// `env:` into each of them, which answers "what does this command run
/// under". Two questions in this suite are about the job itself instead —
/// which jobs it `needs:`, and whether *the job* declares a variable rather
/// than one step of it — and neither survives the flattening. See
/// `tests/regressions/e6_the_coverage_floor_measured_a_stubless_subset.rs`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkflowJob {
    /// The workflow file, repository-relative.
    pub workflow: String,
    /// The job id, the key under `jobs:`.
    pub id: String,
    /// The jobs this one waits on, in file order. Empty when it waits on none.
    pub needs: Vec<String>,
    /// The job-level `env:` mapping, string pairs only.
    pub env: BTreeMap<String, String>,
    /// Every `run:` block of every step of the job, in file order, with `\`
    /// continuations joined the way [`WorkflowStep::commands`] joins them.
    pub commands: Vec<String>,
    /// Every `uses:` of every step of the job, in file order, action reference
    /// and pinned SHA as written.
    pub uses: Vec<String>,
}

impl WorkflowJob {
    /// Whether any command of the job contains `needle`.
    pub fn runs(&self, needle: &str) -> bool {
        self.commands.iter().any(|command| command.contains(needle))
    }

    /// Whether any step of the job `uses:` an action whose reference contains
    /// `needle`.
    pub fn uses_action(&self, needle: &str) -> bool {
        self.uses.iter().any(|action| action.contains(needle))
    }
}

/// Every job of one workflow, in file order.
///
/// # Panics
///
/// If the file is not there or is not valid YAML.
pub fn workflow_jobs(relative: &str) -> Vec<WorkflowJob> {
    let parsed = yaml(relative);
    let mut out = Vec::new();
    let Some(jobs) = parsed
        .as_mapping_get("jobs")
        .and_then(YamlOwned::as_mapping)
    else {
        return out;
    };
    for (id, job) in jobs {
        let steps = job
            .as_mapping_get("steps")
            .and_then(YamlOwned::as_vec)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let mut commands = Vec::new();
        let mut uses = Vec::new();
        for step in steps {
            if let Some(action) = step.as_mapping_get("uses").and_then(YamlOwned::as_str) {
                uses.push(action.to_owned());
            }
            let Some(run) = step.as_mapping_get("run").and_then(YamlOwned::as_str) else {
                continue;
            };
            commands.extend(
                run.replace("\\\n", " ")
                    .lines()
                    .map(|line| line.trim().to_owned()),
            );
        }
        out.push(WorkflowJob {
            workflow: relative.to_owned(),
            id: id
                .as_str()
                .unwrap_or("<a job id that is not a string>")
                .to_owned(),
            needs: string_list(job.as_mapping_get("needs")),
            env: env_map(job.as_mapping_get("env")),
            commands,
            uses,
        });
    }
    out
}

/// One YAML node as a list of strings: a sequence as itself, a bare scalar as
/// a list of one. `needs:` accepts both spellings.
fn string_list(node: Option<&YamlOwned>) -> Vec<String> {
    match node {
        Some(YamlOwned::Sequence(items)) => items
            .iter()
            .filter_map(|item| item.as_str().map(str::to_owned))
            .collect(),
        Some(other) => other.as_str().map(str::to_owned).into_iter().collect(),
        None => Vec::new(),
    }
}

// ----------------------------------------------- the ginary invocations --

/// One invocation of the `ginary` command line tool in a committed workflow
/// or in a committed shell script CI runs.
///
/// Those two files are the one place in this repository where the command
/// line is driven by text nothing type-checks. `cargo test` proves that every
/// flag `tests/cli.rs` passes exists; a `run:` block and a `.sh` prove nothing
/// until the job runs, and a flag that is not there is a job that dies at
/// argument parsing with the artifact it was supposed to build never
/// attempted. See
/// `tests/regressions/e6_the_macos_job_passed_a_flag_the_cli_does_not_have.rs`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GinaryInvocation {
    /// The file the command is in, repository-relative.
    pub source: String,
    /// Where in that file: `job \`macos\` step 7 (\`Package …\`)` for a
    /// workflow, `line 130` for a script.
    pub site: String,
    /// The command line as one line, continuations joined.
    pub line: String,
    /// The subcommand path: `["build"]`, or `["otp", "repack"]`.
    pub path: Vec<String>,
    /// Every long flag the command passes, in order, without its value.
    pub long_flags: Vec<String>,
}

impl std::fmt::Display for GinaryInvocation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}: {}", self.source, self.site, self.line)
    }
}

/// The subcommand path and the long flags of one shell command that runs
/// ginary, or `None` when the command runs something else.
///
/// Three spellings of the program are recognised, because those are the three
/// the committed files use: a path whose last component is `ginary` (however
/// it is quoted, and however it was interpolated), an interpolation of a
/// `GINARY_*BIN` variable, and `cargo run .. -- <subcommand>`. A `cp` of the
/// binary, a `cross build`, a packaged artifact being executed and a
/// diagnostic that quotes a ginary command line are all `None`: none of them
/// runs the tool.
///
/// The program word does not have to be the first word. A shell script writes
/// `if ! (cd .. && VAR=x ginary build ..)`, and the flags in that line are as
/// unchecked as the flags in a workflow, so the scan finds the program in any
/// *command position*: the start of the line, or after a `&&`, a `|`, a `;`,
/// an `if`, a `!`, or a run of `VAR=value` assignments. Anything after a
/// redirection or a command terminator belongs to the shell rather than to
/// ginary and is not read as an argument.
///
/// The subcommand path is the run of leading words before the first flag, so
/// `otp repack --out dist/otp` is `["otp", "repack"]` and `build --target x`
/// is `["build"]`. A `--flag=value` contributes `--flag`; the bare `--` that
/// separates cargo's own arguments contributes nothing. Short flags are
/// counted only as "a flag was seen": the guard is about long flags, which is
/// what a workflow writes.
pub fn parse_ginary_command(line: &str) -> Option<(Vec<String>, Vec<String>)> {
    // A whole-line comment runs nothing, however much of a command line it
    // quotes.
    if line.trim_start().starts_with('#') {
        return None;
    }
    // A bare `(` opening a subshell unquotes to nothing, which is not a word.
    let tokens: Vec<String> = line
        .split_whitespace()
        .map(unquote)
        .filter(|token| !token.is_empty())
        .collect();

    let mut index = 0;
    let mut command_position = true;
    let arguments: &[String] = loop {
        let token = tokens.get(index)?;
        if command_position {
            if is_ginary_program(token) {
                break tokens.get(index + 1..)?;
            }
            if token == "cargo" && tokens.get(index + 1).is_some_and(|word| word == "run") {
                let separator = tokens.iter().skip(index).position(|word| word == "--")? + index;
                break tokens.get(separator + 1..)?;
            }
            if is_assignment(token) {
                // `VAR=value ginary ..`: the assignment is a prefix of the
                // command, so the next word is still the program.
                index += 1;
                continue;
            }
        }
        command_position = opens_a_command(token);
        index += 1;
    };

    let mut path = Vec::new();
    let mut long_flags = Vec::new();
    let mut seen_flag = false;
    for token in arguments {
        if ends_the_command(token) {
            break;
        }
        if let Some(flag) = token.strip_prefix("--") {
            if flag.is_empty() {
                continue;
            }
            seen_flag = true;
            let name = flag.split('=').next().unwrap_or(flag);
            long_flags.push(format!("--{name}"));
            continue;
        }
        if token.starts_with('-') {
            seen_flag = true;
            continue;
        }
        if !seen_flag && token.chars().all(|c| c.is_ascii_lowercase() || c == '-') {
            path.push(token.clone());
        }
    }
    Some((path, long_flags))
}

/// Every ginary command line one committed file runs, in file order.
///
/// A `.yml` or `.yaml` is read as a workflow, step by step; anything else is
/// read as a shell script, line by line, with `\` continuations joined the
/// same way. Both are scanned because CI runs both: `scripts/smoke-matrix.sh`
/// and `scripts/smoke.sh` are `run:` steps, and the flags they pass are as
/// unchecked as the flags in the step that calls them.
///
/// # Panics
///
/// If the file is not there, or if it is a workflow that is not valid YAML.
pub fn ginary_invocations(relative: &str) -> Vec<GinaryInvocation> {
    let is_workflow = relative.ends_with(".yml") || relative.ends_with(".yaml");
    let sited: Vec<(String, String)> = if is_workflow {
        workflow_steps(relative)
            .into_iter()
            .flat_map(|step| {
                let site = format!(
                    "job `{}` step {} (`{}`)",
                    step.job, step.position, step.name
                );
                step.commands()
                    .into_iter()
                    .map(move |line| (site.clone(), line))
            })
            .collect()
    } else {
        script_command_lines(&read(relative))
    };

    let mut out = Vec::new();
    for (site, line) in sited {
        let Some((path, long_flags)) = parse_ginary_command(&line) else {
            continue;
        };
        out.push(GinaryInvocation {
            source: relative.to_owned(),
            site,
            line: line.trim().to_owned(),
            path,
            long_flags,
        });
    }
    out
}

/// One shell script as `(site, command line)` pairs, `\` continuations
/// joined onto the line they start on.
///
/// The site names the line the command *starts* on, because that is the line
/// a reader opens the file at; joining shifts every later number, so the
/// number is taken before the join rather than after it.
fn script_command_lines(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut joined = String::new();
    let mut start = 0usize;
    for (index, raw) in text.lines().enumerate() {
        if joined.is_empty() {
            start = index + 1;
        }
        match raw.strip_suffix('\\') {
            Some(head) => {
                joined.push_str(head);
                joined.push(' ');
            }
            None => {
                joined.push_str(raw);
                out.push((format!("line {start}"), std::mem::take(&mut joined)));
            }
        }
    }
    if !joined.is_empty() {
        out.push((format!("line {start}"), joined));
    }
    out
}

/// One shell word with its quoting and its subshell parenthesis removed.
fn unquote(token: &str) -> String {
    token
        .trim_start_matches('(')
        .trim_end_matches(')')
        .trim_matches(['"', '\''])
        .to_owned()
}

/// Whether a shell word names the ginary binary rather than another program.
///
/// A path (`target/release/ginary`), an interpolated path
/// (`$GITHUB_WORKSPACE/target/release/ginary`) and a whole-word interpolation
/// of a variable that holds the binary (`$GINARY_BIN`, which
/// `scripts/smoke.sh` uses and `ci.yml` sets) all count. A variable is
/// recognised by its name rather than by its value, which no scan can know:
/// `GINARY`-something-`BIN` is the shape this repository uses and the shape
/// `docs/dev/testing.md` documents.
fn is_ginary_program(token: &str) -> bool {
    if let Some(name) = token.strip_prefix('$') {
        let name = name.trim_start_matches('{').trim_end_matches('}');
        if name.starts_with("GINARY") && name.ends_with("BIN") {
            return true;
        }
    }
    let last = token.rsplit(['/', '\\']).next().unwrap_or(token);
    last == "ginary" || last == "ginary.exe"
}

/// Whether a shell word is a `VAR=value` assignment prefixed to a command.
///
/// `GINARY_CATALOG="$catalog" ginary build ..` is one command, and the
/// assignment does not stop the next word being the program.
fn is_assignment(token: &str) -> bool {
    let Some((name, _)) = token.split_once('=') else {
        return false;
    };
    !name.is_empty()
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !name.starts_with(|c: char| c.is_ascii_digit())
}

/// Whether the word after this one starts a new command.
///
/// The shell words that introduce one, and the operators that separate one
/// from the last. Everything else is an argument of the command already
/// running, which is what keeps `cp target/release/ginary dist/` from being
/// read as an invocation.
fn opens_a_command(token: &str) -> bool {
    matches!(
        token,
        "if" | "then"
            | "else"
            | "elif"
            | "do"
            | "while"
            | "until"
            | "!"
            | "{"
            | "&&"
            | "||"
            | "|"
            | ";"
    ) || token.ends_with("&&")
        || token.ends_with("||")
        || token.ends_with(';')
}

/// Whether a word ends the ginary command line: a redirection, or a separator
/// that hands what follows to the shell.
///
/// `ginary verify "$artifact" > "$log" 2>&1; then` passes ginary one argument
/// and the rest belongs to `sh`.
fn ends_the_command(token: &str) -> bool {
    token.starts_with('>')
        || token.starts_with('<')
        || token.contains(">>")
        || (token.contains('>') && token.starts_with(|c: char| c.is_ascii_digit()))
        || matches!(token, "&&" | "||" | "|" | ";" | "&")
        || token.ends_with(';')
}
