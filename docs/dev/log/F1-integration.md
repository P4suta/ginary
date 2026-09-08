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

TLC states under `.cache/tla/states` were named in the upload paths but excluded by the
pinned upload action's hidden-file default. The regression
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

## First integrated execution and corrections

The full Windows fault-injection run completed every planned test with 2,134 successful,
37 explicitly skipped, one failed and none unrun or interrupted. E23's Windows synthetic
artifact is named `hello.exe`; one pre-existing SBOM assertion still expected `hello.spdx.json`.
The actual `hello.exe.spdx.json` output follows the F1 artifact-filename contract. The assertion
now explicitly accounts for the Windows executable suffix. All 23 SBOM tests and the complete
CI-matrix/assurance targets passed on rerun; the original full failure remains available in
`integration-tests-faults/`, with corrected targets in `integration-repair-faults/`.

[The first PR CI run](https://github.com/P4suta/ginary/actions/runs/34279370766) completed
17 jobs successfully, with that same single assertion failing Windows and therefore the
Required CI aggregate. Both real macOS build/launch/signature jobs, all three Linux test
configurations, cross-platform stubs, cross-Linux smoke, MSRV 1.88 and both coverage gates
passed. CodeQL passed independently. These successes do not make the failed run a passing gate.

Downloading the first CI run's `formal-evidence` confirmed that it contained the Java/TLC logs
but no states. The hidden-state regression was extended to both CI and nightly, observed to
fail for CI, and then passed after the same narrowly scoped upload option was added to CI.
The downloaded incomplete artifact and RED/GREEN logs remain in
`integration-formal-upload-red/` and `integration-ci-formal-{red,green}/`.
