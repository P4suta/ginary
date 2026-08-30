<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# A3b — the launcher

Date: 2026-08-31 · Status: RED and GREEN complete, awaiting review

## Housekeeping

One low finding carried over from the A3a review, closed before any A3b product code was
written. The working tree was clean at `d73e24f` apart from the sandbox shims, and this section
touches only `src/payload.rs`, the regression suite and the two documents that describe the
reserved names.

### 1 — the reserved-name check was exact-match only (low)

A3a reserved `ginary.json` and `ginary.index.json` at both ends of the payload format, but both
checks compared the *whole* path for equality with the reserved name. A staging root holding a
**directory** of that name therefore walked through both of them:

- `pack` accepted a listing naming `ginary.json/nested.txt` and wrote it as entry 2 or later,
  producing an artifact that ginary's own reader always refuses — a build failure deferred to
  the machine that runs the binary.
- `unpack` sent the same entry to `unpack_in`, which creates the parents of every entry, so
  `<dest>/ginary.json` was created as a *directory* before the manifest's final `create_new`
  failed on it with an unattributed `Io(AlreadyExists)`. That is exactly the unowned failure,
  and the occupied completeness marker, that reserving the names was meant to end: the A3a fix
  had closed the door and left the frame around it open.

**RED.** `tests/regressions/a3b_a_reserved_name_covered_only_the_exact_path.rs`, written first
and watched failing on assertions rather than on a compile error — 5 failed, 0 passed:

```text
an_entry_under_a_directory_named_like_the_manifest_is_refused
  expected DuplicateEntry, got Io(Os { code: 17, kind: AlreadyExists, ... })
an_entry_under_the_manifest_behind_a_current_directory_component_is_refused
  expected DuplicateEntry, got Io(Os { code: 17, kind: AlreadyExists, ... })
an_entry_under_a_directory_named_like_the_index_is_refused
  expected DuplicateEntry, got Io(Custom { kind: NotADirectory, ... })
packing_a_directory_named_like_the_manifest_is_refused
  ginary may not write an artifact its own reader refuses: Packed { len: 1134, ... }
packing_a_directory_named_like_the_index_is_refused
  ginary may not write an artifact its own reader refuses: Packed { len: 1140, ... }
```

The three unpack shapes are `ginary.json/nested.txt`, `./ginary.json/nested.txt` — the reader
compares the path the entry would *land* on, so the `./` component must not hide the name — and
`ginary.index.json/nested.txt`. The two pack shapes stage a real directory of each reserved name
with a file inside it and list it.

**GREEN.** `src/payload.rs` grows one helper, `reserved_first_component(path) -> Option<usize>`,
which splits on `/` and matches only the first component against `RESERVED_NAMES`; both
`check_not_reserved` (unpack) and `check_no_reserved_names` (pack) now go through it. The
`DuplicateEntry` payload changed with it: `name` now carries the path the entry would land on
rather than the bare reserved name, and both messages say *first path component*, so an operator
reading the error sees the offending path and not a name the archive never literally contained.

The destination assertion is worth stating precisely, because the obvious form of it is wrong:
after a refusal the destination holds `ginary.index.json` and nothing else. Entry 1 is the one
front-matter entry that *is* unpacked, so it is legitimately on disk by the time entry 2 is
rejected; what must be absent is `ginary.json` and anything under either reserved name.

**Gates.** `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features -D warnings`,
`cargo test`, `RUSTDOCFLAGS=-Dwarnings cargo doc --no-deps` and `cargo deny check` all clean;
`tests/regressions` is 47 passed, 0 failed.

Documentation followed the code: `docs/format.md` states the first-component rule where it
stated the exact-name one and records the change under `### v1, milestone A3b`, and
`docs/dev/testing.md` gains the lesson next to the A3a one it corrects — a name check that
compares the whole path is a name check with a hole in it.

## RED

The launcher's tests, fixtures and helpers, written before any of it exists. Six modules are
declared with their whole public API and no behaviour — every body is a stub marked
`// RED: replaced in GREEN` that returns an explicit error or an empty value, so the crate
compiles, the gates are clean, and every new test fails on an *assertion* rather than on a
missing symbol.

### What was added

| file | what it holds |
|---|---|
| `src/error.rs` | `LauncherError`, the five exit-code constants, `exit_code`, `hint`, `report`, `install_panic_hook` |
| `src/selfexe.rs` | `open_self` |
| `src/cache.rs` | `Env`, `Origin`, `CacheDirs`, `resolve`, `fallback_root`, `prepare`, `current_uid`, `sweep`, `ensure_extracted`, `clean` |
| `src/launch.rs` | `LaunchPlan`, `PreflightIssue`, `plan`, `preflight`, `exec`, `supervise`, the removal lists |
| `src/launcher.rs` | `run`, the `GINARY_CMD` constants |
| `src/fault.rs` | `point`, behind the `fault-injection` feature |
| `tests/common/artifact.rs` | `SyntheticArtifact`, the `erlexec` stub, the scrubbed `Runner`, `Run`, `read_trace` |
| `tests/launch.rs` | 29 tests over the pure plan and preflight |
| `tests/cache.rs` | 19 tests over resolution, extraction, the sweep and `clean` |
| `tests/launcher.rs` | 36 tests running real artifact processes |
| `tests/artifact_real.rs` | 3 toolchain-gated tests over one real, hand-assembled artifact |
| `tests/snapshots/launch__launch_plan_canonical.snap` | the argument vector and environment difference, written by hand rather than accepted from a run |

