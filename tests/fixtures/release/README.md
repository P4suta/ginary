<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# Release workflow fixtures

A rule that can only read the file it was written for has one piece of evidence
that it works: it has never fired. That is the same evidence a rule which
*cannot* fire produces, and E5 already paid for the difference once — the
assertion tying the missing-credentials notice to the missing credentials was a
tautology, and flipping the guard it was about left the suite green.

So the rules about the release credentials are functions of a workflow path, or
of a list of them (`crate::common::release`), and this directory holds the
workflows they have to fire on.

- `a_second_step_guarded_on_absent.yml` — `release.yml`'s shape with one step
  added: a step that reads `RELEASE_PLEASE_APP_PRIVATE_KEY` and carries the
  notice's `absent` guard rather than the `configured` one. It is not the
  notice, it needs the credentials, and a rule that excuses every step whose
  condition *contains* the absent guard cannot see it. Read by
  `tests/regressions/e18_a_step_that_was_not_the_notice_wore_the_notice_guard.rs`.
- `a_second_job_reads_the_credentials.yml` — `release.yml`'s job, correct on
  every count, plus an `announce` job that declares the same environment and
  whose **first** step reads `RELEASE_PLEASE_APP_PRIVATE_KEY` with no guard at
  all. Two rules have to fire on it: the step rule, which read one job of the
  workflow and so never looked at this step, and the bound on how many jobs may
  declare the environment, which did not exist. Read by
  `tests/regressions/e18_a_credential_reading_step_outside_the_check_job_was_not_scanned.rs`
  and
  `tests/regressions/e18_nothing_bounded_the_number_of_jobs_that_may_read_the_credentials.rs`.

Each fixture is adversarial in one specific way, and the test that reads it
asserts that it still is — that the injected step still wears the notice's
guard, that the injected job still declares the environment — before asserting
that the rule reports it. Without that, an edit which defuses a fixture leaves
its test green while it pins nothing, which is the failure this directory
exists to avoid rather than one it may reproduce.

Nothing here is run, and nothing here is a workflow of this repository: the
files live outside `.github/`, where the repository-wide scans and actionlint
do not reach them.
