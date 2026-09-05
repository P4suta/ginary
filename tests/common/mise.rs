// SPDX-License-Identifier: MIT OR Apache-2.0
//! Reading a task out of `mise.toml`.
//!
//! `mise.toml` is the one committed file that is neither YAML nor a `.sh` and
//! still holds shell this repository runs — `tests/ci_matrix.rs` already reads
//! it as text for the privileged-container rule. A task whose whole point is
//! *what it does not delete* needs more than a substring: the removals have to
//! be listed, so the rule can be stated over the list rather than over the
//! spelling of one line.
//!
//! A hand-rolled scanner rather than the `toml` crate, for the reason
//! `tests/common/deps.rs` gives: `toml` is behind the `cli` feature, and these
//! assertions hold for the stub flavor too. The grammar it reads is the one
//! `mise.toml` is written in and nothing wider — a `[tasks."name"]` or
//! `[tasks.name]` header, a `description = "..."` line, and a `run` that is a
//! single string, a `'''`/`"""` block, or an array of strings. Anything else
//! is not read, which is a stated limit rather than half a TOML parser.

use crate::common::repo::{read, shell_code};

/// One task of `mise.toml`.
pub struct MiseTask {
    /// The task's name, as it is written after `mise run`.
    pub name: String,
    /// The `description = "..."` line, or the empty string when there is none.
    pub description: String,
    /// Every line of the task's `run`, in order, comments and all.
    pub run: Vec<String>,
}

impl MiseTask {
    /// A task built from a run block rather than read out of `mise.toml`.
    ///
    /// The rule in [`cleaner_violations`] is stated over a task, and a rule is
    /// only as good as the shell it refuses. This is how a test hands it the
    /// shell this repository does *not* carry — the `find … -delete` that the
    /// removal scan cannot see, the `cd` that runs after the removals — so the
    /// refusal is exercised rather than assumed.
    pub fn from_run(name: &str, run: &str) -> Self {
        Self {
            name: name.to_owned(),
            description: String::new(),
            run: run.lines().map(str::to_owned).collect(),
        }
    }

    /// The task's shell, one command per line, with comments removed.
    ///
    /// Blank lines are dropped so a caller can iterate over commands without
    /// filtering. Continuations are *not* joined: no rule here needs them, and
    /// the two that read this file — the removal list and the `cargo clean`
    /// refusal — both name a command by its first word.
    pub fn commands(&self) -> Vec<String> {
        self.run
            .iter()
            .map(|line| shell_code(line).trim().to_owned())
            .filter(|line| !line.is_empty())
            .collect()
    }

    /// Every path the task removes with `rm`, sorted and deduplicated.
    ///
    /// Each line is split into segments at the shell's separators first, so an
    /// `rm` that runs after an `&&`, an `||`, a `;` or a `|` contributes its
    /// paths like any other; reading only the first word of the whole line saw
    /// `[ -d target/stubs ] && rm -rf target/stubs` as a `[` and nothing more.
    ///
    /// Options (`-rf`, `--recursive`) are skipped and a surrounding pair of
    /// quotes is stripped. A word carrying a `$` is returned as it is written,
    /// on purpose: a removal built out of a variable is one this scan cannot
    /// resolve, and the rule over the list refuses it by name rather than
    /// guessing what it expands to.
    pub fn removed_paths(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for command in self.commands() {
            for segment in segments(&command) {
                let mut words = segment
                    .split_whitespace()
                    .skip_while(|word| CONTROL_WORDS.contains(word));
                if words.next() != Some("rm") {
                    continue;
                }
                for word in words {
                    if word.starts_with('-') {
                        continue;
                    }
                    out.push(unquote(word).to_owned());
                }
            }
        }
        out.sort();
        out.dedup();
        out
    }
}

/// A word with one surrounding pair of `'` or `"` removed.
fn unquote(word: &str) -> &str {
    for quote in ['\'', '"'] {
        if let Some(inner) = word.strip_prefix(quote)
            && let Some(inner) = inner.strip_suffix(quote)
        {
            return inner;
        }
    }
    word
}

