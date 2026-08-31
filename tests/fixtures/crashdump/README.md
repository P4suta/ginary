<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# Crash dump fixtures

Two files, both read by `tests/crashdump.rs`.

`synthetic.dump` is hand-written and small, and it is the one the assertions name field by
field. Its shape is copied from a real `erl_crash.dump` written by OTP 29 on this machine — the
`=erl_crash_dump:0.5` tag, the unlabelled date line, `Slogan:`, `System version:`, `Taints:`,
then `=proc:<pid>` sections whose keys are `State:`, `Name:`, `Spawned as:`, `Spawned by:`,
`Message queue length:`, `Reductions:`, `Stack+heap:`, `OldHeap:` and `Memory:` — with the
schedulers, ports, ETS tables, atoms and heap dumps of a real one left out, because none of them
is read. Seven processes rather than five, so "the top five" is a claim and not the whole list;
three of them have no `Name:`, because an unregistered process is the common case and a summary
that only handled named ones would summarise almost nothing; and the largest heap belongs to a
process in the middle of the file, so a reader that took the first five in file order would fail.

`truncated.dump` is the first 780 bytes of `synthetic.dump`, which lands inside a `=proc:`
section. A runtime killed while writing its dump leaves exactly this, and it is the case a reader
most needs summarised rather than refused.

A dump written by the *real* `erl` is not committed: it is 1.8 MB and its every field is a
property of the machine that wrote it. `tests/crashdump.rs` generates one instead, gated on
`require_tools(&["erl"])`; the recipe is in `docs/dev/log/B2.md`.
