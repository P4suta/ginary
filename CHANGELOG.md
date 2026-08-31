<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this
project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

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

[Unreleased]: https://github.com/P4suta/ginary/commits/main
