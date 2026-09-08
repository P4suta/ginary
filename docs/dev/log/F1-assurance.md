<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# F1 — distribution and assurance evidence

Actual releases are prohibited in this session. No tag, draft, release, upload, publication,
commit or push was performed. Workflow and publication changes below are code only. The local
publication rehearsal replaces `gh` with a local shell function and has no hosted effects.

## Evidence summary for final qualification

This summary describes completed assurance checks. The historical test counts later in this
document belong to their named intermediate runs; the root `F1.md` records the final combined
Cargo configurations and fresh coverage measurement. A running outcome report is incomplete
evidence, even when its log already contains successful tests.

All evidence paths in the following table are relative to `.cache/assurance/F1/`.

| Check | Observed result | Retained evidence |
|---|---|---|
| TLC 1.7.4, OpenJDK 25.0.2 | Exit 0; 31,939 generated / 7,860 distinct states; depth 29; four temporal branches; no error; 33 seconds | `formal/tlc-approved.log`, `formal/run-approved.json`, `formal/availability.json` |
| actionlint 1.7.12 with real ShellCheck integration | All seven workflows exited 0; the later CI/Nightly reruns also exited 0; captures complete and children reaped | `actionlint-assurance-workflow-*.json`, `actionlint-smoke-final-ci.json`, `actionlint-smoke-final-nightly.json` |
| Direct ShellCheck 0.11.0 | All six assurance/smoke scripts exited 0, with no suppressed rule | `actionlint-smoke-final-helpers-green.json` and its empty stdout/stderr files |
| Python evidence/parser and script regressions | 11 test methods passed; helper methods include five coverage, five fuzz and seven real-smoke-script mock scenarios | `smoke-evidence-final-green.log`; failure-before-implementation record `smoke-evidence-red.log` |
| macOS workflow shell rehearsal | Six scenarios matched expected runtime/signature/logger verdicts and retained trace/output | `native-ci-extracted-green.log`, `native-ci-extracted/` |
| Coverage gate fixtures | All 15 malformed/boundary/valid-input verdicts matched; this does not measure source coverage | `coverage-gate/green.json`, `coverage-gate/red.json` |
| Mutation enumeration only | 946 candidates in the seven selected modules; zero mutations executed; all 80 shards fit the configured cap | `mutants-final-list.json`, `mutants-cache-invocations-list.json`; committed `tests/fixtures/nightly/mutants-F1-counts.json` distinguishes snapshot scopes |
| Sustained fuzz and hosted/native execution | No sustained fuzz run, hosted CI run or native macOS/Linux smoke execution was performed by this assurance task | CI budgets and local shell mocks are configuration/behavior checks, not substitutes for those executions |

The TLC evidence's model and configuration SHA-256 values still match `formal/Cache.tla` and
`formal/Cache.cfg` at this reconciliation. Its result applies to that finite model; it does not
prove all Rust behavior. The 3,870 whole-tree mutation count predates the final cache/strip
changes and is not a fresh whole-tree total. The latest cache-only count is 282; other selected
counts are appfile 207, closure 98, launch 104, payload 121, trailer 27 and verify 107.
None of these enumeration counts is a caught-mutation result.

Coverage prerequisites and the supported local command contract are recorded in
`F1-nightly-coverage-tools.md`. The local script and mock gate successes above establish no
90% line or 80% branch result. The fresh instrumented run's test/report/gate verdicts must be
read together in the root qualification record.

## Distribution

The previous workflow began after publication, tried to create the same release again, and
flattened every target's `catalog.json` into one directory. macOS also entered the Linux-only
upstream asset resolver; Windows supplied no runtime archive. Assertion failures in
`tests/distribution.rs` established the release-trigger and duplicate-create defects before
the workflow changed.

