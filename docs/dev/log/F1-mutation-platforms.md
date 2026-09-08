# F1 — execute each mutation where its code is compiled

The local execution of all three trailer shards exposed a CI defect: cargo-mutants 27.1.0
discovered two Windows `ReadAt::read_at` mutations even on Linux. Both survived because
`#[cfg(windows)]` excluded their enclosing implementation. The original Ubuntu-only matrix
therefore had two guaranteed failures unrelated to test strength.

The original records remain under `.cache/assurance/F1/linux-mutation/unprivileged/local/`.
All three baselines passed; 27 unique candidates have matching outcomes, logs and diffs:

| Canonical shard | Caught | Unviable | Missed | Exit |
| --- | ---: | ---: | ---: | ---: |
| trailer 0/3 | 8 | 1 | 0 | 0 |
| trailer 1/3 | 6 | 1 | 2 | 2 |
| trailer 2/3 | 9 | 0 | 0 | 0 |

This is a complete execution of three shards and a failed gate, not a successful run of
the entire mutation campaign. The full original enumeration is 80 canonical shards and
946 candidates, at most 13 per shard. Parsing the enclosing Rust cfg produces 906 Linux
assignments, 38 Windows assignments and two macOS assignments without removing a candidate.
The real Windows planning pass generated 94 native jobs. Its first attempt exposed a further
adapter error: cargo-mutants' ceiling division leaves the last payload shard empty (121
candidates over 12 shards yield eleven groups of 11 and a final zero). The corrected adapter
preserves the explicitly enumerated empty shard and creates no job for it, while refusing an
entirely empty campaign. The first failure is retained beside the successful `plan-rerun`.

## Implementation

`scripts/ci/mutation-divisions.json` preserves every original module and shard division.
`mutation.py plan` records each unfiltered `--list --json --diff` result and the relevant
checkout hashes. The standalone, unpublished `tools/mutation-plan` workspace uses `syn`
and source spans to attach native platform applicability to each candidate while preserving
all original fields. Unknown cfg predicates and unsupported or ambiguous ownership fail
planning. The parser handles enclosing modules, implementations, functions and cfg blocks;
function-replacement mutations use the function's cfg rather than the first statement's cfg.

The nightly workflow dispatches one job for every nonempty OS partition of a canonical
shard. Each native job rechecks source hashes, verifies Gleam/OTP startup and re-lists its
exact assigned candidates. The anchored selection regex is applied without a second
`--shard` operation, which would repartition the filtered list and silently omit candidates.
The 120-second build and 420-second test caps, complete baseline and 150-minute job budget
remain explicit. Full output is file-backed; process diagnostics retain a bounded 1 MiB
tail per stream, omitted byte counts, elapsed time and timeout/interruption status.
Complete JSON parsing has a separate 128 MiB cap and refuses overflow, duplicate fields and
non-finite numbers explicitly. A failed process-tree cleanup is retained as an error; it does
not silently turn a killed direct child into a claim that all descendants stopped.

The unconditional final gate downloads every native artifact and independently rereads raw
outcomes. A summary alone cannot establish success: candidate identity, actual build/test
phases, the canonical diff, log presence/hashes, selected command, captured process status
and stream contents must agree. Missing jobs and not-run candidates remain in final counts.
All plans, raw results and summaries have 30-day retention, including failures. No release
workflow is invoked by this change.

## Why assignment is external