`src/main.rs` is deliberately **not** touched. The trailer dispatch is GREEN work, so every
artifact test currently runs the command line half and fails saying so; wiring the dispatch now
would have been production code written before its test failed.

### The RED run

`cargo test --features fault-injection --no-fail-fast`: **109 new tests fail, 0 previously
passing tests regress.** Every pre-existing target is still green (`appfile` 51, `assemble` 53,
`beam` 34, `cli` 32, `closure` 34, `diag` 13, `elf` 16, `manifest` 23, `otp` 29, `payload` 39,
`regressions` 47, `report` 13, `smoke_cli` 6, `stage_run` 12, `strip` 29, `trailer` 13, doc 1).

```text
--lib                24 failed, 118 passed
tests/launch.rs      28 failed,   1 passed
tests/launcher.rs    36 failed,   0 passed
tests/cache.rs       19 failed,   0 passed
tests/artifact_real.rs 2 failed,  1 passed   (gleam + erl + strip present)
```

Representative failures, one per area:

```text
error::tests::a_trailer_failure_is_122
  assertion `left == right` failed
    left: 1
   right: 122

selfexe::tests::open_self_opens_the_test_binary
  open_self must open the running test binary

fault::tests::a_point_and_an_action_split_on_the_first_colon
  assertion `left == right` failed
    left: None
   right: Some(("after-extract", "pause"))

launcher::tests::the_three_commands_are_recognised
  assertion `left == right` failed
    left: Err("directory")
   right: Ok(Directory)

tests/launch.rs the_launcher_sets_rootdir_bindir_emu_and_progname
  the canonical manifest must produce a plan: ginary: not implemented

tests/launch.rs a_launch_path_that_escapes_the_root_is_refused
  assertion `left == right` failed: an unusable manifest is a format failure, not a cache one
    left: 1
   right: 122

tests/cache.rs a_cold_cache_extracts_into_the_key_directory
  a cold cache must extract: Cache { path: ".../cache/hello/51aae92d1d6e26ff",
    source: Custom { kind: Other, error: "cache::ensure_extracted is not implemented" } }

tests/launcher.rs the_runtime_gets_rootdir_bindir_emu_and_progname
  assertion `left == right` failed: the run did not reach the runtime
  --- stderr ---
  Usage: hello <COMMAND>
    left: 2
   right: 7

tests/launcher.rs the_application_owns_help
  --- stdout ---
  Usage: hello <COMMAND>
  ...
    left: 0
   right: 7

tests/artifact_real.rs a_real_artifact_runs_a_gleam_program_with_no_erlang_on_the_machine
  `-extra` did not reach init:get_plain_arguments/0:
  --- stderr ---
  error: unrecognized subcommand '3'
```

The last two are the ones worth reading twice: a packaged application is currently answering
`--help` and rejecting its own arguments as ginary subcommands, which is precisely the failure
the launcher exists to prevent.

### The thirteen new tests that pass in RED, and why that is correct

A test that pins a *declaration* rather than a behaviour passes the moment the declaration
exists. Each of these is a contract that a later change could silently break, so they are kept
rather than contrived into failing:

```text
error::tests::the_five_codes_are_the_documented_ones
error::tests::every_message_begins_with_the_ginary_prefix
cache::tests::a_snapshot_answers_for_the_variables_it_holds
cache::tests::every_origin_names_its_provenance
cache::tests::the_entry_path_is_root_app_key
fault::tests::an_empty_spec_arms_nothing
fault::tests::nothing_is_armed_without_the_feature
launch::tests::the_removed_names_are_the_documented_six
launch::tests::the_checked_binaries_are_the_documented_three_plus_the_program
launch::tests::a_preflight_issue_names_the_file_and_the_fault
launcher::tests::anything_else_is_a_usage_error_carrying_the_value
launcher::tests::the_usage_line_names_all_three
tests/launch.rs a_preflight_failure_is_not_a_launcher_error
tests/artifact_real.rs the_real_artifact_is_one_file_and_its_size_is_recorded
```

`every_message_begins_with_the_ginary_prefix` passes only because the RED stub's placeholder
happens to carry the prefix; it becomes a real assertion the moment the messages do. Two of
them — `nothing_is_armed_without_the_feature` and
`anything_else_is_a_usage_error_carrying_the_value` — assert that something is **not**
recognised, which a stub satisfies by doing nothing; they are the guard rails that keep GREEN
from over-recognising.

### Notes on three tests that could not be written the obvious way

**The magic test is two claims in one.** `the_magic_is_what_decides_the_mode` asserts that a
flipped magic reaches the command line *and* that an intact artifact does not. Split in two, the
first half would pass in RED for the wrong reason — the binary is command-line-only today — and
would have been RED evidence that proves nothing.

