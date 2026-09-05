// SPDX-License-Identifier: MIT OR Apache-2.0

//! The branch policy was credited with more than a policy alone can do.
//!
//! `docs/RELEASE.md` said the App's private key is unreachable from a pull
//! request, from a fork and from every other branch, and offered the
//! environment's deployment-branch policy — `main` and `v*` — as the reason.
//! A policy is only half of that guarantee. GitHub lets a repository
//! administrator force a waiting job past an environment's protection rules by
//! default, and a job released that way is handed the environment's secrets
//! like any other, so the sentence held only for everyone who is not an
//! administrator.
//!
//! The setting is configurable, and the `release` environment of this
//! repository now has it off (`can_admins_bypass: false`), which is what makes
//! the sentence true rather than optimistic. That makes it part of the setup a
//! maintainer restores the environment from — a repository that recreates the
//! environment and leaves the bypass on has quietly undone the guarantee its
//! own documentation makes.
//!
//! This file holds the document to saying so. It cannot reach GitHub to check
//! the live setting; what it pins is that the claim and its precondition are
//! written down together, so the claim cannot outlive the reason it rests on.
//!
//! Reported by CodeRabbit on pull request 4 and confirmed against the docs.

use crate::common::repo::read;

/// The document that makes the claim.
const DOCUMENT: &str = "docs/RELEASE.md";

#[test]
fn the_document_names_the_bypass_its_guarantee_depends_on() {
    let document = read(DOCUMENT);
    for needle in ["administrator", "bypass", "can_admins_bypass: false"] {
        assert!(
            document.contains(needle),
            "{DOCUMENT} does not mention `{needle}`. It tells a maintainer the private key is \
             unreachable from every branch but `main`, and that is true only while administrator \
             bypass is off — a reader restoring this environment from these notes would leave the \
             default on and undo the guarantee without noticing"
        );
    }
}

#[test]
fn the_unreachability_claim_stands_next_to_its_precondition() {
    let document = read(DOCUMENT);
    let claim = document
        .find("unreachable from a pull request")
        .unwrap_or_else(|| panic!("{DOCUMENT} no longer claims the key is unreachable"));
    let precondition = document
        .find("can_admins_bypass: false")
        .unwrap_or_else(|| panic!("{DOCUMENT} states no bypass precondition"));

    // The same paragraph, not merely the same file: a reader meets the claim
    // and the reason it holds in one breath, or the claim reads as absolute.
    let (first, second) = if claim < precondition {
        (claim, precondition)
    } else {
        (precondition, claim)
    };
    let between = &document[first..second];
    assert!(
        !between.contains("\n\n"),
        "{DOCUMENT} separates the unreachability claim from the `can_admins_bypass: false` that \
         makes it true by a paragraph break. A qualifier a reader has to go looking for is one \
         the next editor deletes"
    );
}
