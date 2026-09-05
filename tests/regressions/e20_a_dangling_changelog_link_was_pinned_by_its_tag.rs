// SPDX-License-Identifier: MIT OR Apache-2.0
//! The guard against a changelog claiming a release nobody made was written
//! around the two spellings of `v0.1.0`, and around headings that begin
//! `## [`. Any other version, and any heading without the brackets, walked
//! straight through it.
//!
//! **What went wrong.** `the_changelog_links_no_tag_that_does_not_exist`
//! looped over `["compare/v0.1.0", "releases/tag/v0.1.0"]`, and
//! `released_section_headings` filtered on `line.starts_with("## [")`. E20
//! exists because release-please proposed `0.2.0` for a repository that had
//! released nothing; if that proposal had landed before the manifest was
//! corrected, a re-added
//!
//! ```text
//! ## 0.2.0 - 2026-09-02
//! [0.2.0]: https://github.com/P4suta/ginary/releases/tag/v0.2.0
//! ```
//!
//! would have been reported by neither guard, while the test's own name claims
//! it links no tag that does not exist.
//!
//! **The input.** Any hand-written section or link naming a version other than
//! `0.1.0`, or a heading written `## 0.1.0 - 2026-09-02` rather than
//! `## [0.1.0]`. Both spellings are ordinary Keep a Changelog.
//!
//! **The correct behaviour.** The guards are stated over the shape rather than
//! over one version: a released section is any `##`/`###` heading whose first
//! character is a digit or a `[` — the same shape release-please's own
//! `versionHeaderRegex` uses to find them — and a tag reference is any
//! `releases/tag/…` or `compare/…v<digit>` whatever version it names. While
//! nothing has been released there may be neither.

use crate::common::repo::read;
use crate::common::version::{
    NO_RELEASE_YET, RELEASE_DOC, nothing_has_been_released, released_section_headings,
    tag_references,
};

/// A changelog carrying the claim in the spelling the old guards could not see.
const CHANGELOG_CLAIMING_A_RELEASE_OF_ANOTHER_VERSION: &str = "\
# Changelog

## [Unreleased]

## 0.2.0 - 2026-09-02

### Added

- everything.

[Unreleased]: https://github.com/P4suta/ginary/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/P4suta/ginary/releases/tag/v0.2.0
";

#[test]
fn a_released_section_without_brackets_is_seen() {
    let headings = released_section_headings(CHANGELOG_CLAIMING_A_RELEASE_OF_ANOTHER_VERSION);
    assert_eq!(
        headings,
        vec!["## 0.2.0 - 2026-09-02".to_owned()],
        "`## 0.2.0 - 2026-09-02` is a dated release section written the other ordinary way. A \
         scan that matches only `## [` reports nothing here, and the guard that says the \
         changelog claims no release that has not been cut passes over a changelog claiming one"
    );
}

#[test]
fn a_tag_reference_to_any_version_is_seen() {
    let references = tag_references(CHANGELOG_CLAIMING_A_RELEASE_OF_ANOTHER_VERSION);
    assert_eq!(
        references,
        vec![
            "compare/v0.2.0...HEAD".to_owned(),
            "releases/tag/v0.2.0".to_owned(),
        ],
        "both links point at a tag nobody cut, and a reader following either gets a 404 from the \
         project's own release notes. Pinning the two spellings of `v0.1.0` says nothing about \
         the version release-please was actually proposing"
    );
}

#[test]
fn the_committed_changelog_names_no_tag_and_claims_no_release() {
    if !nothing_has_been_released() {
        return;
    }
    let changelog = read("CHANGELOG.md");
    assert!(
        tag_references(&changelog).is_empty(),
        "{RELEASE_DOC} says `{NO_RELEASE_YET}`, so every tag the changelog could link is one that \
         does not exist: {:?}",
        tag_references(&changelog)
    );
    assert!(
        released_section_headings(&changelog).is_empty(),
        "{RELEASE_DOC} says `{NO_RELEASE_YET}`, and a dated version section is the same false \
         claim `.release-please-manifest.json` was making: {:?}",
        released_section_headings(&changelog)
    );
}
