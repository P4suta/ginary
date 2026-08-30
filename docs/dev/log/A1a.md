<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# A1a — `.app` parsing and OTP discovery

Date: 2026-08-30 (fix round 1: 2026-08-31) · Status: complete, review round 1 closed

## Housekeeping

Four corrections and additions made before any A1a product code was written. No file under
`src/` or `tests/` changed, so the milestone starts from the A0 gate result.

### 1 — the assurance tooling was mis-reported as missing

`docs/dev/testing.md` closed with the claim that none of `cargo-deny`, `cargo-llvm-cov`,
`cargo-mutants`, `cargo-insta`, `cargo-nextest` or `cargo-fuzz` was installed, and
`docs/dev/log/A0.md` listed the same six under "Not installed". All six are installed. The
versions measured on this machine:

```
cargo-deny     0.19.7
cargo-llvm-cov 0.9.0
cargo-mutants  27.1.0
cargo-insta    1.48.0
cargo-nextest  0.9.140
cargo-fuzz     0.13.2
```

`cross` is the only tool from that list genuinely missing, and it is the one A0 also listed.

One caveat is recorded rather than glossed over: `cargo-insta` is installed through mise but has
no version pinned for its shim, so `cargo insta` on `PATH` fails with `No version is set for
shim: cargo-insta`. The binary under
`~/.local/share/mise/installs/cargo-cargo-insta/1.48/bin/cargo-insta` reports `1.48.0` and works.
Nothing in the suite uses snapshots yet, so this is a note, not a blocker.

`docs/dev/testing.md` now has an "Assurance tooling" section listing each tool, the mise task
that runs it, and what is and is not enforced. The A0 log's tool table carries the versions and
a line saying the original claim was wrong and where the correction lives.

### 2 — mise tasks for the assurance tools

| task | command |
|---|---|
| `deny` | `cargo deny check` |
| `cov` | `cargo llvm-cov --all-features --workspace --lcov --output-path target/lcov.info`, then `cargo llvm-cov report --summary-only --fail-under-lines 90` |
| `mutants` | `cargo mutants` |
| `test:nextest` | `cargo nextest run` |

`deny` is folded into `mise run check`, which is now `lint`, `test`, `doc`, `deny`.

Two deviations from the literal task text, both deliberate:

- **`mutants` is `cargo mutants`, not `cargo mutants --in-place false`.** `--in-place` is a
  boolean flag in cargo-mutants 27.1.0, so `--in-place false` would be parsed as a positional
  argument and rejected. Omitting the flag *is* "not in place": the default copies the tree into
  a temporary directory, which is the safe mode wanted. Sharding (`--shard i/n`) is deferred to
  the nightly CI job as planned.
- **`cov` enforces only the 90% line floor.** The 80% branch floor cannot be enforced yet:
  branch coverage needs a nightly `-Z coverage-options=branch` build, and this crate is measured
  on stable, where the branch column of the summary reads `-`. The floor stays documented in
  `docs/dev/testing.md` and moves into the task when the measurement exists — which is what
  "once measurable" in the milestone brief asks for.

The `cov` task collects once and re-renders: `cargo llvm-cov report` reuses the profile data the
first command produced, so the suite is not run twice.

Measured on the A0 tree, all four tasks pass:

```
mise run deny          exit 0   advisories ok, bans ok, licenses ok, sources ok
mise run test:nextest  exit 0   67 tests run: 67 passed, 0 skipped
mise run cov           exit 0   TOTAL lines 97.33%, regions 97.18%, functions 98.50%
cargo mutants --list   exit 0   mutants enumerated across the five modules
```

`cargo mutants` itself was not run to completion here; `--list` is enough to prove the task
line is well formed. `cargo nextest run` reports 67 tests against `cargo test`'s 68 because
nextest does not run doc tests, which is why `mise run test` stays the gate.

### 3 — `CARGO_HOME` is the default again

The sandbox that made `~/.cargo/registry` read-only is gone. The

```toml
[env]
CARGO_HOME = "{{config_root}}/.cache/cargo-home"
```

block is removed from `mise.toml`, and every cargo invocation now uses the default `~/.cargo`.
The workaround survives as documentation, not as configuration: `CONTRIBUTING.md` and
`CLAUDE.md` both say that if `~/.cargo` is read-only (a sandboxed agent, a shared machine) the
fix is `export CARGO_HOME=$PWD/.cache/cargo-home`. `.cache/` stays git-ignored.

The A0 log's gate tables still say the A0 runs used the project-local `CARGO_HOME`. That is a
record of what happened and was left alone.

### 4 — open issue 4 in the A0 log was stale

It said `run_with_timeout` "leaves reader threads detached on timeout". Fix round 2 of A0 made
that true on *every* path: `Drain::take_until` waits on a channel until the deadline and copies
whatever has been published, so a still-blocked reader is abandoned on the success path too, and
the readers are never joined. The issue now states that, and notes that the rule follows the
runner into `src/process.rs` when A1a factors it out of `doctor.rs`.

Open issue 3 was also reworded: `cargo deny` runs here now, so what remains open is only the
missing CI job for it.

### Gates after the housekeeping

