<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
<!-- Title this pull request as a Conventional Commit: type(scope): subject -->

## What changed

Describe the behaviour a user of the CLI or of a packaged artifact can observe, and the
smallest reason the change exists. Link the issue this closes.

## TDD evidence

This project starts every change from a failing test, and a bug fix without a regression test
is rejected. Tick what actually happened, and say so below if a row does not apply.

- [ ] I wrote or changed the test before the implementation.
- [ ] I watched the test fail for the intended reason, not on a compile error.
- [ ] For a bug fix: I added a regression test under `tests/regressions/` naming the defect.
- [ ] The narrow test passes now, and the rest of the suite still does.

## Gates

- [ ] `mise run check` passes: fmt, clippy over both feature sets, the test suite, rustdoc and
      `cargo deny`.
- [ ] Every new `uses:` in a workflow is pinned to a full commit SHA with a `# vX.Y.Z` comment.
- [ ] No `unwrap`, `expect`, `panic!` or `unreachable!` reaches the launcher path, and no new
      `#[allow(unsafe_code)]` arrives without an ADR.

## Documentation

- [ ] Public items are documented, prose hard-wraps at about 100 columns, and a structural
      decision carries an ADR under `docs/adr/`.
- [ ] `CHANGELOG.md`, `README.md` and the milestone log under `docs/dev/log/` are accurate for
      what this change does.

## Verification notes

List the commands you ran, and any check that only CI can run — the macOS and Windows jobs, the
cross-build smoke matrix — so a reviewer knows what is still owed by the run rather than by you.
