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
| `tests/closure.rs` | the closure over fake shipment and OTP trees: seeds, edges, resolution order, determinism, the three errors, `explain` and `chain`, two property tests, and one gated run over a real shipment |
| `tests/cli.rs` | the real binary: `appfile parse` as a table and as JSON, `closure` as a table, JSON, `--explain` and its two footers, `stage` as a table, JSON, `--explain`, `--force` and its two usage errors, and the `otp` field `doctor` now reports |
| `tests/assemble.rs` | the staging root over fake trees: the exact layout, every exclusion, junk removal, modes, symlinks, the error paths, the listing, and determinism |
| `tests/stage_run.rs` | toolchain-gated: stage the `hello_ffi` fixture against the host OTP, strip it, measure it, and boot it through `erlexec` |
| `tests/beam.rs` | the IFF chunk reader: the grammar over hand-built bytes, the shape a compiler emits over three real modules, and the never-panic properties |
| `tests/elf.rs` | read-only ELF inspection, against the running test binary, a non-ELF file, truncations of a real binary, and the host `beam.smp` |
| `tests/strip.rs` | stripping a staged root: the exact `beam_lib` one-liner, the four verification failures, the three option shapes, idempotence, and `StagedRoot::refresh` |
| `tests/report.rs` | the size and dependency account: the rendered table and `needs:` line over a synthetic report, and the measurement over a real staged tree |
| `tests/trailer.rs` | the 64-byte trailer: the encoding, `None` against every error, the geometry arithmetic, and two never-panic properties |
| `tests/manifest.rs` | `ginary.json` and `ginary.index.json`: the wire field order, the unknown-key round trip, `check_version`, the `launch` path rules, `created_at`, and the index over a staging root |
| `tests/payload.rs` | the payload: deterministic packing, the round trip with modes, eight hand-built malicious archives, the two streaming reads, and three never-panic properties |
| `tests/diag.rs` | the recorder through injected sinks: both output shapes, event order, elapsed time, and the four ways it stays off |
| `tests/regressions.rs` | one module per fixed bug, `#[path]`-included from `tests/regressions/`; see the README there |

`src/process.rs` holds the tests that used to live in `src/doctor.rs`: the
timeout runner moved there in A1a, because `otp::discover` needs the same
bounded child, and its tests moved with it unchanged in substance.

Run them with `mise run test` (or `cargo test`). `mise run test:fast` runs `cargo test --lib
--bins --test smoke_cli --test regressions`, named explicitly because it is the subset that
*requires* no external toolchain. `tests/appfile.rs`, `tests/otp.rs`, `tests/closure.rs` and
`tests/cli.rs` are outside it because each holds a handful of gated tests, even though the bulk of
all four runs against fixtures and temporary directories. `tests/assemble.rs` needs no toolchain
either and could join `test:fast`; `tests/stage_run.rs` is entirely gated, because every test in
it runs a real `gleam` and a real `erlexec`.

The four A3a targets — `tests/trailer.rs`, `tests/manifest.rs`, `tests/payload.rs` and
`tests/diag.rs` — need no toolchain at all. Every byte they read is one they wrote, in a
`tempfile` directory or in memory, and nothing in them spawns a process.

The four A2 targets divide the same way. `tests/beam.rs` and `tests/report.rs` need nothing at
all. `tests/strip.rs` needs nothing for all but one test — the stub `erl` a `FakeOtp` writes is
what makes the beam step reachable without an Erlang — and gates the one that runs a real `strip`
on `require_tools(&["strip"])`. `tests/elf.rs` gates the two tests that read the host `beam.smp`
and leaves the rest ungated, because the fixture the others use is the test binary itself.

The library and binary targets spawn only fake shell scripts in temporary directories, never a
program from the machine's `PATH`. Four integration targets do reach it, each for a stated
reason:

| target | what reaches `PATH` | how it is bounded |
|---|---|---|
| `tests/smoke_cli.rs` | `ginary doctor` probes whatever `gleam`, `erl`, `strip` and `docker` are there | none has to be present or to succeed; a hanging probe costs `doctor::PROBE_TIMEOUT` (10 s) before it is killed |
| `tests/cli.rs` | the same, plus the `otp` field, which runs the ambient `erl` | the two `otp` assertions are gated on `require_tools(&["erl"])` |
| `tests/otp.rs`, `tests/appfile.rs`, `tests/closure.rs` | `otp::discover(None)` and the host OTP tree it names | every one of those tests is gated on `require_tools` |
| `tests/stage_run.rs` | `gleam export erlang-shipment`, `otp::discover(None)`, and the `erlexec` of the staged tree | every test is gated on `require_tools(&["gleam", "erl"])`; the launched runtime gets `env_clear()`, an empty `PATH` directory and a `HOME` inside the test's temporary tree, and both children run under a deadline — `fixture::EXPORT_BUDGET` (180 s) and `erl::RUN_BUDGET` (60 s) — with stdin on the null device |
| `tests/regressions.rs` | nothing ambient: it *replaces* `PATH` with a temporary directory holding stub scripts | the stubs exit at once |
| `tests/strip.rs`, `tests/elf.rs` | the one `strip` run and the two `beam.smp` reads | both gated on `require_tools`; everything else in the two files runs against the test binary, a temporary tree, or a stub `erl` written by the builder |

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