| gate | result |
|---|---|
| `cargo fmt --all -- --check` | pass |
| `cargo clippy --all-targets --all-features -- -D warnings` | pass, no warnings |
| `cargo test` | pass — 61 unit, 6 integration, 1 doc test |
| `RUSTDOCFLAGS=-Dwarnings cargo doc --no-deps` | pass |
| `cargo deny check` | pass (four `license-not-encountered` warnings, no errors) |

All four ran with the default `CARGO_HOME=/home/<user>/.cargo`.

### Files touched

```
CLAUDE.md
CONTRIBUTING.md
docs/dev/log/A0.md
docs/dev/log/A1a.md
docs/dev/testing.md
mise.toml
```

## RED

Date: 2026-08-30

Tests, fixtures and test helpers for `appfile.rs` and `otp.rs`, written before either module
exists. 79 tests were added across three new integration targets. 74 of them fail, every one of
them on an assertion or on an explicit `Err`, never on a compile error: the crate builds, clippy
is clean, and `cargo doc` is clean.

### What was written

```
src/appfile.rs                              new, signatures only
src/otp.rs                                  new, signatures only
src/lib.rs                                  the two modules declared
src/doctor.rs                               `otp` field on Report, `OtpReport`, one text line
src/cli.rs                                  `ginary appfile parse <path>... [--json]`
tests/common/mod.rs                         shared-helper module
tests/common/tools.rs                       require_tools / Toolchain
tests/common/fake_otp.rs                    FakeApp, FakeOtp, FakeOtpRoot, FakeShipment
tests/appfile.rs                            43 tests
tests/otp.rs                                26 tests
tests/cli.rs                                10 tests
tests/fixtures/app/{quoted,comments,included,nested,malformed,unsupported_map}.app
tests/fixtures/app/otp/{kernel,stdlib,ssl,inets,crypto}.app          copied from the host OTP
tests/fixtures/app/shipment/{notify,gleam_crypto,mist,gleam_stdlib}.app  copied from a shipment
tests/fixtures/app/README.md                what each fixture pins, and where the copies came from
tests/regressions/README.md                 the one-file-per-bug convention
tests/snapshots/appfile__nested_term_display.snap
tests/snapshots/appfile__parse_error_messages.snap
tests/snapshots/cli__appfile_parse_table.snap
docs/dev/testing.md                         builders, gating, fixture policy, snapshots
Cargo.toml                                  dev-dependencies proptest 1.11, insta 1.48
REUSE.toml                                  licences for the two directories of copied fixtures
.gitignore                                  *.snap.new
```

`REUSE.toml` previously declared the whole tree `MIT OR Apache-2.0` under one `**` annotation,
which would have been a false claim about ten files ginary did not write. The copied fixtures now
have `precedence = "override"` annotations naming Ericsson and the Gleam package authors, both
`Apache-2.0`. Adding SPDX headers to the files themselves was rejected: an edited copy is not a
copy, and the whole reason those files are here is that they were not written by this project.

The copied fixtures came from
`~/.local/share/mise/installs/erlang/29.0.5/lib/<app>-<vsn>/ebin/<app>.app` and from
`gleam export erlang-shipment` run in `/home/<user>/projects/gleam/notify`. That shipment did not
exist and was generated for this purpose; nothing in the ginary tree depends on it staying there,
because the four `.app` files are committed here.

### The stubs, and why they are not placeholders

The house rule forbids `todo!()` and forbids a stub subcommand. What is in `src/` is neither: it
is the full public API with real signatures, real documentation and bodies that return an honest
`Err`. Each one is marked `// RED: replaced in GREEN`.

- `appfile::parse_terms` returns a `ParseError` whose `found` is `an unimplemented parser`;
- `appfile::parse_app_file` and `TryFrom<&[Term]> for AppResource` return
  `AppFileError::NotImplemented`;
- `otp::discover` and `otp::inspect_root` return `OtpError::NotImplemented`;
- `otp::boot_lib_dirs` returns an empty vector;
- `Display for Term` writes nothing.

`AppFileError::NotImplemented` and `OtpError::NotImplemented` are the only two variants that are
placeholders, and both are documented as RED-phase only. GREEN deletes them; nothing outside the
stub bodies constructs either.

One deviation from the milestone brief is recorded rather than glossed over: `ginary appfile
parse` is wired up for real — argument parsing, the table renderer, the JSON schema and the error
path are complete production code — and only the `parse_app_file` call inside it is stubbed. That
is what makes the CLI tests fail on their assertions instead of on a missing command. The command
is therefore visible in `--help` while it cannot yet do its job, which is the one thing the "no
stub subcommands" rule exists to prevent. It is acceptable for the length of this milestone and
not beyond it: if A1a were abandoned here, the `Appfile` variant would have to come back out of
`cli.rs`.

`src/process.rs` is *not* part of RED. Factoring `run_with_timeout` out of `doctor.rs` is a
refactor with no new behaviour to assert, and `otp::discover` cannot exercise it while it is a
stub. It lands in GREEN, with `doctor`'s existing tests moving with it and the rule from A0's
open issue 4 intact: the readers are never joined, only given until the deadline.

### The one behaviour change to an existing module

