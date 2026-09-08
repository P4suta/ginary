<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# F1 integration — preserving the Windows qualification on current main

After the local F1 implementation and qualification, the user authorized commit, push, pull
request and merge. The prohibition on actual releases remains in force. This integration
does not merge the existing release-preparation PR, create a tag, dispatch distribution or
publish a release.

The original F1 change was preserved in a signed local commit before rebasing onto current
`main`, `2e19b9d`. That base contains the E23 Windows launcher/runtime tests and the follow-up
documentation correction from PRs #11 and #12. The F1 measurements in [F1.md](F1.md) describe
the recorded pre-integration source; they are not silently attributed to these combined bytes.

## Integration decisions

- Keep E23's native Windows test runtime, packaged `hello_ffi` cold/warm launches, checked
  application exit codes, rustdoc gate and structured operating-system error assertions.
  Retain F1's independent feature configurations and complete failure-evidence capture.
- Keep E23's Windows system-DLL and API-set validation rules alongside F1's object-format,
  destination-conflict and staged verification results. Both regression sets remain registered.
- Retain F1's cache ownership and publication guarantees. Overlapping Unix-only symbol
  documentation is written so that the Windows documentation build can resolve it.
- Preserve earlier qualification and failing attempts. Hosted checks of the integrated PR
  supply an additional execution record, rather than replacing the earlier local evidence.

## Findings before push

New shell entry points require executable Git modes even when checked out on Windows with
`core.filemode=false`; their staged modes are explicitly `100755`. One JSON fixture arrived
with CRLF under a no-conversion fixture rule. It was normalized to LF without changing its
JSON content. The final staged whitespace check covers untracked additions as well as edits.

