// SPDX-License-Identifier: MIT OR Apache-2.0
//! The fail-closed checklist went on owing evidence it had already produced.
//!
//! **What went wrong.** `docs/dev/v1-readiness.md` was last touched at `2e19b9d`
//! (E23). F1 then ran the `macos` job of `.github/workflows/ci.yml` on both
//! images — run
//! [34281075949](https://github.com/P4suta/ginary/actions/runs/34281075949) —
//! and `docs/dev/log/F1-integration.md` recorded it: "Both native macOS jobs
//! built and launched real artifacts on their matching Intel and ARM runners
//! … native `codesign` reported the artifacts valid on disk and satisfying
//! their Designated Requirement before and after launch."
//!
//! The sweep did not move. Its Phase D table still read `CI-gated` for the
//! macOS launch, the paragraph under it still said "no Mach-O has ever been
//! executed", and `## The deferred items, restated plainly` still opened with
//! **macOS launch** — an item nothing was waiting for.
//!
//! E23 had already fixed the mirror image of this. Its regression,
//! `e23_the_deferred_list_carried_an_item_it_said_was_not_deferred.rs`, forbids
//! a bullet that *says* it is no longer deferred, and the rule it states is the
//! right one: "An item leaves the deferred list the day something in the
//! repository proves it, and takes its evidence to the table above." What that
//! rule could not see is the case where nobody edits the bullet at all. A
//! checklist that lists closed items reads as owing more than it does, which is
//! the same failure as claiming more than it can show, pointing the other way —
//! and it is the failure that actually survived, because it needs no author to
//! write anything wrong. It only needs one to write nothing.
//!
//! **The input.** `tests/fixtures/readiness/settled_item_in_the_deferred_list.md`
//! is the sweep as it stood on 2026-09-13, reduced to the tables and the list:
//! one deferred bullet whose subject a table already marks **done**, and three
//! that no table does — including two near misses that share some of their
//! words with a finished row and must not be reported.
//!
//! **The correct behaviour.** The two halves of the document agree. No bullet
//! of the deferred list names work an evidence table above it marks done; when
//! a hosted run closes an item, the row moves to `done` and the bullet is
//! deleted rather than reworded.

use crate::common::readiness::{SettledItem, settled_deferred_items};
use crate::common::repo::read;

/// The document this rule governs.
const SWEEP: &str = "docs/dev/v1-readiness.md";

/// The fixture the rule is calibrated on before it is turned loose on the
/// sweep.
///
/// A committed file rather than a string in this one, for the reason the
/// portability scan keeps its own: the rule is about a document, and a
/// document written inline is a document nobody can read beside the real one.
const FIXTURE: &str = "tests/fixtures/readiness/settled_item_in_the_deferred_list.md";

/// The findings, one per line, for a failure that reads without the file open
/// beside it.
fn render(settled: &[SettledItem]) -> String {
    settled
        .iter()
        .map(SettledItem::to_string)
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn only_a_deferred_item_a_table_marks_done_is_a_finding() {
    let settled = settled_deferred_items(&read(FIXTURE));
    let subjects: Vec<&str> = settled.iter().map(|item| item.subject.as_str()).collect();

    assert_eq!(
        subjects,
        vec!["macOS launch"],
        "the macOS bullet is the one the tables above already close. The console-control bullet \
         shares `Windows` and `launcher` with a finished row and the catalog bullet shares \
         `catalog` with another, so a rule that matched on any shared word would report them \
         too; the mutation bullet's row is `planned`, so a rule that read every third cell as a \
         status would report that one:\n{}",
        render(&settled)
    );
    assert_eq!(
        settled[0].row.item, "macOS artifact **launch**, `codesign --verify`",
        "the finding names the row that closes it, so the fix is to hand rather than to find"
    );
}

#[test]
fn no_deferred_item_of_the_sweep_is_one_the_tables_already_close() {
    let settled = settled_deferred_items(&read(SWEEP));
    assert!(
        settled.is_empty(),
        "`{SWEEP}` is fail-closed: its deferred section lists what has **not** happened, and an \
         item leaves it the day something proves it. These bullets name work a table above them \
         marks done, so the list overstates what is owed. Delete the bullet — the evidence is \
         already in the row:\n{}",
        render(&settled)
    );
}
