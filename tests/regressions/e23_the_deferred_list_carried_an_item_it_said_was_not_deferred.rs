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
const NOT_DEFERRED: [&str; 4] = [
    "no longer deferred",
    "is not deferred",
    "already done",
    "now done",
];

/// The bullets of the deferred section, each flattened to one line.
///
/// Flattened because the document hard-wraps at about 100 columns, so a phrase
/// this rule looks for is routinely split across two lines and a scan that read
/// the lines as they fall would miss it — which is how a rule about prose
/// quietly stops matching.
fn deferred_entries() -> Vec<String> {
    let sweep = read(SWEEP);
    let section = sweep.split(HEADING).nth(1).unwrap_or_else(|| {
        panic!("{SWEEP} has no `{HEADING}` section, so this rule has no subject")
    });
    let section = section.split("\n## ").next().unwrap_or(section);

    let mut entries: Vec<String> = Vec::new();
    for line in section.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("- ") {
            entries.push(rest.to_owned());
        } else if !trimmed.is_empty()
            && let Some(last) = entries.last_mut()
        {
            last.push(' ');
            last.push_str(trimmed);
        }
    }
    entries
}

#[test]
fn the_rule_this_file_applies_reads_a_bullet_that_spans_two_lines() {
    let entries = deferred_entries();
    assert!(
        entries.len() >= 2,
        "the deferred section has {} entries; this rule has lost its subject",
        entries.len()
    );
    assert!(
        entries.iter().all(|entry| entry.starts_with("**")),
        "every entry of the deferred list names its subject in bold, and this rule reads the \
         whole of each entry rather than its first line:\n{}",
        entries.join("\n")
    );
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
