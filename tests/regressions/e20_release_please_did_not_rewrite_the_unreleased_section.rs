// SPDX-License-Identifier: MIT OR Apache-2.0
//! The changelog and the release document both said release-please rewrites
//! the `## [Unreleased]` section into a dated release section. It does not: it
//! splices the generated section in *above* that heading and never touches
//! what is under it.
//!
//! **What went wrong.** E20 moved the phase summary of phases A through E back
//! under `## [Unreleased]`, which is the right place for work that is done and
//! unreleased, and explained the move with a claim nobody had read:
//!
//! ```text
//! CHANGELOG.md:11   ... release-please writes the version heading over it when a
//!                       release is made.
//! docs/RELEASE.md:156 ... rewrites the `[Unreleased]` section of `CHANGELOG.md` into a
//!                       dated release section.
//! ```
//!
//! The milestone's own method was "read rather than assumed", and its research
//! section quotes `src/manifest.ts`, `src/strategies/base.ts`,
//! `src/strategies/rust.ts` and `schemas/config.json` — but not the one
//! updater the restructuring depends on.
//!
//! **The input.** Any changelog whose first version heading is
//! `## [Unreleased]`, which is this one. `src/updaters/changelog.ts` reads:
//!
//! ```ts
//! const DEFAULT_VERSION_HEADER_REGEX = '\n###? v?[0-9[]';
//! ...
//! const lastEntryIndex = content.search(this.versionHeaderRegex);
//! ...
//! const before = content.slice(0, lastEntryIndex);
//! const after = content.slice(lastEntryIndex);
//! return `${before}\n${this.changelogEntry}\n${after}`.trim() + '\n';
//! ```
//!
//! The `[` is inside the character class, so `\n## [Unreleased]` is the first
//! match and becomes `after`. The generated `## [0.1.0]` section is inserted
//! above it and the `[Unreleased]` body is neither moved nor consumed. Under
//! the false description, the whole phase summary would have been filed as
//! unreleased for ever after the first release, and the two guards that would
//! have noticed both stood down the moment a release was cut. E22 restated
//! them over `.release-please-manifest.json` so that they do not.
//!
//! **The correct behaviour.** The documents say what the updater does, and
//! `docs/RELEASE.md` carries the step that follows from it: when reviewing the
//! release pull request the maintainer folds the released body into the
//! generated section and puts an empty `## [Unreleased]` back above it, because
//! release-please will not. The insertion point itself is pinned here, and E22
//! moved that pin from "exactly one version header" — a claim that is only true
//! before the first release — to "`[Unreleased]` is the *first* version header",
//! which is the property that actually holds. The insertion point of the next
//! release is the first match, so a `[Unreleased]` that has fallen below a
//! release section falls one further with every release after it and never
//! comes back.

use crate::common::repo::read;
use crate::common::version::first_version_header;

/// The claims that were in the tree, in the spelling each document used.
///
/// Kept as data so the failure names the sentence to delete rather than the
/// file to go and read.
const REFUTED: [(&str, &str); 4] = [
    ("CHANGELOG.md", "writes the version heading over"),
    ("docs/RELEASE.md", "rewrites the `[Unreleased]` section"),
    ("docs/RELEASE.md", "rewrites the [Unreleased] section"),
    ("docs/dev/log/E20.md", "writes the version heading over"),
];

/// `text` with every fenced code block removed.
///
/// The rule is about what a document *claims*, and a milestone log quoting the
/// failure that refuted the claim is the record working as intended. Prose is
/// where a reader takes a statement as true.
fn prose(text: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            inside = !inside;
            continue;
        }
        if !inside {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

#[test]
fn no_document_claims_release_please_rewrites_the_unreleased_section() {
    for (file, claim) in REFUTED {
        let text = prose(&read(file));
        assert!(
            !text.contains(claim),
            "{file} says release-please `{claim}` the `[Unreleased]` section. \
             `src/updaters/changelog.ts` inserts the generated section *before* the first heading \
             matching `\\n###? v?[0-9[]`, which is `## [Unreleased]` itself, and leaves everything \
             from that line down alone. A document that describes the opposite is how the phase \
             summary would have been stranded under `[Unreleased]` for ever"
        );
    }
}

/// The paragraphs of a markdown document: prose hard-wraps at about 100
/// columns here, so a claim and its subject routinely land on different lines
/// and a per-line pairing would be a test of where the wrap fell.
fn paragraphs(text: &str) -> Vec<String> {
    text.split("\n\n")
        .map(|block| block.replace('\n', " "))
        .collect()
}

#[test]
fn the_release_document_says_where_the_generated_section_lands_and_who_clears_unreleased() {
    let release = read("docs/RELEASE.md");
    let paragraphs = paragraphs(&release);
    assert!(
        paragraphs
            .iter()
            .any(|block| block.contains("[Unreleased]") && block.contains("above")),
        "docs/RELEASE.md has to say that release-please inserts the generated section *above* \
         `## [Unreleased]`: that is the one sentence a maintainer needs to predict what the \
         release pull request will look like"
    );
    assert!(
        paragraphs
            .iter()
            .any(|block| block.contains("[Unreleased]") && block.contains("by hand")),
        "release-please does not touch what is under `## [Unreleased]`, so clearing it is a step \
         the maintainer performs while reviewing the release pull request. A step nobody wrote \
         down is a step nobody does, and the result is a changelog that files released work as \
         unreleased"
    );
}

#[test]
fn the_unreleased_heading_is_the_first_version_header() {
    // E20 pinned this as "exactly one version header, and it is
    // `[Unreleased]`", which is only true before the first release; the guard
    // stood down the moment one was cut, which is the moment it was needed.
    // What matters is not how many there are but which one is *first*: that
    // line is where release-please splices the next generated section in, and
    // once `[Unreleased]` is not it, `[Unreleased]` is below a release — and
    // below the insertion point of every release after it, sinking one section
    // further each time until the section for work that is not released is at
    // the bottom of the file for ever.
    assert_eq!(
        first_version_header(&read("CHANGELOG.md")).as_deref(),
        Some("## [Unreleased]"),
        "release-please splices its generated section in above the *first* line matching its own \
         `\\n###? v?[0-9[]`. That line has to be `## [Unreleased]`. If a release section is above \
         it, the release pull request has not been finished: move the released body under the \
         generated heading and put an empty `## [Unreleased]` back on top, as `docs/RELEASE.md` \
         step 1 describes"
    );
}
