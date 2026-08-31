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
| `runtime.toml` | the six runtime keys B1 adds, each away from its default |
| `bad_encoding.toml` | `filename_encoding` naming an encoding the emulator has no flag for |
| `bad_env_key.toml` | `env` naming an `ERL_*` variable, which the launcher scrubs |
| `bad_env_name.toml` | `env` naming `ROOTDIR`, which the launcher derives |

Three of the thirteen are valid — `defaults.toml`, `full.toml` and `runtime.toml` — and the other
ten are invalid on purpose. `tests/config.rs` asserts the exact error variant of each of the ten,
so editing one means editing its assertion with it.
