<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# v1 readiness

This is the fail-closed checklist that decides whether ginary is v1. It enumerates every plan
phase, A through F, with the acceptance evidence each one produced, marks each item done or
deferred, and names the commit that closed it. An item is **done** only when a test, a script or
a committed artifact in this repository proves it — or, for work no machine here can run, when a
hosted run this document names by number did. An item that still needs a runner nobody has
pointed at it is **CI-gated**: the workflow is authored and committed, and it closes the day a
run of it can be read. A CI-gated item is never marked done and never hand-waved; it says which
workflow carries it and in which commit.

The rule is fail-closed: an item with no evidence is not v1-ready, and a deferred item is honest
about being deferred rather than quietly counted as done.

## What v1 delivers

ginary packages a Gleam application and a trimmed BEAM runtime into a single executable that runs
on a machine with no Erlang installation, no `PATH` entry and no unpacking step. v1 delivers the
whole pipeline for seven targets — Linux gnu and musl on x86_64 and aarch64, macOS on x86_64 and
arm64, and Windows on x86_64 — together with the tools to read, verify and cross-build an
artifact: a version-locked stub per target, a local-first OTP catalog, native-code reconciliation
for the NIFs and port programs a shipment carries, `ginary verify` and `ginary sbom`, and a
launcher whose cache protocol is modelled in TLA+. The Linux half runs end to end on this
machine today, and the macOS and Windows launches have each been run on a hosted runner of their
own; the catalog publishing and the release provenance are authored as CI jobs and run when a
maintainer cuts a release.

## The evidence, by phase

### Phase A — the build pipeline and the launcher

| item | evidence | status |
|---|---|---|
| Crate scaffold, `version`, `doctor` | `tests/smoke_cli.rs` | done — `9dfc5ce` (A0) |
| `.app` parser, OTP discovery | `tests/appfile.rs`, `tests/otp.rs` | done — `449e8c3` (A1a) |
| Application dependency closure | `tests/closure.rs` | done — `3604fb8` (A1b) |
| Staging root assembly | `tests/assemble.rs`, `tests/stage_run.rs` | done — `a8c65b7` (A1c) |
| ELF/BEAM strip and size report | `tests/strip.rs`, `tests/report.rs` | done — `ec79070` (A2) |
| Payload format v1, trailer, manifest | `tests/payload.rs`, `tests/trailer.rs`, `tests/manifest.rs` | done — `d73e24f` (A3a) |
| Self-extracting launcher, exec contract | `tests/launcher.rs`, `tests/launch.rs`, `tests/cache.rs` | done — `572e93d` (A3b) |
| `ginary build` end to end, `inspect` | `tests/e2e_hello.rs`, `tests/bundle.rs`, `tests/inspect.rs` | done — `607bf8c` (A4) |

The end-to-end proof is `tests/e2e_hello.rs`: `ginary build` in a copy of the `hello_ffi`
fixture, then the artifact run on a machine with the environment scrubbed of Erlang, the warm
cache, and byte-identical rebuilds under `SOURCE_DATE_EPOCH`. The **host artifact for
`hello_ffi` is 5.4 MB**. `scripts/smoke.sh` (`mise run smoke`) runs that artifact inside
`ubuntu:24.04` with `--network none` — a machine that genuinely has no Erlang — and checks that
it runs, that its exit code crosses the container boundary, and that the cache falls back to a
tmpfs under a read-only root.

### Phase B — reading and assuring an artifact

| item | evidence | status |
|---|---|---|
| `ginary verify`: index, objects, portability | `tests/verify.rs` | done — `8730fe1` (B2) |
| `ginary sbom` (SPDX 2.3), `crashdump` | `tests/sbom.rs`, `tests/crashdump.rs` | done — `8730fe1` (B2) |
| Cache locking and age pruning | `tests/cache_lock.rs`, `tests/cache.rs`, `tests/launcher.rs` | done — `62a0992` (B1) |
| TLA+ model of the cache protocol | `formal/Cache.tla`, `tests/formal.rs` | done — `62a0992` / `8730fe1` |

