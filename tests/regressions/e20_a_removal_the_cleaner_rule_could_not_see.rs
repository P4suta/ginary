// SPDX-License-Identifier: MIT OR Apache-2.0
//! The rule that says what `mise run clean:cache` must never delete read only
//! commands whose first word was `rm`, and only asked that a `cd` appear
//! somewhere. A deletion spelled any other way, or a `cd` in the wrong place or
//! to the wrong directory, was invisible to it.
//!
//! **What went wrong.** `removed_paths()` skips every command that does not
//! start with `rm`, and it was the only input to both the rule and the
//! `clean_cache_plan` snapshot. So a task of
//!
//! ```sh
//! rm -rf target/debug
//! find target -mindepth 1 -maxdepth 1 -name stubs -exec rm -rf {} +
//! ```
//!
//! answered `["target/debug"]`: the non-empty guard passed, the `$`/`*` guard
//! passed, the rule about `target/stubs` passed, and the snapshot was
//! byte-identical to the approved one. The same held for `find … -delete`,
//! `find … | xargs rm` and `rsync --delete`. The `cd` assertion was
//! order-blind and target-blind in the same way: it required only that *some*
//! command start with `cd `, so a `cd` after the removals satisfied it, and so
//! did `cd "${MISE_PROJECT_ROOT}/target"` with the paths shortened to `debug`,
//! `release` and `stubs` — against which the rule compares `"stubs"` to
//! `"target/stubs"` and finds no match.
//!
//! **The input.** Any edit to `[tasks."clean:cache"]` that reaches for a verb
//! other than `rm`, or that moves the `cd`. The test existed precisely to pin
//! what the cleaner must not delete, and it would have passed while the cleaner
//! deleted `target/stubs`.
//!
//! **And once more, one separator along.** The first fix read one verb per
//! *pipeline segment*, splitting a line on `|` and nothing else, so a deletion
//! joined to a harmless command with `&&`, `||` or `;` was still unread:
//! `[ -d target/stubs ] && rm -rf target/stubs` answered the verb `[`, and
//! `removed_paths()` — which took the first word of the whole line — answered
//! `["target/debug"]` again. Both halves now split on every one of the shell's
//! separators, with a separator inside quotes treated as the text it is and
//! `>&2` left attached to its redirection.
//!
//! **The correct behaviour.** The rule reads the whole run block, not the `rm`
//! lines of it. Every command's verb has to be one of a small
//! reviewed allowlist, so `find`, `xargs` and `cargo` fail the rule until
//! somebody adds them and explains why; the `cd` has to come before the first
//! removal and has to name the project root itself rather than a directory
//! inside it; and the root has to be checked before anything is removed. The
//! rule lives in `crate::common::mise::cleaner_violations` so it can be shown
//! refusing the shell this repository does not carry, which is the only way to
//! know it refuses anything at all.

use crate::common::mise::{MiseTask, cleaner_violations};

/// The two trees the cleaner must leave alone, as `tests/ci_matrix.rs` states
/// them.
const PRECIOUS: [&str; 2] = ["dist/otp", "target/stubs"];

/// A run block that is the committed cleaner's prologue — the root check, the
/// `cd` into the root, one plain `rm` — with `$tail` appended.
///
/// The prologue matters. Held against a run block that never checks its root,
/// every case below would be refused for *that*, and the refusal this file is
/// about would go unmeasured. Each case here breaks exactly one rule, so a
/// non-empty violation list is attributable to the deletion it hides.
///
/// The `;` inside the quoted `echo` is deliberate: the rule splits a compound
/// command on the shell's separators, and a separator inside quotes is text
/// rather than a separator.
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

/// A run block that removes a kept tree behind shell the scan did not read,
/// paired with the plain `rm` that made the rest of the rule look satisfied.
///
/// The first four hide the deletion behind a verb other than `rm`. The last
/// three hide it behind a *separator*: the verb the rule reads is the harmless
/// one on the left of an `&&`, `||` or `;`, and the deletion on the right was
/// never examined at all.
const REMOVALS_THE_SCAN_CANNOT_SEE: [(&str, &str); 7] = [
    (
        "find -exec rm",
        after_the_root_check!(
            "find target -mindepth 1 -maxdepth 1 -name stubs -exec rm -rf {} +\n"
        ),
    ),
    (
        "find -delete",
        after_the_root_check!("find target/stubs -type f -delete\n"),
    ),
    (
        "xargs rm",
        after_the_root_check!("find target -name stubs -print0 | xargs -0 rm -rf\n"),
    ),
    (
        "rsync --delete",
        after_the_root_check!("rsync -a --delete /var/empty/ target/stubs/\n"),
    ),
    (
        "&& after a test",
        after_the_root_check!("[ -d target/stubs ] && rm -rf target/stubs\n"),
    ),
    (
        "; after an echo",
        after_the_root_check!("echo tidying; find target/stubs -type f -delete\n"),
    ),
    (
        "|| after a test",
        after_the_root_check!("test ! -d dist/otp || rm -rf dist/otp\n"),
    ),
];

