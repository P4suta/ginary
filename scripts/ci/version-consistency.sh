#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# One version, everywhere it is written down.
#
# ginary is version-locked to its stubs: every artifact of one release shares a
# version, so a launcher never reads a payload a different build wrote. The
# single source of that number is `Cargo.toml`, and a release tag has to match
# it or the assets a workflow uploads under `v0.1.0` would carry other internals.
# A release job runs this before it uploads anything.
#
# Usage:
#   scripts/ci/version-consistency.sh <tag>
#   GITHUB_REF_NAME=<tag> scripts/ci/version-consistency.sh
#
# The leading `v` is optional: `v0.1.0` and `0.1.0` both name the same release.
#
# Exit codes:
#   0  the tag matches the Cargo.toml version
#   1  the tag disagrees with the Cargo.toml version
#   2  no tag was given, or Cargo.toml carries no version

set -euo pipefail

root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)

tag="${1:-${GITHUB_REF_NAME:-}}"
if [ -z "$tag" ]; then
  echo "version-consistency: no tag given (pass one as an argument or set GITHUB_REF_NAME)" >&2
  exit 2
fi

# The leading v is optional; v0.1.0 and 0.1.0 both name the release 0.1.0.
tag_version="${tag#v}"

cargo_version=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$root/Cargo.toml" | head -1)
if [ -z "$cargo_version" ]; then
  echo "version-consistency: Cargo.toml carries no version = \"...\" line" >&2
  exit 2
fi

if [ "$tag_version" != "$cargo_version" ]; then
  echo "version-consistency: tag ${tag} declares ${tag_version}, but Cargo.toml is ${cargo_version}; a release tag must match the package version" >&2
  exit 1
fi

echo "version-consistency: tag ${tag} matches Cargo.toml ${cargo_version}"
