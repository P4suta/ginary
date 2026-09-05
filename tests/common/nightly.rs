// SPDX-License-Identifier: MIT OR Apache-2.0
//! The nightly assurance workflow, read as the two plans it runs.
//!
//! Two of the three heavy passes are configured twice — once in
//! `.github/workflows/nightly.yml` and once in `mise.toml`, so a developer can
//! run what CI runs — and a precondition one of them satisfies and the other
//! does not is invisible to a reader of either file. That is exactly how run
//! 33969332537's fuzz shards failed: `mise.toml`'s task creates
//! `fuzz/corpus/<target>` before it starts libFuzzer, the workflow passes the
//! same directory and creates nothing, and git tracks no empty directory. So
//! the two are reduced to a comparable shape here and compared.
//!
//! The mutation half is not a duplicate but a budget, and a budget is a claim
//! about time. `tests/fixtures/nightly/mutants-measured.json` is the measured
//! side of it — how many mutants a module produces and what one costs — and
//! this module reads the configured side so the two can be held against each
//! other. A gate that cannot finish inside its own `timeout-minutes` is not a
//! gate.

use std::collections::BTreeMap;

use saphyr::YamlOwned;

use crate::common::mise;
use crate::common::repo::{WorkflowStep, option_value, read, shell_code, workflow_steps, yaml};

/// The workflow both plans are read out of.
pub const NIGHTLY: &str = ".github/workflows/nightly.yml";

/// The measured record a mutation budget is argued from.
pub const MEASURED_MUTANTS: &str = "tests/fixtures/nightly/mutants-measured.json";

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

/// One row of the mutation matrix, and the `cargo mutants` it runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MutantsShard {
    /// The matrix row, `key=value` pairs joined by a space, for a message.
    pub row: String,
    /// The module named by `--file src/<module>.rs`, or the empty string when
    /// the command names no file and therefore mutates the whole crate.
    pub module: String,
    /// The `i` of `--shard i/n`, or 0 when the command names no shard.
    ///
    /// `cargo mutants` numbers a division from zero — it refuses a `k` that is
    /// not less than `n` — so an undivided command is shard 0 of 1.
    pub index: u64,
    /// The `n` of `--shard i/n`, or 1 when the command names no shard.
    pub shards: u64,
    /// The `--timeout` value, or [`None`] when a single mutant's test run is
    /// uncapped.
    pub timeout: Option<String>,
}

/// The mutation pass the nightly workflow configures.
#[derive(Debug)]
pub struct MutantsPlan {
    /// The job's `timeout-minutes`: the budget one shard has.
    pub timeout_minutes: u64,
    /// One entry per matrix row, in matrix order.
    pub shards: Vec<MutantsShard>,
}

/// The mutation pass as `.github/workflows/nightly.yml` configures it.
///
/// # Panics
///
/// If the workflow declares no `mutants` job, if the job declares no
/// `timeout-minutes`, or if the matrix mixes product keys with `include:`
/// rows — which GitHub reads as *extending* a combination rather than adding
/// one, and this reader deliberately does not model.
pub fn mutants_plan() -> MutantsPlan {
    let job = "mutants";
    let timeout_minutes = job_field(job, "timeout-minutes")
        .and_then(|value| value.as_integer())
        .and_then(|value| u64::try_from(value).ok())
        .unwrap_or_else(|| {
            panic!("{NIGHTLY}'s `{job}` job declares no `timeout-minutes`, so it has no budget")
        });

    let commands: Vec<String> = workflow_steps(NIGHTLY)
        .iter()
        .filter(|step| step.job == job)
        .flat_map(WorkflowStep::commands)
        .filter(|command| command.starts_with("cargo mutants"))
        .collect();

    let shards = matrix_rows(job)
        .into_iter()
        .flat_map(|row| {
            let rendered = row
                .iter()
                .map(|(key, value)| format!("{key}={value}"))
                .collect::<Vec<_>>()
                .join(" ");
            commands
                .iter()
                .map(|command| shard_of(&rendered, &substitute(command, &row)))
                .collect::<Vec<_>>()
        })
        .collect();

    MutantsPlan {
        timeout_minutes,
        shards,
    }
}

/// One `cargo mutants` command, read.
fn shard_of(row: &str, command: &str) -> MutantsShard {
    let module = option_value(command, "--file")
        .map(|value| {
            value
                .trim_start_matches("src/")
                .trim_end_matches(".rs")
                .to_owned()
        })
        .unwrap_or_default();
    let (index, shards) = option_value(command, "--shard")
        .and_then(|value| {
            let (index, shards) = value.split_once('/')?;
            Some((index.parse().ok()?, shards.parse().ok()?))
        })
        .unwrap_or((0, 1));
    MutantsShard {
        row: row.to_owned(),
        module,
        index,
        shards,
        timeout: option_value(command, "--timeout"),
    }
}

