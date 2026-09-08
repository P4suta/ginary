<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# F1 explicit diagnosis and local evidence collection

The original doctor conflated absent, broken, hanging and unrecognized version commands. Its
ELF table silently omitted malformed files and depth-limited or unreadable shipment paths.
The detailed API adds evidence without changing the existing public Report and ToolReport
fields or legacy Report::gather format version.

## Interfaces and behavior

- `doctor::DetailedReport::gather()` produces format version 2 with the original report fields,
  `tool_probes`, and actionable `findings`. Every probe records a stable outcome, status,
  elapsed time, reason, remedy, bounded stdout/stderr, omitted-byte counts and completeness.
- `doctor::probe_version(name, path, args, timeout, parser)` provides an explicit environment
  seam for library consumers and tests. Parsing only accepts successful, complete UTF-8
  output. Missing, spawn failure, timeout, nonzero exit, invalid output, incomplete output
  and wait failure remain distinct. Each stream retains at most 64 KiB. Child reaping and
  cleanup errors survive alongside the primary outcome, including after a timeout.
- Gleam, Erlang, strip and Docker version parsers require recognizable responses. A successful
  program printing an unrelated greeting cannot establish tool availability.
- Shipment native notes report malformed headers, read/stat errors, oversized files, skipped
  non-files/symlinks and the depth limit. Detailed findings also distinguish a configured
  runtime source from a runtime that has actually been read and validated.
- `diagnose::gather(artifact, trace, crashdump)` collects actual known-tool environment checks
  through detailed doctor and keeps only sanitized readiness outcomes and remedies. It
  reads supplied evidence locally, without executing or extracting the supplied artifact.
  `diagnose::summarize` is the evidence-only, read-only seam used by isolated incident tests.
- Diagnose JSON and text contain explicit verifier stages, available counts, trace counts and
  crash-dump heap/process statistics. An unperformed or incomplete contents scan has null
  finding/file counts instead of reporting zero problems. Raw arguments, environment values,
  file paths, trace values and dump terms are omitted. Run IDs are represented by SHA-256
  fingerprints for correlation. Doctor itself retains local paths and bounded tool output;
  the sanitization described here is the additional local incident-summary step.
- Trace schema 1 and 2 are supported. Invalid/unsupported lines remain counted. Inspection is
  bounded to 8 MiB, 64 KiB per line, and 1024 schema-2 runs; exceeding a bound means incomplete.
  Crash-dump collection examines at most a 16 MiB prefix and states truncation explicitly.
- The root task owns CLI integration, the required new output directory, staged publication
  of report.json and summary.txt, and shared schema/documentation updates.

## TDD and verification evidence

All Cargo commands prepend `C:\Users\livec\.cargo\bin` to PATH and use `--locked --offline`.

1. Temporary API scaffolding allowed real assertion RED rather than a missing-symbol compile
   error. `cargo test --locked --offline --test regressions f1_diagnosis_lost_failure_evidence
   -- --nocapture` failed two assertions: a native gleam fixture exiting 7 had no classified
   outcome, and mixed v1/v2 trace evidence had no event/run/failure summary.
2. After implementation, both cases passed. The trace fixture embeds a secret in raw argv and
   token fields and proves it appears in neither collected JSON nor rendered text.
3. `cargo test --locked --offline --test regressions f1_doctor_reports_a_ -- --nocapture`
   failed both assertions before native scan fixes: malformed ELF without declared targets,
   and a shipment nested beyond depth 12, both had empty native_notes. All four diagnosis
   cases then passed after reporting those limitations.
4. `cargo test --locked --offline --test regressions f1_doctor_does_not_accept -- --nocapture`
   failed when a gleam-named program printed `hello stranger`: it was reported available with
   version `stranger`. The parser now classifies this as invalid output.
5. The nine-case diagnosis regression module passed on Windows, exercising real native
   subprocesses for nonzero exit, timeout with retained output, excessive output, spawn
   failure and invalid parsing, plus malformed/deep scans, trace redaction, oversized lines,
   crash-dump term omission, and refusal to execute a supplied non-artifact program.
6. To tolerate loaded Windows runners, the timeout fixture now prints then sleeps 10 seconds,
   with a 2-second probe budget; successful probes use 5 seconds. Tests for full trace-byte
   bounds and inspecting a packaged runtime without extraction were added for the final
   coordinated validation. Their results are recorded by the root suite.
7. A recorder/reader integration RED caught the final trace schema using `start` where an
   earlier API draft had used `begin`: `f1_diagnose_accepts_events` counted only one of two
   real recorder events. The reader now accepts `start`, and the launcher lifecycle regression
   asserts the same vocabulary. No valid current-recorder event is discarded.
