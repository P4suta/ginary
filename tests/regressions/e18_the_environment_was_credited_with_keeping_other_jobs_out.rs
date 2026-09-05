// SPDX-License-Identifier: MIT OR Apache-2.0
//! `docs/RELEASE.md` credited GitHub with an exclusivity the `release`
//! environment does not provide, and left the control that does provide it
//! undocumented.
//!
//! **What went wrong.** The `## One-time setup` section closed the argument
//! for the environment like this:
//!
//! ```text
//! GitHub enforces that from the other side too: an environment's variables
//! and secrets reach a job **only when the job declares that environment**, so
//! no other job of any other workflow can read them by accident.
//! ```
//!
//! The first clause is true and is the whole of what GitHub does. The
//! conclusion drawn from it is not: declaring the environment is not a
//! privilege GitHub hands to one job. *Any* job of *any* workflow of this
//! repository may write `environment: release`, and on a ref the
//! deployment-branch policy admits — a push to `main`, a `v*` tag — it is
//! handed the same client id and the same private key. A second reader is one
//! line of YAML away, and nothing on GitHub's side says no.
//!
//! What keeps this repository to one reader is this repository's own suite,
//! and it takes **two** rules rather than one. The first version of this file
//! credited a single test with it and was wrong in the same shape as the
//! sentence it was retracting:
//! `no_job_of_any_workflow_reads_the_release_credentials_without_declaring_the_environment`
//! requires every site that names a credential to sit in a job that declares
//! the environment, which bounds *where* a credential may be read and places no
//! bound at all on *how many* jobs read it — a second job that writes
//! `environment: release` and the private key satisfies it. The half that
//! bounds the number is
//! `exactly_one_job_of_this_repository_declares_the_release_environment`, which
//! collects every job of every workflow whose `environment:` is the release one
//! and requires there to be exactly one of them. Together, and only together,
//! they make a second reader a red suite in a pull request a human reviews —
//! which is a control, and is worth documenting as one, but it is *ours*.
//!
//! A document that attributes a repository's own discipline to the platform is
//! the document that gets thinned out by the next maintainer who trusts the
//! platform: delete the tests and the sentence still reads as true. A document
//! that attributes it to the wrong test of its own is worse, because the
//! citation looks checkable and the check does not cover what the sentence
//! promises.
//!
//! **The input.** Any record of the release credentials that says GitHub keeps
//! a second job out of the environment, and any citation of the controls that
//! names less than both of them.
//!
//! **The correct behaviour.** No record makes that claim, and the setup
//! section names both tests that are the actual control — which this file then
//! holds against the tree, because a document that cites a test by name is
//! only worth as much as the test still being there.

use crate::common::release::{ENVIRONMENT, RELEASE_WORKFLOW};
use crate::common::repo::{exists, read};

/// The document the claim was written in, and the two records that repeat its
/// argument.
///
/// Named one by one for the same reason as in
/// `e18_the_notice_overclaimed_what_repository_scope_exposes.rs`: the record
/// of this milestone has to quote the sentence to explain it.
const RECORDS: &[&str] = &[RELEASE_WORKFLOW, "docs/RELEASE.md", "docs/dev/log/E17.md"];

/// The claim, as written and in the two rewrites that keep it.
const EXCLUSIVITY_CLAIMS: &[&str] = &[
    "no other job",
    "no other workflow",
    "read them by accident",
    "nothing else can read",
];

/// The two tests that are the actual control, each with the regression that
/// pins it.
///
/// Neither is the control on its own, and the difference is the whole of what
/// this file is about. The first bounds where a credential may be read; the
/// second bounds how many jobs may be handed one.
const CONTROLS: &[(&str, &str)] = &[
    (
        "no_job_of_any_workflow_reads_the_release_credentials_without_declaring_the_environment",
        "tests/regressions/e17_the_release_credentials_were_read_outside_their_environment.rs",
    ),
    (
        "exactly_one_job_of_this_repository_declares_the_release_environment",
        "tests/regressions/e18_nothing_bounded_the_number_of_jobs_that_may_read_the_credentials.rs",
    ),
];

