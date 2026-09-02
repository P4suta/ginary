<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# `.beam` fixtures

Three real compiled BEAM modules, copied verbatim. `src/beam.rs` is an IFF
chunk reader, and a reader that only handles files written by its own tests is
not a reader: the hand-built byte strings in `tests/beam.rs` pin the grammar,
and these pin the shape a real compiler emits — fourteen or fifteen chunks, a
zero-length `StrT`, four-byte padding between chunks, and both `Dbgi` and
`Docs` present, which is exactly what `beam_lib:strip_files/1` removes.

They are *unstripped*, which is the point: a fixture that had already been
stripped could not show what stripping is for.

| file | bytes | source |
|---|---|---|
| `gleam@list.beam` | 49680 | `gleam_stdlib` 1.0.5 |
| `gleam@string.beam` | 18696 | `gleam_stdlib` 1.0.5 |
| `gleam@bool.beam` | 4664 | `gleam_stdlib` 1.0.5 |

All three come from `gleam export erlang-shipment` run on the `notify` project
(`/home/<user>/projects/gleam/notify`, gleam 1.18.1, Erlang/OTP 29.0.5), at
`build/erlang-shipment/gleam_stdlib/ebin/`. `gleam_stdlib.app` from the same
directory is *not* a BEAM file and is not here; the `.app` fixtures live under
`tests/fixtures/app/`.

The three sizes span two orders of magnitude on purpose. `gleam@bool.beam` is
small enough that a test can name every chunk offset in it; `gleam@list.beam`
is large enough that truncating it at every byte offset is a real workout for
the never-panic property.

## They contain an absolute path, on purpose

Each of the three carries a `Dbgi` chunk, and the Erlang compiler records in it
the absolute path of the `.erl` it compiled — here a directory under the home
of the machine `gleam_stdlib` was built on. The regression test
`e5_a_gated_test_defaulted_to_one_developers_machine.rs` scans every tracked
file under `tests/`, `src/`, `scripts/` and `.github/` for exactly that, so
these three are named in its `ALLOWED` list with the reason.

The exception is narrow and it is argued. A compiled artifact copied verbatim
is the one thing in the tree whose bytes nobody chose: rewriting the chunk
would make it no longer what a compiler wrote, which is the whole point of
these files, and recompiling `gleam_stdlib` with a relative `-o` would change
every offset and size in the table above and would no longer be 1.0.5 as
shipped. Nothing reads the path, and nothing falls back to it. The rule it is
carved out of is unchanged: no source or test file may name a path that exists
on one machine.

## Licensing

These are not ginary's files: they are compiled from `gleam_stdlib`, which is
`Apache-2.0`. A binary carries no SPDX header, so `REUSE.toml` declares the
path instead.