**The plan snapshot is hand-written.** `tests/snapshots/launch__launch_plan_canonical.snap` was
written from the specification, not accepted from a run, because there is no run to accept from.
It therefore fails in RED like every other assertion, and GREEN has to produce the argument
vector the format document describes rather than whatever it happens to produce first.

**Trace records carry JSON inside JSON.** `Diag`'s `kv` values are strings, and the `exec`
record has to hold the whole argument vector for a launch to be reproducible from a trace. The
tests therefore require `kv.argv`, `kv.env_set` and `kv.env_remove` to be JSON arrays *encoded
as strings*, which round-trips exactly and keeps the trace one object per line.

### The first real artifact size

`tests/artifact_real.rs` already assembles a whole artifact — export, closure, stage, strip,
pack at zstd 19, append — since every one of those phases is green from A1 and A2. Only the
launching half fails. Its size, measured on the development machine with OTP 29.0.5 and the
`hello_ffi` fixture:

| part | bytes |
|---|---|
| payload (214 staged files, tar + zstd −19) | 4,102,381 |
| trailer | 64 |
| stub, `--release` (`lto = "thin"`, `strip = "symbols"`, `codegen-units = 1`) | 1,596,816 |
| **artifact** | **5,699,261** (5.44 MiB) |

The test itself runs against the *debug* stub, because `CARGO_BIN_EXE_ginary` is whatever
profile the suite was built with, and it therefore asserts only the geometry —
`stub + payload + 64` — and a loose ceiling. A size *gate* belongs in the release checks, not in
a test that would start failing on the day a new ERTS ships.

### Not in this phase

`docs/dev/debugging.md` still marks `GINARY_DEBUG`, `GINARY_TRACE`, `GINARY_SUPERVISE`,
`GINARY_CMD` and `GINARY_FAULT` as planned, `docs/dev/architecture.md` has no launcher sequence
diagram yet, and there is no `docs/adr/0008-launcher-exit-codes-and-env-protocol.md`. All three
describe behaviour that does not exist in this revision, and a document that claims a switch
works before it does is worse than one that says it is planned. They land with GREEN.

## GREEN

Every `// RED: replaced in GREEN` stub is gone and the whole suite is green. The launcher exists:
a hand-assembled artifact — ginary binary, payload, trailer — extracts its runtime, builds an
argument vector, scrubs an environment and `execve`s a real ERTS, and the `hello_ffi` fixture runs
Gleam code on a machine where the launcher never looked at `PATH`.

### What was implemented

| module | what it now does |
|---|---|
| `src/error.rs` | the five exit codes, one-line `ginary: ` diagnostics that render the whole cause chain, the `hint: ` second line, and a panic hook that prints one line and exits 121 |
| `src/selfexe.rs` | `/proc/self/exe` first — an artifact renamed or unlinked while it starts is still readable by inode — and `current_exe` as the fallback |
| `src/cache.rs` | resolution and creation with the `${TMPDIR}` fallback and its one warning, the sweep, the ten extraction steps, and `clean` |
| `src/launch.rs` | the pure `plan`, `preflight`, `exec` with its two hints, and `supervise` |
| `src/launcher.rs` | `mode`, `run`, the `GINARY_CMD` dispatch, and the one preflight repair |
| `src/fault.rs` | `GINARY_FAULT` read once, three actions, compiled out without the feature |
| `src/main.rs` | the mode branch: clap is never constructed on the launcher path, and a damaged artifact never becomes the help text |
| `src/cli.rs` | `ginary cache dir` and `ginary cache clean [--app NAME]`, both with `--json` |
| `src/cache_dir.rs` | now a projection of `cache::resolve`, so the precedence has one implementation |

Six decisions inside that are worth reading twice.

**`chain()` in the error type.** `TrailerError` and `PayloadError` name the layer that failed and
keep the detail in a `source`, which is right for a library and wrong for a launcher: `the
payload's manifest cannot be used by this ginary` does not tell an operator which version they
have. `Display for LauncherError` therefore walks the chain and joins it with `: `, on one line.
That is what makes `a_manifest_format_version_this_build_cannot_read_exits_122` able to assert on
the *version number* rather than on a layer name.

**Every `ManifestError` is 122, not only `UnsupportedVersion`.** A manifest whose `launch.pa[0]`
leaves the extracted root is not a corrupt payload — the bytes hashed correctly — it is a
document this build will not act on, which is the same fault the trailer's version byte reports.

**The lost-race fault suppresses the cache hit as well.** `rename:eexist` has to *reach* the
rename, and the test that arms it necessarily runs against a cache the winner has already
filled. So the point is read once at the top of `ensure_extracted` and, when it is armed, the
step-1 hit is skipped: the process behaves like one that started cold and was overtaken. Finding
this also found a real bug — step 1's "a `<key>` without a manifest is moved aside" branch was
written as `target.exists()` and so deleted a *complete* entry whenever the hit was skipped. It
now checks for the missing manifest explicitly, which is what ADR 0005 says and what the launcher
needs anyway: nothing may move a complete entry aside.

