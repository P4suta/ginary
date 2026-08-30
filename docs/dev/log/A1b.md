<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# A1b — application dependency closure

Date: 2026-08-31 · Status: in progress

## Housekeeping

One documentation correction made before any A1b product code was written. No behaviour
changed, so the milestone starts from the A1a gate result.

### 1 — the `AppResource` struct doc contradicted its own field

The struct-level doc on `AppResource` in `src/appfile.rs` listed `optional_applications` among
the keys that are "parsed and discarded". A1a made that key a public field with its own doc
comment explaining the opposite: the name may legitimately be absent at run time, the closure
has to tell the difference, so the key is read rather than dropped. Rendered rustdoc therefore
told a reader both things a few lines apart, and the wrong half came first.

The struct-level doc now lists only keys that really are dropped:

```text
/// Keys ginary does not use (`runtime_dependencies`, `maxT`, …) are parsed and
/// discarded; only `env` is summarised, by key, since the values can be
/// arbitrarily deep and nothing downstream reads them.
```

The field doc on `optional_applications` is unchanged and is now the only statement about that
key. This matters for A1b specifically: the closure follows `optional_applications` edges only
when they resolve, and records the rest in `AppSet::skipped_optional`, which is only possible
because the parser keeps the key.

### 2 — working tree state

`git status --porcelain` is empty apart from the sandbox character-device shims, which are not
project files and were not touched. A0 and A1a are committed as `9dfc5ce` and `449e8c3`; the
A1b work starts from `449e8c3` with no carried-over uncommitted changes.

### Gates after housekeeping

All five gates pass on the corrected tree:

| Gate | Result |
| --- | --- |
| `cargo fmt --all -- --check` | clean |
| `cargo clippy --all-targets --all-features -- -D warnings` | clean |
| `cargo test` | 199 passed, 0 failed (95 unit, 51 + 29 + 10 + 7 + 6 integration, 1 doctest) |
| `RUSTDOCFLAGS=-Dwarnings cargo doc --no-deps` | clean |
| `cargo deny check` | advisories, bans, licenses, sources ok |

`cargo deny` emits one `license-not-encountered` warning for the `Zlib` allowance in
`deny.toml`; no current dependency carries that licence. It is an unused allowance, not a
violation, and is left in place.

## RED

Date: 2026-08-31

Tests, fixtures and test helpers for `closure.rs` and the `ginary closure` subcommand, written
before the module exists. 41 tests were added across two integration targets. 38 of them fail,
every one on an assertion or on an explicit `Err`, never on a compile error: the crate builds,
`cargo fmt --check` is clean, clippy with warnings denied is clean, and `cargo doc` is clean.

### What was written

```
src/closure.rs                                new, signatures only
src/lib.rs                                    the module declared
src/cli.rs                                    `ginary closure <shipment> [--otp-root PATH]
                                              [--root NAME]... [--extra NAME]... [--json]
                                              [--explain]`, its table, its notes footer and
                                              the `--json` schema
tests/common/fake_otp.rs                      `FakeApp::optional`, writing optional_applications
tests/common/snapshot.rs                      `scrub`, replacing tempdir roots in snapshots
tests/common/mod.rs                           the new helper declared
tests/closure.rs                              34 tests
tests/cli.rs                                  7 tests
tests/snapshots/closure__explain_table.snap
tests/snapshots/closure__app_not_found_message.snap
tests/snapshots/closure__shadowed_otp_application_warning.snap
tests/snapshots/cli__closure_explain_table.snap
docs/dev/architecture.md                      the closure section and the module map
docs/dev/testing.md                           the closure scenarios, the fourth helper, snapshots
```

No dependency was added: `serde`, `serde_json`, `thiserror`, `insta`, `proptest` and `tempfile`
were all already there.

### The stubs, and why they are not placeholders

`src/closure.rs` is the full public API with real signatures, real documentation and bodies that
return an honest value, each marked `// RED: replaced in GREEN`:

- `app_dependency_closure` returns `Err(ClosureError::NotImplemented)`;
- `explain` returns an empty `String`;
- `AppSet::chain` returns an empty `Vec`.

