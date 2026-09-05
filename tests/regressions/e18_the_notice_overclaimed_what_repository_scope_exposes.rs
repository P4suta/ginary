// SPDX-License-Identifier: MIT OR Apache-2.0
//! The missing-credentials notice told a maintainer that a repository-scoped
//! secret is readable by every workflow run there is. It is not, and the
//! sentence was the reason the notice gave for the whole arrangement.
//!
//! **What went wrong.** E17 moved both release credentials into the `release`
//! environment and wrote the reason into the notice, into `docs/RELEASE.md`,
//! into the E17 record and into one of the suite's own assertion messages, in
//! four spellings of one sentence: a value at repository scope is *readable by
//! every workflow run*. Two runs it is not readable by, both of them ordinary:
//!
//! - a workflow triggered by a pull request **from a fork** is given none of
//!   the repository's secrets, which is the case the sentence was reaching for
//!   and the case it gets wrong;
//! - a **Dependabot**-triggered run cannot read Actions secrets at all — it
//!   sees the separate Dependabot secrets and nothing else.
//!
//! A reason that is false is worse than no reason: the next maintainer who
//! knows about fork pull requests reads the sentence, concludes the notice is
//! wrong about how secrets work, and has no way to tell which half of the
//! paragraph to keep. What the environment actually buys is narrower and
//! entirely sufficient: its values reach only a job that **declares** it, and
//! only on a ref its own protection rules admit — here a deployment-branch
//! policy of the `main` branch and the `v*` tags.
//!
//! **The input.** Any record of the release credentials that explains the
//! environment by claiming universal readability of repository scope.
//!
//! **The correct behaviour.** No record makes that claim, and the notice gives
//! the two restrictions the environment does apply, in the words the workflow
//! and the document already use for them.
//!
//! The records are named one by one rather than walked. The milestone record
//! that describes this defect has to quote the sentence in order to explain
//! it, so a scan over every document in the tree would report the file that
//! reports the bug; and the scan is of the four places the claim was actually
//! written, which is a list a reviewer can check by reading it.

use crate::common::release::{ENVIRONMENT, RELEASE_WORKFLOW, notice_step};
use crate::common::repo::read;

/// Every committed record that explains why the credentials live in the
/// environment.
const RECORDS: &[&str] = &[
    RELEASE_WORKFLOW,
    "docs/RELEASE.md",
    "docs/dev/log/E17.md",
    "tests/release_workflow.rs",
];

/// The claim, in every spelling of it that reached the tree.
///
/// A needle list cannot be exhaustive over prose, and this one is not trying
/// to be: it is the sentence as written, four times, plus the two rewrites of
/// it that keep the quantifier. What stops the claim from coming back in some
/// fifth wording is not this list but
/// [`the_notice_gives_the_two_restrictions_the_environment_actually_applies`],
/// which requires the paragraph to say the true thing instead.
const UNIVERSAL_CLAIMS: &[&str] = &[
    "every workflow run",
    "readable by every",
    "readable from every",
    "every run there is",
];

#[test]
fn no_record_says_a_repository_scoped_value_is_readable_by_every_run() {
    for record in RECORDS {
        let text = read(record);
        for claim in UNIVERSAL_CLAIMS {
            let line = text
                .lines()
                .enumerate()
                .find(|(_, line)| line.contains(claim))
                .map_or_else(String::new, |(number, line)| {
                    format!("{}: {}", number + 1, line.trim())
                });
            assert!(
                !text.contains(claim),
                "{record} explains the `{ENVIRONMENT}` environment by saying a value at \
                 repository scope is `{claim}` readable. It is not: a pull request from a fork is \
                 given no repository secret at all, and a Dependabot-triggered run cannot read an \
                 Actions secret at all. The environment restricts the values to the jobs that \
                 declare it, on the refs its deployment-branch policy admits, and that is the \
                 whole of what it buys — {line}"
            );
        }
    }
}

#[test]
fn the_notice_gives_the_two_restrictions_the_environment_actually_applies() {
    let step = notice_step(RELEASE_WORKFLOW).unwrap_or_else(|| {
        panic!("no step of {RELEASE_WORKFLOW} prints the missing-credentials notice")
    });
    // `declares` is the word `release.yml`'s own header comment,
    // `docs/RELEASE.md` and `docs/dev/testing.md` all already use for the
    // binding — "a job's `vars` and `secrets` contexts carry an environment's
    // values only when the job declares that environment" — so requiring it
    // here is requiring the notice to say the thing the rest of the tree says,
    // not a phrasing invented by this test.
    assert!(
        step.run.contains("declares"),
        "the notice says where the credentials go and, since E17, why. The first half of the why \
         is the binding: an environment's values reach a job only when the job *declares* that \
         environment. The notice does not say it, so the only reason it gives is the branch \
         policy — and E18 deletes the false sentence that used to stand beside it:\n{}",
        step.run
    );
    for policy in ["main branch", "v*"] {
        assert!(
            step.run.contains(policy),
            "the notice names the second half of the why, the `{ENVIRONMENT}` environment's \
             deployment-branch policy, which admits `{policy}` and nothing outside `main` and \
             `v*`. Without it a maintainer is told to use an environment and not what the \
             environment refuses:\n{}",
            step.run
        );
    }
}