**`unpack:corrupt` is a reader, not a patch to `src/payload.rs`.** The payload module is A3a's and
is out of this milestone's scope, so the fault sits in `cache.rs` as a `Corrupting<R>` adapter
that flips the first byte the unpacker sees. The artifact on disk is intact and the bytes the
reader sees are not, which is the fault a bit flip in the page cache produces and the one the
digest exists to catch. Without the feature the adapter has no branch at all.

**`rustix`, and the crate keeps `#![forbid(unsafe_code)]`.** `getuid` and `syncfs` are the two
system calls the launcher cannot do without and the standard library does not expose. The RED
report left the choice open — a dependency, or relaxing the crate-level `forbid` — and the
dependency wins: `rustix` is already in the tree through `tempfile`, uses its own raw syscall
backend on Linux so nothing new is dynamically linked, and `forbid(unsafe_code)` is a promise
about the launcher path that is worth more than one crate. `syncfs` is `cfg(target_os = "linux")`;
elsewhere the per-file `sync_all` fallback is the only path, which is what the `sync` phase
records.

**`supervise` takes the crash dump directory as a third parameter**, as the RED report proposed.
`ERL_CRASH_DUMP` is set only-if-unset, so the plan does not always carry the directory, and a
supervisor that reported "no dump" because it was looking in the wrong place would be worse than
one that reported nothing.

### GREEN / test corrections

One test was corrected, and one test helper was made robust. Neither weakens an assertion.

**`a_truncated_artifact_exits_122` could not have passed as written, and the helper was wrong
rather than the launcher.** `SyntheticArtifact::truncate` shortened the artifact from its *end*,
which takes the trailer with it: the last 64 bytes of the shortened file are one payload byte
followed by the first 63 bytes of the trailer, and their first seven bytes are not `GINARY\0`.
`docs/format.md` rule 2 is explicit about what that is — "If `magic[0..7]` does not equal
`GINARY\0`, there is no trailer. The binary is the ginary CLI and parses `argv`" — so the run
exited 2 through the command line, which is exactly what `the_magic_is_what_decides_the_mode`
already covers. Making the launcher exit 122 there would have meant abandoning rule 2 and turning
every plain `ginary` invocation into a search for a trailer that is not at the end.

The fault that *is* worth a test is the one that still carries a trailer and no longer matches
it: a copy that stopped early, an installer that wrote a short file. `truncate` now takes its
bytes out of the payload and re-appends the trailer, so the file is short, the trailer is honest,
and `TrailerError::Geometry` reports it — the same error as `break_geometry` arrived at from the
opposite direction, one rewriting the recorded length and the other the file. The assertion, exit
122 with exactly one `ginary: ` line, is unchanged.

**`Runner::spawn` and `Runner::output` now retry while the kernel answers `ETXTBSY`.** Two or
three launcher tests failed per full-suite run with `Text file busy`, in tests that had nothing to
do with truncation and in different ones each time. It is rust-lang/rust#39189 and it is a
property of the harness: cargo runs these tests as threads of one process, a `fork` for one
test's spawn inherits every descriptor another thread has open — including the one it is writing
the next 46 MB artifact through — and until that child reaches `execve` and `O_CLOEXEC` closes the
inherited copy, the file the other thread has just finished writing still has an open writer.
Nothing about the artifact is wrong. The retry is bounded at ten seconds, after which `ETXTBSY` is
reported like any other spawn failure; serialising the suite would have hidden a race the launcher
does not have. Three consecutive full-suite runs after the change: 23 targets, all ok.

### The CLI half of the cache

The spec asked for `cache::clean` to be wired now rather than left for A4, so `ginary cache dir`
and `ginary cache clean [--app NAME]` exist, both with `--json`, and six tests in `tests/cli.rs`
cover them. `cache dir` prints the provenance — `cache dir: /srv/c (from GINARY_CACHE_DIR)` —
because a path without the rule that produced it does not say why it is that path, and because it
is now the same resolution a packaged application makes: `cache_dir::resolve` was rewritten as a
projection of `cache::resolve`, with the one deliberate difference that the builder answers
`Unresolved` where the launcher falls back to `${TMPDIR:-/tmp}/ginary-<uid>`. A build tool that
silently wrote into `/tmp` would be a build tool nobody could find the output of. Every existing
`cache_dir` and `doctor` test still passes unchanged, which is the evidence that the projection is
faithful.

### Concurrency and the fault points

The three properties that cannot be reached by feeding the launcher a different artifact:

- **Eight real processes on a cold cache.** All eight exit 7, `<cache>/<app>` holds exactly one
  name, and it is the key. Seven of them lose the rename with `ENOTEMPTY`, verify the winner's
  `ginary.json`, remove their own tree and use the winner's — which is why "no residue" is a
  separate assertion from "one entry".
- **`after-extract:pause` + `SIGKILL`.** The killed run leaves `.<key>.tmp-<pid>` behind, and the
  next run's trace holds `cache_sweep removed=1` before it extracts. The sweep reads
  `/proc/<pid>`, so a tree whose process is *alive* is kept: `tests/cache.rs` pins all three cases
  — a dead pid, a live one (a real `sleep 30` child), and this process's own id, which is a
  leftover of a previous run of that id by definition.