The workflow now takes an explicit existing tag, resolves it once to a commit, and propagates
that commit and its source date to every builder. Publishing defaults to false. Native Windows
and macOS builds repack their installed OTP root; that operation validates target, OTP version,
and linkage, copies the source without modifying it, refuses escaping or cyclic links, and
records the deterministic source-tree digest as local-root provenance. It does not claim an
upstream download checksum for a locally installed tree.

`otp merge` assembles seven separate fragments, verifies binary identity, format, architecture,
version and flavor, verifies runtime size and digest, refuses missing/extra/conflicting assets,
and merges all catalogs deterministically. One OTP version must cover every target. The copied
binaries and stubs are verified again before inventory creation. After staging is complete,
`create_dir` exclusively reserves output; per-file publication uses no-replacement semantics
and checks each final temporary file against its inventory digest. `SHA256SUMS` is published
last as the completion marker. A raced existing directory is never replaced; a publication
failure retains staging and names it in the error. The catalog writer stages and syncs JSON
before replacement.

The optional, explicitly enabled publication code requires an existing draft. It uploads only
after local verification, downloads the assets again, checks the exact asset set and every
checksum and attestation, then publishes. A failed check leaves the draft unpublished. The
mock rehearsal proves success, failed attestation, stale extra assets and already-public
refusal. It never invokes the actual GitHub CLI.

## Assurance gates and retained evidence

- CI runs fault-injection, default and launcher-only test configurations independently, with
  fail-fast disabled and thirty-day evidence artifacts even after failure. The evidence runner
  first lists planned tests, then runs serial libtest with captured successful-test output.
  Its versioned outcome report distinguishes successful, failed, reported skipped, not-run and
  interrupted tests. Logs and the last running state survive a killed runner; an unfinished
  record is never represented as successful coverage.
- CI measures stable line coverage and nightly branch coverage independently. Both line
  measurements enforce 90%; the branch measurement also enforces 80%. LCOV reports are retained
  for thirty days. A gate configured in a workflow is not evidence that the floor was achieved.
- Mutation shards allow 120 seconds to build and 420 seconds to test each mutant. The 150-minute
  job budget includes the baseline and evidence margin. Each job enumerates its actual shard
  and refuses more than thirteen mutants before execution, so source growth has a visible
  precondition failure instead of silently exhausting the entire job.
- Every fuzz target receives 600 seconds. Generated corpus and crashes use a stable artifact
  layout with metadata, and the previous completed run's retained corpus is restored. Empty
  corpora still have a metadata artifact. Mutation, fuzz and formal evidence are retained for
  thirty days, including failed runs.
- CI and Nightly run the pinned TLC checker with its SHA-256 verified before execution. The
  cache model now separates the renamed trash from the active entry, so deleting old trash
  cannot delete a new extraction published under the same key. The Rust formal tests validate
  repository/model shape; they do not substitute for TLC execution.

## Validation recorded locally

- Initial distribution workflow assertions failed for the published trigger and duplicate
  release creation. Initial assurance assertions failed for missing retained evidence and the
  short fuzz budget. The replacement behavior then passed.
- `distribution`, `assurance`, `ci_matrix`, `release_workflow`, `formal` and `otp_repack` focused
  integration targets passed. Distribution includes synthetic PE, Mach-O and ELF fixtures for
  all seven targets and local-root repacking for Windows and macOS; it does not execute a
  foreign native runtime. The publication transaction test initially exposed Windows path
  translation in the mock fixture, which was corrected and rerun successfully.
- The Python outcome adapter's assertion fixtures failed before implementation and passed
  afterward. They include a reported tool skip, an ignored test, duplicate test names in
  distinct harnesses, a failed test, a never-started test and an interrupted test.
- The evidence adapter also ran the real `assurance` Rust test target locally: three tests
  were listed, three completed successfully, and `outcomes.json` reported `complete: true`.
