#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# This is executed only by the explicit publish=true workflow path.
set -euo pipefail
mode=${1:?expected check or publish}
tag=${2:?expected existing tag}
assets=${3:?expected verified distribution directory}
case "$mode" in check|publish) ;; *) echo "unknown mode: $mode" >&2; exit 2 ;; esac
case "$tag" in v[0-9]*) ;; *) echo "expected a version tag" >&2; exit 2 ;; esac
test -f "$assets/inventory.json"
test -f "$assets/SHA256SUMS"
draft=$(gh release view "$tag" --json isDraft --jq .isDraft)
test "$draft" = true || { echo "refusing to modify an existing published release: $tag" >&2; exit 1; }
if [ "$mode" = check ]; then exit 0; fi
(cd "$assets" && sha256sum --check SHA256SUMS)
gh release upload "$tag" "$assets"/* --clobber
verify=$(mktemp -d)
trap 'rm -f -- "$verify"/*; rmdir -- "$verify"' EXIT
gh release download "$tag" --dir "$verify"
cmp "$assets/SHA256SUMS" "$verify/SHA256SUMS"
cmp "$assets/inventory.json" "$verify/inventory.json"
diff <(cd "$assets" && find . -maxdepth 1 -type f -printf '%f\n' | LC_ALL=C sort) \
     <(cd "$verify" && find . -maxdepth 1 -type f -printf '%f\n' | LC_ALL=C sort)
(
  cd "$verify"
  sha256sum --check SHA256SUMS
  while read -r _ asset; do
    gh attestation verify "$asset" --repo "${GITHUB_REPOSITORY:?}"
  done < SHA256SUMS
)
# A concurrent publication must never be silently treated as this run's success.
draft=$(gh release view "$tag" --json isDraft --jq .isDraft)
test "$draft" = true || { echo "release changed while verifying: $tag" >&2; exit 1; }
gh release edit "$tag" --draft=false
