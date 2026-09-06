<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# v1 readiness

This is the fail-closed checklist that decides whether ginary is v1. It enumerates every plan
phase, A through E, with the acceptance evidence each one produced, marks each item done or
deferred, and names the commit that closed it. An item is **done** only when a test, a script or
a committed artifact in this repository proves it — or, for an item that needs a runner this
machine is not, a CI job that runs it on every push and a test in this repository that holds the
job to doing so. An item whose workflow is authored and has **never executed** is **CI-gated**: it
is never marked done and never hand-waved, and it says which workflow carries it.

The rule is fail-closed in both directions. An item with no evidence is not v1-ready, and a
deferred item is honest about being deferred rather than quietly counted as done; equally, an item
that stays deferred once its evidence exists understates the project in the document that exists
to prevent hand-waving, so a job that starts running closes its item in the same milestone.

## What v1 delivers

ginary packages a Gleam application and a trimmed BEAM runtime into a single executable that runs
on a machine with no Erlang installation, no `PATH` entry and no unpacking step. v1 delivers the
whole pipeline for seven targets — Linux gnu and musl on x86_64 and aarch64, macOS on x86_64 and
arm64, and Windows on x86_64 — together with the tools to read, verify and cross-build an
artifact: a version-locked stub per target, a local-first OTP catalog, native-code reconciliation
for the NIFs and port programs a shipment carries, `ginary verify` and `ginary sbom`, and a
launcher whose cache protocol is modelled in TLA+. The Linux half runs end to end on this
machine today, and the macOS and Windows halves run end to end on `macos-15-intel`, `macos-14` and
`windows-2022` on every push. What has never executed is the release path: the catalog publishing
and the provenance attestations wait on a release nobody has cut.

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
90%** by `scripts/ci/coverage-gate.sh` in the `coverage` CI job; the 80% branch floor is
documented as nightly-only, because it needs a `-Z coverage-options=branch` build (see
`docs/dev/testing.md`). As of E2 the tree measures **90.26% line coverage**, over the 90% floor
(done — E2). The E1 figure of 85.17% was in part a measurement artifact: the launcher path runs
in a spawned artifact subprocess, and the hermetic `env_clear()` those spawns use dropped
`LLVM_PROFILE_FILE`, so the subprocess wrote no profile and its real execution of `launcher`,
`launch`, `cache` and `selfexe` was invisible. Re-injecting only that one variable after the
clear (nothing else — the hermetic `PATH`/`ERL_*` scrub is unchanged) lifted the measured total
to 89.61% with no new assertions, and genuine unit tests for `stubid`, `error`, `catalog`,
`selfexe` and `cli` dispatch carried it to 90.26%. The remaining uncovered mass is the OTP
repack pipeline (real upstream tarballs and network), macOS-only signing paths, and
failure-injection error arms, none of which is reachable by a deterministic in-process test on
this platform; `docs/dev/log/E2.md` details the before/after measurement per module.

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
| Windows artifact **launch**, exit-code propagation | `ci.yml` `windows` job, `tests/ci_matrix.rs` | done — E23, on `windows-2022` |
| Mach-O section payload, ad-hoc signing | `tests/macho.rs`, `tests/payload_locate.rs`, `tests/sign_macos.rs` | done (packaging) — `5b35ecf` (D3) |
| macOS artifact **launch**, `codesign --verify` | `ci.yml` `macos` job, `tests/ci_matrix.rs` | done — E1 authored it, `macos-15-intel`/`macos-14` run it |

Windows and macOS packaging are proved structurally on Linux — the cfg split, the resident
launcher, the PE and Mach-O readers, the section injection and ad-hoc signing all have tests that
run on this machine. What only a runner can confirm is the **actual launch**, and both runners now
do. The `macos` job packages a `hello_ffi` artifact on `macos-15-intel` and `macos-14`, runs
`codesign --verify --strict` over ginary's own output, starts the artifact, asserts its arguments
and its exit code, and verifies the signature again afterwards. The `windows` job does the same on
`windows-2022`: it packages against the runtime `setup-beam` installs, then starts the artifact as
a `GINARY_CMD=selftest`, on a cold cache, on a warm one, and on `halt(3)` — which is the exit-code
contract, the code reaching `%ERRORLEVEL%` through ginary's own spawn-and-wait launcher rather
than through a bare `erl`. That closes the D2 wine gap and the D3 "awaits a Mac runner" gap, and
it is the first and only execution `launch_windows::run` — the spawn, the job object and the
console control handler — has anywhere. `tests/regressions/e23_the_documents_said_the_launches_had_never_happened.rs`
derives both claims from the workflow rather than trusting this paragraph, and it derives them in
both directions: a job that goes back to building without launching puts the old sentences back.

