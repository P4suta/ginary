#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# The line-coverage gate CI runs over an lcov report.
#
# `cargo llvm-cov --lcov` writes a report; this script sums its per-file line
# records (`LF:` total, `LH:` hit) and fails when the ratio falls below a floor.
# A report it cannot parse — one holding no `LF:` records at all — is a distinct
# error rather than a silent pass, because a gate that divided by zero and
# called the result 100% would be worse than no gate.
#
# Usage:
#   scripts/ci/coverage-gate.sh <lcov.info> <min-percent>
#
# Exit codes:
#   0  coverage is at or above the floor
#   1  coverage is below the floor
#   2  the report is missing or holds no line records

set -euo pipefail

lcov="${1:?usage: coverage-gate.sh <lcov.info> <min-percent>}"
min="${2:?usage: coverage-gate.sh <lcov.info> <min-percent>}"

if [ ! -f "$lcov" ]; then
  echo "coverage-gate: $lcov: no such coverage report" >&2
  exit 2
fi

read -r lh lf < <(awk -F: '
  /^LH:/ { lh += $2 }
  /^LF:/ { lf += $2 }
  END    { print lh + 0, lf + 0 }
' "$lcov")

if [ "$lf" -eq 0 ]; then
  echo "coverage-gate: $lcov holds no line records (no LF: entries); refusing to report the coverage of nothing" >&2
  exit 2
fi

pct=$(awk -v lh="$lh" -v lf="$lf" 'BEGIN { printf "%.2f", (lh * 100) / lf }')
printf '%s%% (%d/%d) lines\n' "$pct" "$lh" "$lf"

# Compare the raw ratio, not the 2-decimal display value: a true 89.996% renders
# as "90.00" and would clear a 90 floor if the rounded number were the one
# tested. The printf above is for humans; the gate reads the exact ratio.
if awk -v lh="$lh" -v lf="$lf" -v m="$min" 'BEGIN { exit !((lh * 100) / lf < m) }'; then
  echo "coverage-gate: line coverage ${pct}% is below the ${min}% floor" >&2
  exit 1
fi

echo "coverage-gate: ${pct}% clears the ${min}% floor"