A test can need more than a program. `tests/closure.rs` needs a real
`gleam export erlang-shipment` output, which `require_tools` knows nothing about, so it reads
`GINARY_TEST_SHIPMENT` (defaulting to the author's `notify` shipment) and applies the same rule by
hand: a directory that is not there is a reported skip, and a failure under
`GINARY_REQUIRE_TOOLCHAIN=1`. Any fixture a gated test needs from outside the repository is
overridable and escalated the same way.

A skipped test must say so. A silent skip is indistinguishable from a passing test and is treated
as a defect.

## Fake trees

`tests/common/fake_otp.rs` builds the two directory layouts every build-side module reads, in a
temporary directory, in milliseconds, with no Erlang installed. `tests/common/script.rs` is the
third builder: it writes an executable `/bin/sh` stub, which is how a test puts a chosen `erl`
on a `PATH` of its own. `tests/common/snapshot.rs` is the fourth helper, and exists because those
trees live in a `tempfile` directory whose name changes on every run: `scrub` replaces each root
with a placeholder, longest path first, so a snapshot pins the sentence rather than the machine.
`tests/common/fixture.rs` and `tests/common/erl.rs` are the two A1c added, and they work on real
trees rather than fake ones: the first copies a fixture Gleam project and exports it, the second
boots what assembly wrote. `tests/common/bounded.rs` is what both of them spawn through, so that
neither can hang the suite; it is the test-side counterpart of `src/process.rs`, which it cannot
call because that function takes neither an environment nor a working directory.
`tests/common/payload.rs` is what A3a added, and it builds no tree at all in the `FakeOtp` sense:
it writes tar headers byte by byte (`RawTar`), the smallest staging root the format tests need
(`staging_tree`), and the two instruments those tests read through, `CountingReader` and
`SharedSink`. The two policy sections below say why each exists.

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

Assembly needed three more things from the builder, and each is the smallest addition that keeps
the "no API for an invalid tree" rule intact. `FakeOtp::extra_erts_bins` puts spare programs in
`erts-<vsn>/bin`, because a runtime holding only the four required binaries cannot show that
assembly refuses the rest. `fake_otp::make_executable` is the counterpart of
`make_non_executable`, so a test can prove a NIF's execute bit survives the copy. And
`fake_otp::boot_bytes_for` writes boot-file bytes naming any `<name>-<vsn>` directories at all,
which is how a test builds the one mismatch assembly exists to catch — a boot file carried over
from a different OTP installation.

A2 added four things to the same builder, and each is the smallest addition that keeps the "no
API for an invalid tree" rule intact. `FakeOtp::with_erl_script` installs a stub `bin/erl` that
writes its own argument vector to `<root>/bin/erl.argv` and exits zero, which
`FakeOtpRoot::erl_argv` reads back; `src/strip.rs` runs the OTP installation's own `erl` by
absolute path, so this is the only way a test asserts on the exact `beam_lib:strip_files/1`
one-liner — and on the exact list of modules it is given — without an Erlang installed.
`FakeOtp::with_shrinking_erl_script` is the same stub with a body: it overwrites every `.beam`
named after `-extra` with a smaller module that still holds `Code`, which is what lets a command
line test tell a `ginary.stage.json` that was refreshed after stripping from one that was not. A
stub that changes no bytes cannot, and the test that used one asserted nothing.
`FakeOtp::with_failing_erl_script` is the same stub with a term on standard error and a non-zero
exit, which is what a real `beam_lib` failure looks like from the outside; the term travels in a
file beside the stub rather than inside its source, because `~p` prints a quoted atom with
apostrophes in it and a single-quoted shell string cannot hold one. All three go through
`script::script` rather than the builder's own `write_executable`, because this is the one stub a
test actually execs and that helper is what waits out the `ETXTBSY` window a sibling thread's
`fork` opens.

`DUMMY_BEAM` changed with them. It was twelve bytes — a bare `FOR1 <size> BEAM` with no chunks —
which was enough for everything that only counted and copied files. Stripping *opens* the
modules, and verifies that none holds `Dbgi` or `Docs` and that every one holds `Code`, so a
tree whose modules have no `Code` at all could not tell a working verification from a broken one.
It is now forty-eight bytes holding `AtU8`, `Code` and `Line`: structurally a module, still not a
loadable one, and *already stripped*, so a stub `erl` that does nothing leaves a tree that
legitimately passes. A test that needs a module carrying `Dbgi` writes one with
`fake_otp::beam_bytes`, in the open, which is the same rule the symlink tests follow.
`tests/snapshots/assemble__stage_explain_table.snap` moved by 288 bytes with it — eight modules
times thirty-six — and nothing else in the suite changed.

To test a *broken* root, build a whole one and break it: `fs::remove_file`, `fs::create_dir` for
a second `erts-*`, or `fake_otp::make_non_executable`. The builder deliberately has no API for
producing an invalid tree, so nothing can be broken by accident. `tests/assemble.rs` follows the
same rule for the things a `FakeApp` cannot describe: the `.appup`, the six excluded directories
and the three symlinks are all written by hand, in the open, by the test that needs them.

## Closure scenarios

`tests/closure.rs` builds a `FakeShipment` and a `FakeOtp` side by side in one temporary
directory — the shipment at `<tmp>/shipment`, the runtime at `<tmp>/otp` — so a single placeholder
scrubs every path that reaches a snapshot. Each test names the one behaviour it proves:

| scenario | what it pins |
|---|---|
| seeds | `kernel` and `stdlib` are present with `seed = Always` when nothing lists them; a `--root` is `Root`, an `--extra` is `Extra`, a name that is both stays `Root` |
| requesters | a seed records no requester even when another application lists it; nothing is ever its own requester; `requested_by` is sorted and deduplicated across two edges from one application |
| edges | `applications` and `included_applications` are followed; an `optional_applications` entry that resolves is bundled, one that does not lands in `skipped_optional` with its requester and is *not* an error; the same name outside `optional_applications` still is one |
| determinism | four permutations of two roots and two extras produce one identical `AppSet` |
| resolution | an application in both trees comes from the shipment, with a warning naming both directories; `crypto-doc`, `crypto-5.9.2.bak`, `crypto-latest` and `crypto-` are not versions; `3` is; a regular file called `crypto-9.9.9` is neither a match nor an ambiguity |
| errors | `crypto-5.9.2` beside `crypto-5.9.3` is `AmbiguousOtpApp` listing both; a missing application carries the exact chain `["app", "gleam_crypto", "crypto"]`, the two searched paths, and the `gleam.toml` hint; a missing *root* says nobody asked for it; a malformed `.app` in a dependency names the file |
| termination | `a -> b -> a` and an application that lists itself both finish, with the expected `requested_by` |
| output | `explain` over a six-application scenario, the borrowed-set iteration, and the JSON: paths as strings, `source` tagged `shipment` or `otp`, `seed` as `root`/`extra`/`always`/`none` |
| properties | over random small DAGs, the closure only grows when `extra` grows, and feeding it its own names back as extras changes nothing |

Four scenarios the first review of the module added, and they live in
`tests/regressions/` rather than here because each one pins a fixed defect:
a shipment copy shadowing a `lib` that holds two versions is a warning and not an
error (`a1b_shadowed_otp_ambiguity_aborted_the_closure.rs`); an ambiguous
`optional_applications` entry is skipped with a warning and not raised
(`a1b_an_ambiguous_optional_edge_was_an_error.rs`); `ClosureError::AppFile` names
the file and leaves the parse failure to its `source()`, so `ginary closure`
prints it once rather than three times (`a1b_app_file_error_repeated_its_cause.rs`);
and `../../escape`, `/etc`, `a/b` and an empty `--root` are refused as names
before any path is built from them (`a1b_app_names_were_used_as_paths.rs`).

Two things those tests need from the builders. `FakeApp::optional` writes `optional_applications`
and adds each name to `applications` if it is not there already, because that is OTP's own rule
and a builder that let the two drift would produce files no real tool writes. And neither builder
can write a broken tree on purpose: a test that needs two versions of one application builds a
whole root and copies a directory, the same way `tests/otp.rs` makes a second `erts-*`.

The gated test at the end of the file runs the same closure over a real
`gleam export erlang-shipment` output with `--root notify`, and asserts what only a real tree can
show: `crypto` resolves to a version that exists under the host `lib/`, every OTP `ebin` is a
directory under that `lib/`, and every shipment application has a directory. The shipment it uses
is `GINARY_TEST_SHIPMENT` when that is set and a path on the author's machine otherwise, and a
missing one is escalated exactly as a missing program is: a reported skip, or a failure under
`GINARY_REQUIRE_TOOLCHAIN=1`. Without that escalation the only test that touches a real tree would
evaporate silently on every machine but one.

## Fixture policy

Two kinds of fixture live under `tests/fixtures/`: `.app` files that a parser reads, and whole
Gleam projects that a toolchain builds.

### `tests/fixtures/hello_ffi/` — the zero-dependency project

`hello_ffi` is a real Gleam project with **no hex dependencies at all**. That is the whole point
of it: `gleam build` and `gleam export erlang-shipment` run offline, behind a committed
`manifest.toml` that locks zero packages, so nothing has to be resolved from hex, there is no
cache for CI to warm, and nothing can fail because a package server is slow.
A project without `gleam_stdlib` has no `io` module, so every observable thing it does happens in
`src/hello_ffi_ffi.erl` through `@external`, which is a feature rather than a workaround: the
four things a staged root has to get right are exactly the four the FFI touches.

| what it does | what it proves about the staged root |
|---|---|
| `init:get_plain_arguments()`, printed as `args=<joined>` | everything after `-extra` reached the application unchanged |
| reads `code:priv_dir(hello_ffi)/greeting.txt` | `priv` was staged beside `ebin` and the code path found it |
| prints `cwd=<file:get_cwd()>` | the process started where it was told to, not where the runtime lives |
| `halt(N)` on the first argument, `erlang:error(boom)` on `--crash` | exit codes propagate, and a crash reaches Gleam's `hello_ffi@@main`, which prints `runtime error` and exits 1 |

`build/` is git-ignored through the existing `tests/fixtures/*/build/` pattern, and
`FixtureProject::copy` skips it, so no test ever builds the fixture in place.

### `tests/fixtures/beam/` — three real compiled modules

`gleam@bool.beam`, `gleam@string.beam` and `gleam@list.beam`, copied verbatim from
`gleam export erlang-shipment` over the `notify` project, with `tests/fixtures/beam/README.md`
recording where each came from. They exist for the reason the copied `.app` files exist: a parser
that only handles files written by its own author is not a parser. What they add that a
hand-built byte string cannot is the *shape a compiler emits* — fourteen chunks, a zero-length
`StrT`, four-byte padding between every pair, and both `Dbgi` and `Docs` present.

They are also uncompressed, which a *stripped* module is not: `beam_lib` writes what it rewrote
through `zlib:gzip/1`, so the fixtures stay the bare `FOR1` form the grammar tests pin their
offsets against. The gzip-wrapped shape is covered twice over. `tests/stage_run.rs` reads modules
a real runtime actually wrote, and `tests/beam.rs` builds members with `flate2` — the same crate
`src/beam.rs` decompresses with, added to `dev-dependencies` for it — so that the branch is held
to the never-panic policy on a machine with no toolchain at all: a member cut short, the magic
followed by rubbish, a proptest that fixes the two magic bytes and randomises the rest, and a
small member that expands past `beam::MAX_FORM_BYTES`, which is the bounded allocation that keeps
a gzip bomb from ending the process.

They are **unstripped on purpose**. A fixture that had already been stripped could not show what
stripping is for, and `every_fixture_module_still_carries_the_debug_information_stripping_removes`
fails if anyone replaces one with a stripped copy.

The three sizes span two orders of magnitude, also on purpose. `gleam@bool.beam` is small enough
that `the_small_fixtures_chunk_table_is_exactly_this` names all fourteen chunk offsets and
lengths; `gleam@list.beam` is large enough that truncating it at every one of its 49 680 byte
offsets is a real workout for the never-panic property. Their licence is `Apache-2.0` and, since
a binary carries no SPDX header, `REUSE.toml` declares the path.

### `tests/fixtures/app/`

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

## The never-panic policy for binary parsers

Every parser in this crate reads bytes ginary did not write: a `.beam` out of somebody's build
tree, an ELF out of somebody's OTP tarball, and later a payload out of an artifact a virus
scanner may have appended to. A packaging tool that panics on a damaged file has told its user
nothing, and on the launcher path a panic is forbidden outright.

The rule is therefore uniform, and it is a rule about the *test suite* as much as about the code:
**every public entry point of a binary parser has a property test that feeds it arbitrary bytes,
and a hand-written test for each way its input can be short.** A branch a random vector cannot
reach in a lifetime of cases — the gzip wrapper `beam::form` unwraps needs two exact magic bytes
and then a decodable deflate stream — gets a property test of its own with the prefix fixed, and
hand-built inputs for its failures; a branch covered only by a toolchain-gated test is not
covered, because the machines the policy exists for are the ones with no toolchain. `src/beam.rs` and `src/elf.rs` were
the first two; `src/trailer.rs` and `src/payload.rs` joined them in A3a, with
`parse_never_panics_on_arbitrary_bytes`, `parse_never_panics_on_the_magic_followed_by_rubbish`,
`unpack_never_panics_on_arbitrary_bytes`, `read_manifest_never_panics_on_arbitrary_bytes` and
`read_manifest_never_panics_on_a_zstd_stream_of_rubbish` — the last of those being the branch a
random vector cannot reach, a well-formed zstd stream whose contents are not a tar archive.
`appfile` joins them when its property test lands.

| what is fed in | where |
|---|---|
| random byte vectors, 0 to 512 bytes | `chunks_never_panics_on_arbitrary_bytes`, `inspect_bytes_never_panics_on_arbitrary_bytes` |
| bytes that start with the magic and then do not | `chunks_never_panics_on_almost_a_beam_file`, `chunks_never_panics_on_almost_a_gzipped_beam_file`, `inspect_bytes_never_panics_on_almost_an_elf` |
| a compressed wrapper that will not unwrap | `a_gzip_member_cut_short_is_reported_rather_than_read_as_garbage`, `the_gzip_magic_followed_by_rubbish_is_not_a_module` |
| a small input that expands without end | `a_member_that_expands_past_the_limit_is_refused_rather_than_allocated` |
| every prefix of a real file, one per byte | `truncating_a_real_module_at_every_byte_offset_never_panics` |
| named truncations of a real binary | `a_truncated_binary_is_an_error_rather_than_a_panic` |
| a length field of `u32::MAX` | `a_chunk_length_of_u32_max_is_reported_rather_than_overflowing` |
| a header cut in half | `a_chunk_header_cut_in_half_is_reported_rather_than_indexed` |

A property test that only asserts "did not panic" is weak on its own, which is why each is paired
with a hand-written test asserting the *exact* error variant and its fields. The property finds
the input nobody thought of; the hand-written test says what the answer has to be.

Arithmetic is part of the rule. `offset + len` over a `u32::MAX` length must not overflow a
`usize` on a 32-bit target, and a size that is subtracted must saturate: a strip that made a file
bigger is a defect to report, not a panic.

## The malicious-archive policy

A payload is a tar archive read out of a file somebody else may have edited, and `src/payload.rs`
is the only code in ginary that writes to a path an attacker chose the name of. The rule for its
tests is therefore stricter than "the error is right":

**every rejection asserts the exact error *and* that nothing appeared outside the destination.**
`tests/payload.rs` builds a `Destination`: a temporary directory holding one `sentinel.txt` and an
empty `dest/`, and `Destination::assert_nothing_escaped` re-lists everything outside `dest` after
the failed `unpack` and requires it to be exactly `["sentinel.txt"]`. A rejection that had already
written the file it rejected satisfies the error assertion on its own, and that is the failure the
whole group exists to catch.

**The archives are written by hand.** `tests/common/payload.rs` has `RawTar` and `RawEntry`, which
lay out the 512-byte `ustar` header field by field and compute the checksum. That is not
fastidiousness: the `tar` crate refuses to *write* most of what `src/payload.rs` has to refuse to
*read*, so an archive with `../x`, an absolute path, a symlink, a hard link, a device node, a FIFO
or a `ustar` prefix cannot be produced by the library being tested with it. Eight archives are
built that way, one per rejection, plus a directory entry that must be *accepted* — a rule that
only rejects is not an allowlist.

Because those headers are hand-written, they get the same treatment `FakeOtp`'s generated `.app`
files get: `the_hand_built_archives_read_back_as_the_entries_they_were_written_as` reads every one
of them back with the `tar` crate and asserts the paths, the entry types, the `ustar` prefix join
and one entry body. A header this file wrote wrongly fails there, rather than making four other
tests pass for a reason nobody chose.

**`unpack_in` returning `false` is an error, not a skip.** The tar crate answers `false` when it
declines to write an entry into the destination and does not fail, and a silently skipped file in
an artifact is exactly the outcome this format may not have. `PayloadError::PathEscape` is that
`false`, and it is separate from `PayloadError::UnsafePath`, which is the check made *before* the
path is used at all. No archive can produce it: the tar crate answers `false` only for a `..`
component or a destination with no parent, and `UnsafePath` refuses the first before the entry is
unpacked, so every hand-built escape in `tests/payload.rs` asserts `UnsafePath`. The variant is
kept as defence in depth against a tar crate that starts declining for a new reason, and because
a variant no test can reach is a variant nobody would notice being deleted, the mapping itself is
pinned by a unit test in `src/payload.rs` — the one place a `bool` can be handed to the code
directly. `docs/format.md` records the same decision.

**A rejection leaves no completeness marker.** `unpack` writes `<dest>/ginary.json` last, after
the digest has matched, because the presence of that file is what the cache reads as "this entry
is finished". `tests/regressions/a3a_a_rejected_payload_left_its_manifest_behind.rs` asserts all
three halves of that rule: a payload that fails its digest leaves no `ginary.json`, one that
passes writes it, and a second unpack into the same destination is refused rather than allowed to
replace it. The third is why the file is created with `create_new` — every other entry is
unpacked under `set_overwrite(false)`, and one writer that overwrites makes the destination's
behaviour depend on which entry is reached first.

**Taking an entry out of the loop takes the overwrite rule with it.** Because entry 0 is read
rather than unpacked, `set_overwrite(false)` no longer applies to its name, and a payload whose
entry 2 was also called `ginary.json` planted the marker during the loop and then failed the final
`create_new` with an unattributed `AlreadyExists`. Both front-matter names are reserved at both
ends now, and `tests/regressions/a3a_a_repeated_front_entry_forged_the_marker.rs` covers the four
shapes that matter: a repeat of each name, a repeat hidden behind a `./` component — the reader
compares the path the entry would *land* on, not the raw header field — and a directory entry
carrying the manifest's name; plus the packing side, where a staging listing naming either file
is `ReservedName` rather than an artifact ginary itself could not read. The general lesson is
worth more than the fix: an entry a reader handles specially is an entry the reader's generic
defences have stopped covering, so it needs its own.

## The `Diag` sink-injection pattern

`src/diag.rs` writes to standard error and to a file, and neither is assertable from inside a test
process without either capturing global state or spawning a child. It therefore takes its outputs
as values:

```rust
pub fn with_sinks(
    debug: Option<Box<dyn Write + Send>>,
    trace: Option<Box<dyn Write + Send>>,
) -> Diag
```

`Diag::from_env` is the thin wrapper that chooses standard error and the `GINARY_TRACE` file, and
it is the only part of the module that opens anything — the same split `cache_dir::resolve` and
`doctor::gather_from` follow, one layer down. `tests/common/payload.rs` supplies `SharedSink`, a
cloneable `Write` over an `Arc<Mutex<Vec<u8>>>`, so a test holds one half and the recorder the
other, and reads back either the debug lines or the JSON objects with no environment mutation and
no child process. `tests/diag.rs` runs entirely on those sinks except for the three tests that are
*about* `from_env`: that a trace path creates its parent directories, that nothing set creates no
file, and that a trace file which cannot be opened leaves the run working.

## Fuzzing

`fuzz/` holds four `cargo-fuzz` targets, one per parser that reads bytes ginary did not write.
They are the coverage-guided half of the never-panic policy above: the proptests state the
property, the fuzzer looks for the input that breaks it.

| target | entry point | why |
|---|---|---|
| `trailer_parse` | `trailer::parse` | the 64 bytes at the end of the running executable |
| `appfile_terms` | `appfile::parse_terms` | `.app` files from a shipment and an OTP library; the parser recurses |
| `beam_chunks` | `beam::chunks` | IFF length fields, and the gzip member a stripped module is |
| `payload_read_manifest` | `payload::read_manifest` | zstd, then tar, then serde, over the payload |

```console
mise run fuzz:build      # cargo +nightly fuzz build
mise run fuzz            # each target for 30 seconds, in turn
```

**`unpack` is deliberately not a target.** It writes to disk and creates directories, so a
fuzzer would spend its time in the kernel and leave a tree behind after every crash.
`read_manifest` covers the same zstd, tar and serde layers over the same untrusted bytes;
`tests/payload.rs` covers the writing half with eight hand-built archives, which is a job for a
test that can assert *where* nothing was written rather than for a fuzzer.

**Seeds are committed and matter.** Three of the four parsers begin with a magic a fuzzer does
not guess — `GINARY\0`, `FOR1`/`BEAM`, a zstd frame header — so `fuzz/seeds/<target>/` holds one
small real input each and the `fuzz` task passes it as a second corpus directory. The generated
`fuzz/corpus/<target>` is where new inputs go and is not committed. The difference is not
marginal: `payload_read_manifest` reached 112 edges in 30 seconds without its seed and 1483 with
it.

`trailer_parse` takes `&[u8]` and splits it itself — 64 bytes of trailer, then eight of file
length — rather than deriving `Arbitrary` for a tuple, so that a seed file is exactly what the
end of a real artifact holds.

**`fuzz/` is a workspace of its own.** A libFuzzer target only builds on nightly, and a workspace
member is compiled by `cargo test`, `cargo clippy --all-targets` and `cargo deny` at the root.
The `[workspace]` table in `fuzz/Cargo.toml` stops cargo looking upwards, the root manifest does
not mention the directory, and the gates stay on the stable toolchain `rust-version` names.

**A crash is a RED test.** Minimise it with `cargo +nightly fuzz tmin <target> <artifact>`, add
the minimised input to `tests/regressions/`, watch it fail, then fix it. The artifact itself
belongs in the commit as the regression's fixture.

## Assembly scenarios

`tests/assemble.rs` builds one six-application scenario and asserts against it from every angle.
Three applications come from a `FakeShipment` and three from a `FakeOtp`, which is the smallest
tree that shows both staged layouts (`lib/<name>` and `lib/<name>-<vsn>`), both `.beam`
categories, a `priv` on each side, and every kind of file assembly refuses to copy. The runtime
carries six spare programs in its `bin` so the exclusion list has something to exclude, and
`crypto` carries one real NIF beside three pieces of junk.

The rule the whole file follows: **a test names the paths it expects, in full and in order.** An
assertion that only checks that a few expected files are present cannot see a file that should
not have been copied, and an allowlist whose failures are invisible is not an allowlist. The
`EXPECTED_TREE` constant at the top of the file is the contract; twenty-odd tests then take it
apart.

| scenario | what it pins |
|---|---|
| layout | the twenty-two paths of the staged tree, exactly, sorted |
| exclusions | a `.appup` beside an `.app` is dropped; `src`, `include`, `doc`, `examples`, `c_src` and `mibs` never appear at the top level of an application, which is the rule the module has — a `priv/mibs/*.bin` beside them *is* staged, because nothing inside `ebin` or `priv` is pruned by name |
| junk | the three removals and their exact byte counts, in path order, with the real NIF beside them untouched; `--keep-junk` records nothing and keeps all three |
| modes | every staged file's mode equals its source's, the NIF stays executable, the data file does not become one, and the listing's `mode` agrees with the tree |
| boot | a boot file naming `kernel-1.0` against a staged `kernel-11.0.3` is `BootReferencesMissingApp` naming *both*; the versions actually checked are reported |
| erts bin | a missing required binary names the path it searched; `--extra-bin heart epmd` stages six programs; an extra that is not there is an error, not a skip; every program left behind is listed with the reason `assemble::excluded_reason` gives, and an extra that *was* staged is not also listed as excluded |
| output | a non-empty `out` is refused and left untouched; an empty one is accepted; `--force` replaces rather than merges; a failure leaves neither `out` nor an `<out>.tmp-*` |
| symlinks | a link inside the application is copied as a plain file with the target's bytes; one that escapes the application directory and one that dangles are both `UnsafeSymlink` (`tests/regressions/a1c_*` add the three the first review found: an `ebin` or a `priv` that is *itself* a link out of the application, a link to a directory outside the subtree being copied, and a link that loops; `a2_a_symlinked_priv_reached_an_excluded_directory.rs` adds the half of the first of those that stayed open, an `ebin` or `priv` that is a link to `src` or into it, which is `ExcludedSymlinkTarget`, and pins that a link to a *non*-excluded sibling directory still stages) |
| accounting | the per-category totals sum to `total_bytes()`, which equals a walk of the tree with the listing excluded; nine named paths are checked against the category the size report will add them to |
| listing | `ginary.stage.json` round-trips through serde, lists every file sorted by path, never lists itself, and names the ERTS version, the release and the OTP version |
| determinism | staging the same inputs into two directories produces the same paths, the same bytes, the same modes and the same listing |

## The launch contract

`tests/common/erl.rs` holds `run_staged`, and it is not a convenience wrapper. It is a *hermetic
subset* of the launch contract ADR 0003 records, written down once so that `src/launch.rs` can be
cross-checked against it in A3: it execs `erts-<vsn>/bin/erlexec` directly, with `env_clear()`
and five variables — `ROOTDIR`, `BINDIR`, `EMU`, `PROGNAME`, `HOME` — plus an empty-directory
`PATH` and an `ERL_CRASH_DUMP` inside `HOME`, and an argument vector of
`-boot <root>/bin/no_dot_erlang -noshell +B -start_epmd false`, one `-pa` per shipment
application, `-eval "'<app>@@main':run('<app>')"` and `-extra <args...>`.

"Subset" is the load-bearing word. ADR 0003 describes a launcher that *inherits* the environment
and removes a denylist from it — `ERL_LIBS`, `ERL_FLAGS`, `ERL_AFLAGS`, `ERL_ZFLAGS`,
`ERL_OTP*_FLAGS`, `ERL_ROOTDIR`, `ERL_EPMD_PORT` — and that supplies `HOME` and `ERL_CRASH_DUMP`
only when the user has not. A test may not inherit the developer's environment and still assert
anything, so this helper clears it and sets both unconditionally; the ADR's optional `+fnu`,
`-args_file` and `-config` arguments are absent too, because no staged tree carries them yet. A
`LaunchPlan` that agrees with this function is therefore not finished: the denylist and the
"only when unset" rules need their own tests over an inherited environment. Inside the overlap,
a difference between the two is a defect in one of them, and `tests/stage_run.rs` — which
actually boots what `stage` wrote — is what says which.

Two details of the overlap are load-bearing rather than cosmetic. `PATH` is an *empty directory*
and not an absent variable, because a program that finds no `PATH` searches a system default
rather than nothing. And `ERL_CRASH_DUMP` points into `HOME`, which is why `tests/stage_run.rs`
asserts that a crashed run left no `erl_crash.dump` in the working directory: dropping a
megabyte of dump into whatever directory the user was standing in is the kind of thing a
packaged application must never do.

Neither helper that starts a real program waits forever. `tests/common/bounded.rs` runs both of
them — `gleam export erlang-shipment` and the staged `erlexec` — with stdin on the null device,
both pipes drained by threads of their own, and a deadline: `fixture::EXPORT_BUDGET` (180 s) and
`erl::RUN_BUDGET` (60 s). A child that outlives its budget is killed and named in the panic,
because the one place in the suite that boots a whole BEAM is the last place that should be able
to hang a test binary with no diagnosis.

## Snapshots

Textual output is asserted with `insta`, and the `.snap` files under `tests/snapshots/` are
committed and reviewed like any other assertion. Twelve exist:

| snapshot | what it pins |
|---|---|
| `appfile__nested_term_display.snap` | `Term`'s re-serialisation of the whole `nested.app` term |
| `appfile__parse_error_messages.snap` | the sentences the two invalid fixtures produce |
| `cli__appfile_parse_table.snap` | the table `ginary appfile parse` prints |
| `closure__explain_table.snap` | `closure::explain` over the six-application scenario |
| `closure__app_not_found_message.snap` | the whole `AppNotFound` message, hint included |
| `closure__shadowed_otp_application_warning.snap` | the warning an application in both trees produces |
| `cli__closure_explain_table.snap` | what `ginary closure --explain` prints, footer included |
| `assemble__stage_explain_table.snap` | what `ginary stage --explain` prints over the six-application scenario |
| `cli__beam_chunks_table.snap` | all fourteen chunks of `gleam@bool.beam` and the `debug_info` line |
| `strip__report_table.snap` | the three-line strip table when both halves ran |
| `strip__report_table_when_nothing_ran.snap` | the same table when one half found nothing and the other was skipped |
| `report__size_report_text.snap` | the size table, the `needs:` line and the warnings block, over a synthetic report |
| `manifest__canonical_manifest_json.snap` | the wire field order of `ginary.json`, which is the struct's declaration order and not the alphabetical order `serde_json::Value` imposes |

A snapshot is a contract, not a recording. `cargo insta review` is for reviewing a *deliberate*
change to output; accepting a snapshot to make a red test pass is the same defect as weakening an
assertion.

## Planned infrastructure

`tests/common/` already holds `tools.rs`, `fake_otp.rs`, `snapshot.rs`, `script.rs`,
`fixture.rs`, `erl.rs`, `bounded.rs` and `payload.rs`, described above. Still to come:

- **`Artifact`** — run `ginary build` once per test binary behind a `OnceLock`, then run the
  artifact under a scrubbed environment and return the exit status, stdout, stderr, the cache
  directory and the trace as structured data. `FixtureProject` and `run_staged` are the halves
  of it that A1c needed and therefore already exist.

One more fixture:

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
  validation. The BEAM and ELF readers already have theirs; see the never-panic policy above.
- **Fuzzing** — four targets exist; see the fuzzing section above. `elf_inspect` is the one from
  the plan's list that does not, because `object` is doing the parsing there and fuzzing it would
  measure that crate rather than this one.
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
| `mise run fuzz:build` | `cargo +nightly fuzz build` | builds the four targets; nightly, and outside the gate |
| `mise run fuzz` | each target for 30 s, in turn | nightly; see the fuzzing section |

The coverage gate is `--fail-under-lines 90`. The 80% branch floor is not enforced yet: branch
coverage needs a nightly `-Z coverage-options=branch` build, and this crate is measured on
stable. When that changes, the floor moves from prose into the `cov` task.

`cargo-fuzz` needs a nightly toolchain to build a target, which is why `fuzz/` is a workspace of
its own; `mise run fuzz` and `mise run fuzz:build` are the two tasks, and neither is part of
`mise run check`. `cargo-insta` is
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