Everything else in the module is finished production code, because none of it is a guess:
`AppSet::iter`, `names`, `get`, `len`, `is_empty`, `otp_apps`, `shipment_apps`, the
`IntoIterator` impl, `SeedKind::is_seed` and `label`, `ResolvedApp::is_shipment` and `is_otp`,
the `Display` and `Error` impls for `ClosureError`, and `render_table`.

`ClosureError::NotImplemented` is the one placeholder variant and is documented as RED-phase
only. GREEN deletes it; nothing outside the three stub bodies constructs it.

The same deviation A1a recorded applies again and for the same reason: `ginary closure` is wired
up for real — argument parsing, both renderers, the JSON schema, the exit codes — and only the
`app_dependency_closure` call inside it is stubbed, which is what makes the CLI tests fail on
their assertions instead of on a missing command. The command is therefore visible in `--help`
while it cannot do its job. That is acceptable for the length of this milestone and not beyond
it.

`render_table` lives in `closure.rs` rather than `cli.rs` because `closure::explain` and
`cli::render_closure_table` must agree on column widths to the character; two copies would drift
and only a snapshot three milestones later would notice.

### Contracts the tests pin, not shapes

The tests were written to make GREEN's choices, not to accept them:

- seeds: `roots` beats `extra` beats `kernel`/`stdlib`, so a name in two lists has one
  `SeedKind`, and `--root kernel` is `Root`, not `Always`;
- `requested_by` holds immediate requesters only, sorted and deduplicated, is **empty for every
  seed** even when another application lists it, and never contains the application itself;
- an OTP directory matches only when the suffix after the last `-` is digits and dots:
  `crypto-doc`, `crypto-5.9.2.bak`, `crypto-latest` and `crypto-` are all rejected beside a real
  `crypto-5.9.2`, and a *regular file* named `crypto-9.9.9` is neither a match nor an ambiguity;
- `AppNotFound.requested_by` is the full chain from a seed, ending in the missing name —
  `["app", "gleam_crypto", "crypto"]` — and `searched` is exactly
  `[<shipment>/crypto/ebin/crypto.app, <otp_lib>/crypto-<vsn>]`, in that order;
- `AppSet::chain` returns a *shortest* path: the fixture gives `leaf` two requesters, `app` and
  `middle`, and the answer must be `["app", "leaf"]`;
- the JSON tags `source` with a `kind` field (`shipment` / `otp` plus `vsn`), renders `seed` as
  `root` / `extra` / `always` / `none`, and renders every path as a string;
- four snapshots pin whole messages, including the two `gleam.toml` keys the missing-application
  hint has to name and the exact column layout of the `explain` table.

Paths that reach a snapshot go through `common::snapshot::scrub` first, so what is committed is
the sentence and the shape of the path, not the temporary directory of one run.

### Toolchain-gated tests

Three tests reach the host toolchain. `closure::the_real_notify_shipment_closes_over_the_host_otp`
is gated on `require_tools(&["erl"])` *and* on the real shipment at
`/home/<user>/projects/gleam/notify/build/erlang-shipment` being present, and prints a skip line
naming whichever is missing. On this machine both are present, so it ran and is in the RED
evidence below. The two `doctor` tests in `tests/cli.rs` are unchanged from A1a.

The resolved OTP application list that test is supposed to record here cannot be recorded yet: it
is the output of the function this milestone has not written. It lands in this file in GREEN,
together with the `explain()` output the test prints to standard error.

### The three tests that pass in RED

| test | why |
|---|---|
| `cli::the_help_lists_the_closure_command` | clap wiring, which is real code, not a stub |
| `cli::closure_without_a_root_is_a_usage_error` | `--root` is `required = true`; clap answers before any closure runs |
| `cli::closure_with_both_json_and_explain_is_a_usage_error` | `conflicts_with`, same |

All three cover behaviour that already exists and is asserted for the first time here.

### RED evidence

