#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# Mutate the lines this change touched, and nothing else.
#
# The whole crate is 920 candidates and hours of runners; a pull request is a
# change, and `cargo mutants --in-diff` keeps only the mutants in lines the
# change touched. The full pass is `mise run mutants` on a developer's machine,
# where it can take as long as it takes.
#
# Environment:
#   BASE     the commit to diff against: a pull request's merge base, or what a
#            push replaced. Absent or all-zero means there is no predecessor,
#            and the empty tree is what git calls that.
#   BUDGET   how many mutants this job will run before refusing (default 40)
set -euo pipefail

output=target/mutants-diff
change=target/change.diff
budget=${BUDGET:-40}

empty=$(git hash-object -t tree /dev/null)
base=${BASE:-}
case ${base:-$empty} in
  0000000000000000000000000000000000000000 | "") base=$empty ;;
esac

mkdir -p target
git diff "$base"...HEAD -- src > "$change"
if [ ! -s "$change" ]; then
  echo "no source change in this diff; there is nothing to mutate"
  exit 0
fi

# Counted before anything is built, so a change too large for this job is a
# named refusal rather than a job that runs out of time with nothing to show.
candidates=$(cargo mutants --list --in-diff "$change" --features fault-injection | wc -l | tr -d ' ')
echo "the diff touches $candidates mutants"
if [ "$candidates" -eq 0 ]; then
  echo "no mutable line in this diff; there is nothing to mutate"
  exit 0
fi
if [ "$candidates" -gt "$budget" ]; then
  # shellcheck disable=SC2016  # the backticks are markdown, not a substitution
  printf '::error::this change touches %s mutants, over the %s this job budgets; run `mise run mutants` locally and record what it found\n' "$candidates" "$budget"
  exit 1
fi

# cargo-mutants exits nonzero for a missed mutant *and* for a timeout, and the
# two are not the same fact -- nor are the two kinds of timeout. The verdict is
# `scripts/ci/mutation-verdict.py`, which says why.
#
# The build budget is a multiple of the baseline build cargo-mutants times in
# this same job: a mutant's build does no more work than the baseline's, so a
# multiple of it fits on any runner, where a number of seconds fits only the one
# it was measured on. The test budget stays a constant because what it bounds is
# a mutant that never terminates rather than a machine that is slow.
cargo mutants --in-diff "$change" --features fault-injection \
  --timeout 420 --build-timeout-multiplier 2 --output "$output" \
  --cargo-test-arg=-- --cargo-test-arg=--show-output || true
python3 scripts/ci/mutation-verdict.py --output "$output"