- An intermediate adapter review removed whole-process-output buffering: logs are spooled in 64-KiB
  chunks and parsed as bounded line chunks. At that stage four Python tests passed, including direct child
  output splitting a libtest result line and an actual child-process rehearsal of skip
  attribution and retained logs. Incomplete outcome evidence makes the runner fail even if
  Cargo's own exit status was zero.
- A read-only cargo-mutants 27.1.0 enumeration found 3,610 mutants in the working tree. Counts
  for the selected modules are retained in `tests/fixtures/nightly/mutants-F1-counts.json`;
  this is an enumeration, not a mutation-test pass. Cache and payload divisions were expanded
  to twenty-four and twelve shards because their new counts exceeded the previous divisions.
  Actual shard 0 enumerations held eleven and ten mutants respectively, below the thirteen cap.
- An earlier focused documentation/workflow recheck passed `ci_matrix` (44), `distribution` (8)
  and `v1_readiness` (14). All-target, all-feature clippy with warnings denied passed for that
  intermediate tree; the root qualification record owns the final combined verdict.
- `bash -n` passed for publication, formal-checking and coverage-gate scripts. Structured
  workflow tests parsed the edited YAML and validated the matrix and gate contracts.

Initial tool inspection found existing stable and nightly Rust, stable
LLVM tools, cargo-mutants 27.1.0, cargo-llvm-cov 0.8.7, nextest 0.9.140 and Git Bash.
The separately authorized nightly LLVM component installation is recorded in
`F1-nightly-coverage-tools.md`; the initial inventory is not a claim that no prerequisites
were later installed.
The CI coverage tool remains pinned to 0.9.0. Java was absent from PATH, but a later targeted
search found OpenJDK 25.0.2 under the user's `.jdks`; `java -version` ran successfully. No TLC
JAR was found in local tool/IDE caches. The ordinary download of the committed, official
TLC 1.7.4 URL failed with a network-sandbox socket permission denial. The supported approval
route then allowed the exact official download; its hash matched the committed pin. The
sandboxed Java attempt failed to initialize `java.security`; the approved local run completed
successfully in 33 seconds, with 31,939 generated states, 7,860 distinct states, depth 29, all
four temporal branches checked and no error found. `.cache/assurance/F1/formal/availability.json`
records model/configuration/JAR hashes and `tlc-approved.log` retains the actual checker output.
Actionlint was absent from the initially inspected PATH. A later prerequisite audit found
actionlint 1.7.12 and ShellCheck 0.11.0. The real Windows linter exposed an upstream pipe
deadlock when an inline Bash step exceeded the pipe buffer: actionlint wrote the script
before starting ShellCheck. Moving the native macOS body into `scripts/ci/macos-smoke.sh`
keeps full direct ShellCheck validation; all seven workflows and that helper then passed.
The native evidence wrapper was executed in six local success/failure scenarios, including
incorrect runtime exits, failed signature checks and failed log writing. These were shell
rehearsals, not native macOS executions. Hosted CI, real macOS execution, the entire mutation matrix and sustained fuzz
runs were not executed here. Coverage results are recorded by the root F1 validation log;
their availability must not be inferred from the workflow definition.

The final evidence parser also distinguishes libtest's display-only ` - should panic` and
rustdoc's ` - compile` suffixes from planned test names. Eight Python tests passed. The raw
first full fault-run report was preserved as `outcomes.initial-parser.json`; reclassification
of unchanged logs accounts for all 1,978 planned tests (1,930 successful, 16 failed, 32 skipped,
zero not-run/interrupted). This repairs attribution and does not change that run's failed
verdict. Subsequent fixes and reruns belong to the root F1 validation record.

The coverage gate rejected malformed input only incompletely: negative counters, hits larger
than totals, duplicate counters and a truncated source record could clear the floor. Fifteen
local Bash fixture cases now pass after adding source-record and counter validation; their
RED/GREEN results are in `.cache/assurance/F1/coverage-gate/`. The Rust gate tests now use
bounded Bash execution on both Unix and Windows with Git Bash. These fixture results validate
the gate, not the project's measured coverage. `docs/dev/testing.md` records the exact local
0.8.7/nightly environment/report sequence for branch coverage and doctest evidence.

