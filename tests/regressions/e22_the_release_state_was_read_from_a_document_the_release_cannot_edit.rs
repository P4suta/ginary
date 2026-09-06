// SPDX-License-Identifier: MIT OR Apache-2.0
//! Whether anything had been released was read from a sentence in
//! `docs/RELEASE.md`. The release automation does not write that file, so the
//! answer was wrong for exactly the one pull request that matters.
//!
//! **What went wrong.** `crate::common::version::nothing_has_been_released`
//! answered `read("docs/RELEASE.md").contains("No release has been cut yet.")`,
//! and five guards stood down on it. release-please rewrites two files and no
//! others — `CHANGELOG.md` and `.release-please-manifest.json`; the diff of
//! pull request #7 is those two and nothing else. On that pull request the
//! sentence was still there, so the suite still believed nothing had been
//! released, while the changelog carried the generated
//!
//! ```text
//! ## 0.1.0 (2026-09-05)
//! ```
//!
//! Four tests failed on a repository whose records were correct, `Required CI`
//! is a required status check, and the first release could not be merged.
//!
//! **The input.** Any tree in which `.release-please-manifest.json` records a
//! version and `docs/RELEASE.md` has not been edited by hand — which is every
//! release pull request release-please has ever opened.
//!
//! **The correct behaviour.** The release state is read from the record the
//! release writes: `.release-please-manifest.json`, whose `0.0.0` is
//! release-please's own spelling of "never released" and whose any other value
//! is the last released version. It is in the tree, so a shallow checkout and a
//! `cargo mutants` copy both read it, which is the property the sentence was
//! chosen for; and it moves in the same commit as the changelog section it has
//! to agree with. The guards are then stated over both states rather than
//! standing down in one of them, and `docs/RELEASE.md` points at the record
//! instead of copying its value.

use crate::common::repo::read;
use crate::common::version::{
    RELEASE_DOC, first_version_header, released_section, sections_claiming_a_release_not_recorded,
    tag_references_not_recorded, unreleased_section,
};

/// The changelog exactly as release-please leaves it on the release pull
/// request: the generated section spliced in *above* `## [Unreleased]`, and the
/// hand-written body left under `[Unreleased]` untouched.
const AS_RELEASE_PLEASE_LEAVES_IT: &str = "\
# Changelog

## 0.1.0 (2026-09-05)

### Features

* **stub:** embedded identity marker (C2)

## [Unreleased]

Phase A through Phase E.

### Added

- everything.

[Unreleased]: https://github.com/P4suta/ginary/commits/main
";

/// The same changelog after the maintainer's release-pull-request edit: the
/// hand-written body folded into the section that releases it, and an empty
/// `[Unreleased]` back on top.
const AS_THE_MAINTAINER_LEAVES_IT: &str = "\
# Changelog

## [Unreleased]

## 0.1.0 (2026-09-05)

Phase A through Phase E.

### Added

- everything.

[Unreleased]: https://github.com/P4suta/ginary/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/P4suta/ginary/releases/tag/v0.1.0
";

#[test]
fn a_section_the_manifest_records_is_not_a_claim_of_a_release_nobody_made() {
    assert_eq!(
        sections_claiming_a_release_not_recorded(AS_RELEASE_PLEASE_LEAVES_IT, Some("0.1.0")),
        Vec::<String>::new(),
        "`.release-please-manifest.json` records 0.1.0, so `## 0.1.0 (2026-09-05)` is the record \
         and the changelog agreeing with it. A guard that reads its answer from a document \
         release-please does not write reports this correct changelog as a false claim, which is \
         what failed the first release pull request"
    );
}

#[test]
fn the_same_section_is_a_false_claim_while_the_manifest_records_nothing() {
    assert_eq!(
        sections_claiming_a_release_not_recorded(AS_RELEASE_PLEASE_LEAVES_IT, None),
        vec!["## 0.1.0 (2026-09-05)".to_owned()],
        "with the manifest recording `0.0.0` nothing has been released, so a dated version \
         section is a release nobody made. The rule has to be the manifest and not the state \
         this checkout happens to be in: the same text is correct in one state and false in the \
         other, and only the record can say which"
    );
}

