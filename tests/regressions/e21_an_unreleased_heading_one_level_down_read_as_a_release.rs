// SPDX-License-Identifier: MIT OR Apache-2.0
//! The changelog rule recognises `##` and `###` headings and then excludes the
//! unreleased one at `##` only, so the same heading one level down reads as a
//! released version.
//!
//! **What went wrong.** `crate::common::version::version_header_lines` is
//! written to release-please's own `DEFAULT_VERSION_HEADER_REGEX`,
//! `'\n###? v?[0-9[]'`, and therefore matches both heading levels.
//! `released_section_headings` then filters that list with
//!
//! ```text
//! .filter(|line| !line.starts_with("## [Unreleased]"))
//! ```
//!
//! which is one `#` narrower than what it filters. A changelog whose living
//! section is written `### [Unreleased]` — the shape a document that keeps its
//! releases under an `## Unreleased` umbrella, or one reflowed by a tool,
//! ends up in — is reported as a *released* section. The assertion that E20
//! exists to make, that this repository claims no release it has not cut, then
//! fails on the one heading that claims nothing at all.
//!
//! **The input.** Any changelog whose `[Unreleased]` heading is an H3.
//! `version_header_lines` was widened to both levels on purpose, and only half
//! of the pair moved with it.
//!
//! **The correct behaviour.** The exclusion is stated over the same shape the
//! match is: whichever heading level release-please would splice at, a heading
//! whose version is `[Unreleased]` is the living section and not a release.

use crate::common::version::{released_section_headings, version_header_lines};

/// A changelog carrying its living section at H3 and one real release at H2.
const H3_UNRELEASED: &str = "\
# Changelog

### [Unreleased]

- something not yet cut

## [0.1.0] - 2026-09-02

- the first release
";

#[test]
fn an_unreleased_heading_at_h3_is_not_a_released_section() {
    let headings = released_section_headings(H3_UNRELEASED);

    assert_eq!(
        headings,
        vec!["## [0.1.0] - 2026-09-02".to_owned()],
        "`### [Unreleased]` is the living section at the other heading level release-please's own \
         regex matches, and it claims no release. The filter excludes `## [Unreleased]` alone, so \
         it reports the one heading in this document that names nothing"
    );
}

#[test]
fn both_heading_levels_are_still_read_as_versions() {
    // The calibration. The exclusion narrows what counts as *released*; it
    // must not narrow what counts as a version heading, or a hand-written
    // `### 0.1.0` becomes invisible to the same rule.
    let text = "## [Unreleased]\n### [Unreleased]\n## [0.1.0]\n### v0.2.0\n## Notes\n";

    assert_eq!(
        version_header_lines(text),
        vec![
            "## [Unreleased]".to_owned(),
            "### [Unreleased]".to_owned(),
            "## [0.1.0]".to_owned(),
            "### v0.2.0".to_owned(),
        ],
        "release-please splices at `##` or `###`, an optional `v`, then a digit or a `[`; \
         `## Notes` is prose"
    );
    assert_eq!(
        released_section_headings(text),
        vec!["## [0.1.0]".to_owned(), "### v0.2.0".to_owned()],
        "and of those, two name a release: neither spelling of `[Unreleased]` does"
    );
}