/// The file both live in.
const CONTROL_FILE: &str = "tests/release_workflow.rs";

/// `docs/RELEASE.md`'s `## One-time setup`, and nothing behind it.
///
/// The rest of the document argues about the release pull request and the
/// draft, so a needle searched for over the whole file is answered by prose
/// about something else — the shape
/// `the_release_document_sends_the_credentials_to_the_environment` already
/// guards against.
fn setup_section() -> String {
    read("docs/RELEASE.md")
        .split("## One-time setup")
        .nth(1)
        .unwrap_or_default()
        .split("\n## ")
        .next()
        .unwrap_or_default()
        .to_owned()
}

#[test]
fn no_record_says_github_keeps_a_second_job_out_of_the_release_environment() {
    for record in RECORDS {
        let text = read(record);
        for claim in EXCLUSIVITY_CLAIMS {
            let line = text
                .lines()
                .enumerate()
                .find(|(_, line)| line.contains(claim))
                .map_or_else(String::new, |(number, line)| {
                    format!("{}: {}", number + 1, line.trim())
                });
            assert!(
                !text.contains(claim),
                "{record} says `{claim}` of the `{ENVIRONMENT}` environment. GitHub gates the \
                 values on the declaration and on the deployment-branch policy and stops there: \
                 any job of any workflow that writes `environment: {ENVIRONMENT}`, on a ref the \
                 policy admits, is handed the same two values. The exclusivity is this \
                 repository's, not GitHub's — {line}"
            );
        }
    }
}

#[test]
fn the_one_time_setup_credits_the_test_that_is_the_actual_control() {
    let setup = setup_section();
    assert!(
        !setup.is_empty(),
        "docs/RELEASE.md has no `## One-time setup` section, so there is nothing that says where \
         the credentials go or what keeps them to one reader"
    );
    // The needle is a phrase only the retraction can supply. `same
    // environment` — what this assertion asked for first — is also what a
    // sentence *describing the control test* says ("requires each site to sit
    // in a job declaring the same environment"), so the retraction could be
    // deleted and the citation kept with this test still green.
    assert!(
        setup.contains("any job of any workflow"),
        "`## One-time setup` does not say what a second reader would look like. Any job of any \
         workflow of this repository may declare the environment, and on a ref the policy admits \
         it is handed the same two values; until the section says so, the reason it gives for the \
         environment is a guarantee nobody makes:\n{setup}"
    );
    assert!(
        setup.contains(CONTROL_FILE),
        "`## One-time setup` does not name `{CONTROL_FILE}`, which is where both halves of the \
         control live. A citation without the file is one the next maintainer cannot \
         check:\n{setup}"
    );
    let control_file = read(CONTROL_FILE);
    for (test, regression) in CONTROLS {
        assert!(
            setup.contains(test),
            "`## One-time setup` does not name `{test}`. What keeps this repository to one reader \
             of the release credentials is two of its own tests — one bounding where a credential \
             may be read, one bounding how many jobs may read it — and a control nobody documents \
             is one the next maintainer deletes as a duplicate of what they believe GitHub \
             already does:\n{setup}"
        );
        assert!(
            setup.contains(regression),
            "`## One-time setup` names `{test}` and not `{regression}`, the regression that pins \
             it. The regression is half of the citation:\n{setup}"
        );
        assert!(
            control_file.contains(&format!("fn {test}(")),
            "the section names `{test}` as half of the control and {CONTROL_FILE} no longer \
             declares it. A cited test that is not there is a documented control that does not run"
        );
        assert!(
            exists(regression),
            "the section names `{regression}` and it is not in the tree. The regression that pins \
             the control is half of the citation"
        );
    }
}
