#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# The cross-Linux smoke matrix: build one artifact per target out of the local
# OTP catalog, then run each one inside a container that has no Erlang, no
# network and, for the aarch64 row, not even the host's instruction set.
#
# This is the claim C3 is about. `scripts/smoke.sh` proves a host artifact runs
# on a clean machine; this proves an artifact built *for another machine* runs
# on that machine, which is the whole reason a catalog exists.
#
#   linux-x86_64-musl:static    alpine:3.20   linux/amd64
#   linux-aarch64-musl:static   alpine:3.20   linux/arm64   (needs binfmt)
#   linux-x86_64-gnu:default    debian:1x     linux/amd64   (image from the
#                                                            entry's libc floor)
#
# Usage:
#   mise run smoke:matrix
#   scripts/smoke-matrix.sh
#
# Environment:
#   GINARY_BIN                a ginary to use instead of `cargo run`
#   GINARY_CATALOG            the catalog to build from (default dist/otp/catalog.json)
#   GINARY_STUB_DIR           where the cross-built stubs are (default target/stubs)
#   GINARY_REQUIRE_TOOLCHAIN  1 turns every reported skip into a failure, the
#                             same rule `tests/common/tools.rs` follows

set -uo pipefail

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
catalog=${GINARY_CATALOG:-$root/dist/otp/catalog.json}
stub_dir=${GINARY_STUB_DIR:-$root/target/stubs}
app=hello_ffi
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$root/Cargo.toml" | head -1)

failures=0
rows=()

pass() { rows+=("PASS  $1"); }
fail() { rows+=("FAIL  $1"); failures=$((failures + 1)); }

# A skip is a reported decision, never a default: `GINARY_REQUIRE_TOOLCHAIN=1`
# is what CI sets so that a machine that cannot run the matrix cannot look like
# a machine on which it passed.
skip() {
  if [ "${GINARY_REQUIRE_TOOLCHAIN:-}" = "1" ]; then
    echo "smoke-matrix: $1 and GINARY_REQUIRE_TOOLCHAIN=1" >&2
    exit 1
  fi
  echo "skipping: $1" >&2
  exit 0
}

for tool in docker gleam erl; do
  command -v "$tool" >/dev/null 2>&1 || skip "$tool is not on PATH"
done
docker info >/dev/null 2>&1 || skip "the docker daemon is unreachable"
[ -f "$catalog" ] || skip "there is no catalog at $catalog; run \`mise run otp:repack\`"
[ -d "$stub_dir" ] || skip "there are no stubs in $stub_dir; run \`mise run stubs:build\`"

# The probe comes before the install: `docker run --privileged` is not
# something to start on a machine that does not need it, and what matters is
# whether *this* daemon can run an arm64 image, not what this kernel has
# registered in /proc/sys/fs/binfmt_misc.
#
# The installer is pinned to a manifest digest, not to a tag. This script is
# run by ci.yml's `smoke-matrix` job, so a tag here would leave the same
# mutable image running with the same host kernel capabilities that the
# workflow's own pin exists to prevent. The digest names the multi-platform
# index and is `tonistiigi/binfmt:qemu-v10.2.3` as of 2026-09-04; re-resolve it
# with `docker buildx imagetools inspect tonistiigi/binfmt:<tag>` when it moves.
arm64=yes
if ! docker run --rm --platform linux/arm64 alpine:3.20 true >/dev/null 2>&1; then
  echo "smoke-matrix: linux/arm64 does not run here; installing a binfmt handler"
  if docker run --privileged --rm tonistiigi/binfmt@sha256:400a4873b838d1b89194d982c45e5fb3cda4593fbfd7e08a02e76b03b21166f0 --install arm64 >/dev/null 2>&1 \
     && docker run --rm --platform linux/arm64 alpine:3.20 true >/dev/null 2>&1; then
    echo "smoke-matrix: linux/arm64 is registered now"
  else
    echo "smoke-matrix: linux/arm64 could not be registered; the aarch64 row is skipped" >&2
    arm64=no
  fi