/// What run 33969332537 measured: the mutant count of each module and what one
/// mutant costs.
#[derive(Debug)]
pub struct MeasuredMutants {
    /// The nightly run the numbers were read from.
    pub run: String,
    /// How long a shard takes to reach `ok Unmutated baseline`.
    pub baseline_minutes: u64,
    /// What one mutant costs, build and test together.
    pub seconds_per_mutant: u64,
    /// How many mutants each module produces.
    pub modules: BTreeMap<String, u64>,
}

impl MeasuredMutants {
    /// The minutes a shard of `mutants` mutants needs, baseline included.
    pub fn minutes_for(&self, mutants: u64) -> u64 {
        self.baseline_minutes + mutants.saturating_mul(self.seconds_per_mutant).div_ceil(60)
    }
}

/// The measured record, parsed.
///
/// # Panics
///
/// If the fixture is not there, is not JSON, or is missing a field. It is the
/// evidence the budget rests on: a budget argued from a record nobody can read
/// is an assertion.
pub fn measured_mutants() -> MeasuredMutants {
    let text = read(MEASURED_MUTANTS);
    let parsed: serde_json::Value = serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("{MEASURED_MUTANTS} is not JSON: {error}"));
    let number = |key: &str| {
        parsed
            .get(key)
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_else(|| panic!("{MEASURED_MUTANTS} carries no `{key}` number"))
    };
    let modules = parsed
        .get("modules")
        .and_then(serde_json::Value::as_object)
        .unwrap_or_else(|| panic!("{MEASURED_MUTANTS} carries no `modules` object"))
        .iter()
        .map(|(module, count)| {
            let count = count
                .as_u64()
                .unwrap_or_else(|| panic!("{MEASURED_MUTANTS}'s `{module}` is not a count"));
            (module.clone(), count)
        })
        .collect();
    MeasuredMutants {
        run: parsed
            .get("run")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_else(|| panic!("{MEASURED_MUTANTS} names no `run`"))
            .to_owned(),
        baseline_minutes: number("baseline_minutes"),
        seconds_per_mutant: number("seconds_per_mutant"),
        modules,
    }
}

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

/// Every combination one job's matrix expands to, in matrix order.
///
/// The two shapes GitHub gives a matrix with no exclusions: the cartesian
/// product of its list-valued keys, and a list of `include:` rows. A matrix
/// that has both is refused rather than guessed at — GitHub reads an
/// `include:` row against an existing combination as an *extension* of it, and
/// a reader that appended it as a row of its own would report a pass that does
/// not exist.
///
/// # Panics
///
/// If the matrix mixes the two shapes.
fn matrix_rows(job: &str) -> Vec<BTreeMap<String, String>> {
    let Some(strategy) = job_field(job, "strategy") else {
        return vec![BTreeMap::new()];
    };
    let Some(matrix) = strategy
        .as_mapping_get("matrix")
        .and_then(YamlOwned::as_mapping)
    else {
        return vec![BTreeMap::new()];
    };

    let mut product: Vec<BTreeMap<String, String>> = vec![BTreeMap::new()];
    let mut keys = 0usize;
    let mut include: Vec<BTreeMap<String, String>> = Vec::new();
    for (key, value) in matrix {
        let Some(key) = key.as_str() else {
            continue;
        };
        if key == "include" {
            for row in value.as_vec().map(Vec::as_slice).unwrap_or_default() {
                include.push(mapping_of(row));
            }
            continue;
        }
        let Some(values) = value.as_vec() else {
            continue;
        };
        keys += 1;
        product = product
            .into_iter()
            .flat_map(|row| {
                values.iter().filter_map(move |value| {
                    let value = value.as_str()?;
                    let mut row = row.clone();
                    row.insert(key.to_owned(), value.to_owned());
                    Some(row)
                })
            })
            .collect();
    }

    assert!(
        keys == 0 || include.is_empty(),
        "{NIGHTLY}'s `{job}` matrix mixes product keys with `include:` rows. GitHub reads an \
         `include:` row that matches an existing combination as an extension of it, and this \
         reader does not model that: write the matrix as one shape or the other"
    );
    if keys == 0 {
        return include;
    }
    product
}

/// One YAML mapping as string pairs.
fn mapping_of(node: &YamlOwned) -> BTreeMap<String, String> {
    node.as_mapping()
        .map(|mapping| {
            mapping
                .iter()
                .filter_map(|(key, value)| {
                    let key = key.as_str()?.to_owned();
                    let value = value
                        .as_str()
                        .map(str::to_owned)
                        .or_else(|| value.as_integer().map(|number| number.to_string()))?;
                    Some((key, value))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// `command` with every `${{ matrix.<key> }}` of `row` substituted.
fn substitute(command: &str, row: &BTreeMap<String, String>) -> String {
    let mut out = command.to_owned();
    for (key, value) in row {
        for spelling in [
            format!("${{{{ matrix.{key} }}}}"),
            format!("${{{{matrix.{key}}}}}"),
        ] {
            out = out.replace(&spelling, value);
        }
    }
    out
}