/// The task `name`, or `None` when `mise.toml` declares no such task.
pub fn task(name: &str) -> Option<MiseTask> {
    let text = read("mise.toml");
    let quoted = format!("[tasks.\"{name}\"]");
    let bare = format!("[tasks.{name}]");
    let mut lines = text.lines();
    lines.find(|line| {
        let line = line.trim();
        line == quoted || line == bare
    })?;

    let mut description = String::new();
    let mut run: Vec<String> = Vec::new();
    // What the reader is in the middle of: a `'''`/`"""` block or an array.
    let mut block: Option<char> = None;
    let mut in_array = false;

    for line in lines {
        let trimmed = line.trim();
        if let Some(fence) = block {
            if trimmed == fence.to_string().repeat(3) {
                block = None;
                continue;
            }
            run.push(line.to_owned());
            continue;
        }
        if in_array {
            if trimmed == "]" {
                in_array = false;
                continue;
            }
            run.push(unquote(trimmed.trim_end_matches(',')).to_owned());
            continue;
        }
        // A new table header ends the task.
        if trimmed.starts_with('[') {
            break;
        }
        if let Some(value) = trimmed.strip_prefix("description = ") {
            description = unquote(value).to_owned();
            continue;
        }
        let Some(value) = trimmed.strip_prefix("run = ") else {
            continue;
        };
        match value {
            "'''" => block = Some('\''),
            "\"\"\"" => block = Some('"'),
            "[" => in_array = true,
            single => run.push(unquote(single).to_owned()),
        }
    }

    Some(MiseTask {
        name: name.to_owned(),
        description,
        run,
    })
}

/// The verbs a cache cleaner's shell may use.
///
/// An allowlist rather than a list of forbidden ones, because the rule below
/// is about deletions the scan cannot see, and the ways to spell a deletion are
/// not enumerable: `find … -delete`, `find … -exec rm {} +`, `xargs rm`,
/// `rsync --delete`, `cargo clean`, `git clean -x`. Every one of them fails
/// this list until somebody adds it deliberately and says why — which is the
/// review the list exists to force. The list is applied to every command a
/// line runs, not to the line's first word, so a deletion hidden after an
/// `&&`, an `||`, a `;` or a `|` is read like any other; see [`segments`].
///
/// `[` is the `test` builtin, which is how the root check is written; `if`,
/// `!`, `then`, `elif`, `while` and `until` are control words and are skipped
/// rather than listed, so the verb they introduce is the one that is checked.
pub const CLEANER_VERBS: [&str; 11] = [
    "[", "awk", "cd", "du", "echo", "exit", "fi", "grep", "rm", "set", "test",
];

/// The shell words that introduce a command rather than being one.
const CONTROL_WORDS: [&str; 8] = ["if", "!", "then", "elif", "else", "while", "until", "do"];

