#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# One version, everywhere it is written down — and one honest record of what has
# been released.
#
# ginary is version-locked to its stubs: every artifact of one release shares a
# version, so a launcher never reads a payload a different build wrote. The
# single source of that number is `Cargo.toml`, and a release tag has to match
# it or the assets a workflow uploads under `v0.1.0` would carry other internals.
# A release job runs this before it uploads anything.
#
# `.release-please-manifest.json` is the third record, and it is not a copy of
# `Cargo.toml`. release-please reads it as *the last released version* and
# derives the next proposal from it, so the two legitimately differ before the
# first release: the manifest records `0.0.0` — release-please's own spelling of
# "this package has never been released" — while `Cargo.toml` carries the version
# being prepared. From the first release onward release-please writes both files
# in one commit, so a tag is only consistent when all three agree.
#
# Usage:
#   scripts/ci/version-consistency.sh <tag>
#   GITHUB_REF_NAME=<tag> scripts/ci/version-consistency.sh
#
# The leading `v` is optional: `v0.1.0` and `0.1.0` both name the same release.
#
# `GINARY_VERSION_ROOT` points the check at a tree other than the one this
# script lives in. It exists for `tests/version_consistency.rs`, which drives
# the check over fixture trees in the release states this checkout is not in;
# a workflow leaves it unset and gets this repository. A workflow that sets it
# is a defect, not a knob: the gate would prove a tag matches some other
# directory's `Cargo.toml`, which is this check passing while not running. The
# suite asserts that no workflow mentions the name.
#
# Exit codes:
#   0  the tag, Cargo.toml and the manifest name one release
#   1  the three records disagree: the message names which two and how
#   2  no tag was given, or a record cannot be read

set -euo pipefail

manifest_file=".release-please-manifest.json"

# The manifest value release-please reads as "never released". Not a convention
# of ours: `Manifest.buildPullRequests` synthesises a previous release from the
# manifest entry only when it is not `0.0.0`, so this is the one spelling that
# leaves a package with no release to bump from. See docs/dev/log/E20.md.
nothing_released="0.0.0"

root="${GINARY_VERSION_ROOT:-$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)}"

tag="${1:-${GITHUB_REF_NAME:-}}"
if [ -z "$tag" ]; then
  echo "version-consistency: no tag given (pass one as an argument or set GITHUB_REF_NAME)" >&2
  exit 2
fi

# The leading v is optional; v0.1.0 and 0.1.0 both name the release 0.1.0.
tag_version="${tag#v}"

if [ ! -f "$root/Cargo.toml" ]; then
  echo "version-consistency: Cargo.toml is not in $root; it holds the single version every artifact of a release is locked to, and this check has nothing to compare the tag against" >&2
  exit 2
fi

# `-r` as well as `-f`, because a record that is there and cannot be opened is a
# different state from one that is gone, and `-f` answers true for it. Without
# this the guard's whole point is lost in the case it was added for: the read
# below fails and `sed`'s message, written in the runner's locale and naming
# neither the record nor what it is for, becomes the entire diagnostic.
if [ ! -r "$root/Cargo.toml" ]; then
  echo "version-consistency: Cargo.toml in $root cannot be read; check its permissions and the user this job runs as" >&2
  exit 2
fi

cargo_version=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$root/Cargo.toml" | head -1)
if [ -z "$cargo_version" ]; then
  echo "version-consistency: Cargo.toml carries no version = \"...\" line" >&2
  exit 2
fi

if [ ! -f "$root/$manifest_file" ]; then
  echo "version-consistency: $manifest_file is not in $root; release-please reads it as the last released version and this check reads it with it" >&2
  exit 2
fi

# The same `-r`, and here it matters more than a leaked message. The read below
# is wrapped in `|| true`, so a read that failed and a manifest with no `"."`
# entry both end in an empty `manifest_version` — and the second is reported.
# That tells a maintainer release-please has never seen this package, about a
# file the script never managed to open.
if [ ! -r "$root/$manifest_file" ]; then
  echo "version-consistency: $manifest_file in $root cannot be read; check its permissions and the user this job runs as" >&2
  exit 2
fi

# The root package's entry, whose key is `.`. No jq: this runs on every release
# runner, and one key of one flat object is not worth a dependency.
#
# The newlines go first. A manifest is JSON, not a line-oriented record, and
# `JSON.stringify` without an indent, `jq -c` and a `prettier` pass all write
# `{".": "1.2.3"}` on one line. A line-anchored read of that file finds no
# entry and reports a manifest release-please has never seen, which is the
# opposite of what the file says; see
# `tests/regressions/e20_a_compact_manifest_read_as_one_with_no_entry.rs`.
#
# `|| true` because `grep` exits 1 when it matches nothing and `pipefail` would
# make that the script's exit status, which would skip the message below and
# report a manifest with no entry as an unexplained failure.
manifest_version=$(tr -d '\n' < "$root/$manifest_file" \
  | grep -o '"\."[[:space:]]*:[[:space:]]*"[^"]*"' \
  | head -1 \
  | sed 's/.*"\([^"]*\)"$/\1/' || true)
if [ -z "$manifest_version" ]; then
  echo "version-consistency: $manifest_file carries no \".\" entry; release-please would read this package as one it has never seen" >&2
  exit 2
fi

if [ "$tag_version" != "$cargo_version" ]; then
  echo "version-consistency: tag ${tag} declares ${tag_version}, but Cargo.toml is ${cargo_version}; a release tag must match the package version" >&2
  exit 1
fi

if [ "$manifest_version" = "$nothing_released" ]; then
  echo "version-consistency: ${manifest_file} records ${manifest_version}, which is how release-please says nothing has been released; the release pull request records ${cargo_version} there before the tag ${tag} is cut" >&2
  exit 1
fi

if [ "$manifest_version" != "$cargo_version" ]; then
  echo "version-consistency: ${manifest_file} records ${manifest_version}, but Cargo.toml is ${cargo_version}; release-please writes both in one commit, so a disagreement is drift" >&2
  exit 1
fi

echo "version-consistency: tag ${tag} matches Cargo.toml ${cargo_version} and ${manifest_file} records ${manifest_version}"
