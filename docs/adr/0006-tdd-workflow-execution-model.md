<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# 0006 — TDD workflow and the developer tooling that comes first

Status: Accepted · 2026-08-30

## Context

ginary is built by orchestrated agents rather than by one person reading a diff, and it fails in
places that are awkward to observe: inside a process that has already been replaced by `execve`,
in a cache directory nobody looked at, in a runtime assembled from a tree that no longer exists
after the build. A defect found by running the finished artifact costs far more to localise than
one found by a test, and an agent that writes code before a test tends to write code that
asserts nothing.

There is also a sequencing question. Diagnostics, tracing and test harnesses are usually
deferred until something is hard to debug. By then the thing that is hard to debug is the same
code that would have to be instrumented, and the instrumentation is written under pressure.

## Decision

**Every milestone runs the same five phases**, and evidence of each is recorded:

```
RED       write the failing test, run it, confirm it fails for the intended reason.
          A compile error is not RED; the failure must be an assertion failure.
GREEN     the smallest implementation that passes it, with everything else still green.
REVIEW    an adversarial pass over boundaries, concurrency, error paths, clippy and docs,
          producing findings.
FIX       per finding: regression test first, watch it fail, then fix.
REFACTOR  tidy with the suite green, ending on `mise run check`.
```

A bug is never fixed without a regression test under `tests/regressions/`, named after its
issue. A fix with no test is returned at review.

**Developer tooling is a deliverable of every milestone, not a later phase.** Each milestone
adds at least one item to the developer-facing surface: `diag` phase timing, `GINARY_DEBUG`,
the `GINARY_TRACE` JSON Lines trace with the `LaunchPlan` recorded immediately before `execve`,
`GINARY_SUPERVISE` for a spawn-and-wait path that shares Windows' code, the `GINARY_CMD`
artifact protocol, `build --explain`, `build --keep-staging`, `inspect --launch-plan`,
`ginary.index.json` with `verify` and `diff`, `crashdump`, and `doctor`. `GINARY_FAULT` fault
injection is compiled in only under `cfg(feature = "fault-injection")` so it cannot exist in a
release build.

**Tests inject their environment.** Anything that reads the process environment, the clock or
`PATH` is split into a pure function over an explicit snapshot plus a thin wrapper that captures
it. Tests exercise the pure half and never mutate global state, which keeps the suite
parallel-safe.

**Toolchain-dependent tests gate rather than fail**, through `require_tools`, and print what
they skipped. `GINARY_REQUIRE_TOOLCHAIN=1` turns a skip into a failure, and CI sets it, so a
missing toolchain in CI cannot silently remove coverage.

**Every milestone writes `docs/dev/log/<milestone>.md`** with what was done, the RED evidence,
gate results, measurements and open questions.

Commits use Conventional Commits with a module scope, one per milestone. Tagging, publishing,
pushing and version changes require a separate explicit request.

## Consequences

The suite states the specification, which is what an agent-driven project needs most: a
requirement that is not a test does not exist. Bugs come with a permanent guard against
recurrence, and the review phase has something concrete to argue with.

Building the diagnostics first means the launcher can be debugged from a trace file rather than
by adding print statements to a process that replaces itself, and the reproduction of a failed
launch is a copy-pasteable command rather than a reconstruction.

The costs are visible in the shape of the code: environment access is split in two, more
surface is public than a minimal implementation would need, and each milestone spends time on
tooling that ships no user-visible feature. The RED-before-GREEN rule also makes progress look
slower per commit, which is accepted — the plan explicitly does not optimise for speed.

`GINARY_FAULT` is a deliberate hole in the abstraction, kept behind a non-default feature so
that the release binary has no such path at all.
