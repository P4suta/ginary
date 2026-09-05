// SPDX-License-Identifier: MIT OR Apache-2.0
//! Quoting the verb rather than the argument walked round the cleaner guard
//! that E21 had just tightened.
//!
//! **What went wrong.** Fixing the quoted-substitution hole taught
//! `crate::common::mise`'s `verbs` to drop the quote characters off the edges
//! of a word, because splitting `echo "$(rm …)"` at the substitution leaves
//! `echo "` behind and a word that is nothing but a quote is not a verb. The
//! other reader of the same shell, `removed_paths`, was left comparing the
//! raw word:
//!
//! ```text
//! if words.next() != Some("rm") { continue; }
//! ```
//!
//! Two readers, one grammar, and now two spellings of a word. `'rm' -rf
//! target/stubs` is a removal bash performs — quotes round a command word are
//! removed before the word is looked up — and after the fix it read to
//! `verbs` as the allowed verb `rm` and to `removed_paths` as no removal at
//! all. Every rule in `cleaner_violations` then passes: the plain
//! `rm -rf target/debug` beside it keeps the removal list non-empty, no
//! removal is a kept tree because the one that is never enters the list, the
//! verb is on the allowlist, and the prologue supplies the `cd` and the root
//! check. The guard approves a cleaner that deletes the five cross-built
//! stubs.
//!
//! Before that fix this case was refused, for the wrong reason: `verbs`
//! answered the untrimmed `'rm'`, which is not on the allowlist. So the
//! tightening of one hole opened another in the same guard — which is the
//! class of defect the thread it came from was about.
//!
//! **The input.** Any future edit of `[tasks."clean:cache"]` that quotes a
//! command word. Nothing exotic: a shell written by somebody who quotes
//! defensively, or a line whose verb arrives from a paste.
//!
//! **The correct behaviour.** A word is normalised once and read the same way
//! by both readers, so `rm`, `'rm'` and `"rm"` are one word to the verb
//! allowlist and to the removal list alike.

use crate::common::mise::{MiseTask, cleaner_violations};

/// The two trees the cleaner must leave alone, as `tests/ci_matrix.rs` states
/// them.
const PRECIOUS: [&str; 2] = ["dist/otp", "target/stubs"];

/// A run block that is the committed cleaner's prologue — the root check, the
/// `cd` into the root, one plain `rm` — with `$tail` appended.
///
/// The same prologue the two E20 and E21 cleaner regressions use, and for the
/// same reason: every case below has to break exactly one rule, so that a
/// non-empty violation list is attributable to the deletion it hides rather
/// than to a missing root check.
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

/// A run block whose deletion of a kept tree spells the verb with quotes round
/// it, paired with the plain `rm` that makes the rest of the rule look
/// satisfied.
const REMOVALS_WITH_A_QUOTED_VERB: [(&str, &str, &str); 4] = [
    (
        "a single-quoted rm",
        after_the_root_check!("'rm' -rf target/stubs\n"),
        "target/stubs",
    ),
    (
        "a double-quoted rm",
        after_the_root_check!("\"rm\" -rf dist/otp\n"),
        "dist/otp",
    ),
    (
        "a quoted rm inside a quoted command substitution",
        after_the_root_check!("echo \"$('rm' -rf target/stubs)\"\n"),
        "target/stubs",
    ),
    (
        "a quoted rm after an &&",
        after_the_root_check!("[ -d dist/otp ] && 'rm' -rf dist/otp\n"),
        "dist/otp",
    ),
];

#[test]
fn a_removal_whose_verb_is_quoted_is_refused() {
    for (label, run, kept) in REMOVALS_WITH_A_QUOTED_VERB {
        let task = MiseTask::from_run("clean:cache", run);
        let violations = cleaner_violations(&task, &PRECIOUS);
        assert!(
            !violations.is_empty(),
            "the rule approved a cleaner that deletes `{kept}` with `{label}`. Bash removes the \
             quotes round a command word before it looks the word up, so this run block removes \
             a kept tree while the verb allowlist reads an ordinary `rm` and the removal list \
             reads nothing at all:\n{run}"
        );
    }
}

#[test]
fn a_removal_whose_verb_is_quoted_is_in_the_removal_list() {
    // The other half, and the one that matters most: `cleaner_violations` can
    // only refuse what `removed_paths` shows it, and the `clean_cache_plan`
    // snapshot in `tests/ci_matrix.rs` is taken over that same list. A
    // deletion missing from it is a deletion no reviewer reads off the diff.
    for (label, run, kept) in REMOVALS_WITH_A_QUOTED_VERB {
        let task = MiseTask::from_run("clean:cache", run);
        let removed = task.removed_paths();
        assert!(
            removed.iter().any(|path| path == kept),
            "`{label}` removes `{kept}` like any other `rm`, and the scan answered {removed:?}"
        );
    }
}

#[test]
fn a_quoted_path_is_still_read_as_the_path_it_names() {
    // The calibration. Trimming the quotes off a word must not change what an
    // argument means: `rm -rf 'target/stubs'` is the same removal as
    // `rm -rf target/stubs`, and the committed cleaner quotes nothing.
    let task = MiseTask::from_run(
        "clean:cache",
        after_the_root_check!("rm -rf 'target/stubs' \"dist/otp\"\n"),
    );

    let removed = task.removed_paths();

    assert_eq!(
        removed,
        vec![
            "dist/otp".to_owned(),
            "target/debug".to_owned(),
            "target/stubs".to_owned()
        ],
        "a quoted argument names the path inside the quotes"
    );
    assert!(
        !cleaner_violations(&task, &PRECIOUS).is_empty(),
        "and both of those are trees the cleaner must not remove"
    );
}
