<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# F1 — the other two thirds of a red campaign

[F1-mutation-clusters.md](F1-mutation-clusters.md) closes `missed 101`, which is one of the three
numbers run [34747498271](https://github.com/P4suta/ginary/actions/runs/34747498271) reconciles to:
`caught 733, unviable 81, missed 101, timeout 30, not_run 15`, with 56 of 95 shards failing. This
is the other two, and neither turned out to be a gap in the tests.

## `timeout 30`, and why twenty-eight shards measured nothing

The shape was the clue. Eleven Windows shards reported `caught=0, missed=0, timeout=N` — a shard
that caught *nothing* is not a shard with weak tests, it is a shard that never ran any. Each
shard's own `mutants.out/outcomes.json` says where the time went, and every one of those timeouts
is in the **Build** phase and lands on exactly `120.0` seconds:

```
mutants-cache-9-windows   Success  Baseline   [('Build', 192.6), ('Test', 226.5)]
                          Timeout  is_occupied      [('Build', 120.1)]
                          Timeout  is_occupied      [('Build', 120.0)]
                          Timeout  is_refusal       [('Build', 120.0)]
                          Timeout  is_refusal       [('Build', 120.0)]
                          Timeout  is_win32_error   [('Build', 120.0)]
```

`120.0` is `build_timeout_seconds`. The baselines across the matrix, from the same run:

| runner | baseline build | baseline test |
|---|---|---|
| linux | 76–83 s | 214–235 s |
| windows | 193–204 s | 221–237 s |
| macos | 252 s | — (its baseline failed first; see below) |

The budget was **above every Linux baseline build and below every Windows and macOS one**. It was
measured on Linux — `tests/fixtures/nightly/mutants-measured.json` names run 33969332537 — and a
number of seconds is a fact about one runner. Nothing about `is_occupied` was ever tested on
Windows; the shard spent seventeen minutes failing to compile five mutants.

The fix is to stop expressing the budget in seconds. **A mutant's build does no more work than the
baseline build of the same crate**, so twice the baseline that cargo-mutants times in that same
job, on that same machine, is right on every runner by construction:
`--build-timeout-multiplier 2` in place of `--build-timeout 120`. It cannot be wrong on a runner
nobody has measured, which is the property the constant did not have.

The test budget stays a constant, because what it bounds is a different thing: a mutant that never
terminates, rather than a machine that is slow. The suite's own runtime is what the baseline
measures, and 420 seconds clears every baseline test in the record.

One measurement is still unavoidable. `timeout-minutes` is a wall clock, and converting a ratio
into one needs a number of seconds from somewhere — so the record gains a `baselines` object with
each runner's figures beside the run they were read from, and both worst-case assertions compute
against the slowest. `13 × (2 × 253 s + 420 s)`, plus that runner's whole baseline and time to
retain evidence, is 224 minutes; the job is cut at 225 rather than 150.

## The other two timeouts, which are kills

`appfile-5-linux` and `verify-8-linux` each had one, and both are in the **Test** phase after a
build that finished. They are mutants that stop a loop advancing — `Parser::skip_trivia` and
`read_entry` — and for those there is no assertion to fail, because nothing reaches one. The suite
failing to terminate *is* the detection, and mutation testing counts it as a kill.

This gate counted every timeout as a failure, so a shard holding one could never go green: the same
shape of unsatisfiable policy as an equivalent mutant in a repository with no exclusion mechanism.
`scripts/ci/mutation.py` tells the two apart by the phase the timeout is in, which cargo-mutants
already records and `valid_phases` already validated:

- a **test-phase** timeout is a `hang`, and a kill. Counted apart from `caught` rather than folded
  into it, so a reader can still see how many there were and ask whether the budget is generous
  against the baseline — which is the other thing a timeout can mean;
- a **build-phase** timeout keeps the name `timeout`, keeps failing the gate, and after the change
  above should not happen at all.

## `not_run 15`, which is neither

Two shards, two unrelated causes, and no budget involved in either.

- **`appfile-3-linux`, 13.** `##[error]The runner has received a shutdown signal.` at 08:55:30,
  eighteen minutes into a job cut at 150 — GitHub reclaimed the runner. The harness did exactly
  what it should: `process_status: interrupted`, `errors: ["runner interrupted"]`, thirteen
  candidates marked `not_run`, gate failed. There is nothing here to fix in this repository, and a
  retry that hid it would be worse than the red shard.
- **`cache-12-macos`, 2.** `ERROR cargo test failed in an unmutated tree, so no mutants were
  tested`, and the baseline log names the test: `selfexe::tests::open_self_opens_the_test_binary`,
  asserting ELF magic against `[207, 250, 237, 254]`. That is a Mach-O, and the assertion was
  already fixed when the suite was first qualified on a macOS host — `src/selfexe.rs` asks
  `platform::object_format_of` for the host's own magic now, and the comment there records it.
  The nightly simply predates the fix.

## What is left

A nightly run. Every one of the three numbers now has an answer, and none of the answers is a
promise about work still to do — but a green campaign is something a run says, not something a
record claims, so `docs/dev/v1-readiness.md` says exactly that.
