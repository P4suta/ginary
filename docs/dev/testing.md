<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# Testing

## What exists now

| file | scope |
|---|---|
| `src/target.rs` unit tests | target names, parsing, round trips, the seven supported targets |
| `src/cache_dir.rs` unit tests | precedence, empty values, relative `XDG_CACHE_HOME`, no variable set |
| `src/doctor.rs` unit tests | version parsers, the probe list, tool reports, report rendering |
| `src/process.rs` unit tests | the `PATH` search, a bounded child, the timeout, a chatty pipe, a grandchild holding the pipes, child reaping |
| `src/appfile.rs` unit tests | the internals no integration test can reach: atom quoting, escaping, float rendering, the nesting bound, the warning paths |
| `src/cli.rs` unit tests | clap definition validity, parsing, JSON and text command output |
| `tests/smoke_cli.rs` | the real binary: `--help`, `version`, `version --json`, no-argument exit 2, `doctor`, `doctor --json` |
| `tests/appfile.rs` | the `.app` reader: the term grammar, `Term`'s re-serialisation, `AppResource`, the error positions, and every fixture under `tests/fixtures/app/` |
| `tests/otp.rs` | `inspect_root` against fake roots that are whole and broken, `boot_lib_dirs`, and `discover` with and without an override |
| `tests/cli.rs` | the real binary: `appfile parse` as a table and as JSON, its exit codes, and the `otp` field `doctor` now reports |
| `tests/regressions.rs` | one module per fixed bug, `#[path]`-included from `tests/regressions/`; see the README there |

`src/process.rs` holds the tests that used to live in `src/doctor.rs`: the
timeout runner moved there in A1a, because `otp::discover` needs the same
bounded child, and its tests moved with it unchanged in substance.

Run them with `mise run test` (or `cargo test`). `mise run test:fast` runs `cargo test --lib
--bins --test smoke_cli --test regressions`, named explicitly because it is the subset that
*requires* no external toolchain. `tests/appfile.rs`, `tests/otp.rs` and `tests/cli.rs` are
outside it because each holds a handful of gated tests, even though the bulk of all three runs
against fixtures and temporary directories.

The library and binary targets spawn only fake shell scripts in temporary directories, never a
program from the machine's `PATH`. Four integration targets do reach it, each for a stated
reason:

| target | what reaches `PATH` | how it is bounded |
|---|---|---|
| `tests/smoke_cli.rs` | `ginary doctor` probes whatever `gleam`, `erl`, `strip` and `docker` are there | none has to be present or to succeed; a hanging probe costs `doctor::PROBE_TIMEOUT` (10 s) before it is killed |
| `tests/cli.rs` | the same, plus the `otp` field, which runs the ambient `erl` | the two `otp` assertions are gated on `require_tools(&["erl"])` |
| `tests/otp.rs`, `tests/appfile.rs` | `otp::discover(None)` and the host OTP tree it names | every one of those tests is gated on `require_tools` |
| `tests/regressions.rs` | nothing ambient: it *replaces* `PATH` with a temporary directory holding stub scripts | the stubs exit at once |

Those bounds are what keeps `test:fast` fast; they are not a claim that nothing external runs.

## Conventions

**Unit tests live beside the code** in a `#[cfg(test)] mod tests`. They cover pure functions and
anything that takes an injected environment. Integration tests under `tests/` drive the real
binary through `assert_cmd` and assert only on the user-visible contract: exit codes, output
shape, and JSON schemas.

**Environment is injected, never read, in testable code.** `cache_dir::resolve` takes an
`EnvSnapshot`; `doctor::find_in_path` takes the `PATH` value; `doctor::Report::gather_from`
takes the probe list, the `PATH` value, an `EnvSnapshot` and the OTP result. Only the thin
`from_env` and `gather` wrappers touch the process environment, so tests never mutate global
state and can run in parallel.