fi

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cp -R "$root/tests/fixtures/$app" "$work/$app"
rm -rf "$work/$app/build"

ginary() {
  if [ -n "${GINARY_BIN:-}" ]; then
    "$GINARY_BIN" "$@"
  else
    cargo run --quiet --manifest-path "$root/Cargo.toml" -- "$@"
  fi
}

# The glibc row's image is chosen from the catalog rather than assumed: debian
# 11 is glibc 2.31, and a runtime that needs more than that has to be run
# somewhere newer or the row would fail for a reason nobody could read.
glibc_min=$(sed -n 's/.*"min": "\([0-9.]*\)".*/\1/p' "$catalog" | sort -V | tail -1)
debian=debian:11
if [ -n "$glibc_min" ] && [ "$(printf '%s\n2.31\n' "$glibc_min" | sort -V | tail -1)" != "2.31" ]; then
  debian=debian:12
fi
echo "smoke-matrix: catalog glibc floor ${glibc_min:-none}, glibc row on $debian"

# target:variant, image, docker platform
matrix=(
  "linux-x86_64-musl alpine:3.20 linux/amd64"
  "linux-aarch64-musl alpine:3.20 linux/arm64"
  "linux-x86_64-gnu $debian linux/amd64"
)

for row in "${matrix[@]}"; do
  set -- $row
  target=$1; image=$2; platform=$3

  if [ "$platform" = "linux/arm64" ] && [ "$arm64" = "no" ]; then
    fail "$target: linux/arm64 is not registered with binfmt"
    continue
  fi
  stub="$stub_dir/ginary-stub-$version-$target"
  if [ ! -f "$stub" ]; then
    fail "$target: no stub at $stub; run \`mise run stubs:build\`"
    continue
  fi

  # A section of its own per target, appended to a copy of the fixture rather
  # than to the fixture itself.
  config="$work/$app/gleam.toml"
  cp "$root/tests/fixtures/$app/gleam.toml" "$config"
  printf '\n[tools.ginary.target."%s"]\nerts = "catalog"\n' "$target" >> "$config"

  # From inside the project: `ginary build` searches upward for a gleam.toml,
  # and the repository root is not a Gleam project.
  echo "smoke-matrix: building $app for $target"
  if ! (cd "$work/$app" && GINARY_CATALOG="$catalog" ginary build \
        --target "$target" --stub "$stub") > "$work/$target.build.log" 2>&1; then
    fail "$target: the build failed; see $work/$target.build.log"
    sed 's/^/    /' "$work/$target.build.log" >&2
    continue
  fi
  artifact="$work/$app/build/ginary/$app-$target"
  if [ ! -f "$artifact" ]; then
    fail "$target: the build wrote no $artifact"
    continue
  fi

  if ! ginary verify "$artifact" > "$work/$target.verify.log" 2>&1; then
    fail "$target: \`ginary verify\` found something; see $work/$target.verify.log"
    continue
  fi

  # `--network none` is not a convenience: an artifact that fetched a runtime
  # at run time would pass every other check in this file.
  output=$(docker run --rm --network none --platform "$platform" \
    -v "$artifact:/app:ro" "$image" /app 3 a b 2>&1)
  status=$?
  size=$(wc -c <"$artifact" | tr -d ' ')
  case $output in
    *"args=3 a b"*)
      if [ "$status" -eq 3 ]; then
        pass "$target on $image ($platform), $size bytes"
      else
        fail "$target on $image: exit $status, expected 3"
      fi
      ;;
    *) fail "$target on $image: $output" ;;
  esac
done

echo
echo "smoke-matrix results:"
for row in "${rows[@]}"; do
  echo "  $row"
done

if [ "$failures" -eq 0 ]; then
  echo "smoke-matrix: ${#rows[@]} rows, 0 failures"
else
  echo "smoke-matrix: $failures of ${#rows[@]} rows failed" >&2
  exit 1
fi
