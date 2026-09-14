// SPDX-License-Identifier: MIT OR Apache-2.0
//! The nightly assurance workflow, read as the plan it runs.
//!
//! The fuzz pass is configured twice — once in `.github/workflows/nightly.yml`
//! and once in `mise.toml`, so a developer can run what CI runs — and a
//! precondition one of them satisfies and the other does not is invisible to a
//! reader of either file. That is exactly how run 33969332537's fuzz shards
//! failed: `mise.toml`'s task creates `fuzz/corpus/<target>` before it starts
//! libFuzzer, the workflow passes the same directory and creates nothing, and
//! git tracks no empty directory. So the two are reduced to a comparable shape
//! here and compared.

use saphyr::YamlOwned;

use crate::common::mise;
use crate::common::repo::{WorkflowStep, shell_code, workflow_steps, yaml};

/// The workflow both plans are read out of.
pub const NIGHTLY: &str = ".github/workflows/nightly.yml";

/// What a fuzz target's name is replaced by, so the workflow's
/// `${{ matrix.target }}` and the task's `"$target"` reduce to one shape.
pub const TARGET: &str = "<target>";

// ------------------------------------------------------------ the fuzzers --

/// How one of the two callers runs the fuzz targets.
///
/// Only the parts that can drift are kept. The toolchain is legitimately
/// different — the workflow installs nightly with an action and names
/// `--target` for the sanitizer, the task says `cargo +nightly` — and a
/// comparison that included them would fail for a reason nobody should fix.
#[derive(Debug, PartialEq, Eq)]
pub struct FuzzPlan {
    /// Where this plan was read from, for a failure message.
    pub source: String,
    /// The targets it runs, in the order it names them.
    pub targets: Vec<String>,
    /// The directories it creates before the first `cargo fuzz run`.
    pub creates: Vec<String>,
    /// The directories it passes to `cargo fuzz run`, in argument order.
    pub directories: Vec<String>,
    /// The libFuzzer arguments after the `--`.
    pub flags: Vec<String>,
}

impl FuzzPlan {
    /// The plan `.github/workflows/nightly.yml`'s `fuzz` job runs.
    ///
    /// # Panics
    ///
    /// If the workflow declares no `fuzz` job, or its matrix names no targets.
    pub fn from_workflow() -> Self {
        let job = "fuzz";
        let targets = matrix_values(job, "target");
        assert!(
            !targets.is_empty(),
            "{NIGHTLY}'s `{job}` job declares no `matrix.target`, so there is no plan to read"
        );
        let commands: Vec<String> = workflow_steps(NIGHTLY)
            .iter()
            .filter(|step| step.job == job)
            .flat_map(WorkflowStep::commands)
            .collect();
        Self::read(
            &format!("{NIGHTLY} job `{job}`"),
            targets,
            &commands,
            &["${{ matrix.target }}", "${{matrix.target}}"],
        )
    }

    /// The plan `mise run fuzz` runs.
    ///
    /// # Panics
    ///
    /// If `mise.toml` declares no `fuzz` task, or its loop names no targets.
    pub fn from_mise() -> Self {
        let task = mise::task("fuzz").expect("mise.toml declares a [tasks.fuzz]");
        // Continuations are joined here rather than by `MiseTask::commands`,
        // which deliberately does not join them: the rules that reader was
        // written for name a command by its first word, and this one reads the
        // arguments — the libFuzzer flags of the committed task are on the
        // second line of the `cargo fuzz run` it belongs to. The joining is
        // the same `WorkflowStep::commands` does, so both sides of the
        // comparison are shaped alike.
        let script = task.run.join("\n").replace("\\\n", " ");
        let commands: Vec<String> = script
            .lines()
            .map(|line| shell_code(line).trim().to_owned())
            .filter(|line| !line.is_empty())
            .collect();
        let targets = loop_values(&commands, "target");
        assert!(
            !targets.is_empty(),
            "mise.toml's `fuzz` task names no targets in a `for target in ...` line, so there is \
             no plan to read"
        );
        Self::read(
            "mise.toml task `fuzz`",
            targets,
            &commands,
            &["\"$target\"", "${target}", "$target"],
        )
    }

