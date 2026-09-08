<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# F1 — a long workflow script filled actionlint's pipe before its child started

The real-toolchain Windows fault run stalled in
`e1_the_sha256sums_step_read_and_wrote_one_file::actionlint_accepts_every_workflow`.
The existing test used `Command::output` without a deadline. The owned actionlint process
remained alive for minutes with almost no CPU use and no child processes. The root task
terminated that verified owned process so the full run could record its failure and continue.

The regression now uses `common::bounded::run_bounded` with 60 seconds per workflow. A timeout
retains stdout, stderr, elapsed time, process cleanup status and, when configured through
`GINARY_TEST_EVIDENCE_DIR`, exact output files and JSON metadata. Its actionlint requirement
and successful-exit assertion remain in force.

## Isolating the failure

The installed executable was actionlint 1.7.12, built with Go 1.26.1 for Windows/amd64, at
`C:\Users\livec\AppData\Local\mise\installs\actionlint\1.7.12\actionlint.exe`.
The installed ShellCheck was 0.11.0 at
`C:\Users\livec\AppData\Local\mise\installs\shellcheck\0.11.0\shellcheck.exe`.

Supported escalated probes used owned process handles, explicit deadlines and cleanup:

| Probe | Result |
|---|---|
| actionlint `-version` | exit 0, 51 ms |
| actionlint on the complete `ci.yml` | timeout at 40 seconds, empty output, killed and reaped |
| actionlint debug mode | parsed the workflow and visited all 11 jobs in milliseconds, then waited for ShellCheck |
| internal lint without optional child linters, for diagnosis only | exit 0, 37 ms, zero findings |
| explicit config file, for diagnosis only | exit 0, 37 ms |
| closed stdin; `GOMAXPROCS=1` | both still timed out at 15 seconds |
| ShellCheck `--version` | exit 0, 51 ms |
| ShellCheck with actionlint's exact stdin flags and a short script | exit 0, 44 ms, `[]` |
| actionlint on a harmless workflow with a 3500-character Bash comment | exit 0, 175 ms |
| the same workflow with a 5000-character comment | timeout at 5 seconds, killed and reaped |
| ShellCheck directly on the actual 4859-byte CI Bash script | exit 0, 103 ms |

The temporary diagnostic commands that disabled optional integrations isolated the cause;
they were not used to satisfy the lint gate. Every final acceptance check retains ShellCheck.
Raw probe reports, fixtures and streams are under `.cache/assurance/F1/actionlint-*`.

The upstream implementation explains the result: actionlint 1.7.12 creates a stdin pipe and
writes the entire script before starting the child with `Output`/`CombinedOutput`. Once the
Windows pipe fills, that write cannot finish because no ShellCheck reader has started. See
the versioned official [process implementation](https://raw.githubusercontent.com/rhysd/actionlint/v1.7.12/process.go).
The observed short/long boundary and absence of a child establish this failure mechanism;
the problem is unrelated to workflow YAML parsing or the script's validity.

The root task authorized extracting the long macOS Bash block into a checked-in helper and
checking that helper directly with ShellCheck. The workflow agent made that change, and the
workflow remains subject to complete actionlint validation. The regression and CI lint job
both invoke ShellCheck on `scripts/ci/macos-smoke.sh` by file path.

The first complete lint after extraction finished all seven workflow checks in 137–321 ms,
instead of stalling. Six workflows passed. The now-observable CI check reported two SC2086
findings for unquoted optional flag variables; the direct helper check reported SC2016 for
a log predicate whose single quotes prevented its new environment variable from expanding.
These were handed to the workflow agent for correction, preserving argument boundaries and
the intended predicate rather than suppressing either lint rule.

## Complete lint acceptance

After those corrections, the bounded real-tool driver exited 0. All seven committed
workflows passed actionlint with its ShellCheck integration enabled: CI in 426 ms, and the
other workflows in 184–290 ms. Direct ShellCheck on the extracted macOS helper passed in
102 ms, with no rule exclusions. Every child exited and was reaped, and both output streams
were captured completely. Final reports and exact streams are saved under
`.cache/assurance/F1/actionlint-final-workflow-*` and
`.cache/assurance/F1/actionlint-final-macos-helper.*`.

This is real Windows execution of the linter tools against every workflow and the actual
helper. It does not execute the macOS smoke script or claim a native macOS runtime result.

No Cargo build or test was run by this agent during the investigation. No release, upload,
tag or push was performed, and no lint rule or coverage threshold was lowered.