#[test]
fn a_tag_reference_past_the_recorded_release_is_reported_and_one_at_it_is_not() {
    assert_eq!(
        tag_references_not_recorded(AS_THE_MAINTAINER_LEAVES_IT, Some("0.1.0")),
        Vec::<String>::new(),
        "both links name v0.1.0, which the manifest records; release-please writes the manifest \
         in the same commit that creates the tag, so a link to it is not dangling"
    );
    let ahead = AS_THE_MAINTAINER_LEAVES_IT.replace("v0.1.0", "v0.2.0");
    assert_eq!(
        tag_references_not_recorded(&ahead, Some("0.1.0")),
        vec![
            "compare/v0.2.0...HEAD".to_owned(),
            "releases/tag/v0.2.0".to_owned(),
        ],
        "a link to a tag past the last released version is a 404 in the project's own release \
         notes, whatever any document says about the release state"
    );
}

#[test]
fn the_generated_section_lands_above_unreleased_and_the_heading_says_so() {
    assert_eq!(
        first_version_header(AS_RELEASE_PLEASE_LEAVES_IT).as_deref(),
        Some("## 0.1.0 (2026-09-05)"),
        "release-please splices its section in above the first line matching `\\n###? v?[0-9[]`, \
         which is `## [Unreleased]` itself. Left alone, `[Unreleased]` is no longer the first \
         version header — and it is the insertion point of the *next* release too, so it sinks \
         one section further with every release and never comes back"
    );
    assert_eq!(
        first_version_header(AS_THE_MAINTAINER_LEAVES_IT).as_deref(),
        Some("## [Unreleased]"),
        "the maintainer's edit is what puts the insertion point back where `docs/RELEASE.md` \
         describes it. A rule stated over the first version header notices the sinking on the \
         release pull request, where it can still be fixed"
    );
}

#[test]
fn the_body_belongs_to_the_section_that_releases_it() {
    assert!(
        unreleased_section(AS_RELEASE_PLEASE_LEAVES_IT).contains("Phase A"),
        "release-please neither moves nor consumes what is under `[Unreleased]`, so before the \
         maintainer's edit the released work is still filed as unreleased"
    );
    assert!(
        !unreleased_section(AS_THE_MAINTAINER_LEAVES_IT).contains("Phase A"),
        "after the edit `[Unreleased]` is empty: the work it described is released"
    );
    let released = released_section(AS_THE_MAINTAINER_LEAVES_IT, "0.1.0")
        .expect("the changelog has a section for 0.1.0");
    assert!(
        released.contains("Phase A") && released.contains("### Added"),
        "the hand-written body is the release note for 0.1.0 and belongs under its heading; \
         release-please writes commit subjects and cannot write prose, so this is the one edit \
         a maintainer makes to the release pull request"
    );
}

/// Standing statements of the current release state, in the spellings a
/// document reaches for.
///
/// Kept as data so the failure names the sentence to delete rather than the
/// file to go and read.
const STANDING_CLAIMS: [&str; 3] = [
    "No release has been cut yet",
    "no release has been cut yet",
    "Nothing has been released yet",
];

#[test]
fn the_release_document_carries_no_standing_claim_about_the_release_state() {
    let document = read(RELEASE_DOC);
    for claim in STANDING_CLAIMS {
        assert!(
            !document.contains(claim),
            "{RELEASE_DOC} says `{claim}`. That is a statement of the current release state in a \
             file release-please never writes: it is true until the release pull request merges \
             and false for ever after, and nothing in the release path edits it. The state is \
             recorded in `.release-please-manifest.json`; this document says what that record \
             means and reads its value from nowhere"
        );
    }
}

#[test]
fn the_release_document_points_at_the_record_instead_of_copying_it() {
    let document = read(RELEASE_DOC);
    assert!(
        document
            .split("\n\n")
            .any(|block| block.contains(".release-please-manifest.json") && block.contains("0.0.0")),
        "{RELEASE_DOC} has to name the record and the value that means `never released` in one \
         paragraph: that is what a maintainer reads instead of a sentence somebody has to \
         remember to delete"
    );
}

#[test]
fn a_recorded_version_that_cannot_be_read_covers_nothing() {
    // The fail-closed direction, and the one worth pinning because the natural
    // reading goes the other way: a ceiling that will not parse is not a
    // ceiling of infinity. A manifest holding something that is not a version
    // is a record nobody can check a claim against, and a rule that shrugged at
    // it would pass every changelog there is.
    assert_eq!(
        sections_claiming_a_release_not_recorded(
            AS_RELEASE_PLEASE_LEAVES_IT,
            Some("not-a-version")
        ),
        vec!["## 0.1.0 (2026-09-05)".to_owned()],
        "a recorded release this cannot read proves nothing, so it covers nothing"
    );
}