The **TLA+ model** (`formal/Cache.tla`) models the extraction, locking and pruning protocol; its
configuration names four invariants `I1`–`I4`, and `mise run formal` runs TLC over it with
deadlock checking on. TLC found two violations during B, both in the model rather than the code,
and both were corrected; the model now checks clean. `tests/formal.rs` holds the model, its
configuration and `docs/dev/formal.md` against the tree so none rots. **Line coverage is gated at
90% and branch coverage at 80%**, both by `scripts/ci/coverage-gate.sh` and both required on a
pull request: the `coverage` job runs twice, as `Coverage (lines)` and `Coverage (branches)`, and
only the second needs the nightly compiler's `-Z coverage-options=branch` (see
`docs/dev/testing.md`). The 80% branch floor was documented here as nightly-only until F1; it is
a standing job, and the toolchain it needs is not the schedule it runs on. The latest hosted
measurement is run [34281075949](https://github.com/P4suta/ginary/actions/runs/34281075949):
**90.75% lines** (18,152 / 20,002) and **83.80% branches** (2,457 / 2,932), each over its floor.
The two line denominators belong to their own instrumentation runs and are not merged.

The floor was reached in E2, and the history is worth keeping because it is a measurement lesson.
The E1 figure of 85.17% was in part a measurement artifact: the launcher path runs in a spawned
artifact subprocess, and the hermetic `env_clear()` those spawns use dropped `LLVM_PROFILE_FILE`, so
the subprocess wrote no profile and its real execution of `launcher`, `launch`, `cache` and
`selfexe` was invisible. Re-injecting only that one variable after the clear (nothing else — the
hermetic `PATH`/`ERL_*` scrub is unchanged) lifted the measured total to 89.61% with no new
assertions, and genuine unit tests for `stubid`, `error`, `catalog`, `selfexe` and `cli` dispatch
carried it to 90.26%. The remaining uncovered mass is the OTP repack pipeline (real upstream
tarballs and network), macOS-only signing paths, and failure-injection error arms, none of which is
reachable by a deterministic in-process test on this platform; `docs/dev/log/E2.md` details the
before/after measurement per module.

### Phase C — cross-target builds

| item | evidence | status |
|---|---|---|
| Version-locked stubs, `cli` feature split | `tests/stub.rs`, `tests/stubid.rs`, `tests/stub_flavor.rs` | done — `b33cddc` (C2) |
| Multi-target plumbing, honest provenance | `tests/target.rs`, `tests/erts_source.rs` | done — `526a13d` (C1) |
| Local-first OTP catalog, `ginary otp` | `tests/catalog.rs`, `tests/otp_cli.rs`, `tests/otp_repack.rs` | done — `304025b` (C3) |
| Cross-Linux artifacts run in a container | `tests/e2e_cross.rs`, `scripts/smoke-matrix.sh` | done — `304025b` (C3) |
| Native-code reconciliation (NIFs, ports) | `tests/native.rs`, `tests/e2e_native.rs` | done — `f02ca5f` (C4) |

`tests/e2e_cross.rs` cross-builds `hello_ffi` out of the committed catalog for
`linux-x86_64-musl`, `linux-aarch64-musl` and `linux-x86_64-gnu` and runs each in a container
with no Erlang and no network — the aarch64 row behind a binfmt probe, the gnu row on the oldest
Debian its catalog entry allows. `scripts/smoke-matrix.sh` (`mise run smoke:matrix`) is the same
matrix as a script. The cross artifact sizes are the acceptance evidence the plan fixed:
**`linux-x86_64-musl` 6.3 MB, `linux-aarch64-musl` 6.6 MB, `linux-x86_64-gnu` 4.9 MB**. A real
application confirms the shape at scale: the **`notify` shipment packages to 12.2 MB**.

### Phase D — Windows and macOS

| item | evidence | status |
|---|---|---|
| Windows cfg split, resident launcher, stub | `tests/windows.rs`, `tests/windows_build.rs` | done (packaging) — `380de43` (D2) |
| Windows artifact **launch**, the launcher's half | `tests/launcher.rs` on a Windows host, `tests/regressions/e23_*` | done — E23 |
| Windows artifact **launch** with a real `erl.exe` | `ci.yml` `windows` job, run [34023412195](https://github.com/P4suta/ginary/actions/runs/34023412195) | done — E23, on `windows-2022` |
| Mach-O section payload, ad-hoc signing | `tests/macho.rs`, `tests/payload_locate.rs`, `tests/sign_macos.rs` | done (packaging) — `5b35ecf` (D3) |
| macOS artifact **launch**, `codesign --verify` | `ci.yml` `macos` job, run [34281075949](https://github.com/P4suta/ginary/actions/runs/34281075949) | done — F1, on `macos-15-intel` and `macos-14` |

macOS packaging is proved structurally on Linux — the cfg split, the Mach-O reader, the section
injection and ad-hoc signing all have tests that run there. What only a runner could confirm was the
**actual launch**, and until F1 no Mach-O ginary produced had ever been executed or had `codesign
--verify --strict` run against it. **It has now.** Run
[34281075949](https://github.com/P4suta/ginary/actions/runs/34281075949) ran the `macos` job on both
images: each built its own darwin stub natively, packaged the `hello_ffi` fixture against the
runner's own ERTS, ran the artifact and checked its arguments and exit status, and `codesign`
reported the artifact valid and satisfying its Designated Requirement *before and after* the launch
— the second check being the one that proves extracting a payload did not rewrite the file it came
out of. That closes the D3 "awaits a Mac runner" gap.

What the `macos` job does not claim is the rest of the suite. It is a build, launch and signature
qualification on two images; the full test configurations, the coverage measurements and the
mutation campaign run elsewhere, and the [Phase F](#phase-f--product-completeness) table says
where.

**Windows reached the same place one milestone earlier.** E23 was the first milestone worked on a
Windows host, and the launcher's own suite runs there now: `tests/launcher.rs` was `#![cfg(unix)]`
in its entirety until then, because the fixture's `erlexec` was a `#!/bin/sh` script and Windows
starts no such thing. It stages a real program instead, and 59 of the file's claims — the cache, the
argument vector, the environment difference, the five numbered exit codes, the `GINARY_CMD`
commands, the trace, the lock, pruning and uninstalling — run natively and pass, with five gated
`#[cfg(unix)]` for reasons each one states. `a_killed_launcher_takes_its_runtime_with_it` is the
first test of the job object that `docs/adr/0015-windows-launcher-stays-resident.md` argues the
crate's only `#[allow(unsafe_code)]` for.

Two halves remain, and both are named rather than absorbed:

- **A real `erl.exe`.** The development host has no Erlang, so what the suite proves is the
  launcher spawning, waiting for and mirroring a *stub* runtime. That `halt(3)` inside a real
  packaged application reaches `%ERRORLEVEL%` is the `windows` CI job's, and until E23 that job
  did not do it — it ran `erl.exe` directly and started no artifact. It now packages the
  `hello_ffi` fixture, runs the artifact cold and warm, and compares the code.
- **A console control event.** `SetConsoleCtrlHandler` is called on every Windows launch, so the
  handler is installed; no Ctrl-C has ever been delivered to a launcher holding one. Driving one
  needs `GenerateConsoleCtrlEvent`, a Win32 call with no safe counterpart, and `CLAUDE.md`
  requires an ADR for any new `#[allow(unsafe_code)]` — the test tree included. E23 declined it
  on that ground rather than faking it.

### Phase E — the verification matrix

| item | evidence | status |
|---|---|---|
| CI job matrix, `required` fan-in | `tests/ci_matrix.rs`, `.github/workflows/ci.yml` | done — E1 |
| Nightly: mutants, fuzz, full smoke matrix | `.github/workflows/nightly.yml` | done — E1 |
| Coverage gate: 90% lines, 80% branches | `tests/coverage_gate.rs`, `scripts/ci/coverage-gate.sh` | done — E1, both required since F1 |
| Version-consistency check | `tests/version_consistency.rs`, `scripts/ci/version-consistency.sh` | done — E1 |
| Documentation-completeness scan | `tests/docs.rs` | done — E1 |
| release-please + distribute workflows | `tests/release_workflow.rs`, `.github/workflows/{release,distribute}.yml` | authored — E1 |
| Catalog publishing, release **provenance** | `distribute.yml` (`attest-build-provenance`) | CI-gated — authored in E1, runs on the release runner |

Every workflow is `actionlint`-clean and every third-party `uses:` is pinned to a full commit
SHA with a version comment; the SHA-pin table is in `docs/dev/log/E1.md`. The Linux-runnable jobs
were exercised locally and their transcripts recorded in that log. The **release and provenance**
half is authored and never run: no tag, no publish, no attestation is produced until a
maintainer cuts a release per `docs/RELEASE.md`.

### Phase F — product completeness

| item | evidence | status |
|---|---|---|
| Atomic artifact, manifest and SBOM publication | `tests/regressions/f1_build_outputs_were_not_isolated.rs`, `tests/assurance_paths.rs` | done — `d82107f` (F1) |
| Multi-target builds keep their partial results | `tests/regressions/f1_multi_target_cli_lost_results.rs` | done — `d82107f` (F1) |
| Cache maintenance never discards live work | `tests/regressions/f1_cache_maintenance_discarded_live_work.rs` | done — `d82107f` (F1) |
| `selftest` uses the launcher's environment and lock | `tests/regressions/f1_selftest_did_not_use_the_launchers_environment_or_lock.rs` | done — `d82107f` (F1) |
| Verification reports `passed`/`failed`/`not_run`/`incomplete` | `tests/regressions/f1_verify_accepted_inconsistent_destinations.rs`, `tests/diagnostic_acceptance.rs` | done — `d82107f` (F1) |
| Target paths survive a foreign host's parser | `tests/regressions/f1_foreign_payload_paths_used_the_hosts_separators.rs` | done — `d82107f` (F1) |
| Bounded process evidence, trace v2 | `tests/regressions/f1_process_timeouts_lost_their_output.rs`, `tests/regressions/f1_trace_could_not_be_shared_or_correlated.rs` | done — `d82107f` (F1) |
| An independent verifier for the signed Mach-O | `tests/regressions/f1_macos_signature_verification_was_only_a_test_helper.rs` | done — `d82107f` (F1) |
| `distribute.yml` runs the event SHA, never a moved tag | `tests/regressions/f1_distribution_executed_a_tag_in_the_default_branch_context.rs` | done — `d82107f` (F1) |
| Native Windows build, launch and exit codes | `ci.yml` `windows` job, run [34281075949](https://github.com/P4suta/ginary/actions/runs/34281075949) | done — F1 |
| Native macOS build, launch and signature | `ci.yml` `macos` job, run [34281075949](https://github.com/P4suta/ginary/actions/runs/34281075949) | done — F1 |
| Every mutation assigned to a runner that can run it | `tools/mutation-plan`, `scripts/ci/mutation.py`, `nightly.yml` | planned — F1; **the campaign itself is red, below** |

F1 is the milestone that stopped treating a packaged artifact as the end of the pipeline and
started treating the *user's* directory as part of it: an export that fails half-way leaves the
old bytes, a build of four targets reports the three that finished and the one that did not,
and a cache sweep that cannot attribute an entry keeps it and says why. Its subsystem records
are `docs/dev/log/F1*.md`, and the hosted qualification is `docs/dev/log/F1-integration.md`.

Run [34281075949](https://github.com/P4suta/ginary/actions/runs/34281075949) is the run all three
rows above cite. All 19 jobs passed, including `Required CI`, both coverage measurements, the
Linux plain, stub and fault suites, cross-Linux smoke, MSRV 1.88 and the formal model — which
generated 31,939 states, found 7,860 distinct ones and reached depth 29.

## The mutation and fuzz status

- **Mutation testing** runs in `.github/workflows/nightly.yml`, sharded over the highest-value
  modules (`trailer`, `payload`, `cache`, `closure`, `appfile`, `launch`, `verify`); a surviving
  mutant fails its shard. F1 replaced one flat list with a plan that assigns each candidate to a
  runner whose `cfg` can actually compile it — 95 native jobs — and `mise run mutants` runs it
  locally.

  **The campaign has been red since 2026-09-06, and every one of its three failures now has an
  answer.** Run [34747498271](https://github.com/P4suta/ginary/actions/runs/34747498271)
  reconciles to `caught 733, unviable 81, missed 101, timeout 30, not_run 15`, with 56 of the 95
  shards failing. They are different things and are not counted as one:

  - **`missed 101`** was a test-suite gap. 87 are killed and 14 are gone with the redundancy they
    lived in; [F1-mutation-clusters.md](log/F1-mutation-clusters.md) accounts for each, and there
    is still no `mutants.toml` and no `#[mutants::skip]` anywhere in `src/`.
  - **`timeout 30`** was two things. 28 were Windows and macOS shards whose *build* phase hit
    `--build-timeout 120` — a constant above every Linux baseline build and below theirs, so those
    shards measured nothing at all; the budget is a multiple of the baseline the job itself times
    now. The other 2 are Linux mutants that stop a loop advancing, where the suite not terminating
    is the only detection there can be; those are counted as kills, as mutation testing counts
    them.
  - **`not_run 15`** was neither a gap nor a budget. 13 are one job whose runner GitHub shut down
    eighteen minutes in, which the harness reported as `interrupted` and failed on. 2 are a macOS
    baseline that failed on an ELF-magic assertion against a Mach-O, fixed when the suite was
    first qualified on a macOS host.

  A green campaign is a nightly run away rather than a piece of work away, and is claimed here
  when a run says so. Nothing else in the nightly workflow is failing — fuzz, the formal model and
  the cross-Linux smoke matrix are green in the same run.
- **Fuzzing** runs in the nightly workflow too, 600 seconds per target over the four libFuzzer
  targets (`trailer_parse`, `appfile_terms`, `beam_chunks`, `payload_read_manifest`), seeded from
  the committed corpus. `mise run fuzz` runs it locally.

## The full test count

The full suite is run by `mise run test` (the full command line tool, `--features
fault-injection`) and `mise run test:stub` (the launcher-only flavor). The exact pass count of
the whole suite at the E1 commit is recorded in `docs/dev/log/E1.md` under `## GREEN`.

## Known limitations

These are the honest caveats consolidated from across the phase logs. None is a defect; each is a
property of the technique, stated here so a user meets it in documentation rather than in a
failure.

- **The static musl runtime cannot load a NIF.** The default variant for a musl target is the
  fully static build, which needs no dynamic loader and therefore runs on any Linux — and for the
  same reason cannot `dlopen` a `priv/lib/*.so`. An application with a NIF needs the dynamic musl
  variant (`otp_variant = "dynamic"`) or a `linux-*-gnu` target; the artifact's manifest records
  `nif_loading` either way.
- **The gnu variant has a glibc floor of 2.36.** A dynamically linked gnu artifact needs the C
  library of the machine it was built against, or newer; the `needs:` line every build prints
  states the exact floor. An artifact built on Ubuntu 24.04 will not start on a host older than
  its floor, and ginary says so at build time rather than leaving it to the user's loader.
- **No hot-code upgrade.** `releases/` is not shipped and `release_handler` is not available; a
  packaged application is a single immutable runtime, not an upgradable release.
- **Distribution requires a node name in config.** A distributed runtime (`distribution = true`)
  with no `-name` or `-sname` in `erl_flags` or the args file is a runtime nothing can reach; the
  build warns, because the launcher cannot invent a node name.
- **Ad-hoc macOS signing satisfies the kernel, not Gatekeeper.** The ad-hoc signature ginary
  applies satisfies the arm64 kernel's load-time requirement that every mapped page be signed. It
  does **not** satisfy Gatekeeper on a file downloaded from the network: a quarantined
  ad-hoc-signed binary still prompts the user. Clearing that needs a real Developer ID signature,
  which is out of scope for v1.
- **The host OTP major version must match.** A runtime is read for its own target, linkage and
  libc, but ginary does not rewrite BEAM across OTP major versions: an artifact's bundled runtime
  and the modules in it are one OTP major, and a catalog entry whose `otp_release` differs from
  the host's is refused at repack time.

## The deferred items, restated plainly

What is listed here is what has **not** happened. An item leaves this list the day something in
the repository proves it, and takes its evidence to the table above; a bullet that has to explain
it is no longer deferred is a bullet in the wrong section.

- **A console control event reaching the Windows launcher** — nothing, yet. Declined in E23 with
  a reason: delivering one needs `GenerateConsoleCtrlEvent`, and a new `#[allow(unsafe_code)]`
  needs an ADR of its own. This is the one mechanism of
  `docs/adr/0015-windows-launcher-stays-resident.md` still resting on argument rather than on a
  run.
- **Catalog publishing and release provenance** — `distribute.yml`. Builds every target's binary,
  stub and OTP tarball, produces `attest-build-provenance` attestations, and verifies the
  re-downloaded assets before flipping the release out of draft. Runs when a maintainer cuts a
  release. Its consequence for the catalog is visible today: `dist/otp/catalog.json` carries the
  three Linux entries this machine repacked, and the other four targets of `target::ALL` have no
  published runtime, so a cross build for them finds none and says so.
- **A green mutation campaign** — `nightly.yml`. The plan covers every candidate and the shards
  run; what has not happened is a reconciliation with no survivor, no timeout and no unrun
  mutation. The numbers, and what each of the three failures is, are under
  [the mutation and fuzz status](#the-mutation-and-fuzz-status).

Nothing above is tagged, pushed or published now. The release and distribution workflows are
`actionlint`-clean and have never been run: they wait on a maintainer cutting a release, not on
a remote, which exists.
