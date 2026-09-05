// SPDX-License-Identifier: MIT OR Apache-2.0
//! The guard over `mise run clean:cache` treats a quoted argument as text, and
//! a command substitution inside one is not text: bash runs it.
//!
//! **What went wrong.** `crate::common::mise::segments` stops splitting inside
//! `'…'` and `"…"`, which is right for a separator — `echo "a; b"` is one
//! command — and wrong for a substitution. `$( … )` inside double quotes is
//! executed, and its output is what gets quoted. So
//!
//! ```sh
//! echo "$(rm -rf target/stubs)"
//! ```
//!
//! reads to the scan as a single `echo` with one quoted argument: the verb is
//! `echo`, which is on the cleaner allowlist, and `removed_paths()` finds no
//! command whose first word is `rm`. Every rule in `cleaner_violations`
//! passes, the `clean_cache_plan` snapshot stays byte-identical, and bash
//! deletes the five cross-built stubs. The same hole swallows a verb the
//! allowlist exists to refuse — `echo "$(find target/stubs -delete)"` — and
//! the backtick spelling of the same thing.
//!
//! The module's own doc-comment says a `$( … )` is deliberately *not*
//! protected, and gives `before=$(du … | awk …)` as the reason. That is true
//! only of an unquoted one. The quoted spelling walks around the guard, and a
//! guard that can be walked around is decoration.
//!
//! **The input.** Any future edit of `[tasks."clean:cache"]` that puts a
//! removal inside a quoted substitution. Nothing exotic: capturing what a
//! command printed is how a run block reports what it reclaimed, and the
//! committed cleaner already assigns from `$(du … | awk …)` twice.
//!
//! **The correct behaviour.** The contents of a command substitution are read
//! as commands wherever the substitution appears, quoted or not, in both its
//! spellings. Quoting still protects a separator that is only text: `echo "a;
//! b"` stays one command, and a quoted string that merely *mentions* `rm` is
//! not a removal.

use crate::common::mise::{MiseTask, cleaner_violations};

/// The two trees the cleaner must leave alone, as `tests/ci_matrix.rs` states
/// them.
const PRECIOUS: [&str; 2] = ["dist/otp", "target/stubs"];

/// A run block that is the committed cleaner's prologue — the root check, the
/// `cd` into the root, one plain `rm` — with `$tail` appended.
///
/// The same prologue `e20_a_removal_the_cleaner_rule_could_not_see.rs` uses,
/// and for the same reason: every case below has to break exactly one rule, so
/// that a non-empty violation list is attributable to the deletion it hides
/// rather than to a missing root check.
macro_rules! after_the_root_check {
    ($tail:literal) => {
        concat!(
            "set -eu\n",
            "root=\"${MISE_PROJECT_ROOT:-$PWD}\"\n",
            "if [ ! -f \"$root/Cargo.toml\" ] || [ ! -f \"$root/mise.toml\" ]; then\n",
            "  echo \"clean:cache: $root is not a project root; nothing removed\" >&2\n",
            "  exit 1\n",
            "fi\n",
            "cd \"$root\"\n",
            "rm -rf target/debug\n",
            $tail,
        )
    };
}

/// A run block whose deletion of a kept tree is inside a quoted command
/// substitution, paired with the plain `rm` that makes the rest of the rule
/// look satisfied.
const REMOVALS_INSIDE_A_QUOTED_SUBSTITUTION: [(&str, &str); 4] = [
    (
        "an rm inside a quoted $( )",
        after_the_root_check!("echo \"$(rm -rf target/stubs)\"\n"),
    ),
    (
        "an rm inside a quoted $( ) on the right of an assignment",
        after_the_root_check!("kept=\"$(rm -rf dist/otp)\"\n"),
    ),
    (
        "a find -delete inside a quoted $( )",
        after_the_root_check!("echo \"$(find target/stubs -type f -delete)\"\n"),
    ),
    (
        "an rm inside a quoted backtick substitution",
        after_the_root_check!("echo \"`rm -rf target/stubs`\"\n"),
    ),
];

#[test]
fn a_removal_inside_a_quoted_command_substitution_is_refused() {
    for (label, run) in REMOVALS_INSIDE_A_QUOTED_SUBSTITUTION {
        let task = MiseTask::from_run("clean:cache", run);
        let violations = cleaner_violations(&task, &PRECIOUS);
        assert!(
            !violations.is_empty(),
            "the rule approved a cleaner that deletes a kept tree with `{label}`. A command \
             substitution is executed wherever it appears — the quotes go round its *output* — \
             so this run block removes `target/stubs` while every guard reads one harmless \
             `echo`:\n{run}"
        );
    }
}

#[test]
fn a_removal_inside_a_quoted_command_substitution_is_in_the_removal_list() {
    // The other half. `cleaner_violations` may refuse the line through its
    // verb; the removal *list* is what the `clean_cache_plan` snapshot in
    // `tests/ci_matrix.rs` is taken over, and a deletion missing from it is a
    // deletion no reviewer reads off the diff.
    let task = MiseTask::from_run(
        "clean:cache",
        after_the_root_check!("echo \"$(rm -rf target/stubs)\"\n"),
    );

    let removed = task.removed_paths();

    assert!(
        removed.iter().any(|path| path == "target/stubs"),
        "`rm -rf target/stubs` inside a quoted substitution is a removal like any other, and the \
         scan answered {removed:?}"
    );
}

#[test]
fn a_separator_inside_a_plain_quoted_string_is_still_text() {
    // The calibration, and the reason the fix is "read inside a substitution"
    // rather than "stop reading quotes". A quoted argument that merely
    // mentions a removal runs nothing, and a rule that refused it would refuse
    // the committed cleaner's own diagnostic — `echo "clean:cache: $root is
    // not a project root; nothing removed" >&2` — which is one command
    // carrying one `;`.
    let task = MiseTask::from_run(
        "clean:cache",
        after_the_root_check!("echo \"tidying; rm -rf target/stubs is what this does not do\"\n"),
    );

    let violations = cleaner_violations(&task, &PRECIOUS);
    let removed = task.removed_paths();

    assert!(
        violations.is_empty(),
        "quoting still protects text. This block runs one `echo` and removes only \
         `target/debug`:\n- {}",
        violations.join("\n- ")
    );
    assert_eq!(
        removed,
        vec!["target/debug".to_owned()],
        "and the removal list holds the one removal there is"
    );
}