#[test]
fn a_removal_written_as_anything_but_rm_is_refused() {
    for (label, run) in REMOVALS_THE_SCAN_CANNOT_SEE {
        let task = MiseTask::from_run("clean:cache", run);
        let violations = cleaner_violations(&task, &PRECIOUS);
        assert!(
            !violations.is_empty(),
            "the rule approved a cleaner that deletes a kept tree with `{label}`. It reads one \
             verb per line and only the lines whose first word is `rm`, so this run block \
             answers `[\"target/debug\"]` and satisfies every guard there is:\n{run}"
        );
    }
}

#[test]
fn a_removal_after_a_shell_separator_is_part_of_the_removal_list() {
    // The rule refuses the three separator cases above through their verbs; the
    // removal *list* is the other half, and it is what the `clean_cache_plan`
    // snapshot in `tests/ci_matrix.rs` is taken over. A deletion the list
    // cannot see is a deletion no reviewer reads off the diff, even when some
    // other rule happens to refuse the line it is written on.
    let task = MiseTask::from_run(
        "clean:cache",
        after_the_root_check!("[ -d target/stubs ] && rm -rf target/stubs\n"),
    );
    let removed = task.removed_paths();
    assert!(
        removed.iter().any(|path| path == "target/stubs"),
        "`rm -rf target/stubs` on the right of an `&&` is a removal like any other, and the scan \
         answered {removed:?}. It read the first word of the whole line, saw `[`, and moved on — \
         so the snapshot of what the cleaner removes stays byte-identical while it removes the \
         one tree it must not"
    );
}

#[test]
fn a_cd_that_runs_after_the_removals_is_refused() {
    let task = MiseTask::from_run(
        "clean:cache",
        "set -eu\nrm -rf target/debug\ncd \"${MISE_PROJECT_ROOT:-$PWD}\"\n",
    );
    assert!(
        !cleaner_violations(&task, &PRECIOUS).is_empty(),
        "the removals are relative paths, so a `cd` that runs after them anchors nothing. The \
         rule asked only that some command start with `cd `, which this satisfies while deleting \
         the caller's own `target/debug`"
    );
}

#[test]
fn a_cd_into_a_subdirectory_of_the_project_is_refused() {
    let task = MiseTask::from_run(
        "clean:cache",
        "set -eu\ncd \"${MISE_PROJECT_ROOT:-$PWD}/target\"\nrm -rf debug\nrm -rf stubs\n",
    );
    assert!(
        !cleaner_violations(&task, &PRECIOUS).is_empty(),
        "`target/stubs` is written relative to the project root, so a cleaner that works from \
         `target/` removes `stubs` and the rule compares that against `target/stubs` and finds no \
         match. The `cd` names the root itself or the list of what must survive means nothing"
    );
}

#[test]
fn a_cleaner_that_removes_nothing_before_it_has_checked_where_it_is_is_refused() {
    let task = MiseTask::from_run(
        "clean:cache",
        "set -eu\ncd \"${MISE_PROJECT_ROOT:-$PWD}\"\nrm -rf target/debug\n",
    );
    assert!(
        !cleaner_violations(&task, &PRECIOUS).is_empty(),
        "`MISE_PROJECT_ROOT` is set by `mise` and by nothing else, so the `:-$PWD` fallback is \
         whatever directory the caller was in. A cleaner that removes seven trees by relative \
         path has to establish that it is in the project it thinks it is in before it removes \
         the first one"
    );
}

#[test]
fn the_committed_cleaner_still_passes_the_rule_it_is_held_to() {
    // The other half: a rule that refuses everything is no more use than one
    // that refuses nothing. `tests/ci_matrix.rs` asserts this too, over the
    // task as committed; it is repeated here so the four refusals above are
    // known to be refusals of something and not of any run block at all.
    let Some(task) = crate::common::mise::task("clean:cache") else {
        panic!("mise.toml declares no [tasks.\"clean:cache\"]");
    };
    let violations = cleaner_violations(&task, &PRECIOUS);
    assert!(
        violations.is_empty(),
        "the committed cleaner breaks its own rule:\n- {}",
        violations.join("\n- ")
    );
}
