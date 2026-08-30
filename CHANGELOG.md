<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this
project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Project scaffolding: crate layout, licences, REUSE metadata, mise tasks, CI and the
  contributor documentation.
- `ginary version`, with `--json` reporting `version`, `target` and `format_version`.
- `ginary doctor`, with `--json`, reporting the host target, the resolved cache directory, and
  whether `gleam`, `erl`, `strip` and `docker` are present and which version they report. Each
  probe is bounded by a ten-second timeout.
- The `Target` model (`<os>-<arch>[-<libc>]`) used by manifests and artifact names.
- Cache directory resolution: `GINARY_CACHE_DIR`, then `XDG_CACHE_HOME/ginary`, then
  `HOME/.cache/ginary`.

[Unreleased]: https://github.com/P4suta/ginary/commits/main
