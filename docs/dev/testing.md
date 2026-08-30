<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# Testing

## What exists now

| file | scope |
|---|---|
| `src/target.rs` unit tests | target names, parsing, round trips, the seven supported targets |
| `src/cache_dir.rs` unit tests | precedence, empty values, relative `XDG_CACHE_HOME`, no variable set |
| `src/doctor.rs` unit tests | `PATH` search, version parsers, probe timeout, child reaping, report rendering |
| `src/cli.rs` unit tests | clap definition validity, parsing, JSON and text command output |
| `tests/smoke_cli.rs` | the real binary: `--help`, `version`, `version --json`, no-argument exit 2, `doctor`, `doctor --json` |

Run them with `mise run test` (or `cargo test`). `mise run test:fast` runs `cargo test --lib
--bins --test smoke_cli`, named explicitly because it is the subset that *requires* no external
toolchain — no test outside those targets exists yet.

The library and binary targets spawn only fake shell scripts in temporary directories, never a
program from the machine's `PATH`. `tests/smoke_cli.rs` is the exception: it drives the real
binary, and `ginary doctor` probes whatever `gleam`, `erl`, `strip` and `docker` are on the
ambient `PATH`. None of the four has to be present, or to succeed, for the test to pass, but a
probe that is present costs its own runtime, and one that hangs costs `doctor::PROBE_TIMEOUT`
(10 s) before it is killed. That bound is what keeps `test:fast` fast; it is not a claim that
nothing external runs.

## Conventions

**Unit tests live beside the code** in a `#[cfg(test)] mod tests`. They cover pure functions and
anything that takes an injected environment. Integration tests under `tests/` drive the real
binary through `assert_cmd` and assert only on the user-visible contract: exit codes, output
shape, and JSON schemas.

**Environment is injected, never read, in testable code.** `cache_dir::resolve` takes an
`EnvSnapshot`; `doctor::find_in_path` takes the `PATH` value; `doctor::Report::gather_from`
takes the probe list, the `PATH` value and an `EnvSnapshot`. Only the thin `from_env` and
`gather` wrappers touch the process environment, so tests never mutate global state and can run
in parallel. `Report::gather` itself is covered end to end by `tests/smoke_cli.rs`, which drives
the real binary and is therefore the only place a real toolchain is reached at all.

**One test asserts one behaviour**, and its name is the sentence it proves.

## Toolchain gating

Tests that need `gleam`, `erl`, `strip` or `docker` call

```rust
require_tools(&["gleam", "erl", "strip"])
```

which returns whether every named tool was found. When one is missing the test prints what it
skipped and returns. Setting `GINARY_REQUIRE_TOOLCHAIN=1` makes the same call panic instead, so
a CI job that is supposed to have the toolchain cannot silently skip its coverage. CI sets it on
the test job.

A skipped test must say so on stdout. A silent skip is indistinguishable from a passing test and
is treated as a defect.

## Planned infrastructure

`tests/common/` will hold:

- **`FakeOtp`** — builds a fake OTP tree in a temporary directory: `.app` files, a boot file and
  dummy executables. It makes the closure, assemble and strip paths testable in milliseconds
  with no Erlang installed.
- **`FixtureProject` and `Artifact`** — copy a fixture into a temporary directory, run
  `ginary build` once per test binary behind a `OnceLock`, then run the artifact under a
  scrubbed environment (`env_clear()`, `PATH` pointing at an empty directory, `HOME` and
  `XDG_CACHE_HOME` in the temporary tree) and return the exit status, stdout, stderr, the cache
  directory and the trace as structured data.
- **`require_tools`** — the gate described above.

Two fixtures:

- **`hello_ffi`** — no hex dependencies at all, reaching the runtime through `@external`
  (`init:get_plain_arguments`, `code:priv_dir`, `halt`). It builds offline.
- **`hello_crypto`** — `gleam_stdlib`, `gleam_erlang`, `gleam_crypto` and `argv`, with a
  committed `manifest.toml`, to exercise the `crypto.so` NIF path. CI warms the hex cache.

Planned test categories:

- **Determinism** — build the same input twice and compare bytes; `SOURCE_DATE_EPOCH` is
  honoured.
- **Concurrency** — start N real processes on a cold cache at once, then assert exactly one
  extracted directory, no leftover temporary trees, and every process exiting 0.
- **Fault injection** — `GINARY_FAULT` under `cfg(feature = "fault-injection")`. The canonical
  case: `after-extract:pause`, `SIGKILL` the process to leave a half-written temporary tree, and
  assert the next run cleans it up and succeeds.
- **Trace assertions** — end-to-end tests read the JSON Lines trace and assert on phase order,
  on `cache hit` for the second run, and on per-phase time bounds.
- **Property tests** — `proptest` over the trailer encoding, the `.app` parser and tar path
  validation.
- **Fuzzing** — `cargo-fuzz` targets for `trailer`, `appfile` and `payload_unpack`.
- **Mutation testing** — `cargo-mutants`, sharded in a nightly CI job.
- **Coverage** — `cargo llvm-cov`, gated at 90% lines and 80% branches.
- **Regressions** — `tests/regressions/`, one file per fixed bug, named after its issue.

None of `cargo-deny`, `cargo-llvm-cov`, `cargo-mutants`, `cargo-insta`, `cargo-nextest` or
`cargo-fuzz` is installed on the current development machine; `deny.toml` and the plans above
are in place for when they are.
