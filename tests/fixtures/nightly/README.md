<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# What the nightly mutation campaign cost, while there was one

Nothing here is read by a test any more. It is kept because
[`docs/dev/log/F1-assurance.md`](../../../docs/dev/log/F1-assurance.md) and
[`E21.md`](../../../docs/dev/log/E21.md) cite it as the evidence their claims rest on, and a log
that points at a deleted file is a log nobody can check.

`mutants-F1-counts.json` is the F1 discovery snapshot: 946 candidates across the seven modules the
campaign sharded, with the source and discovery hashes each count was read from. It claims no
baseline and no executed mutation.

## Why there is nothing else here

The campaign this directory sized is gone. It cut the crate into 89 canonical shards over 106
native jobs and ran every night; sizing it needed two records — how many mutants each module
produces, and what one costs — and keeping those true needed them re-measured whenever the source
moved. They were not, which is how `src/launch.rs` grew past `8 x 13` candidates with every test
still green and took a whole night's campaign down before it mutated anything.

**Mutation is a pull-request check over the diff now.** `scripts/ci/mutation-diff.sh` mutates the
lines a change touched and nothing else, so its cost is the size of the change rather than the size
of the crate, and there is no budget to keep true. The whole-crate pass is `mise run mutants` on a
developer's machine, where it can take as long as it takes.

`mutants-measured.json` — the per-mutant cost and per-runner baselines — and
`mutants-F1-integration-counts.json` — the post-E23 enumeration the shard cap was held against —
were deleted with it. Both are in the history, at `refs` before this directory was emptied, if the
campaign is ever rebuilt.