/// Every way `task` breaks the rule a cache cleaner is held to, one sentence
/// each, empty when it breaks none.
///
/// `precious` is the list of trees the cleaner must leave alone, written as
/// paths relative to the project root — the same form the removals are written
/// in, because a rule comparing two different spellings of a path is a rule
/// that passes by accident.
///
/// The rule lives here rather than inline in `tests/ci_matrix.rs` so it can be
/// driven over shell this repository does not contain. A rule that has only
/// ever seen the task it approves has never been shown to refuse anything; see
/// `tests/regressions/e20_a_removal_the_cleaner_rule_could_not_see.rs`.
///
/// What it holds a cleaner to:
///
/// 1. it removes something, and every removal is a literal relative path;
/// 2. no removal is, or contains, one of `precious`;
/// 3. every verb it uses is in [`CLEANER_VERBS`], so a deletion spelled some
///    other way cannot hide from the removal scan;
/// 4. it changes into the project root before the first removal, and into the
///    root itself rather than a directory inside it, because that is what the
///    relative paths are relative to;
/// 5. it satisfies itself that the root is this project's before it removes
///    anything, and refuses with a non-zero exit when it is not.
pub fn cleaner_violations(task: &MiseTask, precious: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let removed = task.removed_paths();
    if removed.is_empty() {
        out.push(
            "the task removes nothing, so it reclaims nothing and every rule below is measuring \
             an empty list"
                .to_owned(),
        );
    }
    for path in &removed {
        if path.contains('$') || path.contains('*') {
            out.push(format!(
                "`{path}` is built out of a variable or a glob, so what it removes cannot be read \
                 off the diff. Every removal is a literal path relative to the project root, \
                 because that is the only form a reviewer can check against the list of what must \
                 survive"
            ));
        }
        let path = path.trim_end_matches('/');
        for keep in precious {
            if path == *keep {
                out.push(format!(
                    "the cleaner removes `{keep}`, which is exactly what it must not: it costs \
                     `cross`, a docker daemon and minutes per target to rebuild, and nine gated \
                     tests read it"
                ));
            }
            if keep.starts_with(&format!("{path}/")) {
                out.push(format!(
                    "the cleaner removes `{path}`, which contains `{keep}`. Reclaiming the 26 GB \
                     of build output must not take the 165 MB beside it that costs an afternoon"
                ));
            }
        }
    }

    let commands = task.commands();
    for command in &commands {
        for verb in verbs(command) {
            if !CLEANER_VERBS.contains(&verb.as_str()) {
                out.push(format!(
                    "`{verb}` is not one of the verbs a cleaner may use ({}). `{command}` may \
                     delete something the removal scan, which reads `rm` and nothing else, cannot \
                     see — and what a cleaner deletes is the whole of what this rule is about",
                    CLEANER_VERBS.join(", ")
                ));
            }
        }
    }

    let first_removal = commands
        .iter()
        .position(|command| verbs(command).contains(&"rm".to_owned()));
    let change_directory = commands
        .iter()
        .position(|command| command.split_whitespace().next() == Some("cd"));
    match (first_removal, change_directory) {
        (Some(removal), None) => out.push(format!(
            "the removals are relative paths, so the task has to change to the project root \
             before it runs any of them; `{}` runs wherever the caller happened to be",
            commands[removal]
        )),
        (Some(removal), Some(directory)) if directory > removal => out.push(format!(
            "`{}` runs before the `cd`, so it removes a path relative to the caller's directory \
             rather than to the project root. A `cd` after the removals anchors nothing",
            commands[removal]
        )),
        _ => {}
    }
    if let Some(directory) = change_directory {
        let assignments = assignments_before(&commands, directory);
        let target = commands[directory]
            .split_whitespace()
            .nth(1)
            .unwrap_or_default();
        if !resolves_to_project_root(target, &assignments) {
            out.push(format!(
                "`{}` does not change into the project root itself. Every path in `precious` is \
                 written relative to that root, so a cleaner working from anywhere else removes \
                 `stubs` while the rule looks for `target/stubs` and finds no match",
                commands[directory]
            ));
        }
    }

    let before_removal = &commands[..first_removal.unwrap_or(commands.len())];
    let checks_the_root = before_removal
        .iter()
        .any(|command| command.contains("Cargo.toml"));
    let refuses = before_removal
        .iter()
        .any(|command| command.starts_with("exit ") && command.trim() != "exit 0");
    if !removed.is_empty() && !(checks_the_root && refuses) {
        out.push(
            "`MISE_PROJECT_ROOT` is set by `mise` and by nothing else, so a `:-$PWD` fallback is \
             whatever directory the caller was in. Before the first removal the task has to \
             satisfy itself that the root is this project's — a marker such as `Cargo.toml` — and \
             refuse with a non-zero `exit` when it is not"
                .to_owned(),
        );
    }
    out
}