```
$ cargo test --no-fail-fast
   lib (src/*)          95 passed;  0 failed
   bin (src/main.rs)     0 passed;  0 failed
   tests/appfile.rs     51 passed;  0 failed
   tests/cli.rs         13 passed;  4 failed
   tests/closure.rs      0 passed; 34 failed
   tests/otp.rs         29 passed;  0 failed
   tests/regressions.rs  7 passed;  0 failed
   tests/smoke_cli.rs    6 passed;  0 failed
   doc-tests             1 passed;  0 failed
```

The 38 failing tests are the 34 in `tests/closure.rs` and these four in `tests/cli.rs`:
`closure_prints_a_table_of_every_application_and_its_ebin`,
`closure_explain_prints_the_origin_of_every_application`,
`closure_json_carries_the_documented_keys` and
`closure_reports_a_missing_application_and_exits_one`.

Representative failures, one per failure mode:

```
---- kernel_and_stdlib_are_seeds_even_when_nothing_lists_them stdout ----
thread 'kernel_and_stdlib_are_seeds_even_when_nothing_lists_them' panicked at tests/closure.rs:73:27:
the closure of ["solo"] + [] should succeed: the application dependency closure is not implemented yet

---- two_version_directories_for_one_otp_application_are_ambiguous stdout ----
thread 'two_version_directories_for_one_otp_application_are_ambiguous' panicked at tests/closure.rs:377:18:
expected AmbiguousOtpApp, got NotImplemented

---- a_malformed_app_file_in_a_dependency_names_the_path stdout ----
thread 'a_malformed_app_file_in_a_dependency_names_the_path' panicked at tests/closure.rs:590:18:
expected AppFile, got NotImplemented

---- the_missing_application_message_ends_with_the_gleam_toml_hint stdout ----
thread 'the_missing_application_message_ends_with_the_gleam_toml_hint' panicked at tests/closure.rs:531:5:
the hint must be the last thing a reader sees:

---- the_closure_only_grows_when_extra_grows stdout ----
thread 'the_closure_only_grows_when_extra_grows' panicked at tests/closure.rs:708:1:
Test failed: the closure of ["a0"] + [] should succeed: the application dependency closure is
not implemented yet. minimal failing input: dag = SmallDag { size: 2, edges: {} }

---- the_real_notify_shipment_closes_over_the_host_otp stdout ----
thread 'the_real_notify_shipment_closes_over_the_host_otp' panicked at tests/closure.rs:807:23:
the real shipment should close: the application dependency closure is not implemented yet

---- closure_json_carries_the_documented_keys stdout ----
thread 'closure_json_carries_the_documented_keys' panicked at tests/cli.rs:361:10:
Unexpected failure. code=1
stderr="error: the application dependency closure is not implemented yet\n"
command=`ginary closure /tmp/.tmpmKJrz9/shipment --otp-root /tmp/.tmpmKJrz9/otp --root notify --extra sasl --json`
```

Not one of them is a compile error, and not one of the four snapshot assertions was reached, so
the four `.snap` files are hand-written contracts for GREEN rather than recordings of stub
output. Reviewing them is reviewing what the code is *supposed* to print.

`tests/closure.proptest-regressions` is written on every RED run and deleted again: its
"counterexample" is the stub refusing, not a property failure, and committing it would persist a
seed that means nothing once the module exists.

### Gates after RED

| Gate | Result |
| --- | --- |
| `cargo fmt --all -- --check` | clean |
| `cargo clippy --all-targets --all-features -- -D warnings` | clean |
| `cargo test` | 202 passed, 38 failed — the intended RED |
| `RUSTDOCFLAGS=-Dwarnings cargo doc --no-deps` | clean |
| `cargo deny check` | advisories, bans, licenses, sources ok |

## GREEN

Date: 2026-08-31

`src/closure.rs` is implemented. The three RED stubs are gone, `ClosureError::NotImplemented`
is deleted, and all 41 tests written in RED pass with no assertion touched.

### What was implemented

