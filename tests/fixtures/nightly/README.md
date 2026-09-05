<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# What the nightly assurance run cost

`mutants-measured.json` is the record a mutation budget can be argued from: how
many mutants each sharded module produces, and how long one costs. It is
*measured*, not estimated, and it names the run it was measured from so a
reader can check it.

Nightly run
[33969332537](https://github.com/P4suta/ginary/actions/runs/33969332537) is the
first in this project's history in which every mutation shard printed
`ok Unmutated baseline`, so it is the first that reports a mutant count per
module at all. One shard — `trailer` — also ran to completion inside the
90-minute cap:

```text
28 mutants tested in 50m: 8 missed, 18 caught, 2 unviable
```

50 minutes over 28 mutants is 107 seconds each. **That average is not what
`seconds_per_mutant` records**, and the difference matters: `trailer` is the
smallest and fastest module in the crate, and a figure read off it alone prices
every other module at the speed of the one that had the least to compile. The
same run's own per-mutant lines read about 210 seconds — roughly 39 seconds of
build and 171 of test — and 210 is the number in the file, because a budget
argued from the optimistic reading is a budget that keeps passing right up to
the run in which a shard is cancelled again. `baseline_minutes` is what every
shard took to reach `ok Unmutated baseline` — under four minutes, each of them.

The two figures are worth keeping apart when this file is next revised. 107 is
a measured completion; 210 is a measured cost per mutant on the shards that did
not complete. Until a *large* shard finishes, 210 is the honest one, and
re-measuring from the first nightly in which one does is the way to replace it
with something better than a conservative reading.

The other six shards never printed a total: five were `cancelled` at
`timeout-minutes: 90` and `appfile` died on a runner shutdown. Their entries in
`modules` are the mutant counts they *announced*, which is what a shard prints
before it starts testing, and those are exact.

`tests/ci_matrix.rs` reads this file to answer one question: can the pass the
nightly workflow configures actually finish inside the budget it is given? A
gate that cannot finish is not a gate. Re-measure and update the file when the
suite's runtime changes materially; the workflow points at it by name so that
the two move together.