    /// The shape both callers reduce to.
    ///
    /// The stated limit: a directory argument is a word carrying a `/`, which
    /// is what separates `fuzz/corpus/<target>` from the target's own name and
    /// from an option's value. No committed caller passes a directory without
    /// one, and half a `cargo fuzz` argument parser would be worse than a
    /// limit written down.
    fn read(
        source: &str,
        targets: Vec<String>,
        commands: &[String],
        placeholders: &[&str],
    ) -> Self {
        let mut creates: Vec<String> = Vec::new();
        let mut directories: Vec<String> = Vec::new();
        let mut flags: Vec<String> = Vec::new();
        let mut started = false;

        for command in commands {
            let words = words_of(command, placeholders);
            let Some(verb) = words.first() else {
                continue;
            };
            if verb == "mkdir" && !started {
                creates.extend(
                    words[1..]
                        .iter()
                        .filter(|word| !word.starts_with('-'))
                        .cloned(),
                );
                continue;
            }
            let Some(run) = fuzz_run_arguments(&words) else {
                continue;
            };
            started = true;
            let mut after_the_separator = false;
            for word in run {
                if word == "--" {
                    after_the_separator = true;
                    continue;
                }
                if after_the_separator {
                    flags.push(word.clone());
                } else if word.contains('/') && !word.starts_with('-') {
                    directories.push(word.clone());
                }
            }
        }

        Self {
            source: source.to_owned(),
            targets,
            creates,
            directories,
            flags,
        }
    }

    /// The plan as text, without the source line, so two plans that agree
    /// render identically.
    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str("targets\n");
        for target in &self.targets {
            out.push_str(&format!("  {target}\n"));
        }
        out.push_str("creates before it starts\n");
        for path in &self.creates {
            out.push_str(&format!("  {path}\n"));
        }
        out.push_str("passes to cargo fuzz run\n");
        for path in &self.directories {
            out.push_str(&format!("  {path}\n"));
        }
        out.push_str("libFuzzer arguments\n");
        for flag in &self.flags {
            out.push_str(&format!("  {flag}\n"));
        }
        out
    }

    /// Every directory the plan passes that it does not create first.
    pub fn uncreated(&self, under: &str) -> Vec<String> {
        self.directories
            .iter()
            .filter(|path| path.starts_with(under))
            .filter(|path| !self.creates.contains(path))
            .cloned()
            .collect()
    }
}

/// The arguments of a `cargo fuzz run`, or `None` when `words` is some other
/// command.
///
/// `cargo fuzz run` and `cargo +nightly fuzz run` are the two spellings this
/// repository uses, so the toolchain word is stepped over rather than listed.
fn fuzz_run_arguments(words: &[String]) -> Option<&[String]> {
    let mut rest = words.strip_prefix(&["cargo".to_owned()])?;
    if rest.first().is_some_and(|word| word.starts_with('+')) {
        rest = &rest[1..];
    }
    let rest = rest.strip_prefix(&["fuzz".to_owned(), "run".to_owned()])?;
    Some(rest)
}

/// One command as words, with every placeholder replaced by [`TARGET`] and
/// every quote removed.
///
/// The substitution happens before the split because
/// `${{ matrix.target }}` carries spaces.
fn words_of(command: &str, placeholders: &[&str]) -> Vec<String> {
    let mut text = command.to_owned();
    for placeholder in placeholders {
        text = text.replace(placeholder, TARGET);
    }
    text.split_whitespace()
        .map(|word| word.replace(['\'', '"'], ""))
        .collect()
}

/// The words a `for <name> in a b c; do` line lists.
fn loop_values(commands: &[String], name: &str) -> Vec<String> {
    let opening = format!("for {name} in ");
    for command in commands {
        let Some(rest) = command.trim().strip_prefix(&opening) else {
            continue;
        };
        let rest = rest.trim();
        let rest = rest.strip_suffix("do").unwrap_or(rest).trim();
        let rest = rest.strip_suffix(';').unwrap_or(rest);
        return rest
            .split_whitespace()
            .map(|word| word.replace(['\'', '"'], ""))
            .collect();
    }
    Vec::new()
}

// ------------------------------------------------------------ the mutants --

// -------------------------------------------------------- reading the YAML --

/// One field of one job of the nightly workflow.
fn job_field(job: &str, field: &str) -> Option<YamlOwned> {
    yaml(NIGHTLY)
        .as_mapping_get("jobs")?
        .as_mapping_get(job)?
        .as_mapping_get(field)
        .cloned()
}

/// The values one matrix key lists.
fn matrix_values(job: &str, key: &str) -> Vec<String> {
    let Some(strategy) = job_field(job, "strategy") else {
        return Vec::new();
    };
    strategy
        .as_mapping_get("matrix")
        .and_then(|matrix| matrix.as_mapping_get(key))
        .and_then(YamlOwned::as_vec)
        .map(|values| {
            values
                .iter()
                .filter_map(|value| value.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}