| item | what it does |
| --- | --- |
| `app_dependency_closure` | the worklist: seeds, resolution, the three edge kinds, the three errors |
| `explain` | the four-column table, `origin` being the seed word or the chain |
| `AppSet::chain` | breadth-first backwards over `requested_by` to the nearest seed |
| `seed_kinds` | `roots` then `extra` then `ALWAYS`, first kind wins |
| `shortest_chain` | one BFS used twice: by `AppSet::chain` and by `AppNotFound` |
| `upstream_of` | the requesters of a name mid-walk, empty for a seed |
| `locate` / `Found` | shipment first, OTP second, the loser recorded for the warning |
| `OtpLibrary` | the `lib` listing, indexed once, `<name>-<vsn>` only |
| `split_versioned` / `is_version` | `^[0-9]+(\.[0-9]+)*$` on the tail after the last `-` |
| `searched_paths` | the two paths `AppNotFound` names, in the order they are tried |
| `source_label` | the one place `shipment` / `otp` is spelled, shared with `cli.rs` |

Four decisions the tests left open and the code had to make:

1. **The worklist pops in name order** (`BTreeSet::pop_first`), not in discovery order. Any
   discovery order would give the same `apps`, because the map is keyed by name — but not the
   same `warnings` order, and not the same *first* error when a tree holds two problems.
   Popping by name puts the input order out of reach of every field.
2. **`requested_by` is filled in after the walk**, from a separate `BTreeMap<String,
   BTreeSet<String>>`. An application is resolved the first time it is reached, which is usually
   before the last edge into it is found; writing the requesters at resolution time would have
   recorded whichever subset the walk had seen by then.
3. **Required edges are `applications ∪ included_applications` minus `optional_applications`.**
   OTP's rule is that `optional_applications` is a *subset* of `applications`, so reading
   `applications` alone would make every optional dependency a hard one — and
   `an_optional_application_that_is_absent_is_skipped_and_is_not_an_error` would fail with an
   `AppNotFound` for a name the file explicitly marked optional.
4. **Two OTP versions of one application are an error even when the shipment shadows it.** The
   shipment copy would win and the OTP directory would go unused, so the closure could have
   stayed silent. It does not: two versions in one `lib` is a broken installation, and a broken
   installation found while resolving `crypto` will not have fixed itself by the time
   `public_key` is resolved.

An unreadable `otp_lib` is indexed as empty rather than reported on the spot. The first
application that needed it then fails with `AppNotFound`, whose `searched` list names
`<otp_lib>/<name>-<vsn>`, so the path is in the message either way and there is no error variant
with no test behind it.

### Test corrections

None. No test was weakened, deleted or edited.

One production-code fix was needed to satisfy a contract the RED snapshot already pinned.
`NOT_FOUND_HINT` in `src/closure.rs` was written as a `\`-continued string literal:

```rust
const NOT_FOUND_HINT: &str = "\
  hint: add it to `[erlang] extra_applications` (bundled and started) or
