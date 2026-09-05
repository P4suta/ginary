// SPDX-License-Identifier: MIT OR Apache-2.0
//! The rule that every credential-dependent step of `release.yml` carries the
//! configured guard excused the notice by the *text* of its condition, so it
//! excused every other step that carried the same text with it.
//!
//! **What went wrong.** E5's rule, and its twin in
//! [`tests/release_workflow.rs`](../release_workflow.rs), built the list of
//! steps the guard is about like this:
//!
//! ```text
//! .filter(|step| step.position != 1 && !step.cond.contains(ABSENT))
//! ```
//!
//! Two steps of the job are not what the rule is about: the check at position
//! one, which computes the answer, and the notice, which runs on its
//! complement and is asserted on separately. The first exception names the
//! step it excuses. The second names a *string*, and the notice is not the
//! only step that can carry it. Any later step guarded on
//! `steps.credentials.outputs.state == 'absent'` — a step that reads the
//! private key and wears the notice's guard because it was copied from the
//! notice — is dropped from the scan rather than reported by it, and the rule
//! that exists to say "this step runs whether or not the credentials are
//! there" says nothing at all about the one step that runs *because* they are
//! not.
//!
//! Nothing about that has a symptom a green run could show: the run that
//! reaches the miswritten step is the run of a repository with no credentials,
//! which is the run nobody is watching.
//!
//! **The input.** Any step of the credentials job, other than the notice,
//! whose `if:` contains the absent guard.
//! `tests/fixtures/release/a_second_step_guarded_on_absent.yml` is that
//! workflow: `release.yml`'s shape, plus one step that writes
//! `secrets.RELEASE_PLEASE_APP_PRIVATE_KEY` to a file and calls the API behind
//! the notice's condition.
//!
//! **The correct behaviour.** The notice is excused by *which step it is* —
//! the position [`notice_step`] already located it at — and every other step
//! that needs the credentials is required to carry the configured guard,
//! whatever its condition happens to say. A rule that reads a workflow it was
//! not written for is the only kind that can be shown to fire, so the rule is
//! a function of a path and this file hands it the fixture.

use crate::common::release::{
    ABSENT, CONFIGURED, RELEASE_WORKFLOW, notice_step, steps_that_need_the_credentials,
    steps_that_need_the_credentials_and_are_not_gated,
};

/// The workflow with one step too many.
const FIXTURE: &str = "tests/fixtures/release/a_second_step_guarded_on_absent.yml";

/// The name of the step in it that the rule has to report.
const INJECTED: &str = "Open the follow-up issue as the App";

#[test]
fn a_step_that_is_not_the_notice_does_not_inherit_the_notices_excuse() {
    let notice = notice_step(FIXTURE)
        .unwrap_or_else(|| panic!("{FIXTURE} has no missing-credentials notice to excuse"));
    assert!(
        notice.cond.contains(ABSENT),
        "the fixture's notice is the step that carries `{ABSENT}`; this one carries `{}`, so the \
         fixture no longer poses the question this file is about",
        notice.cond
    );

    let reported: Vec<String> = steps_that_need_the_credentials_and_are_not_gated(FIXTURE)
        .into_iter()
        .map(|step| step.name)
        .collect();
    assert_eq!(
        reported,
        vec![INJECTED.to_owned()],
        "step `{INJECTED}` of {FIXTURE} writes `$PRIVATE_KEY` to a file and calls the API behind \
         `{ABSENT}` — the notice's guard, on a step that is not the notice. It needs the \
         credentials and it runs exactly when they are absent, and the rule has to say so. \
         Excusing the notice by the text of its condition excuses this step with it: the notice \
         is excused by the position it was found at, `{}`, and nothing else is",
        notice.position
    );

    // The other half of what makes the fixture adversarial, and the half the
    // assertion above cannot see: the injected step has to be *wearing the
    // notice's guard*. Reported-and-not-guarded-on-absent is a step the rule
    // before E18 reported too, so a fixture edited to `if: always()`, or to no
    // condition at all, would leave this file green while it pinned nothing.
    // Read out of the rule's own list rather than off the file text, so that
    // the step asserted about is the step the rule saw.
    let scanned = steps_that_need_the_credentials(FIXTURE);
    let injected = scanned
        .iter()
        .find(|step| step.name == INJECTED)
        .unwrap_or_else(|| {
            panic!("{FIXTURE} no longer holds a step named `{INJECTED}`: {scanned:#?}")
        });
    assert!(
        injected.cond.contains(ABSENT),
        "the injected step of {FIXTURE} is reported, but its `if:` is `{}` rather than the \
         notice's `{ABSENT}`. A step that carries no guard at all is one the rule reported before \
         E18 as well, so the fixture no longer distinguishes the repaired rule from the broken \
         one: the question this file asks is what happens to a step wearing the notice's guard",
        injected.cond
    );

    assert!(
        steps_that_need_the_credentials_and_are_not_gated(RELEASE_WORKFLOW).is_empty(),
        "the same rule reports a step of {RELEASE_WORKFLOW}, where every step that needs the \
         credentials carries `{CONFIGURED}`. A rule wide enough to see the fixture must not have \
         become one that reports the notice, or the release job's own guard, as well: {:#?}",
        steps_that_need_the_credentials_and_are_not_gated(RELEASE_WORKFLOW)
    );
}