The [upstream cfg discovery issue](https://github.com/sourcefrog/cargo-mutants/issues/50)
is still applicable to the pinned tool. Its [27.1.0 outcome implementation](https://github.com/sourcefrog/cargo-mutants/blob/v27.1.0/src/outcome.rs)
classifies a caught mutant from an actual failing test process, and an unviable mutant from
an actual failing build. The adapter validates those phases rather than trusting just their
labels. Conditional `mutants::skip` annotations are not used: cargo-mutants does not evaluate
the condition before honoring a nested skip, which could remove the very native candidates
this change is intended to exercise.

## Qualification

The AST parser's own adversarial fixtures and the Python adapter's fault fixtures are
required CI tests. The latter execute real subprocess failure, bounded capture and timeout
paths, and deliberately corrupt plans, selections, outcomes and evidence. CI contract tests
verify that planning, native execution and the unconditional aggregate consume the same
plan and preserve individual job identities. Local real execution and hosted runner coverage
are recorded separately; a configured native job is not evidence that macOS ran locally.

The final AST suite passed all 18 tests on Windows and Linux, and Windows Clippy passed.
`mutation-plan-final-qualification.json` records the final Windows parser qualification;
the earlier `mutation-plan-qualification.json` remains a historical 15-test result.
The final mutation-adapter suite passed all 42 tests on both operating systems, with no
skips; the broader helper run before the last baseline regression passed all 52 cases.
The 52 CI contract
tests passed independently in plain, stub and fault-injection configurations, as did all
three Clippy configurations. A first stub compile failure from new tests using an optional
TOML dependency was repaired with existing feature-independent manifest readers; the failed
run remains in `mutation-ci-final`, and the successful rerun in `mutation-ci-repair`.
The historical E21 shard-budget regression also passed. All seven final workflows passed
actionlint. Source hashes, full logs and process records are retained under the corresponding
`mutation-plan-*`, `mutation-adapter-qualified`, `mutation-linux-qualified`,
`mutation-regressions-final`, `mutation-ci-*` and `mutation-actionlint`
directories in `.cache/assurance/F1/`.

The initial independent Docker run passed 18 AST tests and 40 adapter tests; two later
isolated adapter runs passed 41 and then 42 tests. The final 42-test run is retained in
`mutation-linux-qualified/`, with no failures or skips, no network, read-only source,
two CPUs and 3 GiB memory. The `mutation-linux-tool/` and `mutation-linux-environment/`
records remain historical. A later real Windows baseline found the existing E7 contract's
missing `--no-fail-fast` flags in the two newly added planner test steps. Both flags were added.
The failed attempt remains intact with 589 passing regression tests and one failure, zero
executed mutations. Its raw result also exposed a reporting error: a legitimate failed
baseline was rejected as an unknown phase summary. The adapter now retains `baseline: Failure`,
its phases/log evidence and all unrun mutations. The exact failure has its own regression.

The next local attempt reused a Cargo target directory from another cargo-mutants scratch
copy. Its cached test binary still embedded the now-deleted copy's `CARGO_MANIFEST_DIR`,
causing seven fixture/source lookup failures. That attempt remains separately recorded.
The adapter now removes inherited Cargo target-directory overrides from the child environment
so cargo-mutants owns its build scratch. Parent environment values remain unchanged. The real
retry used a fresh private target, preserving both earlier failures instead of clearing them.
That actual retry used the frozen adapter from before the final automatic target-override
removal, with an equivalent fresh private target. The final environment default was checked
separately by the 42-test adapter suites; the latest adapter then independently reconciled
the retained native execution evidence.

The final Windows retry completed its actual baseline (about 92 seconds build and 268 seconds
tests), then caught **both** assigned `ReadAt::read_at` mutations. The native job and driver
exited zero, with no missed, unviable, timed-out or unrun mutations. All 608 frozen source files
remained unchanged; 159 bounded subprocess evidence files accompany the raw baseline/mutant
logs, diffs and outcomes. `windows-mutation/qualification.json` records the source and plan
identity, tool versions, explicit skips, earlier failures and final result.

The latest adapter independently reread the actual result in
`mutation-final-reconciliation/`. Given only that one native job's evidence, it correctly
qualified the two caught candidates and refused the full campaign: **94 planned jobs,
one qualified job, two caught candidates and 944 not run**. This validates the incomplete-run
gate with real artifacts; it does not claim a completed 946-candidate campaign. macOS native
execution and hosted workflow execution remain unperformed.

A separate trailer-only comparison matches all 27 canonical candidates and the exact trailer
source bytes across the original Linux and Windows runs: 25 caught and two unviable, with no
remaining applicable candidate. `mutation-native-trailer-union.json` records that limited
comparison while preserving the original Linux misses. It does not insert legacy evidence
from a different plan into the strict latest-plan gate.