- **`rename:eexist`.** The trace's last `rename` record says `reused=true` and the loser's tree is
  gone.

`unpack:corrupt` is the fourth: exit 123, and no `<key>` directory for a later run to trust.

### Diagnostics

`GINARY_DEBUG=1` prints `read_manifest`, `cache_sweep`, `cache_tmp`, `extract`, `chmod`, `sync`,
`rename` (or `cache_hit`), `preflight_retry` and `exec`. `GINARY_TRACE` writes the same events as
JSON Lines, and the `exec` record carries `program`, `argv`, `env_set` and `env_remove` — the
three lists as JSON arrays encoded in strings, which round-trips exactly and keeps the trace one
object per line. The test asserts *reproduction completeness* rather than presence: every `-pa`
path the manifest names has to be in the recorded argv, because a trace that summarised the launch
would be a trace nobody could replay.

A successful launch is silent. `nothing_is_printed_when_neither_switch_is_set` asserts standard
error is empty, because the application owns it from the moment `execve` returns.

### The first real single-file artifact

`tests/artifact_real.rs` now runs end to end on this machine, with `gleam`, `erl` and `strip`
present: the fixture exports, resolves, stages, strips, packs at zstd −19 and is appended to a
ginary binary, and the result runs with `env_clear`.

- `args=3 a b` reaches `init:get_plain_arguments/0`, so `-extra` is doing its job;
- `code:priv_dir/1` finds the extracted `priv`;
- the application starts in the caller's working directory — the launcher never `chdir`s;
- its exit code, 3, survives `execve`;
- `--crash` is exit 1 with `runtime error` on standard error, and the working directory is left
  clean: `ERL_CRASH_DUMP` points into `<cache>/hello_ffi/`.

| part | bytes |
|---|---|
| payload (214 staged files, tar + zstd −19) | 4,102,502 |
| trailer | 64 |
| stub, `--release` (`lto = "thin"`, `strip = "symbols"`, `codegen-units = 1`) | 2,131,112 |
| **artifact** | **6,233,678** (5.94 MiB) |

The release stub grew from 1,596,816 bytes in A3a to 2,131,112: the launcher, `rustix` and the
`cache`/`launch`/`launcher`/`error` modules are 534 KB of it. The gated test itself runs against
whichever profile the suite was built with — 42 MB of debug binary on this machine — so it asserts
only the geometry, `stub + payload + 64`, and a loose ceiling. A size *gate* belongs in the release
checks, not in a test that would start failing on the day a new ERTS ships.

### The gates

```text
cargo fmt --all -- --check                                    clean
cargo clippy --all-targets --all-features -- -D warnings      clean
cargo test --features fault-injection                         23 targets, 0 failed
cargo test                                                    23 targets, 0 failed
RUSTDOCFLAGS=-Dwarnings cargo doc --no-deps                   clean
cargo deny check                                              advisories ok, bans ok, licenses ok, sources ok
```

Per target, with `--features fault-injection`:

```text
--lib                   142 passed      tests/launch.rs          29 passed
tests/appfile.rs         51 passed      tests/launcher.rs        36 passed
tests/artifact_real.rs    3 passed      tests/manifest.rs        23 passed
tests/assemble.rs        34 passed      tests/otp.rs             29 passed
tests/beam.rs            32 passed      tests/payload.rs         39 passed
tests/cache.rs           19 passed      tests/regressions.rs     47 passed
tests/cli.rs             59 passed      tests/report.rs          13 passed
tests/closure.rs         34 passed      tests/smoke_cli.rs        6 passed
tests/diag.rs            13 passed      tests/stage_run.rs       12 passed
tests/elf.rs             16 passed      tests/strip.rs           29 passed
                                        tests/trailer.rs         13 passed
                                        doc-tests                 1 passed
```

`tests/cli.rs` is 59 rather than 53 because of the six new `ginary cache` tests. Nothing else
changed count, and nothing that passed before this milestone fails now. (The RED section above
records `cli 32`, `assemble 53` and `beam 34`; the measured counts are 53, 34 and 32 — two of
those three numbers were transposed when that table was written. Nothing in either phase depends
on them.)

### Documentation

`docs/dev/debugging.md` no longer says "planned" for anything that works: `GINARY_CACHE_DIR`,
`GINARY_DEBUG`, `GINARY_TRACE`, `GINARY_SUPERVISE`, `GINARY_CMD`, `GINARY_ERL_FLAGS` and
`GINARY_FAULT` are each described by what they now do, the exit-code table 121 to 125 says what to
look at for each number, and "Reproducing a launch by hand" is a worked `jq` transcript over a real
trace rather than a sketch. `docs/dev/architecture.md` gains the launcher sequence — every step
from `execve` to `execve`, with each numbered exit on its own dotted branch — and marks the A3a and
A3b modules as existing. `docs/adr/0008-launcher-exit-codes-and-env-protocol.md` records why the
codes start at 121 (everything below is the shell's or the application's), why a manifest version is
122 rather than 123, why the panic hook exists, and why maintenance travels in `GINARY_CMD` instead
of in `argv`.

