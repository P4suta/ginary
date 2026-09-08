#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Entirely local: gh is a readonly shell function, never an external program.
set -euo pipefail
publisher=$1
work=$2
scenario=$3
if command -v cygpath >/dev/null 2>&1; then
  publisher=$(cygpath -u "$publisher")
  work=$(cygpath -u "$work")
fi
assets="$work/assets"
mkdir -p "$assets" "$work/tmp"
export TMPDIR="$work/tmp"
export GITHUB_REPOSITORY=fixture/repository
printf 'fixture binary\n' > "$assets/ginary-fixture"
printf '{"assets":[]}\n' > "$assets/inventory.json"
(cd "$assets" && sha256sum ginary-fixture inventory.json > SHA256SUMS)
gh() {
  printf '%s\n' "$*" >> "$work/commands.log"
  case "$1:$2" in
    release:view)
      if [ "$scenario" = already-public ]; then echo false; else echo true; fi ;;
    release:upload) return 0 ;;
    release:download)
      cp "$assets"/* "$5/"
      if [ "$scenario" = extra ]; then printf 'unverified\n' > "$5/stale-asset"; fi ;;
    attestation:verify)
      if [ "$scenario" = fail ]; then return 1; fi ;;
    release:edit) printf 'published\n' > "$work/published" ;;
    *) echo "unexpected mock command: $*" >&2; return 97 ;;
  esac
}
readonly -f gh
# Sourcing keeps the readonly function in scope: no test can reach a real gh binary.
source "$publisher" publish v0.1.0 "$assets"
