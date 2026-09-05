// SPDX-License-Identifier: MIT OR Apache-2.0

//! `notice_step` picked the wrong step when an earlier one failed silently.
//!
//! The notice is the credential-reading step that does not exit non-zero: the
//! release job has two such steps to tell apart, and only one of them is a
//! green report of an empty environment. E18 selected it by asking whether any
//! of the step's commands was an `exit` with a non-zero code.
//!
//! A script does not need `exit` to fail. Every `run:` in these workflows is a
//! `bash -e` script, so a bare `false` — or any command that returns non-zero
//! as its last act — fails the step just as surely. Such a step reads both
//! credential names, never says `exit`, and was therefore selected as the
//! notice. Everything downstream then followed: the guard scan excuses the
//! notice by position, so it excused the failing step and let the real notice,
//! or any other unguarded reader after it, pass unreported.
//!
//! `tests/fixtures/release/a_failing_step_before_the_notice.yml` is that
//! workflow. Its second step reads both credentials and ends in `false`; the
//! third is the real notice. The rule must skip past the failing step, find the
//! notice, and report the failing step as an unguarded reader — it carries no
//! guard at all.
//!
//! Reported by CodeRabbit on pull request 4 and confirmed against the tree.

use crate::common::release::{
    ABSENT, notice_step, steps_that_need_the_credentials_and_are_not_gated,
};

/// The workflow whose first credential-reading step fails without saying so.
const FIXTURE: &str = "tests/fixtures/release/a_failing_step_before_the_notice.yml";

/// The step in it that fails, and that the rule must not mistake for a notice.
const FAILING: &str = "Refuse a half-configured environment";

/// The step in it that really is the notice.
const NOTICE: &str = "Say what the release environment is missing";

#[test]
fn a_step_that_fails_without_saying_exit_is_not_the_notice() {
    let notice = notice_step(FIXTURE)
        .unwrap_or_else(|| panic!("{FIXTURE} has no missing-credentials notice to find"));
    assert_eq!(
        notice.name, NOTICE,
        "step `{FAILING}` of {FIXTURE} reads both credential names and ends in `false`, which \
         fails the step under `bash -e` as surely as `exit 1` would. It is not a green report of \
         an empty environment, so it is not the notice — `{NOTICE}` is, and a rule that reads \
         only `exit` codes picks the wrong one",
    );
    assert!(
        notice.cond.contains(ABSENT),
        "the notice runs on the complement of the guard; this one runs on `{}`",
        notice.cond
    );
}

#[test]
fn the_step_that_fails_is_reported_as_an_unguarded_reader() {
    let reported: Vec<String> = steps_that_need_the_credentials_and_are_not_gated(FIXTURE)
        .into_iter()
        .map(|step| step.name)
        .collect();
    assert_eq!(
        reported,
        vec![FAILING.to_owned()],
        "`{FAILING}` reads both credentials behind no guard at all, so the scan has to report it. \
         While it was mistaken for the notice it was excused by position instead, which is the \
         whole of the defect: one wrong answer upstream silenced the rule downstream",
    );
}
