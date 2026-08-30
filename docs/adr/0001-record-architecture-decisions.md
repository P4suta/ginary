<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# 0001 — Record architecture decisions

Status: Accepted · 2026-08-30

## Context

ginary reaches its v1 shape through a long sequence of choices that are individually reasonable
and collectively load-bearing: how the runtime is obtained, how the payload is framed, how the
cache stays correct under concurrent first runs, why the launcher calls `execve` instead of
`entrypoint.sh`. Several of them were settled only after measuring or after reading OTP source,
and the reasoning does not survive in the code. Without a written record the next change either
repeats the investigation or silently undoes a constraint it did not know about.

The plan document holds the current design, but a plan is rewritten as it is executed. What is
needed alongside it is an append-only record of individual decisions, each with the situation
that forced it.

## Decision

Architecture decisions are recorded as numbered Markdown files in `docs/adr/`, using
[MADR](https://adr.github.io/madr/) with the sections **Context**, **Decision** and
**Consequences**, a `Status:` line naming a status and a date, and an
`SPDX-License-Identifier` comment.

- Numbers are allocated sequentially and never reused.
- A record is written when a decision constrains future work: the payload format, the execution
  model, the source of the bundled runtime, the cache protocol, the workflow.
- Records are never edited to change their decision. A decision that no longer holds gets a new
  record, and the old one is marked `Superseded by NNNN`.
- Status values are `Proposed`, `Accepted`, `Superseded by NNNN` and `Deprecated`.
- Investigations that produced measurements cite them, so a later reader can tell a measured
  claim from an assumption.

## Consequences

Every reviewer can see why a constraint exists before proposing to remove it, and a superseding
change has to argue against a written position rather than against nothing. The cost is one
short file per structural decision, and the discipline of not rewriting history when a decision
turns out to be wrong. Records 0002 to 0006 capture the decisions taken before any code was
written.