`otp::discover(None)` is the one function that reads `PATH` itself, because it is the entry
point a caller uses without an override. It is split so that everything below the read is
injectable anyway: `find_erl` takes the `PATH` value and `probe_root` takes the program and the
budget, and the four ways a probe can fail to answer — absent, silent, one line, a release that
is not a number, never exiting — are unit-tested against stub scripts. The whole function is
covered from outside by `tests/regressions/a1a_doctor_dropped_the_otp_error.rs`, which sets
`PATH` to a directory holding a stub `erl` and asserts on what `ginary doctor` prints.

**One test asserts one behaviour**, and its name is the sentence it proves.

## Toolchain gating

Tests that need `gleam`, `erl`, `strip` or `docker` open with

```rust
let Some(tools) = require_tools(&["gleam", "erl"]) else {
    return;
};
```

`require_tools` (in `tests/common/tools.rs`) returns `Some(Toolchain)` when every named program
is on `PATH`, and the `Toolchain` hands back the resolved path of each one, so the test runs the
program it checked for rather than trusting `PATH` a second time. When one is missing it prints
`skipping: <tool> not on PATH` on standard error and returns `None`, and the test returns without
pretending to have covered anything.

Setting `GINARY_REQUIRE_TOOLCHAIN=1` turns the same call into a panic, so a CI job that is
supposed to have the toolchain cannot silently skip its coverage. CI sets it on the test job.

A skipped test must say so. A silent skip is indistinguishable from a passing test and is treated
as a defect.

## Fake trees

`tests/common/fake_otp.rs` builds the two directory layouts every build-side module reads, in a
temporary directory, in milliseconds, with no Erlang installed. `tests/common/script.rs` is the
third builder: it writes an executable `/bin/sh` stub, which is how a test puts a chosen `erl`
on a `PATH` of its own.

`FakeOtp` writes a runtime root that `otp::inspect_root` accepts as it stands — `erts-<vsn>/bin`
holding the four required binaries as executable shell stubs, `bin/no_dot_erlang.boot`,
`lib/<app>-<vsn>/{ebin,priv}`, `releases/start_erl.data` and `releases/<rel>/OTP_VERSION`. It is
seeded with `kernel` and `stdlib`, because a root without them is not an OTP installation:

```rust
let dir = tempfile::tempdir().expect("tempdir");
let otp = FakeOtp::new()
    .erts_vsn("17.0.5")
    .release(29)
    .otp_version("29.0.5")
    .app("kernel", "11.0.3", &["stdlib"])
    .app_with("ssl", "11.7.4", |app| {
        app.applications(&["crypto", "public_key"])
            .priv_file("lib/x.so", b"..")
    })
    .build_in(dir.path());
```

`FakeShipment` writes what `gleam export erlang-shipment` writes, `<dir>/<app>/ebin/<app>.app`
with dummy `.beam` files and `priv/` contents alongside. Both take applications through the same
`FakeApp` description, so an application can be moved between a runtime root and a shipment
without being rewritten — which is exactly what the closure tests turn on.

Neither builder writes anything that could be executed usefully. The ERTS binaries are shell
scripts that exit 0 and the `.beam` files are twelve bytes. What they carry is the *structure*,
which is all that discovery, closure and assembly read. A test that needs a real runtime is gated
on the host toolchain instead.

Both builders write `.app` files that the parser in `src/appfile.rs` reads back, and
`tests/appfile.rs` checks exactly that — a shipment and a runtime root are built with every
property the builder can write, parsed, and asserted field by field. A defect in the generated
Erlang therefore fails here rather than three milestones later. Names that are not bare atoms
(`my-app`) are quoted on the way out for the same reason.

To test a *broken* root, build a whole one and break it: `fs::remove_file`, `fs::create_dir` for
a second `erts-*`, or `fake_otp::make_non_executable`. The builder deliberately has no API for
producing an invalid tree, so nothing can be broken by accident.

## Fixture policy

`tests/fixtures/app/` holds two kinds of file and `tests/fixtures/app/README.md` records which is
which and where each copied file came from.