The coverage job now invokes `scripts/ci/coverage.sh`: `show-env` prepares instrumentation,
the outcome adapter records every planned test, and LCOV reporting still runs after a test
failure. Separate test, report, line, branch and profile-copy verdicts preserve the original
failure. A fresh build directory avoids stale profiles; LCOV, raw profiles, subprocess evidence
and logs are retained for thirty days. Nightly branches include doctest instrumentation;
stable lines use the ordinary stable test configuration. These are runner changes, not a claim
that either coverage floor has been reached locally.

The actual nightly measurement later exposed a report-helper defect: report help listed
`--all-features`, but the 0.8.7 parser rejected it even in a fresh environment; CI's 0.9.0
parser has the same restriction. The flag remains on `cargo test` and has been removed only
from `cargo llvm-cov report`. Five stricter mocked scenarios first failed on that argument,
then all eleven Python tests passed after correction. Actual ShellCheck also passed the
changed helper. `coverage-report-contract-{red,green}.log` and
`actionlint-coverage-report-contract-shellcheck.json` retain these checks under
`.cache/assurance/F1/`; `F1-nightly-coverage-tools.md` records the exact source/probe evidence.
The root rerenders preserved successful-test profiles rather than discarding them or
rerunning the suite merely because report argument parsing failed.

The fuzz wrapper retains both output streams, duration, command and log exit codes, and
interrupted status. A job that never starts the fuzzer records `not_run`; generated corpus and
crashes remain in the thirty-day artifact. Restoring corpus paginates the artifacts API, so
the eighty mutation shard artifacts cannot hide a later fuzz artifact. At that stage ten Python tests passed,
including five coverage and five fuzz subprocess failure-path cases. Actual actionlint passes
all seven workflows, and actual ShellCheck passes all four checked-in assurance helpers.
The evidence is retained under `.cache/assurance/F1/assurance-scripts-final-test.log` and
`actionlint-assurance-*`.

A later read-only mutation enumeration after the then-current production changes found 3,870 candidates,
937 in the seven selected modules. Cache has 273, payload 121 and verify 107; verify therefore
uses all nine shards instead of eight. Every current division holds at most thirteen mutants.
The updated fixture records zero executed mutants, and the raw enumeration is
`.cache/assurance/F1/mutants-final-list.json`. Four 600-second fuzz targets require at least
forty minutes serially, or ten minutes in parallel, plus builds. The installed cargo-fuzz
0.13.2 requires a Unix runner; having its executable on Windows is not a successful fuzz run.
The subsequent extraction-invocation ownership correction raised the cache-only recount to
282, leaving its twenty-four-shard division within the cap. Selected module counts therefore
total 946. The fixture labels the earlier whole-tree total as a historical snapshot rather
than treating a module-only recount as a new whole-tree measurement.

The final Nightly smoke-matrix path now also preserves its command/log verdicts, elapsed time,
trace, build and verification logs, each container's actual output and exit, and generated
artifacts for thirty days. `GINARY_SMOKE_EVIDENCE_DIR` retains a unique workspace directory;
ordinary local smoke runs still remove their temporary fixture. Seven bounded mock scenarios
execute the real smoke script: successful runtime, build failure, verification failure, wrong
runtime exit, log failure, combined command/log failure, and default cleanup. The new assertions
failed before the wrapper existed; all eleven Python tests now pass. CI/Nightly actionlint and
direct ShellCheck over all six scripts pass. These mocks do not run Docker or native Linux
artifacts; their RED/GREEN evidence is `.cache/assurance/F1/smoke-evidence-*.log`.

README, release instructions, testing documentation and formal-model documentation now
describe these behaviors and distinguish implemented runner gates from observed executions.

## Final-artifact signature verification

