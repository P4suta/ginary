#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# Clean-room smoke test: run a packaged Gleam application inside a container
# that has no Erlang, no network and, in the third check, no writable
# filesystem at all.
#
# Every other test in this repository runs on a machine that has `gleam` and
# `erl` installed, and scrubs the environment to pretend otherwise. This one
# does not have to pretend: `ubuntu:24.04` genuinely has no Erlang, and the
# first check asserts that before it runs the artifact. What it proves is the
# claim the whole project is about — copy one file to a machine and run it.
#
#   1. no Erlang            `! command -v erl` and then the application runs
#   2. exit codes           the application's own status reaches the caller
#   3. a read-only rootfs   the cache falls back to ${TMPDIR}/ginary-<uid>
#
# Usage:
#   scripts/smoke.sh
#
# Environment:
#   GINARY_BIN                a ginary to use instead of `cargo run`
#   GINARY_IMAGE              the image to run in (default ubuntu:24.04)
#   GINARY_REQUIRE_TOOLCHAIN  1 makes an unreachable docker daemon a failure
#                             rather than a reported skip, the same rule
#                             `tests/common/tools.rs` follows

set -euo pipefail

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
image=${GINARY_IMAGE:-ubuntu:24.04}
app=hello_ffi

failures=0

pass() { printf 'PASS  %s\n' "$1"; }
fail() { printf 'FAIL  %s\n' "$1"; failures=$((failures + 1)); }

if ! docker info >/dev/null 2>&1; then
  if [ "${GINARY_REQUIRE_TOOLCHAIN:-}" = "1" ]; then
    echo "smoke: the docker daemon is unreachable and GINARY_REQUIRE_TOOLCHAIN=1" >&2
    exit 1
  fi
  echo "skipping: the docker daemon is unreachable" >&2
  exit 0
fi

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# A copy, so that a smoke run never writes into the repository's fixture and
# never inherits another run's `build/`.
cp -R "$root/tests/fixtures/$app" "$work/$app"
rm -rf "$work/$app/build"

echo "smoke: packaging $app"
if [ -n "${GINARY_BIN:-}" ]; then
  (cd "$work/$app" && "$GINARY_BIN" build)
else
  (cd "$work/$app" && cargo run --quiet --manifest-path "$root/Cargo.toml" -- build)
fi

artifact="$work/$app/build/ginary/$app"
if [ ! -x "$artifact" ]; then
  echo "smoke: the build produced no artifact at $artifact" >&2
  exit 1
fi
printf 'smoke: artifact %s bytes\n' "$(wc -c <"$artifact" | tr -d ' ')"

# `--network none` is not a convenience: an artifact that phoned home for a
# runtime would still pass every other check in this file. The three runs are
# written out rather than wrapped in a helper, because the flags before the
# image and the command after it are not interchangeable and a helper that
# blurred the two is what the first draft of this file got wrong.

# 1 - the machine has no Erlang, and the application runs anyway.
if output=$(docker run --rm --network none -e HOME=/tmp \
  -v "$artifact:/app:ro" "$image" \
  sh -c '! command -v erl >/dev/null && /app 0 x y' 2>&1); then
  case $output in
    *"args=0 x y"*"hello from priv"*) pass "no Erlang: the application ran and read its priv" ;;
    *) fail "no Erlang: the application ran and printed something else: $output" ;;
  esac
else
  fail "no Erlang: the application did not run: $output"
fi

# 2 - the application's own exit code reaches the caller through execve.
if output=$(docker run --rm --network none -e HOME=/tmp \
  -v "$artifact:/app:ro" "$image" \
  sh -c '/app 7; test $? = 7' 2>&1); then
  pass "exit codes: 7 propagated out of the container"
else
  fail "exit codes: 7 did not propagate: $output"
fi

# 3 - a read-only root filesystem. `HOME` is on it, so the cache cannot be
# created there and has to fall back to ${TMPDIR:-/tmp}/ginary-<uid>, which is
# the one writable mount. The tmpfs is mounted `exec` on purpose: docker's
# default is `noexec`, and a runtime cache nobody may exec from is the one
# failure the launcher already reports with a hint of its own.
if output=$(docker run --rm --network none --read-only --tmpfs /tmp:rw,exec -e HOME=/root \
  -v "$artifact:/app:ro" "$image" \
  /app 0 read only 2>&1); then
  case $output in
    *"args=0 read only"*) pass "read-only rootfs: the cache fell back to the tmpfs" ;;
    *) fail "read-only rootfs: the application printed something else: $output" ;;
  esac
else
  fail "read-only rootfs: the application did not run: $output"
fi

if [ "$failures" -eq 0 ]; then
  echo "smoke: 3 checks, 0 failures"
else
  echo "smoke: $failures of 3 checks failed" >&2
  exit 1
fi
