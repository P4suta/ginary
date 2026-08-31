<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# `gleam.toml` fixtures

Each file here is a whole `gleam.toml`, named after the one thing it pins. They are read by
`tests/config.rs` through `ProjectConfig::from_toml`, which takes the text and a path, so
none of them has to be a real project on disk.

| file | what it is |
|---|---|
| `defaults.toml` | a project with no `[tools.ginary]` table at all: every setting is a default |
| `full.toml` | every key the table has, each with a value that is not its default |
| `unknown_key.toml` | a key `[tools.ginary]` does not have, which `deny_unknown_fields` must name |
| `bad_level.toml` | `compression_level = 23`, one past what zstd accepts |
| `bad_erl_flags.toml` | `erl_flags` holding `-pa`, which the launcher builds itself |
| `bad_extra_bin.toml` | `erts_extra_bins` holding a path, which is not a program name |
| `bad_name.toml` | a project name that is not a Gleam name |
| `no_name.toml` | a manifest with no `name` key |
| `malformed.toml` | TOML that does not parse |

Two of the nine are valid — `defaults.toml` and `full.toml` — and the other seven are invalid on
purpose. `tests/config.rs` asserts the exact error variant of each of the seven, so editing one
means editing its assertion with it.