## Fix round 1

Twelve review findings: one high, six medium, five low. All twelve are fixed. Every finding
marked behavioural got its test first, and the test was watched failing before the fix went in.
Four of the findings are coverage gaps rather than defects — the behaviour was already right and
nothing checked it — so RED for those was produced by removing the production behaviour the new
test claims to pin, watching it fail, and putting it back. That is recorded below for each.

### 1 (high) `cache::clean --app` deleted any directory the caller could name

`cache::clean` joined the `--app` value onto the cache root and called `remove_dir_all` on the
result. An absolute value replaces the whole path; `..` walks out of the root. `src/cli.rs`
passed clap's `Option<String>` straight through, and the only test used `--app hello`.

Test: `tests/regressions/a3b_cache_clean_app_escaped_the_root.rs`, both through the library and
through the built binary.

```text
---- a3b_cache_clean_app_escaped_the_root::clean_refuses_an_application_name_that_is_not_one_component stdout ----
panicked at tests/regressions/a3b_cache_clean_app_escaped_the_root.rs:60:13:
`--app /tmp/.tmpW0vIyu/precious` must be refused rather than removing a directory

---- a3b_cache_clean_app_escaped_the_root::the_command_line_refuses_it_too_and_leaves_the_directory stdout ----
panicked at tests/regressions/a3b_cache_clean_app_escaped_the_root.rs:107:10:
Unexpected success
command=`env -i GINARY_CACHE_DIR="/tmp/.tmpgeFQ7J/cache" ".../target/debug/ginary" "cache" "clean" "--app" "/tmp/.tmpgeFQ7J/precious"`
code=0
stdout=```
removed /tmp/.tmpgeFQ7J/precious
total: 1 directory, 0 bytes
```
```

Fix: `cache::check_app` — one path component or nothing — reusing `manifest::check_name`, which
is now `pub(crate)`. It is called by `clean` and by `ensure_extracted`, so both the CLI's value
and a library caller's are checked at the moment before the join. The refusal is
`LauncherError::Cache` (124), because the caller's next act would have been to write under that
root. `ginary cache clean` checks the value itself first, through `cache::is_app_name`, so a
mistake made at a terminal reads as one line rather than as a cache that is unusable; both
spellings share the sentence, `cache::AppNameRefusal`.

```console
$ ginary cache clean --app ..
error: `..` is not an application name: it must be one path component, so that it cannot name a
directory outside the cache
```

### 2 (medium) the manifest's `app` was interpolated into paths unchecked

`LaunchSpec::validate` checked `launch.program`, `launch.bindir`, `launch.boot` and every
`launch.pa[i]`. Nothing checked `Manifest::app`, which the launcher uses exactly the same way:
`CacheDirs::app_dir` is `root.join(app)`, and `ensure_extracted` creates that directory, chmods
it 0700 and renames an entry into it.

Tests: five in `tests/manifest.rs` over the new `Manifest::validate`, and
`tests/regressions/a3b_the_manifest_app_was_not_a_name.rs` end to end.

```text
---- an_application_name_that_walks_out_of_the_cache_is_refused stdout ----
panicked at tests/manifest.rs:416:10:
an application name that is not one path component is refused: ()
(and the same for `a/b`, `/etc`, `.` and ``)

---- a3b_the_manifest_app_was_not_a_name::an_application_name_that_is_not_one_component_exits_122_and_creates_nothing ----
panicked at tests/regressions/a3b_the_manifest_app_was_not_a_name.rs:41:5:
assertion `left == right` failed: a manifest this build must not act on is a format failure
  left: 7
 right: 122
```

Exit 7 is the stub's: the artifact ran, out of a cache directory beside the one it was given.

Fix: `Manifest::validate` checks `app` with `check_name` and then the launch spec, and it is what
`launcher::read_manifest`, `cache::extract_into` and `launch::plan` now call. Reading the
manifest is where the check lands first, so a hostile `app` is 122 before any directory exists —
the bytes are well formed and the *format* is wrong, the same fault the trailer's version byte
reports.

### 3 (medium) the `${TMPDIR:-/tmp}/ginary-<uid>` root was created and trusted

The module documentation said a directory another user owns is not one this process may trust;
the implementation was `create_dir_all`, which succeeds on a pre-existing directory whatever its
owner and mode and follows a symlink. On a shared machine, whoever wins the race to create
`/tmp/ginary-<victim-uid>` owns the parent of the tree the launcher extracts programs into and
then executes them from.

Tests: three in `tests/cache.rs`.

```text
---- a_fallback_root_somebody_else_may_write_to_is_refused ----
panicked at tests/cache.rs:540:6:
a world-writable fallback root must be refused: CacheDirs { root: ".../tmp/ginary-1000", origin: Fallback, is_fallback: true }

---- a_symlink_in_the_place_of_the_fallback_root_is_refused ----
panicked at tests/cache.rs:570:6:
a symlinked fallback root must be refused: CacheDirs { root: ".../tmp/ginary-1000", origin: Fallback, is_fallback: true }

---- a_fallback_root_this_process_owns_is_created_private ----
assertion `left == right` failed: the shared-directory fallback must be private to its owner
  left: 509
 right: 448
```