The upload paths named `.cache/tla/states`, but the pinned upload action's hidden-file
default would exclude any state files retained there after a failure. The regression
`f1_formal_state_evidence_was_hidden` first failed its explicit assertion (`None` versus
`Some(true)`), then passed after enabling hidden files for the two narrowly scoped formal
evidence paths. The earlier failed run remains in `.cache/assurance/F1/integration-formal-*`.
This follows the [pinned action's documented hidden-file behavior](https://github.com/actions/upload-artifact/blob/043fb46d1a93c77aae656e7c1c64a875d1fc6a0a/README.md#uploading-hidden-files).

`cargo deny check --hide-inclusion-graph` passed advisories, bans, licenses and sources during
integration. Its output remains in `.cache/assurance/F1/integration-cargo-deny.log`.

## Publication boundary

Before integration, the remote had no tags or releases; the release-preparation PR was open,
and no merged release PR awaited processing. The main-push release maintenance workflow can
update that preparation PR but does not publish it. Distribution has only explicit dispatch
or call triggers and defaults `publish` to false. These repository states must be rechecked
before merge; no release workflow is manually invoked as part of this work.

## Integrated mutation inventory

Discovery-only enumeration with cargo-mutants 27.1.0 after E23 integration found
960 candidates: appfile 207, cache 282, closure 98, launch 104, payload 121,
trailer 27 and verify 121. Each source hash remained unchanged across its
individual offline discovery command. Full candidate JSON, source/output hashes
and the summary remain under `.cache/assurance/F1/integration-mutation-recount/`.

Verification grew from 107 to 121 candidates. The previous nine divisions would
assign fourteen candidates to a shard and violate the existing thirteen-candidate
cap (`budget-red.json`). Ten divisions pass the same bound (`budget-green.json`),
bringing the complete canonical plan to 81 divisions. The build/test caps remain
120/420 seconds and the per-shard workload cap remains thirteen.

`tests/fixtures/nightly/mutants-F1-integration-counts.json` is the current count
fixture, read through `common::nightly::CURRENT_MUTANT_COUNTS`. The original F1
count fixture and E21 timing fixture are unchanged. The 80 canonical divisions,
946 candidates and 94 native jobs in the earlier F1 logs describe their recorded
snapshots; they are not claims about this integrated enumeration. No new mutation
baseline or mutation execution is claimed by this recount.

The integrated native planner then preserved and routed all 960 candidates exactly once:
920 Linux, 38 Windows and two macOS candidates across 95 nonempty native jobs. The canonical
empty payload tail remains in the plan. This is discovery qualification, not mutant execution;
`integration-mutation-plan/qualification.json` retains the plan and source identities.
It records zero executed mutants. The full 960-candidate campaign remains incomplete;
the earlier two caught Windows mutations describe their earlier frozen source and do not
qualify this integrated campaign.

## First integrated execution and corrections

The full Windows fault-injection run completed every planned test with 2,134 successful,
37 explicitly skipped, one failed and none unrun or interrupted. E23's Windows synthetic
artifact is named `hello.exe`; one pre-existing SBOM assertion still expected `hello.spdx.json`.
The actual `hello.exe.spdx.json` output follows the F1 artifact-filename contract. The assertion
now explicitly accounts for the Windows executable suffix. All 23 SBOM tests and the complete
CI-matrix/assurance targets passed on rerun; the original full failure remains available in
`integration-tests-faults/`, with corrected targets in `integration-repair-faults/`.

`integration-windows-qualification.json` reconciles the full run with complete replacement
results for SBOM, CI-matrix and assurance, plus the strengthened formal-upload regression.
At `3d809c26ec73e221f1595701da5197aa14e9dac2`, it records 2,135 successful, 37 skipped,
zero failed, zero unrun and zero interrupted tests. These are replacement results, not
additional passes added to the failed run. The source hashes show that product code is
unchanged from the full-run commit `b2d27dd`; the explicit skips remain separate from passes.

[The first PR CI run](https://github.com/P4suta/ginary/actions/runs/34279370766) completed
17 jobs successfully, with that same single assertion failing Windows and therefore the
Required CI aggregate. Both real macOS build/launch/signature jobs, all three Linux test
configurations, cross-platform stubs, cross-Linux smoke, MSRV 1.88 and both coverage gates
passed. The separate CodeQL workflow also completed successfully; workflow execution is not
the CodeQL code-scanning result. These successes do not make the failed run a passing gate.

Downloading the first CI run's `formal-evidence` confirmed that it contained the Java/TLC logs
but no states. This observation alone does not show an incomplete upload. The hidden-state
regression was extended to both CI and nightly, observed to
fail for CI, and then passed after the same narrowly scoped upload option was added to CI.
The original downloaded artifact and RED/GREEN logs remain unchanged in
`integration-formal-upload-red/` and `integration-ci-formal-{red,green}/`.

The corrected CI artifact also contains only `java.log` and `tlc.log`. The pinned TLC source
and JAR bytecode explain why: successful model checking attempts recursive deletion of its
state directory. Deletion is best effort; absent state files after a successful Ubuntu run
are expected. The [pinned cleanup implementation](https://github.com/tlaplus/tlaplus/blob/v1.7.4/tlatools/org.lamport.tlatools/src/tlc2/tool/ModelChecker.java#L763-L784)
uses nonrecursive deletion after an error, preserving nonempty failure-state directories.

An isolated Windows rehearsal with the exact pinned JAR confirmed an invariant failure
(exit 12) retained a 39-byte `Counter-0.st`, three empty metadata files and the full
`x = 0` to `x = 1` to `x = 2` counterexample. Its successful control exited zero and retained
only four empty files. The real model and its configuration were unchanged.
`integration-formal-state-rehearsal/cleanup-audit.json` preserves the commands, inventories,
hashes, primary sources and correction to the initial byte-total summary. This proves local
failure-state retention; it does not claim that a failing model was uploaded by hosted CI.
The hidden-file upload option protects files TLC retains and does not require success states
to exist. The hosted inventory and its corrected interpretation are retained separately in
`integration-hosted/34281075949/formal-evidence-{qualification,interpretation}.json`.

## Hosted qualification at the corrected head

[CI run 34281075949](https://github.com/P4suta/ginary/actions/runs/34281075949) at
`3d809c26ec73e221f1595701da5197aa14e9dac2` completed all 19 jobs successfully, including
Required CI. The independent coverage executions reported:

| Execution | Line coverage | Branch coverage | Required floor |
| --- | --- | --- | --- |
| Lines | 18,152 / 20,002 (90.75%) | Not measured | 90% lines |
| Branches | 17,546 / 19,425 (90.33%) | 2,457 / 2,932 (83.80%) | 90% lines, 80% branches |

The line denominators belong to their respective instrumentation runs; they are not merged
into one measurement. The native Windows job passed the repaired SBOM assertion, the full
native test step, rustdoc with warnings denied and explicit exit-code checks. Its real
packaged `hello_ffi` printed arguments, `hello from priv` and the caller's working directory,
then propagated exit zero, exit three and exit three again from the warm cache.

Both native macOS jobs built and launched real artifacts on their matching Intel and ARM
runners. They checked arguments and application exit status, and native `codesign` reported
the artifacts valid on disk and satisfying their Designated Requirement before and after
launch. These are build/launch/signature qualifications for those jobs; they do not claim
that the full mutation campaign or every test configuration ran on macOS. Linux plain,
stub and fault suites, cross-Linux smoke, MSRV 1.88 and the formal model job also passed.
The formal run generated 31,939 states, found 7,860 distinct states and reached depth 29.

The [CodeQL workflow](https://github.com/P4suta/ginary/actions/runs/34281075886) and dependency
review completed successfully, but the separate CodeQL code-scanning check
`102245856238` failed on five high-severity `actions/cache-poisoning/poisonable-step` alerts
in `.github/workflows/distribute.yml`. Alerts 10 through 14 identify execution of the selected
distribution commit in the default-branch cache context. This security result blocks PR
qualification despite the 19 successful CI jobs. Auto-merge was disabled, the PR remained
open, and no alert was dismissed or bypassed. The workflow security fix is pending at this
recorded head.

The complete check rollup, security annotations and alert responses, native and coverage
logs, their hashes, and the unmerged PR state are retained under
`integration-hosted/34281075949/`. Its `qualification.json` explicitly distinguishes CI
success and CodeQL workflow success from code-scanning failure. The earlier failed CI run
remains under `integration-hosted/34279370766/`; none of these hosted observations replace
the earlier local F1 measurements or complete the 960-candidate mutation campaign.

## Distribution execution-context correction

The security correction binds all five distribution checkouts directly to the workflow event
SHA. An inline, case-sensitive Bash guard rejects branch runs and mismatched tag inputs before
checkout. The version job then peels the fetched tag and compares both it and HEAD with the
event SHA before running repository code. Later builders retain that immutable SHA even if
the tag moves. A reusable workflow receives its caller's context and must pass the same guard.
The manual rehearsal instructions now specify both the workflow ref and matching tag input.

The three initial regressions failed against the old workflow for the intended reasons:
input-selected checkout and the missing execution-context guard. After correction, five tests
passed, including 15 actual Bash context cases and six private Git-fixture revision cases.
The latter accept lightweight and annotated tags, and refuse moved/missing tags, a wrong event
SHA and a wrong HEAD without producing accepted revision outputs. Temporary fixture tags never
touch the project repository. The complete release-workflow, distribution and CI-matrix targets
recorded 97 successes, one explicit Windows skip and no failed/unrun/interrupted tests.
Actionlint also passed. Evidence remains in `integration-distribution-context-*`, including an
initial invalid feature-name command separately from the meaningful assertion RED.

An independent review checked the context boundary, dependency propagation and immutable
checkouts. The follow-up hosted run must confirm both ordinary CI and the separate CodeQL
security result before auto-merge is restored. No scan finding is suppressed or dismissed.