**Hand-written fixtures** pin one construct each and stay small, so a failing assertion points at
one thing. Two of them — `malformed.app` and `unsupported_map.app` — are invalid on purpose, and
the tests assert the exact line and column of the error, so editing either file means editing the
assertions with it.

**Copied fixtures** are real files, taken verbatim from the host OTP installation and from a real
`gleam export erlang-shipment`, because a parser that only handles files written by its own
author is not a parser. They are never reformatted: their whitespace, comments and indentation
are the point. The README names the source directory and the version of every one.

The two are complementary. The copies keep the coverage on a machine with no Erlang; the gated
`parses_every_app_in_host_otp` walks the live OTP root and asserts every `.app` in it parses,
which is coverage the copies cannot give.

## Snapshots

Textual output is asserted with `insta`, and the `.snap` files under `tests/snapshots/` are
committed and reviewed like any other assertion. Three exist:

| snapshot | what it pins |
|---|---|
| `appfile__nested_term_display.snap` | `Term`'s re-serialisation of the whole `nested.app` term |
| `appfile__parse_error_messages.snap` | the sentences the two invalid fixtures produce |
| `cli__appfile_parse_table.snap` | the table `ginary appfile parse` prints |

A snapshot is a contract, not a recording. `cargo insta review` is for reviewing a *deliberate*
change to output; accepting a snapshot to make a red test pass is the same defect as weakening an
assertion.

## Planned infrastructure

`tests/common/` already holds `tools.rs` and `fake_otp.rs`, described above. Still to come:

- **`FixtureProject` and `Artifact`** — copy a fixture into a temporary directory, run
  `ginary build` once per test binary behind a `OnceLock`, then run the artifact under a
  scrubbed environment (`env_clear()`, `PATH` pointing at an empty directory, `HOME` and
  `XDG_CACHE_HOME` in the temporary tree) and return the exit status, stdout, stderr, the cache
  directory and the trace as structured data.

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
- **Regressions** — `tests/regressions/` exists and is wired up; see the "what exists now" table
  above and `tests/regressions/README.md`.

## Assurance tooling

`cargo-deny`, `cargo-llvm-cov`, `cargo-mutants`, `cargo-insta`, `cargo-nextest` and `cargo-fuzz`
are all installed on the current development machine, and each has a mise task:

| task | command | notes |
|---|---|---|
| `mise run deny` | `cargo deny check` | advisories, bans, licences, sources; part of `mise run check` |
| `mise run cov` | `cargo llvm-cov ... --lcov --output-path target/lcov.info`, then a summary | gated at 90% lines |
| `mise run mutants` | `cargo mutants` | copies the tree; not `--in-place`; sharded in CI later |
| `mise run test:nextest` | `cargo nextest run` | nextest does not run doc tests, so it is not a replacement for `mise run test` |

The coverage gate is `--fail-under-lines 90`. The 80% branch floor is not enforced yet: branch
coverage needs a nightly `-Z coverage-options=branch` build, and this crate is measured on
stable. When that changes, the floor moves from prose into the `cov` task.

`cargo-fuzz` needs a nightly toolchain to build a target; the fuzz targets themselves
(`trailer`, `appfile`, `payload_unpack`) arrive with the code they cover. `cargo-insta` is
installed through mise but has no version pinned for its shim, so `cargo insta` on `PATH` fails
with `No version is set for shim: cargo-insta`; pin it, or call the binary under
`~/.local/share/mise/installs/cargo-cargo-insta/1.48/bin/`. The `insta` *crate* is a normal dev
dependency and needs none of that: `cargo test` compares snapshots and writes `.snap.new` beside
a mismatch, and only reviewing them wants the subcommand.

`proptest` is a dev dependency too. Its failure-persistence file for an integration test lands at
`tests/<target>.proptest-regressions`; a file that records a real counterexample is committed with
the fix it belongs to.

`cross` is the one tool from the plan's list that is *not* installed; cross-compilation is not
exercised locally.