```

A backslash-newline in a Rust string literal eats the newline *and* the leading whitespace of
the next line, so the constant began `hint:` with no indent while
`tests/snapshots/closure__app_not_found_message.snap` requires `  hint:`, indented to line up
under `searched:`. The constant is now a `concat!` of three explicitly terminated lines and the
snapshot passes unmodified. The snapshot was the contract; the constant was the defect.

### The real shipment

`the_real_notify_shipment_closes_over_the_host_otp` ran against
`/home/<user>/projects/gleam/notify/build/erlang-shipment` with `--root notify`, over the host
OTP 29.0.5 `lib`. It resolved 31 applications, 6 from OTP and 25 from the shipment, with no
warnings and no skipped optional applications:

| from OTP | vsn | why |
| --- | --- | --- |
| `asn1` | 5.5 | `notify -> mist -> glisten -> ssl -> public_key -> asn1` |
| `crypto` | 5.9.2 | `notify -> gleam_crypto -> crypto` |
| `kernel` | 11.0.3 | always |
| `public_key` | 1.21.4 | `notify -> mist -> glisten -> ssl -> public_key` |
| `ssl` | 11.7.4 | `notify -> mist -> glisten -> ssl` |
| `stdlib` | 8.0.3 | always |

`crypto` resolves from OTP at 5.9.2 and `<lib>/crypto-5.9.2` exists, every OTP `ebin` is a
directory under the host `lib/`, and every shipment application has a directory. The full
`explain()` output the test prints to standard error:

```
name          vsn     source    origin
argus         1.0.4   shipment  notify -> argus
asn1          5.5     otp       notify -> mist -> glisten -> ssl -> public_key -> asn1
bcrypt        1.2.2   shipment  notify -> beecrypt -> bcrypt
beecrypt      0.4.0   shipment  notify -> beecrypt
crypto        5.9.2   otp       notify -> gleam_crypto -> crypto
esqlite       0.9.0   shipment  notify -> sqlight -> esqlite
exception     2.1.1   shipment  notify -> mist -> exception
gleam_crypto  1.6.0   shipment  notify -> gleam_crypto
gleam_erlang  1.3.0   shipment  notify -> gleam_erlang
gleam_http    4.3.0   shipment  notify -> gleam_http
gleam_json    3.1.0   shipment  notify -> gleam_json
gleam_otp     1.3.0   shipment  notify -> gleam_otp
gleam_quic    0.1.0   shipment  notify -> http3 -> gleam_quic
gleam_stdlib  1.0.5   shipment  notify -> gleam_stdlib
glisten       9.0.1   shipment  notify -> mist -> glisten
gramps        6.0.1   shipment  notify -> mist -> gramps
hpack         0.3.0   shipment  notify -> mist -> hpack
http3         0.1.0   shipment  notify -> http3
jargon        1.1.0   shipment  notify -> argus -> jargon
kernel        11.0.3  otp       always
logging       1.5.0   shipment  notify -> mist -> logging
mist          6.0.3   shipment  notify -> mist
mug           3.1.0   shipment  notify -> postgleam -> mug
notify        0.1.0   shipment  root
notify_core   0.1.0   shipment  notify -> notify_core
poolboy       1.5.2   shipment  notify -> beecrypt -> bcrypt -> poolboy
postgleam     0.8.0   shipment  notify -> postgleam
public_key    1.21.4  otp       notify -> mist -> glisten -> ssl -> public_key
sqlight       1.2.0   shipment  notify -> sqlight
ssl           11.7.4  otp       notify -> mist -> glisten -> ssl
stdlib        8.0.3   otp       always

