// SPDX-License-Identifier: MIT OR Apache-2.0
//! The fail-closed checklist listed a closed item among the open ones.
//!
//! **What went wrong.** E23 proved the Windows launch end to end — run
//! [34023412195](https://github.com/P4suta/ginary/actions/runs/34023412195)
//! printed `the artifact left exit code 3 for halt(3)` — and moved the table
//! row from `CI-gated` to done. The bullet under
//! `## The deferred items, restated plainly` was updated in place instead of
//! being removed, so the section that exists to say *what has not happened*
//! opened its Windows entry with **"no longer deferred."**
//!
//! ```text
//! ## The deferred items, restated plainly
//!
//! Three kinds of work are CI-gated rather than done, ...
//!
//! - **Windows launch with a real runtime** — **no longer deferred.** ...
//! ```
//!
//! Every word of it was true. What was wrong is that a reader — or a release
//! checklist — scanning the deferred list for what still owes evidence finds an
//! item that owes none, and the section's own count ("three kinds of work")
//! disagreed with the four bullets under it. `docs/dev/v1-readiness.md` is the
//! document this repository keeps precisely so that a claim and its evidence
//! cannot drift apart, and it is fail-closed by design: an item with no
//! evidence is not v1-ready. A list that carries closed items reads as owing
//! more than it does, which is the same failure as claiming more than it can
//! show, pointing the other way.
//!
//! **The input.** The committed document. Nothing needs to run.
//!
//! **The correct behaviour.** An item leaves the deferred list the day
//! something in the repository proves it, and takes its evidence to the table
//! above. A bullet that has to explain it is *not* deferred is a bullet in the
//! wrong section, so that is the rule, stated over every entry rather than over
//! the one that went wrong.

use crate::common::repo::read;

/// The document whose deferred section this file reads.
const SWEEP: &str = "docs/dev/v1-readiness.md";

/// The heading the deferred items live under.
const HEADING: &str = "## The deferred items, restated plainly";

/// The ways a bullet admits it does not belong in that section.
///
/// Not a spell-check of one wording: each of these is a phrase that only makes
/// sense written *about* a deferred item by someone who has just discovered it
/// is not one.
/// `not deferred` bare rather than `is not deferred`, because
/// `— not deferred.` is the shorter way to write the same admission and the
/// longer phrase contains it anyway.
const NOT_DEFERRED: [&str; 4] = [
    "no longer deferred",
    "not deferred",
    "already done",
    "now done",
];

/// The lines of the deferred section, between its heading and the next one.
fn deferred_section() -> String {
    let sweep = read(SWEEP);
    let section = sweep.split(HEADING).nth(1).unwrap_or_else(|| {
        panic!("{SWEEP} has no `{HEADING}` section, so this rule has no subject")
    });
    section.split("\n## ").next().unwrap_or(section).to_owned()
}

/// The bullets of the deferred section, each flattened to one line.
///
/// Flattened because the document hard-wraps at about 100 columns, so a phrase
/// this rule looks for is routinely split across two lines and a scan that read
/// the lines as they fall would miss it — which is how a rule about prose
/// quietly stops matching.
///
/// A continuation line is an *indented* one, which is what markdown requires of
/// text belonging to a bullet. The distinction is not pedantry: the list is
/// followed by a closing paragraph at the left margin, and a reader that took
/// every non-empty line would append that paragraph to the last entry and then
/// answer questions about the list using prose that is not in it.
fn deferred_entries() -> Vec<String> {
    let mut entries: Vec<String> = Vec::new();
    for line in deferred_section().lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("- ") {
            entries.push(rest.to_owned());
            continue;
        }
        if trimmed.is_empty() {
            continue;
        }
        let Some(last) = entries.last_mut() else {
            // Prose before the first bullet: the paragraph introducing the
            // list, which is not in it. The rule below reads entries, and this
            // introduction has to be able to state the rule it governs.
            continue;
        };
        if !line.starts_with([' ', '\t']) {
            // Unindented and not a bullet: the list has ended.
            break;
        }
        last.push(' ');
        last.push_str(trimmed);
    }
    entries
}

#[test]
fn the_rule_this_file_applies_reads_a_bullet_that_spans_two_lines() {
    let section = deferred_section();
    let entries = deferred_entries();
    assert!(
        entries.len() >= 2,
        "the deferred section has {} entries; this rule has lost its subject",
        entries.len()
    );
    assert!(
        entries.iter().all(|entry| entry.starts_with("**")),
        "every entry of the deferred list names its subject in bold:\n{}",
        entries.join("\n")
    );

    // The flattening is the point, so prove it happened rather than assume it:
    // no entry can be longer than the longest bullet *line* unless lines were
    // joined. A reader that stopped at the first physical line would fail here
    // and go on passing every other assertion in this file, which is how a
    // phrase split across a wrap would slip through the rule below.
    let longest_line = section
        .lines()
        .filter_map(|line| line.trim().strip_prefix("- "))
        .map(str::len)
        .max()
        .expect("the deferred section has bullets");
    assert!(
        entries.iter().any(|entry| entry.len() > longest_line),
        "every entry is at most one line long ({longest_line} characters), so nothing was joined \
         and a forbidden phrase broken across a wrap would not be seen"
    );

    // And the closing paragraph after the list is not part of the last entry.
    // *After* the list: the paragraph introducing it is unindented too, and a
    // guard that found that one instead would be asserting something no parser
    // could get wrong.
    let closing = section
        .lines()
        .skip_while(|line| !line.trim().starts_with("- "))
        .find(|line| {
            let trimmed = line.trim();
            !line.starts_with([' ', '\t'])
                && !trimmed.is_empty()
                && !trimmed.starts_with("- ")
                && trimmed.len() > 40
        })
        .map(str::trim);
    if let Some(closing) = closing {
        assert!(
            !entries.iter().any(|entry| entry.contains(closing)),
            "the prose at the left margin after the list was appended to an entry, so this rule \
             would answer about the list using text that is not in it:\n{closing}"
        );
    }
}

#[test]
fn no_entry_of_the_deferred_list_says_it_is_not_deferred() {
    let mut offenders: Vec<String> = Vec::new();
    for entry in deferred_entries() {
        let lowered = entry.to_ascii_lowercase();
        for phrase in NOT_DEFERRED {
            if lowered.contains(phrase) {
                offenders.push(format!("`{phrase}` in: {entry}"));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "`{SWEEP}`'s deferred section lists what has *not* happened, and an item leaves it the \
         day something in this repository proves it — carrying its evidence to the table above. \
         A bullet that has to explain it is no longer deferred is a bullet in the wrong section, \
         and a fail-closed checklist that lists closed items reads as owing more than it does:\n{}",
        offenders.join("\n")
    );
}