509 is 0o775, the umask's answer; 448 is 0o700.

Fix: `cache::create_fallback_root`. `create_dir` with an explicit 0700 when this process makes
it, and when it is already there a `symlink_metadata` check — a real directory, owned by `uid`,
with no group or other write bit — or `LauncherError::Cache`. Both entries into the fallback
(nothing set, and the primary refused) go through it.

### 4 (medium) the panic hook was never exercised

`main` installs a hook that turns a launcher panic into one `ginary: internal error (this is a
bug in ginary): ` line and exit 121. Nothing triggered it, so its installation, its exit code and
its backtrace suppression were all unasserted: deleting `install_panic_hook()` from `main` broke
no test.

The behaviour was already correct, so RED was made by deleting that line and running the new
test, `a_panic_on_the_launcher_path_is_one_line_and_121`:

```text
---- a_panic_on_the_launcher_path_is_one_line_and_121 stdout ----
assertion `left == right` failed: a bug in ginary is a ginary failure, not an application exit code
thread 'main' (3155896) panicked at src/fault.rs:64:24:
GINARY_FAULT=launcher:panic
  left: 101
 right: 121
```

101 is the Rust default, and the backtrace note is exactly what a user must never see. The line
went back in and the test passes. The trigger is a fifth fault point, `launcher:panic`, fired
first thing in `launcher::run`; like every other point it is compiled out without
`--features fault-injection`, so the one `panic!` this adds cannot exist in a release artifact.

### 5 (medium) two supervised behaviours had no test

`supervise` turns a signal into `128 + signo` and reports a crash dump that appeared during the
run. Neither was reached by any test: the `erlexec` stub could only exit.

The stub learned `--signal N` and `--dump`, and three tests went in. RED by breaking both
behaviours — `(None, Some(_)) => u8::MAX` and the slogan report behind `if false`:

```text
---- a_supervised_child_killed_by_a_signal_exits_128_plus_the_signal ----
assertion `left == right` failed: a parent has an exit code and nothing else with which to report a signal
  left: 255
 right: 137

---- a_crash_dump_written_during_a_supervised_run_is_reported ----
assertion `left == right` failed: the slogan is the one line a supervised crash is worth
  left: []
 right: ["ginary: Slogan: init terminating in do_boot (ginary test stub)"]
```

The third test is the other side of the `before` comparison: a dump planted before the run is not
this run's news, is not reported, and is left with its modification time untouched.

### 6 (medium) `hint_for` was untested

The `ENOENT`/`EACCES` hint selection — the launcher's one piece of actionable operator advice —
had no test at all; only the two constants' rendering did. Inverting either guard passed every
gate.

Five unit tests in `src/launch.rs` and one process-level test. RED by inverting the `ENOENT` guard
and by dropping the `hint_for` call from `exec`:

```text
---- launch::tests::enoent_from_a_program_that_is_on_disk_is_about_the_loader ----
  left: None
 right: Some("the runtime is dynamically linked against glibc; ...")

---- launch::tests::enoent_from_a_program_that_is_not_there_says_nothing ----
  left: Some("the runtime is dynamically linked against glibc; ...")
 right: None

---- a_runtime_whose_interpreter_is_missing_exits_125_with_a_hint ----
assertion `left == right` failed: the failure and its hint, and nothing else:
["ginary: cannot start .../erts-17.0.5/bin/erlexec: No such file or directory (os error 2)"]
  left: 1
 right: 2
```

The process-level test rewrites the extracted `erlexec` with a shebang naming an interpreter that
is not there. Preflight passes — the file exists and is executable — so `execve` is reached and
answers `ENOENT` for a program that is on disk, which is the real shape of the glibc failure the
hint is about. Chmodding `erlexec` to 0644 would not have worked: preflight catches that first
and re-extracts, which is its job.

### 7 (medium) the GREEN bug fix had no regression test

Step 1's move-aside branch was `if target.exists()` and so deleted a *complete* entry whenever
the hit was skipped. It was fixed during GREEN and its only cover was a `fault-injection` test,
so a plain `cargo test` could not see it come back.

The branch is now `cache::discard_incomplete`, a documented function `ensure_extracted` calls,
and `tests/regressions/a3b_the_move_aside_branch_deleted_a_complete_entry.rs` drives it directly
— no feature gate, no `GINARY_FAULT`. RED by restoring the original condition:

```text
---- a3b_the_move_aside_branch_deleted_a_complete_entry::a_complete_entry_is_never_moved_aside ----
panicked at tests/regressions/a3b_the_move_aside_branch_deleted_a_complete_entry.rs:53:5:
an entry holding `ginary.json` is complete and must be left alone
```

### 8 to 12 (low and non-behavioural)

- **`a_warning_sink_is_written_through_and_flushed` asserted nothing it was named for.** It ran
  `prepare` on a writable root, so no warning was ever produced and the sink was never read. It
  now passes a `CountingSink`, forces the fallback, and asserts both that the line arrived and
  that `flush` was called exactly once before `prepare` returned.