`doctor::Report` gained `otp: Option<OtpReport>`, `Report::gather_from` gained a fourth injected
parameter, and `render_text` gained a line — `otp: <version> (release <n>, erts <v>)` plus
`otp root: <path>`, or `otp: not found`. `the_text_report_has_one_line_per_subject` asserted the
report *ended* with the last tool line; it now asserts it contains that line and ends with the
`otp` line. That is an assertion following a specified change, not an assertion being loosened.

### Assertions are exact

The tests pin values, not shapes:

- `malformed.app` must fail at **line 5, column 3**, with `expected` = ``` `,` or `}` ``` and
  `found` = ``` `{` ```;
- `unsupported_map.app` must fail at **line 7, column 21**, with `found` = ``` a map (`#{`) ```;
- `$a`, `$\n`, `$\\`, `$t` parse to `97`, `10`, `92`, `9`;
- `-2.0e3` parses to `-2000.0` and re-serialises as `-2000.0`;
- `Term::Float(1e-7).to_string()` must be `1.0e-7`, not Rust's `1e-7`, because Erlang cannot read
  the latter back;
- duplicate keys produce exactly
  `["duplicate key `vsn`; the last value wins", "duplicate key `applications`; the last value wins"]`,
  in that order;
- `kernel.app`'s seven `env` keys are asserted in file order, `ssl.app`'s four `applications` in
  file order, `notify.app`'s thirteen by count and first element;
- `boot_lib_dirs` must return `["kernel-11.0.3", "stdlib-8.0.3"]` — first appearance, no repeats
  — and must ignore `$ROOT/lib/kernel-11.0.3/priv`, `$ROOT/lib/noversion/ebin` and an absolute
  path that merely contains `lib/<name>-<vsn>/ebin`;
- `AmbiguousErts` and `AmbiguousLibApp` must carry the offending directory names, sorted.

### Toolchain-gated tests

Five tests reach the host toolchain, all through `require_tools(&["erl"])`:
`parses_every_app_in_host_otp`, `boot_lib_dirs_reads_the_real_no_dot_erlang_boot`,
`discover_finds_the_erl_on_the_path`, `the_discover_program_prints_root_release_and_erts_version`,
and the two `doctor` tests in `tests/cli.rs`. On this machine `erl` is present, so all of them
ran; none was skipped, and the RED evidence below includes them.

### The five tests that pass in RED

| test | why |
|---|---|
| `cli::the_help_lists_the_appfile_command` | clap wiring, which is real code, not a stub |
| `cli::appfile_parse_without_a_path_is_a_usage_error` | same |
| `cli::appfile_without_a_subcommand_is_a_usage_error` | same |
| `otp::the_discover_program_prints_root_release_and_erts_version` | runs the real `erl` with the `DISCOVER_EVAL` constant; no stub is involved |
| `otp::boot_lib_dirs_finds_nothing_in_bytes_that_hold_no_paths` | asserts an *empty* result, which the stub also returns |

The last one cannot be red by construction and is kept anyway: it is the boundary case, and it
would catch a GREEN implementation that returned a spurious entry for empty input. The other four
are covering behaviour that already exists.

### RED evidence

```
$ cargo test --no-fail-fast
   lib (src/*)          61 passed;  0 failed
   bin (src/main.rs)     0 passed;  0 failed
   tests/appfile.rs      0 passed; 43 failed
   tests/cli.rs          3 passed;  7 failed
   tests/otp.rs          2 passed; 24 failed
   tests/smoke_cli.rs    6 passed;  0 failed
   doc-tests             1 passed;  0 failed
```

Representative failures, one per failure mode:

```
---- bare_atoms_parse_as_atoms stdout ----
thread 'bare_atoms_parse_as_atoms' panicked at tests/appfile.rs:33:23:
`kernel.` should parse, but: line 1, column 1: expected a term, found an unimplemented parser

---- the_unsupported_map_fixture_names_the_construct stdout ----
thread 'the_unsupported_map_fixture_names_the_construct' panicked at tests/appfile.rs:578:9:
expected a parse error, got NotImplemented

---- display_round_trips_through_parse_terms stdout ----
thread 'display_round_trips_through_parse_terms' panicked at tests/appfile.rs:336:1:
Test failed: `.` did not parse: line 1, column 1: expected a term, found an unimplemented parser.
minimal failing input: original = Atom("a")

---- the_failure_messages_read_as_sentences stdout ----
Snapshot file: tests/snapshots/appfile__parse_error_messages.snap
    1       │-malformed.app: line 5, column 3: expected `,` or `}`, found `{`
    2       │-unsupported_map.app: line 7, column 21: expected a term, found a map (`#{`)
          1 │+reading `.app` files is not implemented yet
          2 │+reading `.app` files is not implemented yet

---- parses_every_app_in_host_otp stdout ----
thread 'parses_every_app_in_host_otp' panicked at tests/appfile.rs:735:23:
`erl` is on PATH but discovery failed: OTP discovery is not implemented yet

---- inspect_root_reads_every_field_from_the_tree stdout ----
thread 'inspect_root_reads_every_field_from_the_tree' panicked at tests/otp.rs:24:23:
/tmp/.tmpoPISRd should be usable, but: OTP discovery is not implemented yet

---- boot_lib_dirs_lists_each_library_once_in_order_of_appearance stdout ----
thread 'boot_lib_dirs_lists_each_library_once_in_order_of_appearance' panicked at tests/otp.rs:295:5:
assertion `left == right` failed
  left: []
 right: ["kernel-11.0.3", "stdlib-8.0.3"]

---- appfile_parse_reports_a_malformed_file_and_exits_one stdout ----
thread 'appfile_parse_reports_a_malformed_file_and_exits_one' panicked at tests/cli.rs:132:5:
the message must locate the problem: error: cannot read the application file `tests/fixtures/app/malformed.app`
  caused by: reading `.app` files is not implemented yet

---- doctor_json_reports_the_otp_installation stdout ----
thread 'doctor_json_reports_the_otp_installation' panicked at tests/cli.rs:170:5:
`erl` is on PATH, so `otp` must not be null
```

The 43 failing `tests/appfile.rs` tests:

```
a_duplicate_key_keeps_the_last_value_and_records_a_warning
a_fun_is_rejected_by_name
a_list_tail_is_rejected_by_name
a_map_is_rejected_by_name
a_minimal_application_term_yields_a_resource
a_missing_file_is_an_io_error_naming_the_path
a_non_atom_in_applications_is_an_error
a_percent_inside_a_string_is_not_a_comment
a_source_of_only_comments_holds_no_terms
a_term_that_is_not_an_application_is_an_error
a_term_without_a_final_full_stop_is_an_error
a_variable_is_rejected_by_name
an_application_without_a_vsn_is_an_error
an_empty_source_holds_no_terms
an_unterminated_string_is_an_error
bare_atoms_parse_as_atoms
binaries_parse_with_and_without_contents
character_literals_are_integers
comments_never_reach_the_resource
display_always_writes_a_float_erlang_can_read_back
display_escapes_strings_and_binaries
display_quotes_an_atom_only_when_it_has_to
display_round_trips_through_parse_terms
display_separates_elements_with_a_comma_and_a_space
floats_accept_a_sign_and_an_exponent
included_applications_stay_separate_from_applications
integers_accept_a_leading_minus
more_than_one_top_level_term_is_an_error
nested_env_values_are_summarised_by_key_in_file_order
no_terms_at_all_is_an_error
parses_every_app_in_host_otp
quoted_atoms_keep_their_unquoted_name_and_escapes
quoted_names_survive_into_the_resource
several_top_level_terms_are_returned_in_order
strings_unescape_quotes_backslashes_and_control_characters
the_copied_otp_fixtures_parse_with_the_versions_they_shipped_with
the_copied_shipment_fixtures_parse_as_gleam_wrote_them
the_error_position_counts_lines_and_characters
the_failure_messages_read_as_sentences
the_malformed_fixture_reports_line_five_column_three
the_nested_fixture_re_serialises_to_one_line
the_unsupported_map_fixture_names_the_construct
tuples_and_lists_nest_and_may_be_empty
```

The 24 failing `tests/otp.rs` tests:

```
boot_lib_dirs_ignores_anything_that_is_not_a_versioned_ebin_path
boot_lib_dirs_lists_each_library_once_in_order_of_appearance
boot_lib_dirs_reads_the_boot_file_a_fake_root_writes
boot_lib_dirs_reads_the_real_no_dot_erlang_boot
discover_finds_the_erl_on_the_path
discover_reports_an_override_that_is_not_an_otp_installation
discover_with_an_override_inspects_that_root
inspect_root_accepts_the_oldest_supported_release
inspect_root_cannot_guess_the_release_from_two_numeric_directories
inspect_root_falls_back_to_the_release_string_without_an_otp_version_file
inspect_root_falls_back_to_the_single_numeric_release_directory
inspect_root_ignores_a_lib_directory_without_a_numeric_version
inspect_root_names_whichever_erts_binary_is_missing
inspect_root_reads_every_field_from_the_tree
inspect_root_rejects_a_missing_boot_file
inspect_root_rejects_a_release_older_than_the_minimum
inspect_root_rejects_a_root_without_an_erts_directory
inspect_root_rejects_a_root_without_kernel
inspect_root_rejects_a_root_without_stdlib
inspect_root_rejects_an_erts_binary_that_is_not_executable
inspect_root_rejects_two_erts_directories
inspect_root_rejects_two_versions_of_the_same_library
inspect_root_takes_the_release_from_the_second_field_of_start_erl_data
inspect_root_trims_the_otp_version_file
```

The 7 failing `tests/cli.rs` tests:

```
appfile_parse_json_carries_every_field_of_the_resource
appfile_parse_keeps_the_files_in_the_order_they_were_given
appfile_parse_prints_one_labelled_block_per_file
appfile_parse_reports_a_malformed_file_and_exits_one
appfile_parse_reports_a_missing_file_and_exits_one
doctor_json_reports_the_otp_installation
doctor_text_names_the_otp_root_and_version
```

### Gates on the RED tree

| gate | result |
|---|---|
| `cargo fmt --all -- --check` | pass |
| `cargo clippy --all-targets --all-features -- -D warnings` | pass, no warnings |
| `cargo test` | **fails as intended**: 74 of 79 new tests red, the 68 A0 tests still green |
| `RUSTDOCFLAGS=-Dwarnings cargo doc --no-deps` | pass |
| `cargo deny check` | pass (four `license-not-encountered` warnings, no errors; `proptest` and `insta` and their trees are MIT/Apache-2.0) |

### Notes for GREEN

1. `ParseError` carries no path. `AppFileError::Parse` adds it, and its `Display` is
   `{path}: {source}` — the snapshot depends on that.
2. `boot_lib_dirs` returns the `<name>-<vsn>` component, not the whole `$ROOT/lib/.../ebin`
   string. That is what `assemble` will want, and the tests assert it.
3. `env` values are discarded; only the keys are kept, in file order. Nothing downstream reads a
   value, and keeping them would mean keeping arbitrary nesting alive for no reader.
4. The `kernel-doc` decoy test is the one that dictates the glob: the version suffix must be
   digits and dots, so `kernel-doc`, `kernel-` and `kernel-1a.2` are all not `kernel`.
5. `Display for Term` must emit a float Erlang can read: a bare `1e-7` from Rust's `{:?}` is not
   valid Erlang, and the round-trip property will find it.

## GREEN

Every stub marked `// RED: replaced in GREEN` is gone, and so are the two
`NotImplemented` error variants that only existed to hold their place. All 164
tests pass, gates included.

### What was implemented

**`src/appfile.rs`** — a hand-written recursive-descent parser over `Vec<char>`,
plus the resource reader on top of it.

- `parse_terms` walks characters rather than bytes, because a column has to
  count characters: the fixture with `é` in it is exactly the case a byte offset
  gets wrong.
- Unsupported constructs are recognised *before* they are consumed, so the error
  can name them: `describe_found` reports a map, a fun, a variable or a list
  tail at the position the construct starts, and falls back to the token itself
  (`` `{` ``) everywhere else. `fun` is checked as a reserved word on the atom
  path with a non-allocating comparison, so every atom does not pay for it.
- `Display` re-serialises: an atom is left bare only when it is ASCII-lowercase
  and made of `[A-Za-z0-9_@]`, and quoted otherwise, which is always safe. A
  float goes through `{value:?}` — Rust's shortest round-trip form — and then
  gains the `.0` Erlang needs in a mantissa, so `1e-7` is written `1.0e-7`.
  Control characters are written as three-digit octal escapes, a fixed width the
  parser reads back exactly even when a digit follows.
- `TryFrom<&[Term]>` requires exactly one `{application, Name, Props}`, reads the
  eight keys ginary uses, and discards the rest. `env` is summarised by key: the
  values nest arbitrarily and nothing downstream reads one.

**`src/otp.rs`** — `inspect_root` is the whole judgement, and `discover` only
decides which directory it is pointed at. The `erl` probe's other two lines are
still required and checked (a release that parses, a non-empty ERTS version):
that is what makes it a probe rather than a `code:root_dir()` call, while the
tree stays the point of truth. `boot_lib_dirs` scans for `$ROOT/lib/<name-vsn>/ebin`
byte strings and returns the `<name>-<vsn>` component, first appearance first.

**`src/process.rs`** — new, as planned in RED: `find_in_path` and
`run_with_timeout` moved out of `doctor.rs` unchanged in behaviour, because
`otp::discover` needs the same bounded child. The A0 rule is intact and still
covered — the readers are never joined, only given until the deadline, and only
the direct child is killed and reaped.

### GREEN / test corrections

None. No test was edited, weakened or deleted; the RED suite passed as written.

### Decisions worth a reviewer's attention

1. **A bare atom starts with any lowercase letter, not `[a-z]`.** The
   documented grammar said `[a-z]`, but `the_error_position_counts_lines_and_characters`
   feeds the parser `é#{}` and demands the error at the `#`, one column past the
   `é` — that is, `é` must parse as an atom. Erlang agrees: its atoms start with
   a lowercase *Latin-1* letter. `char::is_lowercase` is the closest thing in the
   standard library and is a superset, so every file `erl` accepts is accepted
   here. `parse_terms`' documentation now says so, and `Display` still quotes
   anything outside ASCII, so the round trip is unaffected.
2. **Nesting is bounded at 100 tuples/lists.** The parser recurses, and a file of
   nothing but `[` would abort the process with a stack overflow — not a panic,
   but not an error a caller can report either. Two unit tests pin both sides of
   the bound. Real resource files nest fewer than ten deep.
3. **`AppFileError::InvalidValue` is new.** `NonAtomEntry` covers the four
   atom-list properties; a `vsn` or `description` that is not a string needed its
   own message rather than a misleading `MissingVsn`. Both `NotImplemented`
   variants are deleted.
4. **A skipped property is a recorded warning, never silent.** A property that is
   not `{Key, Value}`, or an `env` entry that is not, is reported in
   `AppResource::warnings` and dropped. Duplicate keys are warned about only for
   the eight keys ginary reads: a repeated key nothing reads shadows nothing.
5. **`discover` runs `erl` with `-env ERL_CRASH_DUMP <null device>`**, the same
   guard `doctor`'s probe carries and for the same reason: a broken installation
   must not leave an `erl_crash.dump` in the user's working directory.
   `DISCOVER_EVAL` itself is untouched, which is what
   `the_discover_program_prints_root_release_and_erts_version` runs.
6. **`doctor::find_in_path` is now a re-export** of `process::find_in_path`, so
   the public path `ginary::doctor::find_in_path` that `tests/common/tools.rs`
   uses did not change. `ProbeOutput` and `ProbeError` became `ProcessOutput` and
   `ProcessError` on the way, since nothing about them is a probe any more; the
   variants and their messages are unchanged. The `script`/`wait_until_executable`
   test helper moved to `process::test_support` and is shared by both modules'
   tests rather than copied.
7. **`ginary appfile parse` now does what it says**, which closes the deviation
   recorded in RED: the command is in `--help` and works.

### Gates

| gate | result |
|---|---|
| `cargo fmt --all -- --check` | pass |
| `cargo clippy --all-targets --all-features -- -D warnings` | pass, no warnings |
| `cargo test` | pass |
| `RUSTDOCFLAGS=-Dwarnings cargo doc --no-deps` | pass |
| `cargo deny check` | pass (the same four pre-existing `license-not-encountered` warnings) |

```
running 78 tests (src/lib.rs)      test result: ok. 78 passed; 0 failed
running 43 tests (tests/appfile.rs) test result: ok. 43 passed; 0 failed
running 10 tests (tests/cli.rs)     test result: ok. 10 passed; 0 failed
running 26 tests (tests/otp.rs)     test result: ok. 26 passed; 0 failed
running  6 tests (tests/smoke_cli.rs) test result: ok. 6 passed; 0 failed
   Doc-tests ginary                 test result: ok. 1 passed; 0 failed
```

The suite was run three times end to end with the same result, and once with
`GINARY_REQUIRE_TOOLCHAIN=1` so that the six toolchain-gated tests had to run
rather than skip. The round-trip property was also run with
`PROPTEST_CASES=8000`; it found nothing, and no `.proptest-regressions` file was
written.

### Files touched in GREEN

`src/appfile.rs`, `src/otp.rs`, `src/process.rs` (new), `src/doctor.rs`,
`src/lib.rs`, `docs/dev/architecture.md`, `docs/dev/testing.md`,
`docs/dev/log/A1a.md`. No test file, fixture or snapshot was changed.

## Fix round 1

The adversarial review of the GREEN tree raised eleven findings: one high, three
medium, seven low. All eleven are addressed below. Every behavioural finding got
its test first, and the failure it produced is quoted verbatim.

### RED — the three bugs

`tests/regressions.rs` is new: the target that `#[path]`-includes one file per
fixed bug, as `tests/regressions/README.md` has described since A1a started.
Three files went in before any production change, and `cargo test --test
regressions` failed as follows.

```
---- a1a_display_left_reserved_words_bare::a_reserved_word_atom_survives_display_and_a_second_parse stdout ----
panicked at tests/regressions/a1a_display_left_reserved_words_bare.rs:38:27:
`fun.` did not parse back: line 1, column 1: expected a term, found a fun (`fun`)

---- a1a_display_left_reserved_words_bare::a_reserved_word_is_quoted_inside_a_nested_term stdout ----
panicked at tests/regressions/a1a_display_left_reserved_words_bare.rs:49:5:
assertion `left == right` failed
  left: "{mod, [fun, kernel]}"
 right: "{mod, ['fun', kernel]}"

---- a1a_env_duplicate_keys_were_unreported::a_duplicate_env_key_is_listed_once_and_warned_about stdout ----
panicked at tests/regressions/a1a_env_duplicate_keys_were_unreported.rs:37:5:
assertion `left == right` failed
  left: ["a", "b", "a"]
 right: ["a", "b"]

---- a1a_doctor_dropped_the_otp_error::the_text_report_says_why_an_unusable_installation_was_rejected stdout ----
panicked at tests/regressions/a1a_doctor_dropped_the_otp_error.rs:63:5:
the report must name what was wrong with the root:
host target: linux-x86_64-gnu
rustc/cargo: not required (neither ginary nor its artifacts need a Rust toolchain)
cache dir: /home/yasunobu/.cache/ginary (from HOME)
gleam: not found
erl: found, version unknown (/tmp/.tmpsPlzW8/bin/erl)
strip: not found
docker: not found
otp: not found

---- a1a_doctor_dropped_the_otp_error::the_json_report_carries_the_reason_beside_the_null_installation stdout ----
panicked at tests/regressions/a1a_doctor_dropped_the_otp_error.rs:93:28:
otp_error must hold the reason: {"cache_dir":"...","format_version":1,"otp":null,
"rustc_required":false,"tools":[...{"found":true,"name":"erl",...}...]}

---- a1a_doctor_dropped_the_otp_error::a_machine_without_erl_is_told_that_that_is_the_reason stdout ----
panicked at tests/regressions/a1a_doctor_dropped_the_otp_error.rs:113:28:
otp_error must hold the reason: {... "otp":null ...}

test result: FAILED. 1 passed; 6 failed
```

### RED — the two coverage findings

Two findings were that a rule had no test, not that the rule was wrong: the
`erl` probe's validation of what it read, and the "`.` followed by whitespace"
rule. A test that passes the moment it is written proves nothing about either,
so each new test was watched failing against a tree with the rule *removed*, and
the rule was then restored. The mutations are not in the tree; the tests are.

Removing the arity and release checks from `otp::probe_root`:

```
test otp::tests::a_probe_whose_release_is_not_a_number_is_not_an_answer ... FAILED
test otp::tests::a_probe_that_prints_only_a_root_is_not_an_answer ... FAILED
panicked at src/otp.rs:592:25:
`printf '/opt/otp\nRelease 29\n17.0.5\n'` should not answer, but reported /opt/otp
panicked at src/otp.rs:592:25:
`printf '/opt/otp\n'` should not answer, but reported /opt/otp
```

Removing the exit-status check from the same function:

```
test otp::tests::a_probe_that_fails_after_printing_an_answer_is_not_trusted ... FAILED
panicked at src/otp.rs:597:25:
'; exit 1` should not answer, but reported /opt/otp
```

Removing the rejection branch from `appfile::Parser::expect_full_stop`:

```
test a_full_stop_must_be_followed_by_whitespace_or_the_end_of_input ... FAILED
panicked at tests/appfile.rs:47:22:
`a.b.` should not parse, but it produced [Atom("a"), Atom("b")]
```

### GREEN — what changed, finding by finding

1. **`doctor` now reports why discovery failed** (high). `Report` gained
   `otp_error: Option<String>`, set from the `OtpError` that `Report::gather`
   used to drop with `.ok()`. `gather_from` takes a `Result<OtpReport, String>`
   rather than an `Option`, so the error cannot be discarded by accident on the
   way in. The text report prints `otp: unusable (<reason>)`; the JSON keeps
   `otp: null` — the shape the spec asks for — and carries the sentence beside
   it. `otp: not found` remains only for a report that recorded neither, which
   is what a hand-built one does.
2. **Reserved words are quoted by `Display`** (medium). `is_bare_atom` now
   consults a sorted `RESERVED_WORDS` list of all 29 words, so `Term::Atom("fun")`
   renders as `'fun'`. A unit test pins that the list is sorted, since
   `is_reserved_word` binary-searches it, and the proptest atom generator now
   names `fun`, `end` and `maybe` explicitly instead of waiting a million cases
   to stumble on them.
3. **The `erl` probe's failure paths are tested** (medium). `ask_erl_for_its_root`
   split into `find_erl(path_var)` and `probe_root(erl, timeout)`, which take the
   `PATH` value and the program instead of reading the ambient environment. Seven
   unit tests drive them against stub scripts: no `erl` on `PATH`, no `PATH` at
   all, a well-formed answer, one line, a non-numeric release, silence, a
   non-zero exit after a well-formed answer, and a program that never exits
   (which also exercises the budget). The end-to-end path is covered by the
   `doctor` regression file, which puts a stub `erl` on a `PATH` of its own.
4. **The full-stop rule is tested** (medium). `a.b.` is an error at 1:2, and `a.`
   at end of input, before a comment and before a newline still parse.
5. **`ProcessOutput` carries `stderr`** (low). It was drained and dropped, so a
   child that failed with its diagnosis on standard error was reported with an
   empty explanation. `OtpError::ErlOutput` now quotes both streams through
   `describe_output`, which says `nothing at all` when there was nothing, names
   which stream it is quoting, and cuts at 400 characters. `doctor`'s tool
   probes are deliberately unchanged: a tool that answers nothing usable is
   already reported as `found, version unknown` with its path, and putting a
   program's stderr in the `tools` JSON is a schema change this milestone does
   not need.
6. **Warnings name a shape, not a tree** (low). Both warning paths use
   `describe_term`, as every error path already did, so a malformed property
   that happens to be a 50 000-element list produces one short sentence. A unit
   test pins it with a 1000-element list.
7. **A root that is not there says so** (low). `inspect_root` starts with an
   `is_dir` check and a new `OtpError::NoSuchRoot`, so a mistyped override is no
   longer reported as "has no `erts-*` directory". Three tests: a missing path, a
   path that is a file, and `discover(Some(missing))`.
8. **The fake builders are exercised** (low). `tests/appfile.rs` now builds a
   `FakeShipment` and a `FakeOtp` root with every property the builder can
   write, parses the `.app` files back with `parse_app_file`, and asserts field
   by field, plus the dummy `.beam` and the `priv` file on disk. That found a
   real defect while it was being written: `FakeApp::app_text` wrote names bare,
   so an application called `my-app` produced a file this parser rejects. Names
   now go through an `atom` helper that quotes when it has to, and a third test
   pins it. `Toolchain::get` and `Toolchain::names` were deleted rather than
   given a test: nothing needs them, and the spec asks only that the toolchain
   expose paths. The module example that rustdoc never compiles is now fenced as
   `text` and points at the two tests that do run.
9. **`optional_applications` is read** (low). It is a ninth `KNOWN_KEYS` entry
   and a new `AppResource` field, in the JSON and in the table. Four applications
   in the host OTP 29.0.5 tree declare it (`et`, `reltool`, `observer`,
   `debugger`), and A1b's closure has to know that such a dependency may
   legitimately be absent at run time rather than treating it as a hard
   requirement. This is one field beyond the spec's list, recorded here as the
   deviation it is; the alternative — dropping the key with a warning — would
   have made every one of those four applications noisy for no gain.
10. **A duplicate `env` key is de-duplicated and warned about** (low), by the
    same rule the eight top-level keys already follow.
11. **The documentation says what is true** (low). `docs/dev/testing.md` now
    lists the four targets that reach the ambient `PATH` and how each is
    bounded, names `otp::discover(None)` as the one exception to the injection
    rule and says what was done about it, describes `tests/common/script.rs`,
    and records that the fake builders are checked against the parser. The
    status line of this log is set. `mise run test:fast` gained `--test
    regressions`, which needs no external toolchain.

### Snapshot

`tests/snapshots/cli__appfile_parse_table.snap` gained one
`optional_applications` row per block. That is a deliberate output change
(finding 9), reviewed line by line, not an accepted mismatch.

### Gates after the fixes

| gate | result |
|---|---|
| `cargo fmt --all -- --check` | pass |
| `cargo clippy --all-targets --all-features -- -D warnings` | pass, no warnings |
| `cargo test` | pass |
| `RUSTDOCFLAGS=-Dwarnings cargo doc --no-deps` | pass |
| `cargo deny check` | pass (the same four pre-existing `license-not-encountered` warnings) |

```
running 95 tests (src/lib.rs)         test result: ok. 95 passed; 0 failed
running 51 tests (tests/appfile.rs)   test result: ok. 51 passed; 0 failed
running 10 tests (tests/cli.rs)       test result: ok. 10 passed; 0 failed
running 29 tests (tests/otp.rs)       test result: ok. 29 passed; 0 failed
running  7 tests (tests/regressions.rs) test result: ok. 7 passed; 0 failed
running  6 tests (tests/smoke_cli.rs) test result: ok. 6 passed; 0 failed
   Doc-tests ginary                   test result: ok. 1 passed; 0 failed
```

164 tests before, 199 after. The suite was run with `GINARY_REQUIRE_TOOLCHAIN=1`
so the gated tests had to run rather than skip.

### Files touched in fix round 1

`src/appfile.rs`, `src/otp.rs`, `src/process.rs`, `src/doctor.rs`, `src/cli.rs`,
`tests/appfile.rs`, `tests/cli.rs`, `tests/otp.rs`, `tests/regressions.rs`
(new), `tests/regressions/a1a_display_left_reserved_words_bare.rs` (new),
`tests/regressions/a1a_doctor_dropped_the_otp_error.rs` (new),
`tests/regressions/a1a_env_duplicate_keys_were_unreported.rs` (new),
`tests/regressions/README.md`, `tests/common/mod.rs`,
`tests/common/script.rs` (new), `tests/common/fake_otp.rs`,
`tests/common/tools.rs`, `tests/snapshots/cli__appfile_parse_table.snap`,
`mise.toml`, `docs/dev/architecture.md`, `docs/dev/testing.md`,
`docs/dev/log/A1a.md`. No fixture was changed.

## Final gate

Independent re-run of every gate on 2026-08-31, nothing modified except this
section.

| Gate | Result |
| --- | --- |
| `cargo fmt --all -- --check` | pass (no diff) |
| `cargo clippy --all-targets --all-features -- -D warnings` | pass (no warnings) |
| `cargo test` | pass, 199 tests |
| `RUSTDOCFLAGS=-Dwarnings cargo doc --no-deps` | pass |
| `cargo deny check` | pass (advisories, bans, licenses, sources ok) |
| `GINARY_REQUIRE_TOOLCHAIN=1 cargo test` | pass, 199 tests, no skips |

Per-binary summaries, identical in both test runs:

```
unittests src/lib.rs     test result: ok. 95 passed; 0 failed; 0 ignored
unittests src/main.rs    test result: ok.  0 passed; 0 failed; 0 ignored
tests/appfile.rs         test result: ok. 51 passed; 0 failed; 0 ignored
tests/cli.rs             test result: ok. 10 passed; 0 failed; 0 ignored
tests/otp.rs             test result: ok. 29 passed; 0 failed; 0 ignored
tests/regressions.rs     test result: ok.  7 passed; 0 failed; 0 ignored
tests/smoke_cli.rs       test result: ok.  6 passed; 0 failed; 0 ignored
Doc-tests ginary         test result: ok.  1 passed; 0 failed; 0 ignored
```

The toolchain-gated tests ran rather than skipped in both passes: no `skipping:`
line appeared on either run, and `parses_every_app_in_host_otp`,
`boot_lib_dirs_reads_the_real_no_dot_erlang_boot`,
`discover_finds_the_erl_on_the_path` and
`the_discover_program_prints_root_release_and_erts_version` each reported `ok`.

`cargo deny check` emits four `license-not-encountered` warnings for allowances
in `deny.toml` (`BSD-3-Clause`, `CDLA-Permissive-2.0`, `ISC`, `Zlib`) that no
current dependency uses. They are warnings only and the check exits zero.

`git status --short` shows no sandbox shim entry staged; the working tree is the
A0 scaffold plus the A1a additions listed in the fix-round section above.
