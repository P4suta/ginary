// SPDX-License-Identifier: MIT OR Apache-2.0
//! The changelog's dangling-link check reads the first reference on a line and
//! stops, so a second one on the same line is never examined.
//!
//! **What went wrong.** `crate::common::version::tag_references` looks for two
//! needles, `releases/tag/` and `compare/`, with
//!
//! ```text
//! let Some(at) = line.find(needle) else { continue };
//! ```
//!
//! `str::find` answers the *first* occurrence. A markdown link-reference
//! block normally puts one definition on a line, and nothing enforces that: a
//! reflow, a hand edit, or a footnote that names two comparisons in a sentence
//! puts two on one line, and only the first is checked. The link that escapes
//! is a 404 in the project's own release notes — precisely what E20's
//! `e20_a_dangling_changelog_link_was_pinned_by_its_tag.rs` was written to
//! prevent — and it escapes in the direction that passes.
//!
//! It is worse for `compare/` than for `releases/tag/`, because a `compare/`
//! reference that names no tag is deliberately *not* returned: a branch range
//! such as `compare/main...HEAD` is a legitimate link. So a line beginning
//! with the `[Unreleased]` definition — which is exactly that branch range, and
//! which is the first `compare/` on it — hides every tag comparison after it
//! and the scan answers with nothing at all.
//!
//! **The input.** Two references on one line of `CHANGELOG.md`, with the
//! harmless one first.
//!
//! **The correct behaviour.** Every occurrence of each needle on a line is
//! examined, in the order it is written.

use crate::common::version::tag_references;

/// The repository the fixture links into, so the assertions read as URLs a
/// person could click.
const REPOSITORY: &str = "https://github.com/P4suta/ginary";

#[test]
fn a_tag_reference_after_a_branch_range_on_the_same_line_is_read() {
    let line = format!(
        "[Unreleased]: {REPOSITORY}/compare/main...HEAD [0.1.0]: {REPOSITORY}/compare/v0.0.9...v0.1.0\n"
    );

    let references = tag_references(&line);

    assert_eq!(
        references,
        vec!["compare/v0.0.9...v0.1.0".to_owned()],
        "`str::find` answers the first `compare/` on the line, and the first one here is the \
         branch range the check deliberately ignores. So the tag comparison beside it — the \
         reference that can dangle — was never looked at"
    );
}

#[test]
fn every_release_reference_on_a_line_is_read() {
    let line = format!(
        "see {REPOSITORY}/releases/tag/v0.1.0 and {REPOSITORY}/releases/tag/v0.2.0 for the notes\n"
    );

    let references = tag_references(&line);

    assert_eq!(
        references,
        vec![
            "releases/tag/v0.1.0".to_owned(),
            "releases/tag/v0.2.0".to_owned(),
        ],
        "two release links in one sentence are two links, and the second can dangle exactly as \
         the first can"
    );
}

#[test]
fn a_branch_range_is_still_not_a_tag_reference() {
    // The calibration. `compare/` spells a branch range as well as a tag
    // comparison, and `[Unreleased]: …/compare/main...HEAD` is a link this
    // repository's changelog is meant to carry. Reading every occurrence must
    // not turn that into a tag nobody has cut.
    let line = format!("[Unreleased]: {REPOSITORY}/compare/main...HEAD\n");

    assert_eq!(
        tag_references(&line),
        Vec::<String>::new(),
        "a range between two branches names no tag"
    );
}
