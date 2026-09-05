<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this
project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0](https://github.com/P4suta/ginary/compare/v0.1.0...v0.2.0) (2026-09-05)


### Features

* **appfile,otp:** Erlang .app term parser, OTP root discovery, test builders (A1a) ([449e8c3](https://github.com/P4suta/ginary/commit/449e8c39ef9e167b8a4d1b2a4f2954e5f952b459))
* **assemble:** stage the runtime root with boot validation and file listing (A1c) ([a8c65b7](https://github.com/P4suta/ginary/commit/a8c65b7b49e5f7be285dd0ee1f7a038a9e562700))
* **build,inspect:** ginary build end-to-end and artifact inspection (A4) ([607bf8c](https://github.com/P4suta/ginary/commit/607bf8cf44772a336c75a401d99bdcc5583d4b7c))
* **catalog,otp-repack:** local-first OTP catalog and cross-Linux artifacts (C3) ([304025b](https://github.com/P4suta/ginary/commit/304025b2f747e4a4c145ae3638a7312179c6baf8))
* **closure:** application dependency closure over shipment and OTP lib (A1b) ([3604fb8](https://github.com/P4suta/ginary/commit/3604fb8b64e160f86545bb8e3e72fa160fb48bb5))
* **launcher:** self-extracting launcher with cache, exec contract, fault injection (A3b) ([572e93d](https://github.com/P4suta/ginary/commit/572e93d43d7598b830678ecc380abfb1fd839791))
* **macho,sign-macos:** Mach-O section payload container and ad-hoc signing (D3) ([5b35ecf](https://github.com/P4suta/ginary/commit/5b35ecfadde075a7321fc11b2bf28f4ec574fa81))
* **native:** NIF and native-code reconciliation across targets (C4) ([f02ca5f](https://github.com/P4suta/ginary/commit/f02ca5f158a6e01e9cd8d800a1446769310117be))
* **runtime-config,cache:** vm_args/sys_config/distribution/env/heart, flock and pruning (B1) ([62a0992](https://github.com/P4suta/ginary/commit/62a0992fec93979b46550319fe25295ec034f01b))
* scaffold ginary crate with version and doctor commands (A0) ([9dfc5ce](https://github.com/P4suta/ginary/commit/9dfc5cef149e7569a76cc235ff7b4bd9bf134d41))
* **strip,report:** ELF/BEAM stripping with verification and size report (A2) ([ec79070](https://github.com/P4suta/ginary/commit/ec79070504d0c9daa6ad112b8b621658504a07ec))
* **stub:** embedded identity marker, cli feature split, stub acquisition (C2) ([b33cddc](https://github.com/P4suta/ginary/commit/b33cddc1997066e5efb2f84fd68f14087a705643))
* **target,erts-source:** multi-target plumbing with honest provenance (C1) ([526a13d](https://github.com/P4suta/ginary/commit/526a13da802e3696237d0319914943ae50ebcddf))
* **trailer,manifest,payload,diag:** payload format v1 and event recorder (A3a) ([d73e24f](https://github.com/P4suta/ginary/commit/d73e24f3046bc14ef6a7ca011c1172969eac20be))
* **verify,doctor,sbom,crashdump,formal:** deep verification and developer tooling (B2) ([8730fe1](https://github.com/P4suta/ginary/commit/8730fe13146d43f23ba55589af3859a0a1a261d4))
* **windows:** cfg split, resident spawn launcher, windows stub (D2) ([380de43](https://github.com/P4suta/ginary/commit/380de43aac9ac96c7955f073bd944017b8ad20bb))


### Bug Fixes

* **cache:** route every removal path through the long-path helper (D2 follow-up) ([df225b4](https://github.com/P4suta/ginary/commit/df225b4bb7cfad27af645d9bce2d98ff2c8b3ddd))

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
