#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# The line/branch coverage gate CI runs over an lcov report.
#
# `cargo llvm-cov --lcov` writes a report; this script sums its per-file line
# records (`LF:` total, `LH:` hit) and fails when the ratio falls below a floor.
# A report it cannot parse — one holding no `LF:` records at all — is a distinct
# error rather than a silent pass, because a gate that divided by zero and
# called the result 100% would be worse than no gate.
#
# Usage:
#   scripts/ci/coverage-gate.sh <lcov.info> <min-percent> [lines|branches]
#
# Exit codes:
#   0  coverage is at or above the floor
#   1  coverage is below the floor
#   2  invalid arguments, malformed/truncated report, or no selected records

set -euo pipefail

lcov="${1:?usage: coverage-gate.sh <lcov.info> <min-percent>}"
min="${2:?usage: coverage-gate.sh <lcov.info> <min-percent>}"
metric="${3:-lines}"
if ! awk -v minimum="$min" 'BEGIN {
  exit !(minimum ~ /^[0-9]+([.][0-9]+)?$/ && minimum + 0 <= 100)
}'; then
  echo "coverage-gate: invalid minimum $min; expected a percentage from 0 to 100" >&2
  exit 2
fi
case "$metric" in
  lines) found=LF; hit=LH ;;
  branches) found=BRF; hit=BRH ;;
  *) echo "coverage-gate: unknown metric $metric" >&2; exit 2 ;;
esac

if [ ! -f "$lcov" ]; then
  echo "coverage-gate: $lcov: no such coverage report" >&2
  exit 2
fi

summary=$(awk -F: -v found="$found" -v hit="$hit" '
  function refuse(message) {
    print "coverage-gate: invalid coverage records: " message > "/dev/stderr"
    failed = 1
    exit 2
  }
  { sub(/\r$/, "", $0) }
  $1 == "SF" {
    if (opened) refuse("missing end_of_record before next source")
    opened = 1
    found_seen = hit_seen = source_found = source_hit = 0
    next
  }
  $1 == found || $1 == hit {
    if (!opened || NF != 2 || $2 !~ /^[0-9]+$/)
      refuse("non-integer or misplaced " $1 " counter")
    if ($1 == found) {
      if (found_seen++) refuse("duplicate " found " counter")
      source_found = $2 + 0
    } else {
      if (hit_seen++) refuse("duplicate " hit " counter")
      source_hit = $2 + 0
    }
    next
  }
  $0 == "end_of_record" {
    if (!opened) refuse("end_of_record without a source")
    if (found_seen != hit_seen) refuse("unpaired total/hit counters")
    if (source_hit > source_found) refuse("hits exceed the total")
    lh += source_hit
    lf += source_found
    opened = 0
  }
  END {
    if (failed) exit 2
    if (opened) refuse("truncated source record")
    printf "%.0f %.0f\n", lh + 0, lf + 0
  }
' "$lcov") || exit 2
read -r lh lf <<< "$summary"

if [ "$lf" -eq 0 ]; then
  echo "coverage-gate: $lcov holds no $metric records (no $found: entries); refusing to report the coverage of nothing" >&2
  exit 2
fi

pct=$(awk -v lh="$lh" -v lf="$lf" 'BEGIN { printf "%.2f", (lh * 100) / lf }')
printf '%s%% (%d/%d) %s\n' "$pct" "$lh" "$lf" "$metric"

# Compare the raw ratio, not the 2-decimal display value: a true 89.996% renders
# as "90.00" and would clear a 90 floor if the rounded number were the one
# tested. The printf above is for humans; the gate reads the exact ratio.
if awk -v lh="$lh" -v lf="$lf" -v m="$min" 'BEGIN { exit !((lh * 100) / lf < m) }'; then
  echo "coverage-gate: $metric coverage ${pct}% is below the ${min}% floor" >&2
  exit 1
fi

echo "coverage-gate: ${pct}% clears the ${min}% floor"