skipped optional applications: []
```

Two things that list is evidence for. The OTP side is *four* applications plus the two seeds,
not the whole distribution: a real Gleam web application with TLS, a database and password
hashing reaches `ssl`, `public_key`, `asn1` and `crypto` and nothing else, which is the whole
argument for closing over `.app` files rather than shipping `lib/`. And every shipment
application resolves from the shipment, including `bcrypt`, `esqlite`, `poolboy` and `hpack`,
which are Erlang dependencies `gleam export erlang-shipment` copied in — none of them was taken
from the host runtime.

### Documentation

`docs/dev/architecture.md`: the determinism bullet of the closure section now states what the
implementation actually does — the `lib` listing indexed once, the worklist popping in name
order, `requested_by` collected in a `BTreeSet` and filled in after the walk. The rest of the
section was written in RED against this design and is unchanged. `docs/dev/testing.md` needed no
change: its closure-scenario table describes the tests, and no test changed.

### Gates after GREEN

| Gate | Result |
| --- | --- |
| `cargo fmt --all -- --check` | clean |
| `cargo clippy --all-targets --all-features -- -D warnings` | clean |
| `cargo test` | 240 passed, 0 failed |
| `RUSTDOCFLAGS=-Dwarnings cargo doc --no-deps` | clean |
| `cargo deny check` | advisories, bans, licenses, sources ok |

```
$ cargo test
   lib (src/*)          95 passed;  0 failed
   bin (src/main.rs)     0 passed;  0 failed
   tests/appfile.rs     51 passed;  0 failed
   tests/cli.rs         17 passed;  0 failed
   tests/closure.rs     34 passed;  0 failed
   tests/otp.rs         29 passed;  0 failed
   tests/regressions.rs  7 passed;  0 failed
   tests/smoke_cli.rs    6 passed;  0 failed
   doc-tests             1 passed;  0 failed
```

`cargo deny` emits four `license-not-encountered` warnings for allowances in `deny.toml` that no
current dependency uses. They are unused allowances, not violations, and are unchanged from A1a.

`tests/closure.proptest-regressions` is not committed: both property tests pass on every seed
tried, so there is no counterexample to persist.

## Fix round 1

Date: 2026-08-31

Eight review findings, three of them medium, all fixed. The four behavioural ones got a
regression test first, under `tests/regressions/`, and each was watched failing on an assertion
before any production code moved.

### 1 — a shadowed OTP ambiguity aborted the closure (medium)

`locate` asked the OTP library for a version *before* it looked in the shipment, so a stale
second version directory failed a build for an application ginary would never have read from
OTP. That contradicts the documented resolution order — shipment first, OTP second — and GREEN
decision 4, which chose it deliberately, was pinned by no test at all.

The decision is reversed rather than pinned. The ambiguity is refused where it decides
something (a required application that has to come from the OTP library) and reported where it
decides nothing (a shipment copy wins; both ignored directories are named in a warning).

```
---- a1b_shadowed_otp_ambiguity_aborted_the_closure::a_shadowed_application_with_two_otp_versions_is_a_warning_not_an_error stdout ----
panicked at tests/regressions/a1b_shadowed_otp_ambiguity_aborted_the_closure.rs:51:23:
the shipment copy wins, so the OTP versions cannot matter: application `crypto` matches more
than one directory in the OTP library: crypto-5.9.2, crypto-5.9.3
```

`two_version_directories_for_one_otp_application_are_ambiguous` in `tests/closure.rs` still
holds, unmodified: it builds a shipment that does *not* contain `crypto`, which is exactly the
case where the ambiguity still is an error.

### 2 — an ambiguous optional edge was an error (medium)

The optional-edge probe used the same fallible `locate` the required edges use, and its `?` let
`AmbiguousOtpApp` escape from what is only a resolvability question. The spec is explicit that an
optional edge that does not resolve is recorded and never raised.

```
---- a1b_an_ambiguous_optional_edge_was_an_error::an_optional_application_with_two_otp_versions_is_skipped_not_an_error stdout ----
panicked at tests/regressions/a1b_an_ambiguous_optional_edge_was_an_error.rs:49:23:
an optional dependency that does not resolve is never an error: application `crypto` matches
more than one directory in the OTP library: crypto-5.9.2, crypto-5.9.3
```

`locate` is now infallible and answers a `Resolution` — `Found`, `Missing`, `Ambiguous` or
`Unusable`. The worklist turns the last three into the errors above; the optional probe turns
them into a `skipped_optional` entry plus, for the two a reader could not guess, a warning saying
which reason it was. Skipping stays a reported decision.

### 3 — the `.app` error said the same thing three times (medium)

`ClosureError::AppFile` interpolated `{source}` into its own `Display` *and* returned that same
error from `Error::source`, so `src/main.rs`, which prints one line per link, repeated the parse
failure three times and the path twice.

```
---- a1b_app_file_error_repeated_its_cause::the_command_prints_the_file_on_its_own_line stdout ----
panicked at tests/regressions/a1b_app_file_error_repeated_its_cause.rs:85:5:
assertion `left == right` failed: the first line says which file, and nothing else:
error: cannot read the application file `<tmp>/shipment/dep/ebin/dep.app`: <tmp>/shipment/dep/ebin/dep.app: line 2, column 1: expected a term, found end of input
  caused by: <tmp>/shipment/dep/ebin/dep.app: line 2, column 1: expected a term, found end of input
  caused by: line 2, column 1: expected a term, found end of input
```

The layer now names only the file and leaves the reason to `source()`, which the second test
asserts is still reachable, so nothing is lost by the shortening. The remaining repetition
between the two `caused by:` lines belongs to `AppFileError::Parse`, whose `Display` is
`"{path}: {source}"` over its own `#[source]`; that is A1a's contract, pinned by
`appfile__parse_error_messages.snap` and by the `appfile parse` command, and is not touched here.

### 4 — an application name was used as a path component (low, behavioural)

Names come out of `.app` files, which ginary does not write, and every lookup interpolated one
into a path. `{applications, ['../../escape']}` parses, and `Path::join` with an absolute name
discards the prefix entirely, so a lookup could leave both trees; a later milestone would have
handed assembly that `ebin`.

```
---- a1b_app_names_were_used_as_paths::a_dependency_name_that_is_a_path_is_rejected stdout ----
panicked at tests/regressions/a1b_app_names_were_used_as_paths.rs:42:22:
`../../escape` must be refused as a name, got Err(AppNotFound { name: "../../escape",
requested_by: ["app", "../../escape"], searched: ["<tmp>/shipment/../../escape/ebin/../../escape.app",
"<tmp>/otp/lib/../../escape-<vsn>"] })
```

`ClosureError::InvalidAppName { name, requested_by }` is new. The check is in `locate`, before
any path is built, so an unusable name is not stat'ed even once, and the worklist raises the
error whether the name came from a seed (`--root ""`) or from an `.app` file. Its message names
the chain that asked for the name and states the rule; `AppNotFound` and it now share one
`required by:` writer.

### The four non-behavioural findings

- **The gated real-shipment test was keyed to one machine.** It reads `GINARY_TEST_SHIPMENT`
  now, defaulting to the same path, and escalates a missing directory exactly as `require_tools`
  escalates a missing program: a reported skip, or a panic under `GINARY_REQUIRE_TOOLCHAIN=1`.
  Verified both ways — the default path still runs the real closure, and
  `GINARY_REQUIRE_TOOLCHAIN=1 GINARY_TEST_SHIPMENT=/nonexistent/shipment` fails the test rather
  than skipping it. Recorded in `docs/dev/testing.md` beside the `require_tools` paragraph.
- **The `warnings:` footer of `ginary closure` had no test.**
  `closure_reports_an_application_taken_from_the_shipment_instead_of_otp` in `tests/cli.rs` puts
  `crypto` in both trees and asserts the heading and both directories.
- **`AppSet::is_empty` documented a condition it can never report.** The doc now says what is
  true: a default `AppSet` is empty, one `app_dependency_closure` returned never is, because
  `kernel` and `stdlib` are unconditional seeds.
- **`impl IntoIterator for &AppSet` had no caller.**
  `otp_apps_and_shipment_apps_partition_the_closure` now iterates the borrowed set and asserts it
  yields `names()`.

### Test corrections

None. No assertion was weakened, deleted or edited; the four snapshots are unchanged, and the
two closure tests that grew (`a_missing_root_is_reported_with_a_one_element_chain`, which now
also renders the singular `required by:` sentence, and the partition test above) only gained
assertions.

### The real shipment, again

`the_real_notify_shipment_closes_over_the_host_otp` was re-run after the refactor and resolves
the same 31 applications, from the same trees, with no warnings and no skipped optional
applications: the `explain()` table recorded in GREEN above is reproduced line for line. The
resolution rewrite changed which *problems* are errors, not which applications a healthy pair of
trees produces.

### Gates after fix round 1

| Gate | Result |
| --- | --- |
| `cargo fmt --all -- --check` | clean |
| `cargo clippy --all-targets --all-features -- -D warnings` | clean |
| `cargo test` | 248 passed, 0 failed |
| `RUSTDOCFLAGS=-Dwarnings cargo doc --no-deps` | clean |
| `cargo deny check` | advisories, bans, licenses, sources ok |

```
$ cargo test
   lib (src/*)          95 passed;  0 failed
   bin (src/main.rs)     0 passed;  0 failed
   tests/appfile.rs     51 passed;  0 failed
   tests/cli.rs         18 passed;  0 failed
   tests/closure.rs     34 passed;  0 failed
   tests/otp.rs         29 passed;  0 failed
   tests/regressions.rs 14 passed;  0 failed
   tests/smoke_cli.rs    6 passed;  0 failed
   doc-tests             1 passed;  0 failed
```

`cargo deny` still emits its unused `license-not-encountered` allowances, unchanged from A1a.

## Final gate

An independent run of every gate on the staged A1b tree, from a clean shell, with nothing
modified except this section.

| Gate | Command | Result |
| --- | --- | --- |
| format | `cargo fmt --all -- --check` | clean, exit 0 |
| lint | `cargo clippy --all-targets --all-features -- -D warnings` | clean, exit 0 |
| test | `cargo test` | 248 passed, 0 failed, 0 ignored |
| doc | `RUSTDOCFLAGS=-Dwarnings cargo doc --no-deps` | clean, exit 0 |
| deny | `cargo deny check` | advisories ok, bans ok, licenses ok, sources ok |
| gated test | `GINARY_REQUIRE_TOOLCHAIN=1 cargo test` | 248 passed, 0 failed, 0 ignored, no skip |

Per-binary summary lines from `cargo test`:

```
   unittests src/lib.rs    95 passed;  0 failed;  0 ignored
   unittests src/main.rs    0 passed;  0 failed;  0 ignored
   tests/appfile.rs        51 passed;  0 failed;  0 ignored
   tests/cli.rs            18 passed;  0 failed;  0 ignored
   tests/closure.rs        34 passed;  0 failed;  0 ignored
   tests/otp.rs            29 passed;  0 failed;  0 ignored
   tests/regressions.rs    14 passed;  0 failed;  0 ignored
   tests/smoke_cli.rs       6 passed;  0 failed;  0 ignored
   doc-tests ginary         1 passed;  0 failed;  0 ignored
```

`GINARY_REQUIRE_TOOLCHAIN=1 cargo test` produces the identical counts, and the run emits no
`skipping:` line, so every toolchain-gated test ran rather than passing vacuously. The gated
closure test `the_real_notify_shipment_closes_over_the_host_otp` was also re-run alone with
`--nocapture`: it prints the 31-application `explain()` table recorded above, with `asn1` 5.5,
`crypto` 5.9.2, `kernel` 11.0.3, `public_key` 1.21.4, `ssl` 11.7.4 and `stdlib` 8.0.3 resolved
from the host OTP, the other 25 from the shipment, and `skipped optional applications: []`.

`cargo deny` still prints its four `license-not-encountered` warnings for `BSD-3-Clause`,
`CDLA-Permissive-2.0`, `ISC` and `Zlib`, unchanged from A0 and A1a; they are allowances the
current dependency set does not exercise, not findings.

Spot checks alongside the gates: every `.rs` file under `src/` and `tests/` opens with the SPDX
line; `src/closure.rs` contains no `unwrap`, `expect`, `panic!`, `unreachable!` or `todo!`; no
emoji in the new source, tests or docs; the only lines past 100 columns in the touched Markdown
are table rows and fenced output, not prose.

`git status --short` shows 21 staged paths and no sandbox shim entry:

```
M  docs/dev/architecture.md
A  docs/dev/log/A1b.md
M  docs/dev/testing.md
M  src/appfile.rs
M  src/cli.rs
A  src/closure.rs
M  src/lib.rs
M  tests/cli.rs
A  tests/closure.rs
M  tests/common/fake_otp.rs
M  tests/common/mod.rs
A  tests/common/snapshot.rs
M  tests/regressions.rs
A  tests/regressions/a1b_an_ambiguous_optional_edge_was_an_error.rs
A  tests/regressions/a1b_app_file_error_repeated_its_cause.rs
A  tests/regressions/a1b_app_names_were_used_as_paths.rs
A  tests/regressions/a1b_shadowed_otp_ambiguity_aborted_the_closure.rs
A  tests/snapshots/cli__closure_explain_table.snap
A  tests/snapshots/closure__app_not_found_message.snap
A  tests/snapshots/closure__explain_table.snap
A  tests/snapshots/closure__shadowed_otp_application_warning.snap
```

Nothing is committed. A1b is green.
