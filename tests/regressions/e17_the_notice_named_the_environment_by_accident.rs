// SPDX-License-Identifier: MIT OR Apache-2.0
//! The test that was supposed to prove the missing-credentials notice sends a
//! maintainer to the `release` **environment** could not fail.
//!
//! **What went wrong.** E17 moved both release credentials out of repository
//! scope and into the `release` GitHub Environment, and rewrote the notice a
//! repository with no credentials prints so that it names the environment as
//! the place to add them. The test written for that,
//! `the_notice_sends_a_maintainer_to_the_environment_and_not_to_repository_scope`,
//! found the notice step by the credential names it prints and then asserted
//!
//! ```text
//! step.run.contains("release")
//! ```
//!
//! The notice cannot avoid the word: it says `release-please did not run`,
//! `the release-please job did nothing` and `the release-please GitHub App`.
//! So the assertion was satisfied by text that has nothing to do with the
//! environment, and deleting every mention of the environment from the notice
//! — or sending the maintainer to the wrong one — left it green. It is the
//! same tautology class E5 already paid for once in
//! `e5_the_credentials_notice_was_not_tied_to_the_missing_credentials`, where
//! an assertion satisfied by the text it searched "could not fail, whatever
//! the notice job's `if:` said".
//!
//! **The input.** Any notice that stops naming the environment both values
//! belong to, or names a different one. A maintainer who follows it adds the
//! private key where the job cannot read it, and the Release workflow goes on
//! reporting an unconfigured repository.
//!
//! **The correct behaviour.** The rule reads the whole click-path down to the
//! environment's own name, `Settings -> Environments -> release`, which only
//! an environment-scoped instruction can produce. [`sends_to_the_environment`]
//! is that rule, and it is held here against the committed notice *and*
//! against a notice with the environment name changed — the exact edit the old
//! assertion could not see.

use crate::common::repo::{WorkflowStep, workflow_steps};

/// The workflow the notice lives in.
const RELEASE: &str = ".github/workflows/release.yml";

/// The variable the notice names.
const CLIENT_ID_VAR: &str = "RELEASE_PLEASE_APP_CLIENT_ID";

/// The secret the notice names.
const PRIVATE_KEY_SECRET: &str = "RELEASE_PLEASE_APP_PRIVATE_KEY";

/// The environment both credentials live in.
const ENVIRONMENT: &str = "release";

/// Whether a notice sends a maintainer to the `release` environment.
///
/// The needle is the click-path with the environment's own name on the end.
/// `Settings -> Environments` alone names a page listing every environment,
/// and the name alone is a word the notice prints anyway.
fn sends_to_the_environment(notice: &str) -> bool {
    notice.contains(&format!("Settings -> Environments -> {ENVIRONMENT}"))
}

/// The rule as it was written, kept only to show what it could not see.
fn names_the_word_release(notice: &str) -> bool {
    notice.contains(ENVIRONMENT)
}

/// The step that tells a maintainer which credentials are missing.
///
/// Found by the credential names it prints and by not failing — the notice is
/// green, the half-configured check is red. Selecting it by content is why
/// every assertion about its *content* has to name something the search did
/// not.
fn notice_step() -> WorkflowStep {
    workflow_steps(RELEASE)
        .into_iter()
        .find(|step| {
            step.run.contains(CLIENT_ID_VAR)
                && step.run.contains(PRIVATE_KEY_SECRET)
                && !step
                    .commands()
                    .iter()
                    .any(|command| command.starts_with("exit ") && command != "exit 0")
        })
        .unwrap_or_else(|| {
            panic!(
                "no step of {RELEASE} names both `{CLIENT_ID_VAR}` and `{PRIVATE_KEY_SECRET}` \
                 without failing: there is nothing that could tell a maintainer where to add them"
            )
        })
}

#[test]
fn the_committed_notice_sends_a_maintainer_to_the_named_environment() {
    let notice = notice_step();
    assert!(
        sends_to_the_environment(&notice.run),
        "step {} of `{}` is the notice a repository with no release credentials prints, and it \
         does not name the `{ENVIRONMENT}` environment as the place both values go. A maintainer \
         following it adds them at repository scope, where the job that declares the environment \
         cannot read them:\n{}",
        notice.position,
        notice.job,
        notice.run
    );
}

#[test]
fn a_notice_that_names_another_environment_is_rejected() {
    // The committed notice with one edit: the environment it sends a
    // maintainer to. Everything else about it is untouched, including every
    // occurrence of `release-please`.
    let flipped = notice_step().run.replace(
        &format!("Settings -> Environments -> {ENVIRONMENT}"),
        "Settings -> Environments -> staging",
    );
    assert!(
        names_the_word_release(&flipped),
        "this test is the demonstration that `contains(\"{ENVIRONMENT}\")` cannot fail, so the \
         flipped notice has to still contain the word — it does, in `release-please`, whatever \
         the environment it names. If it no longer does, the notice was rewritten and the \
         demonstration needs rewriting with it:\n{flipped}"
    );
    assert!(
        !sends_to_the_environment(&flipped),
        "a notice sending a maintainer to `staging` passes the rule that is supposed to prove it \
         sends them to `{ENVIRONMENT}`. The rule is reading something every notice contains \
         rather than something only the right one does:\n{flipped}"
    );
}
