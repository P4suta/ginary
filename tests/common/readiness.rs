// SPDX-License-Identifier: MIT OR Apache-2.0
//! `docs/dev/v1-readiness.md` read as a structure rather than as prose.
//!
//! The sweep is a fail-closed checklist with two halves that have to agree: the
//! per-phase evidence tables, which say what is proved, and `## The deferred
//! items, restated plainly`, which says what is not. Two regression rules read
//! those halves, so the parsing lives here once rather than twice:
//!
//! - [`deferred_entries`] flattens the bullets of the deferred section, which
//!   `tests/regressions/e23_the_deferred_list_carried_an_item_it_said_was_not_deferred.rs`
//!   scans for a bullet that admits it is not deferred.
//! - [`settled_deferred_items`] crosses the two halves, which
//!   `tests/regressions/f1_the_deferred_list_outlived_its_own_evidence.rs`
//!   uses to catch the opposite drift: an item the tables already mark **done**
//!   still sitting in the list of what has not happened.

use std::fmt;

/// The heading the deferred items live under.
pub const DEFERRED_HEADING: &str = "## The deferred items, restated plainly";

/// The lines of the deferred section, between its heading and the next one.
///
/// Panics when the heading is absent: every rule built on this section has that
/// section as its subject, and answering "no offenders" about a document that
/// does not have one is the silent skip `CLAUDE.md` forbids.
pub fn deferred_section(sweep: &str) -> String {
    let section = sweep.split(DEFERRED_HEADING).nth(1).unwrap_or_else(|| {
        panic!("the sweep has no `{DEFERRED_HEADING}` section, so this rule has no subject")
    });
    section.split("\n## ").next().unwrap_or(section).to_owned()
}

/// The bullets of the deferred section, each flattened to one line.
///
/// Flattened because the document hard-wraps at about 100 columns, so a phrase
/// a rule looks for is routinely split across two lines and a scan that read
/// the lines as they fall would miss it — which is how a rule about prose
/// quietly stops matching.
///
/// A continuation line is an *indented* one, which is what markdown requires of
/// text belonging to a bullet. The distinction is not pedantry: the list is
/// followed by a closing paragraph at the left margin, and a reader that took
/// every non-empty line would append that paragraph to the last entry and then
/// answer questions about the list using text that is not in it.
pub fn deferred_entries(sweep: &str) -> Vec<String> {
    let mut entries: Vec<String> = Vec::new();
    for line in deferred_section(sweep).lines() {
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
            // list, which is not in it.
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

/// One row of a per-phase evidence table: what the item is, and its status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceRow {
    /// The first cell, naming the work.
    pub item: String,
    /// The last cell: `done — <commit>`, `CI-gated — …`, `authored — …`.
    pub status: String,
}

impl EvidenceRow {
    /// Whether this row claims the work is finished.
    ///
    /// `done (packaging) — …` counts: the prefix is the claim, and the
    /// parenthesis narrows *what* is done rather than withdrawing it.
    pub fn is_done(&self) -> bool {
        self.item_status().starts_with("done")
    }

    /// The status with markdown emphasis removed, lowercased.
    fn item_status(&self) -> String {
        self.status
            .trim()
            .trim_start_matches('*')
            .to_ascii_lowercase()
    }
}

/// Every three-cell row of every evidence table in the sweep.
///
/// The header and its `|---|---|---|` separator are not rows; neither is a
/// table with a different shape, because this rule is about the phase tables
/// and reading some other table's third column as a status would invent
/// claims.
pub fn evidence_rows(sweep: &str) -> Vec<EvidenceRow> {
    let mut rows = Vec::new();
    for line in sweep.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with('|') || !trimmed.ends_with('|') {
            continue;
        }
        let cells: Vec<&str> = trimmed
            .trim_start_matches('|')
            .trim_end_matches('|')
            .split('|')
            .map(str::trim)
            .collect();
        if cells.len() != 3 {
            continue;
        }
        if cells
            .iter()
            .all(|cell| cell.chars().all(|c| c == '-' || c == ':'))
        {
            continue;
        }
        if cells[0] == "item" && cells[2] == "status" {
            continue;
        }
        rows.push(EvidenceRow {
            item: cells[0].to_owned(),
            status: cells[2].to_owned(),
        });
    }
    rows
}

/// The words of a cell or a bullet that identify *which* work it is.
///
/// Markdown emphasis, backticks and punctuation are separators rather than
/// characters, so `macOS artifact **launch**` and `**macOS launch**` yield the
/// same identifying words. Words shorter than four characters and the
/// connectives below carry no identity and would match anything.
fn identifying_words(text: &str) -> Vec<String> {
    const CONNECTIVES: [&str; 26] = [
        "that", "this", "with", "from", "when", "then", "than", "into", "over", "each", "every",
        "which", "what", "does", "must", "will", "have", "been", "they", "them", "their", "there",
        "here", "only", "also", "same",
    ];
    let mut words: Vec<String> = text
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|word| word.len() >= 4)
        .map(str::to_ascii_lowercase)
        .filter(|word| !CONNECTIVES.contains(&word.as_str()))
        .collect();
    words.sort();
    words.dedup();
    words
}

/// The bold subject a deferred bullet opens with, if it has one.
///
/// Every entry of the list names its subject in bold — a rule the E23
/// regression pins — so a bullet with no bold span is a bullet this rule
/// cannot identify, and it is reported rather than skipped.
fn bold_subject(entry: &str) -> Option<&str> {
    let rest = entry.strip_prefix("**")?;
    let end = rest.find("**")?;
    Some(&rest[..end])
}

/// A deferred bullet the evidence tables already mark done.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettledItem {
    /// The bold subject of the bullet still in the deferred list.
    pub subject: String,
    /// The row above that says the same work is finished.
    pub row: EvidenceRow,
}

impl fmt::Display for SettledItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "deferred: **{}**  —  but the table says: | {} | {} |",
            self.subject, self.row.item, self.row.status
        )
    }
}

/// Deferred bullets whose subject is also an item a table marks **done**.
///
/// The match is by identifying words rather than by string equality: the two
/// halves of the document are prose written months apart, and an item that
/// only reads the same when spelled the same is an item this rule would never
/// catch. A row matches a bullet when the row is done and its item cell names
/// every identifying word of the bullet's subject.
pub fn settled_deferred_items(sweep: &str) -> Vec<SettledItem> {
    let rows: Vec<(EvidenceRow, Vec<String>)> = evidence_rows(sweep)
        .into_iter()
        .filter(EvidenceRow::is_done)
        .map(|row| {
            let words = identifying_words(&row.item);
            (row, words)
        })
        .collect();

    let mut settled = Vec::new();
    for entry in deferred_entries(sweep) {
        let Some(subject) = bold_subject(&entry) else {
            continue;
        };
        let wanted = identifying_words(subject);
        if wanted.is_empty() {
            continue;
        }
        for (row, words) in &rows {
            if wanted.iter().all(|word| words.contains(word)) {
                settled.push(SettledItem {
                    subject: subject.to_owned(),
                    row: row.clone(),
                });
                break;
            }
        }
    }
    settled
}