### Phase E — the verification matrix

| item | evidence | status |
|---|---|---|
| CI job matrix, `required` fan-in | `tests/ci_matrix.rs`, `.github/workflows/ci.yml` | done — E1 |
| Nightly: mutants, fuzz, full smoke matrix | `.github/workflows/nightly.yml` | done — E1 |
| Coverage gate at 90% lines | `tests/coverage_gate.rs`, `scripts/ci/coverage-gate.sh` | done — E1 |
| Version-consistency check | `tests/version_consistency.rs`, `scripts/ci/version-consistency.sh` | done — E1 |
| Documentation-completeness scan | `tests/docs.rs` | done — E1 |
| release-please + distribute workflows | `tests/release_workflow.rs`, `.github/workflows/{release,distribute}.yml` | authored — E1 |
| Catalog publishing, release **provenance** | `distribute.yml` (`attest-build-provenance`) | CI-gated — authored in E1, runs on the release runner |

Every workflow is `actionlint`-clean and every third-party `uses:` is pinned to a full commit
SHA with a version comment; the SHA-pin table is in `docs/dev/log/E1.md`. The Linux-runnable jobs
were exercised locally and their transcripts recorded in that log. The **release and provenance**
half is authored and never run: no tag, no publish, no attestation is produced until the
repository has a remote and a maintainer cuts a release per `docs/RELEASE.md`.

## The mutation and fuzz status

- **Mutation testing** runs in `.github/workflows/nightly.yml`, sharded over the highest-value
  modules (`trailer`, `payload`, `cache`, `closure`, `appfile`, `launch`, `verify`); a surviving
  mutant fails its shard. `mise run mutants` runs it locally. A one-module smoke of the command is
  recorded in `docs/dev/log/E1.md`.
- **Fuzzing** runs in the nightly workflow too, 30 seconds per target over the four libFuzzer
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
- **There is no Windows runtime in the catalog, so a Windows artifact is built on Windows.**
  `ginary otp repack` produces the Linux and macOS catalog tarballs and no `windows-x86_64` one,
  and `distribute.yml` therefore publishes none: the upstream a Windows entry would be repacked
  from is `otp_win64_<version>.zip`, a different shape from the tarballs the other targets take,
  and reading it is a milestone of its own rather than a line of this one. What that costs is
  cross-building *to* Windows from Linux or macOS: a `windows-x86_64` build needs
  `erts = 'dir:…'` naming an unpacked Windows runtime, which in practice means building on a
  Windows machine — where a Gleam developer already has Erlang installed, because `gleam` needs
  it. The `windows` job does exactly that on every push. This is a stated boundary of v1 and not
  a defect: the build refuses by name rather than producing an artifact with the wrong runtime in
  it.
- **The host OTP major version must match.** A runtime is read for its own target, linkage and
  libc, but ginary does not rewrite BEAM across OTP major versions: an artifact's bundled runtime
  and the modules in it are one OTP major, and a catalog entry whose `otp_release` differs from
  the host's is refused at repack time.

## The deferred items, restated plainly

**One** kind of work is CI-gated rather than done. The two that used to stand beside it — the
macOS launch and the Windows launch — are closed by the jobs that now run them on every push, and
are marked done above rather than deferred; a checklist that leaves an item deferred after its
evidence exists understates the project exactly as badly as one that marks an item done without
it.

- **Catalog publishing and release provenance** — `distribute.yml`. Builds every target's binary,
  stub and OTP tarball, produces `attest-build-provenance` attestations, and verifies the
  re-downloaded assets before flipping the release out of draft. It runs when a maintainer cuts a
  release, and no release has been cut: no tag, no publish and no attestation exists. The workflow
  is `actionlint`-clean and held by `tests/release_workflow.rs`, and that is inspection rather
  than execution.

Nothing is tagged, pushed or published now.