- **`rename_into_place`'s `!taken` guard was dead.** Every branch reaching it bound `taken =
  true`. The binding is gone; the forced case is `if !forced { match rename … }` and what is left
  is the manifest check that can actually fire.
- **`mod libc_errno` hardcoded Linux errno numbers** in a file that already `cfg`s for non-Linux.
  `ENOTEMPTY` is 39 on Linux and 66 on the BSDs, so the lost-race branch would have missed there
  and turned a reuse into exit 124. The module is gone; `is_occupied` and `is_refusal` go through
  `rustix::io::Errno`, which is already a dependency.
- **`Runner::output` and the concurrency waits had no time budget.** Both now go through
  `bounded::wait_bounded` — the half of `run_bounded` that takes an already-spawned child, added
  for this — with `artifact::RUN_BUDGET`. The `ETXTBSY` spawn retry is unchanged and still wraps
  only the spawn.
- **`nothing_is_armed_without_the_feature` compiled to an empty body** in exactly the
  configuration `mise run test` and CI use. The `cfg` moved to the function, so it is absent
  rather than vacuously passing there, and a companion test under the feature asserts that a spec
  naming an action this build does not implement (`rename:enospc`) arms nothing.

### Gates after the round

```text
cargo fmt --all -- --check                                    clean
cargo clippy --all-targets --all-features -- -D warnings      clean
cargo test --features fault-injection --no-fail-fast          23 targets, 0 failed
cargo test                                                    23 targets, 0 failed
RUSTDOCFLAGS=-Dwarnings cargo doc --no-deps                   clean
cargo deny check                                              advisories ok, bans ok, licenses ok, sources ok
```

Per target, with `--features fault-injection`, the counts that moved: `--lib` 142 to 147 (the
five `hint_for` tests), `tests/cache.rs` 19 to 22, `tests/launcher.rs` 36 to 41,
`tests/manifest.rs` 23 to 29, `tests/regressions.rs` 47 to 54. Nothing else changed count and
nothing that passed before this round fails now. `tests/artifact_real.rs` really ran here (3
passed): the toolchain is present on this machine.

## Final gate

An independent re-run of every gate on the A3b tree, with nothing modified but this section.

| gate | result |
|---|---|
| `cargo fmt --all -- --check` | clean, no diff |
| `cargo clippy --all-targets --all-features -- -D warnings` | clean |
| `cargo test` | 711 passed, 0 failed, 0 ignored |
| `RUSTDOCFLAGS=-Dwarnings cargo doc --no-deps` | clean |
| `cargo deny check` | advisories ok, bans ok, licenses ok, sources ok |
| `GINARY_REQUIRE_TOOLCHAIN=1 cargo test` | 711 passed, 0 failed, 0 ignored |
| `cargo test --features fault-injection` | 715 passed, 0 failed, 0 ignored |

Per-binary counts for the default `cargo test` run: lib 147, `appfile` 51, `artifact_real` 3,
`assemble` 34, `beam` 32, `cache` 22, `cli` 59, `closure` 34, `diag` 13, `elf` 16, `launch` 29,
`launcher` 37, `manifest` 29, `otp` 29, `payload` 39, `regressions` 54, `report` 13, `smoke_cli` 6,
`stage_run` 12, `strip` 29, `trailer` 13, doc-tests 1. The `--features fault-injection` run adds
four to `tests/launcher.rs` (37 to 41); every other target keeps its count.

The toolchain-gated tests really run rather than skip: under `GINARY_REQUIRE_TOOLCHAIN=1`,
`tests/artifact_real.rs` reports three passing tests by name —
`a_real_artifact_runs_a_gleam_program_with_no_erlang_on_the_machine`,
`a_real_artifact_reports_a_runtime_error_as_exit_one_and_leaves_the_cwd_clean`, and
`the_real_artifact_is_one_file_and_its_size_is_recorded` — in 9.69s, and no target reports a
non-zero ignored count.

`cargo deny check` prints three `license-not-encountered` warnings for allowances the current
dependency set does not use (`BSD-3-Clause`, `CDLA-Permissive-2.0`, `ISC`). They are warnings about
`deny.toml` being wider than the tree, not findings against it, and every check reports `ok`.

`git status --short` lists 38 paths, all of them project files: the eleven new sources and tests
(`src/cache.rs`, `src/error.rs`, `src/fault.rs`, `src/launch.rs`, `src/launcher.rs`,
`src/selfexe.rs`, `tests/artifact_real.rs`, `tests/cache.rs`, `tests/common/artifact.rs`,
`tests/launch.rs`, `tests/launcher.rs`), the four A3b regression files, the launch-plan snapshot,
ADR 0008, this log, and modifications to `Cargo.toml`/`Cargo.lock`, `mise.toml`,
`.github/workflows/ci.yml`, the touched `src/` and `tests/` modules, and the docs. No sandbox shim
name (`.bashrc`, `.zshrc`, `.idea`, `.vscode`, `.gitconfig`, `.gitmodules`, `.mcp.json`,
`.profile`, `.ripgreprc`, `.bash_profile`, `.zprofile`) appears in the index. Nothing is committed.