A final review found that macOS assembly signed the temporary file and published it without
independently verifying the completed signature. `sign_macos::verify_ad_hoc` is now a separate
reader of the emitted format: it validates load commands, section/segment bounds, the complete
CodeDirectory profile, every code-page SHA-256, and the payload trailer/digest. The bundle path
also opens and verifies the completed artifact before chmod, synchronization and publication.
The signing fault seam and publication-preservation regression are recorded in the F1 build
log; the portable signature regression mutates signed pages, hash slots and each unsigned
signature-header field, truncates the file, changes command bounds, and supplies a false
payload digest under otherwise valid page hashes. These checks supplement native macOS CI;
they do not execute macOS or claim Gatekeeper acceptance on this Windows host.

## Measured coverage follow-up and stage ownership

The first recovered local nightly report measured 16,960/19,714 lines (86.03%) and
2,271/2,910 branches (78.04%). Both configured floors remained failures. The follow-up
adds public API tests in `tests/assurance_paths.rs` and `tests/assurance_runtime_paths.rs`:
real child supervision, crash-dump freshness and the 64-line read bound, Windows argument
round trips, native and stub input refusals, and Windows share-mode I/O failures. POSIX
native hooks retain their `/bin/sh` contract; these tests do not substitute a shell to
pretend that successful Unix hook execution was measured on Windows.

The assembly review reproduced another ownership failure: both a successful stage and a
failed one removed caller data planted at `out.tmp-<current PID>`. The two actual RED
results are `.cache/assurance/F1/stage-ownership-red.log`. Staging now exclusively creates
an invocation-specific sibling temporary directory and cleans only that directory.
Publication uses an atomic no-replace rename; cached rustix 1.1.4 exposes the Linux
`RENAME_NOREPLACE` and macOS `RENAME_EXCL` mappings. Failure to clean an owned directory
retains its path and both errors through additive `StageCleanupError` details inside the
existing `AssembleError::Io` variant, preserving exhaustive downstream matches. A
Windows unit regression holds a real file handle which denies deletion, then checks the
typed original error and recoverable residue. The additive inspection path is
`AssembleError::Io { source, .. }` followed by
`source.get_ref().and_then(|error| error.downcast_ref::<StageCleanupError>())`;
`StageCleanupError` exposes `operation`, `path` and `cleanup`, and its error source is
the original operation. Existing exhaustive matches on `AssembleError` remain valid.

The first Windows publication unit run then exposed an incorrect implementation assumption:
`std::fs::rename` also replaces an existing empty directory on this host. That run has two
passing tests and one failure; its original filename,
`.cache/assurance/F1/stage-ownership-units-green.log`, is retained despite the failed verdict.
The corrected path uses `MoveFileExW` with zero flags inside the existing Win32 unsafe
boundary. Long Unicode paths and embedded-NUL refusal are additional native regression
cases. The Win32 API reference is
[MoveFileExW](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-movefileexw);
the corrected implementation was then executed on Windows. All 63 integration tests across
`assemble`, `assurance_paths`, `assurance_runtime_paths` and `diagnostic_acceptance` passed,
with zero failed, skipped, not-run or interrupted outcomes. All four assembly unit tests
also passed, including actual no-clobber publication, long Unicode paths, NUL refusal and
cleanup-error downcasting with recoverable locked-file residue. The evidence is:

- `.cache/assurance/F1/coverage-stage-and-diagnosis-rerun/outcomes.json`
- `.cache/assurance/F1/coverage-stage-and-diagnosis-rerun/tests.log`
- `.cache/assurance/F1/stage-ownership-units-rerun.log`
- `.cache/assurance/F1/coverage-stage-and-diagnosis-rerun-exits.json`

These focused GREEN results establish the exercised behaviors. They do not turn the earlier
86.03%/78.04% coverage measurement into a passing floor verdict; fresh complete coverage
qualification is recorded separately in the final F1 evidence record.
