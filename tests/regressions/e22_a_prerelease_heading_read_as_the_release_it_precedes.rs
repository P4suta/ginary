// SPDX-License-Identifier: MIT OR Apache-2.0
//! The version scanners took the leading digits of a heading and stopped at
//! whatever came next, so `## 1.2.3-rc.1` read as `1.2.3` — a release that had
//! been made — and a section claiming one that had not walked through.
//!
//! **What went wrong.** E22 states both changelog rules over a *ceiling*: the
//! version `.release-please-manifest.json` records as released. Reading the
//! version out of a heading meant taking the run of digits and dots and parsing
//! it, and the character that ends the run was allowed to be anything, because
//! the spellings the rule was written for end it with a space, a `]` or a `)`:
//!
//! ```text
//! ## [0.1.0] - 2026-09-02
//! ## 0.2.0 - 2026-09-02
//! ## 0.1.0 (2026-09-05)
//! ```
//!
//! A `-` directly against the digits is none of those, and semantic versioning
//! spells a pre-release exactly that way. With the manifest recording `1.2.3`,
//! `## 1.2.3-rc.1` and a link to `releases/tag/v1.2.3-rc.1` both read as
//! `1.2.3`, were found to be at the ceiling, and passed — a section and a link
//! for a release nobody cut, reported by neither rule, which is the whole of
//! what the two rules are for.
//!
//! **The input.** Any pre-release or build-metadata version: `1.2.3-rc.1`,
//! `1.2.3-beta`, `1.2.3+build.7`. Nothing exotic — it is what a project reaches
//! for the first time it wants to ship a candidate.
//!
//! **The correct behaviour.** The digits have to be the whole version, so the
//! run has to end where a version ends: at the end of the line, or at a space,
//! a `]` or a `)`. Anything else means the heading names something this scanner
//! cannot read, and an unreadable claim is not a proven one — it is reported,
//! which is the fail-closed direction and the one the rest of the rule already
//! takes for a version that will not parse at all.

use crate::common::version::{
    sections_claiming_a_release_not_recorded, tag_references_not_recorded,
};

/// A changelog claiming a candidate of the version that *was* released.
const A_CANDIDATE_OF_THE_RELEASED_VERSION: &str = "\
# Changelog

## [Unreleased]

## 1.2.3-rc.1 - 2026-09-05

### Added

- a candidate nobody cut.

## 1.2.3 - 2026-09-01

[1.2.3-rc.1]: https://github.com/P4suta/ginary/releases/tag/v1.2.3-rc.1
[1.2.3]: https://github.com/P4suta/ginary/releases/tag/v1.2.3
";

#[test]
fn a_prerelease_section_is_not_covered_by_the_release_it_precedes() {
    assert_eq!(
        sections_claiming_a_release_not_recorded(
            A_CANDIDATE_OF_THE_RELEASED_VERSION,
            Some("1.2.3")
        ),
        vec!["## 1.2.3-rc.1 - 2026-09-05".to_owned()],
        "`1.2.3-rc.1` is not `1.2.3`, and it is not below it either: the manifest records one \
         release and the changelog claims two. A scan that stops the version at the first \
         character it does not recognise reads the candidate as the release and finds nothing \
         wrong, which is the one answer this rule exists to prevent"
    );
}

#[test]
fn a_prerelease_tag_link_is_not_covered_by_the_release_it_precedes() {
    assert_eq!(
        tag_references_not_recorded(A_CANDIDATE_OF_THE_RELEASED_VERSION, Some("1.2.3")),
        vec!["releases/tag/v1.2.3-rc.1".to_owned()],
        "the same reading on the link side, and the same 404: `v1.2.3-rc.1` is a tag nobody cut, \
         and `v1.2.3` beside it is one that exists. Only the first is reported"
    );
}

#[test]
fn the_three_ordinary_spellings_of_a_release_heading_still_read() {
    for heading in [
        "## [1.2.3] - 2026-09-02",
        "## 1.2.3 - 2026-09-02",
        "## 1.2.3 (2026-09-05)",
        "## v1.2.3",
        "## 1.2.3",
    ] {
        let changelog = format!("# Changelog\n\n## [Unreleased]\n\n{heading}\n\nbody.\n");
        assert_eq!(
            sections_claiming_a_release_not_recorded(&changelog, Some("1.2.3")),
            Vec::<String>::new(),
            "`{heading}` names the version the manifest records. Tightening what may end a \
             version must not stop the scanner reading the spellings this changelog and \
             release-please's own output actually use"
        );
    }
}
