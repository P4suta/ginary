<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this
project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2026-09-02

The first release. ginary turns a Gleam application into one self-contained executable that runs
on a machine with no Erlang installation. Five phases, A through E, built it:

- **Phase A — the build pipeline and the launcher.** `ginary build` reads a Gleam project, runs
  `gleam export erlang-shipment`, resolves the application closure against an OTP installation,
  stages and strips a trimmed runtime, packs a deterministic tar + zstd payload, and appends it
  with a 64-byte trailer to a copy of the ginary binary. The same binary is the launcher: it
  reads the trailer at its own tail, extracts the runtime into a per-user cache atomically, and
  `execve`s the BEAM, mapping every failure to a numbered exit code.
- **Phase B — reading and assuring an artifact.** `ginary verify`, `ginary sbom` and
  `ginary crashdump` read a packaged application from the outside; the cache gained per-runtime
  locking and age pruning that never removes an entry in use; and the cache protocol is modelled
  in TLA+ under `formal/`.
- **Phase C — cross-target builds.** Version-locked stubs for every target, the local-first OTP
  catalog (`ginary otp`), and native-code reconciliation for the NIFs and port programs a
  shipment carries.
- **Phase D — Windows and macOS.** The resident Windows launcher, and the Mach-O section payload
  with ad-hoc signing for macOS.
- **Phase E — the verification matrix.** The CI job matrix, the release-please and distribute
  workflows, the coverage and version-consistency gates, and the v1 readiness sweep.

### Added

- Every ginary binary carries a 128-byte identity marker naming the version it was built by, the
  target it runs on, the payload format it reads and whether it holds the command line half.
  `docs/format.md` specifies it.
- The `cli` Cargo feature, on by default. `cargo build --no-default-features` produces a
  launcher-only *stub* — no clap, no TOML reader and none of the build-side modules — which is
  what a cross-target artifact is made of. Run on its own it prints what it is and which target
  it is for. `mise run lint:stub` and `mise run test:stub` hold that build to the same gate as
  the full one, and both are in `check`.
- `ginary build --stub PATH`, and a search for one: `$GINARY_STUB_DIR/ginary-stub-<version>-
  <target>`, then `ginary-<version>-<target>`, then the running executable when the target is the
  host, then `<cache>/stubs/<version>/<target>`. Stubs are version-locked, and a file has to pass
  seven gates — size, exactly one marker, version, payload format, target, an object header that
  agrees with the marker, and no trailer of its own — before a payload is appended to it. A
  target with no stub is refused with every path that was tried.
- `mise run stubs:build` cross-builds the launcher-only stubs into `target/stubs`. It attempts
  the four Linux targets and `windows-x86_64`; the four Linux ones build, the Windows one does
  not compile yet because the launcher path is Unix-only, and the task reports it and exits
  non-zero rather than dropping it. macOS stubs cannot be built on Linux and come from the
  release build.

### Changed

- A build for a target other than the host is no longer refused outright. What it needs now is a
  stub, which can be built, and a runtime, which still has to be named:
  `[tools.ginary.target.<name>] erts = "dir:..."`. A cross target with no runtime named for it is
  refused before the project is exported, quoting the table to write.

- `ginary build`: a Gleam project in, one executable out. It reads `[tools.ginary]` from
  `gleam.toml`, runs `gleam export erlang-shipment`, resolves the application closure against
  the host OTP installation, stages a trimmed runtime, strips it, packs a deterministic
  tar + zstd payload and appends it with a 64-byte trailer to a copy of the ginary binary. The
  artifact is written through a temporary file in the output directory and renamed into place at
  mode 0755. Flags: `--out`, `--no-strip`, `--strip-elf-only`, `--strip-beams-only`,
  `--otp-root`, `--skip-export`, `--keep-staging`, `--compression-level`, `--extra-otp-app`,
  `--extra-bin`, `--report text|json`, `--explain` and `-v`. A packaged application handed to
  `build` as a stub is refused: a bundled executable cannot build.
- `ginary inspect <exe>`: the manifest, the versions, the geometry and the ten largest files of
  a packaged application, with `--verify` (re-hash the payload against the trailer, exit 1 on a
  mismatch), `--launch-plan` (the argv and environment the launcher would use, against a
  placeholder root) and `--json`.
- `[tools.ginary]` in `gleam.toml`: `output`, `strip`, `strip_elf`, `strip_beams`,
  `compression_level`, `otp_applications`, `erts_extra_bins` and `erl_flags`. Unknown keys are
  refused by name, and `erl_flags` may not repeat a flag the launcher builds itself.
- `scripts/smoke.sh` and `mise run smoke`: the artifact is run in `ubuntu:24.04` with
  `--network none` and no Erlang installed, its exit code is checked through the container
  boundary, and a `--read-only` run proves the cache falls back to the tmpfs. Wired into CI as a
  required job.
- `mise run package` packages the `hello_ffi` fixture and prints its size report.
- Project scaffolding: crate layout, licences, REUSE metadata, mise tasks, CI and the
  contributor documentation.
- `ginary version`, with `--json` reporting `version`, `target` and `format_version`.
- `ginary doctor`, with `--json`, reporting the host target, the resolved cache directory, and
  whether `gleam`, `erl`, `strip` and `docker` are present and which version they report. Each
  probe is bounded by a ten-second timeout.
- The `Target` model (`<os>-<arch>[-<libc>]`) used by manifests and artifact names.
- Cache directory resolution: `GINARY_CACHE_DIR`, then `XDG_CACHE_HOME/ginary`, then
  `HOME/.cache/ginary`.

### Changed

- The payload's compressor is flushed after `ginary.json` and `ginary.index.json`, so the two
  entries every reader takes on their own decode without the rest of the stream. `ginary inspect`
  can therefore still say what a damaged artifact was supposed to be.

[Unreleased]: https://github.com/P4suta/ginary/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/P4suta/ginary/releases/tag/v0.1.0