/// Every command `command` runs, one per segment between the shell's
/// separators.
///
/// A separator is an unquoted `|`, `;` or `&`, which covers `&&` and `||` as
/// the pair of empty-separated segments they split into. Splitting on them is
/// what makes `find … | xargs rm` two commands rather than one, and
/// `[ -d target/stubs ] && rm -rf target/stubs` a `[` *and* an `rm` rather
/// than a `[` alone.
///
/// Two things are deliberately not separators. One inside `'…'` or `"…"` is
/// text — `echo "no Cargo.toml here; nothing removed"` is one command — and an
/// `&` immediately after a `>` or a `<` is part of a redirection, so `>&2`
/// stays attached to the `echo` it belongs to. A `$( … )` is *not* protected:
/// splitting inside it is what gives `before=$(du … | awk …)` both its verbs.
///
/// Segments that are entirely whitespace are dropped, so a caller sees one
/// entry per command rather than one per separator character.
fn segments(command: &str) -> Vec<&str> {
    let mut out: Vec<&str> = Vec::new();
    let mut start = 0;
    let mut quote: Option<char> = None;
    let mut previous = ' ';
    for (index, character) in command.char_indices() {
        match quote {
            Some(open) if character == open => quote = None,
            Some(_) => {}
            None if character == '\'' || character == '"' => quote = Some(character),
            None if matches!(character, '|' | ';')
                || (character == '&' && !matches!(previous, '>' | '<')) =>
            {
                out.push(&command[start..index]);
                start = index + character.len_utf8();
            }
            None => {}
        }
        previous = character;
    }
    out.push(&command[start..]);
    out.retain(|segment| !segment.trim().is_empty());
    out
}

/// The verb of every segment of `command`.
///
/// An assignment is unwrapped to the command inside its `$( … )`, if it has
/// one, so `before=$(du …)` is a `du` and `kept=$(rm …)` is an `rm`.
fn verbs(command: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for segment in segments(command) {
        let mut words = segment
            .split_whitespace()
            .skip_while(|word| CONTROL_WORDS.contains(word));
        let Some(word) = words.next() else {
            continue;
        };
        let word = match word.split_once('=') {
            Some((name, value)) if is_name(name) => match value.strip_prefix("$(") {
                Some(inner) => inner,
                // A literal assignment runs no command at all.
                None => continue,
            },
            _ => word,
        };
        out.push(word.trim_end_matches(';').to_owned());
    }
    out
}

/// Whether `word` is a shell variable name.
fn is_name(word: &str) -> bool {
    !word.is_empty()
        && word
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
        && !word.starts_with(|character: char| character.is_ascii_digit())
}

/// Every literal assignment among the commands before index `upto`.
fn assignments_before(commands: &[String], upto: usize) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for command in &commands[..upto] {
        let Some(word) = command.split_whitespace().next() else {
            continue;
        };
        if let Some((name, value)) = word.split_once('=')
            && is_name(name)
        {
            out.push((name.to_owned(), value.to_owned()));
        }
    }
    out
}

/// Whether `target` is the project root itself rather than some directory
/// derived from it.
///
/// One level of indirection is resolved, because `root="${MISE_PROJECT_ROOT:-$PWD}"`
/// followed by `cd "$root"` is the shape a task with a root check is written
/// in. Anything else — a suffix after the expansion, a name assigned from
/// somewhere the scan cannot follow — is not the root.
fn resolves_to_project_root(target: &str, assignments: &[(String, String)]) -> bool {
    let target = unquote(target);
    if !is_single_expansion(target) {
        return false;
    }
    if target.contains("MISE_PROJECT_ROOT") {
        return true;
    }
    let name = expansion_name(target);
    assignments
        .iter()
        .rev()
        .find(|(assigned, _)| *assigned == name)
        .is_some_and(|(_, value)| {
            let value = unquote(value);
            is_single_expansion(value) && value.contains("MISE_PROJECT_ROOT")
        })
}

/// Whether `word` is one parameter expansion and nothing else: `$name` or
/// `${name…}` with nothing after the closing brace.
fn is_single_expansion(word: &str) -> bool {
    let Some(rest) = word.strip_prefix('$') else {
        return false;
    };
    match rest.strip_prefix('{') {
        Some(inner) => match inner.strip_suffix('}') {
            Some(inner) => !inner.contains('}'),
            None => false,
        },
        None => !rest.is_empty() && is_name(rest),
    }
}

/// The variable a single expansion names.
fn expansion_name(word: &str) -> String {
    let word = word.trim_start_matches('$');
    let word = word.trim_start_matches('{').trim_end_matches('}');
    word.split([':', '-', '}'])
        .next()
        .unwrap_or_default()
        .to_owned()
}
