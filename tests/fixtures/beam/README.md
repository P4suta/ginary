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

## Licensing

These are not ginary's files: they are compiled from `gleam_stdlib`, which is
`Apache-2.0`. A binary carries no SPDX header, so `REUSE.toml` declares the
path instead.