8. Earlier coordinated GREEN, before the final review additions: `cargo test --locked --offline --features fault-injection --test
   cache --test doctor --test launcher --test smoke_cli --test regressions -- --nocapture`
   exited 0: all 550 compiled regression cases, all 36 doctor cases and all 6 smoke CLI cases
   reported success. Tool-gated tests state their missing tools; the Windows host does not
   execute the Unix-only launcher integration file. The diagnosis regression module's twelve
   cases execute without requiring a real Gleam/Erlang installation, including full-byte
   trace limits, current recorder compatibility and packaged-runtime non-execution.

No release, upload, tag or push was performed. Unix-only behavior remains for Unix CI to
execute; this local session provides Windows behavioral evidence.

## Final independent review

The same regression module now covers requested JSON output for build preflight failure,
including protected output aliases, missing projects and invalid project TOML. Each failure
must produce a version-2 failed report and a trace failure outcome. It also covers a relative
trace path surviving a process working-directory change, adjacent URL credentials remaining
redacted, and Windows trace initialization during a brief competing writer lock. The root
task owns the associated CLI/recorder fixes and their final consolidated validation evidence.

`docs/dev/debugging.md` and `docs/dev/testing.md` now document local diagnosis, trace schema
1/2 reading, sensitive-value opt-in, build/cache-clean/doctor/verify JSON migrations, legacy
library wrappers, and all output/signing fault points. The supplied artifact is never run
by diagnosis; the environment portion probes only the known developer tools. Local reports
are created only in a newly reserved output directory, protecting existing report directories.

## Cache probe data preservation

The final review reproduced an actual Windows data-loss bug without a new Cargo build.
The bounded `.cache/assurance/F1/doctor-collision-probe.ps1` copied the existing debug CLI
into an isolated test directory, delayed its known Gleam fixture, and planted the predictable
`.ginary-doctor-probe-<pid>-0.cmd` path before the cache probe. Doctor PID 17952 exited 0 and
reported writable/executable, but deleted the planted user file. The final preservation
assertion failed. Evidence is retained in
`.cache/assurance/F1/doctor-collision-25ec29f6408141d69ea348d280f83ea1/evidence.json`.

The probe now creates a randomized temporary executable exclusively, writes through its
owned handle, closes that handle before execution, and cleans up only its owned temporary
path. Preexisting files and hard links are preserved. Execution has the existing ten-second
probe budget and bounded child cleanup; the short ETXTBSY retry remains. A deletion refusal
retains the actual capability answer plus the exact owned path and error in `cache_probe.detail`.
Detailed doctor additionally reports `cache_probe_cleanup_failed` and its remedy.

The regression launches an isolated test process for each preexisting-file and hard-link
case, then exercises the real cache executable. A Windows unit holds an actual file handle
that denies deletion after a successful probe and asserts the cleanup evidence. Another
injects execution refusal and confirms cleanup preserves the user's note. The root owns
the consolidated Cargo validation; no competing Cargo invocation was started here.

GREEN on the same actual Windows CLI reproduction: after the root rebuilt the fixed CLI,
the bounded driver copied it into a fresh evidence directory and exited 0. Doctor PID 13368
also exited 0, preserved the preexisting file and its exact bytes, reported writable and
executable with no cleanup detail, and left precisely the one preexisting cache file.
The executed copy's SHA-256 was
`31389c1a2af7bb94a8c76c9be19642e8795bd43b27d68fd189de6eb6ba04a838`.
The complete doctor report and compact acceptance assertions are retained at
`.cache/assurance/F1/doctor-collision-2040ea27086648f7a881f5400ede390c/evidence.json`
and `verification.json`. The shared build executable was only copied and was never held
open for the reproduction. No Cargo command or production edit was needed for this check.

## Focused evidence after the final review

- `.cache/assurance/F1/diagnosis-review-green.log`: 17 diagnosis/recorder regression tests
  passed, including real native failure/timeout probes, Windows trace contention, bounded
  evidence, secret omission, build failure JSON and artifact non-execution.
- `.cache/assurance/F1/diagnosis-stages-red.log`: two of 11 verification/diagnosis assertions
  failed because unknown contents were represented as zero findings or as a completed scan.
  `diagnosis-stages-green.log` then passed all 11 after preserving explicit stage outcomes.
- `.cache/assurance/F1/review-final-units-driver.log`: 214 unit tests passed in that build,
  including the actual Windows deletion-denying handle and cleanup-evidence assertion.
  This is a historical focused build count, not the final total after later strip/cache work.
- The standalone doctor collision evidence above supplies the exact copied CLI hash and
  successful native reproduction separately from any combined suite's aggregate outcome.

`diagnose.complete` describes collection of the requested incident evidence. Environment
health is reported by its own tool outcomes and finding codes; a completed collection does
not claim that the machine is ready to build. Supplied artifacts are never executed, but
known-tool and cache executable probes do run when gathering the environment. Probe budgets
and collection limits are deliberate: a timed-out or truncated observation remains visible
and is not accepted as a successful version or complete incident scan.

All local behavioral evidence here is Windows-native. Unix-only launch, permission and link
cases await their supported runner; the final full feature configurations and coverage have
their own records and must not be inferred from these focused successes.
