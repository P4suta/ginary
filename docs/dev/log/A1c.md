<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# A1c — staging root assembly

Date: 2026-08-31 · Status: in progress

## Housekeeping

One documentation correction made before any A1c product code was written. No behaviour
changed, so the milestone starts from the A1b gate result.

### 1 — the `# Errors` section of `app_dependency_closure` was two rounds stale

The `# Errors` rustdoc on `app_dependency_closure` in `src/closure.rs` was written against the
first A1b draft and never updated afterwards. It was wrong in two ways at once.

It omitted `ClosureError::InvalidAppName` entirely. That variant is returned by the function —
`locate` answers `Resolution::Unusable` for a name that is empty or holds a separator, a `..`
or a NUL byte, and the worklist turns that into `InvalidAppName` before any path is built from
the name. `tests/regressions/a1b_app_names_were_used_as_paths.rs` pins all three shapes of it,
so the documented error list was missing a variant the tests require. A caller matching on
`ClosureError` from the documented list alone would have had no reason to expect it.

It also still stated the pre-fix `AmbiguousOtpApp` rule: "when the OTP library holds two
versions of one application". Fix round 1 of A1b reversed that. `locate` now probes the
shipment first and a shipment hit ends the lookup, so two OTP versions of an application the
shipment provides are a warning naming both ignored directories, not an error;
`tests/regressions/a1b_shadowed_otp_ambiguity_aborted_the_closure.rs` is exactly that case. The
variant's own doc comment was corrected at the time, but the function's `# Errors` section was
not, so rendered rustdoc stated the old rule and the new one a screen apart.

The section now matches the code and the regression tests:

```text
/// # Errors
///
/// [`ClosureError::AppNotFound`] when a required application is in neither
/// tree; [`ClosureError::InvalidAppName`] when a required name cannot be used
/// as a directory name, before any path is built from it;
/// [`ClosureError::AmbiguousOtpApp`] when a required application has to come
/// from the OTP library and the library holds more than one version of it —
/// a shipment copy ends the lookup, so an application the shipment provides
/// is a warning rather than an error however many OTP versions sit beside it;
/// and [`ClosureError::AppFile`] when an `.app` file cannot be read.
///
/// Only a *required* application reaches any of these. An optional dependency
/// that fails to resolve, for any of those reasons, is recorded in
/// [`AppSet::skipped_optional`] and [`AppSet::warnings`] instead.
```

The closing paragraph is the part A1c leans on. `assemble::stage` consumes an `AppSet`, never a
`Result`, so the set it is handed has already had every optional edge either resolved or
recorded, and staging can copy what the set names without asking again why something is not in
it. Staging does *not* read `AppSet::skipped_optional`, and nothing in the staged tree or in
`explain()` reports the optional applications the closure left out — reporting them beside the
sizes is owed work, and it is recorded here rather than claimed.

### 2 — working tree state

`git status --porcelain` is empty apart from the sandbox character-device shims, which are not
project files and were not touched. A0, A1a and A1b are committed as `9dfc5ce`, `449e8c3` and
`3604fb8`; the A1c work starts from `3604fb8` with no carried-over uncommitted changes.

### Gates after housekeeping

All five gates pass on the corrected tree:

| Gate | Result |
| --- | --- |
| `cargo fmt --all -- --check` | clean |
| `cargo clippy --all-targets --all-features -- -D warnings` | clean |
| `cargo test` | 248 passed, 0 failed (95 unit, 51 + 34 + 29 + 18 + 14 + 6 integration, 1 doctest) |
| `RUSTDOCFLAGS=-Dwarnings cargo doc --no-deps` | clean |
| `cargo deny check` | advisories, bans, licenses, sources ok |

`cargo deny` emits one `license-not-encountered` warning for the `Zlib` allowance in
`deny.toml`; no current dependency carries that licence. It is an unused allowance, not a
violation, and is left in place, as in A1b.

## RED

The tests, the fixture and the two new helpers are written; `assemble::stage` is not. The
public shape the tests call is declared in full — `StageOptions`, `StagedRoot`, `StagedApp`,
`StagedFile`, `StageListing`, `Category`, `StagedSource`, `ExcludedBin` and `AssembleError` —
and every function that can be honestly implemented over that data is. Exactly one body is a
stub, marked `// RED: replaced in GREEN`:

```rust
pub fn stage(
    set: &AppSet,
    otp: &OtpInfo,
    opts: &StageOptions,
    out: &Path,
) -> Result<StagedRoot, AssembleError> {
    // RED: replaced in GREEN
    let _ = (set, otp, opts, out);
    Err(AssembleError::NotImplemented)
}
```

`AssembleError::NotImplemented` is the one variant that exists only for this phase, and its
rustdoc says so. Nothing else is a placeholder: `total_bytes`, `bytes_by_category`, `listing`,
`explain`, `excluded_reason` and every `Display` are real implementations over data that
`stage` will fill in, and `ginary stage` is wired end to end — it computes the closure, calls
`stage`, and prints the error it gets. No command claims to work.

### The fixture

`tests/fixtures/hello_ffi/` is a real Gleam project with an empty `[dependencies]`, so
`gleam build` and `gleam export erlang-shipment` need no network and no warmed hex cache.
`gleam` 1.18.1 was run in it and the output recorded:

```console
$ cd tests/fixtures/hello_ffi && gleam build && gleam export erlang-shipment
  Compiling hello_ffi
   Compiled in 0.32s
  Compiling hello_ffi
   Compiled in 0.31s
   Exported hello_ffi
```

| source (committed) | bytes | export output | bytes |
|---|---|---|---|
| `gleam.toml` | 572 | `build/erlang-shipment/entrypoint.sh` | 618 |
| `manifest.toml` | 321 | `build/erlang-shipment/entrypoint.ps1` | 708 |
| `priv/greeting.txt` | 16 | `hello_ffi/ebin/hello_ffi.app` | 209 |
| `src/hello_ffi.gleam` | 566 | `hello_ffi/ebin/hello_ffi.beam` | 1604 |
| `src/hello_ffi_ffi.erl` | 1523 | `hello_ffi/ebin/hello_ffi@@main.beam` | 7708 |
| | | `hello_ffi/ebin/hello_ffi_ffi.beam` | 2436 |
| | | `hello_ffi/priv/greeting.txt` | 16 |

`manifest.toml` is committed — Gleam's own header in it says to — and it locks the project to
no packages at all, which is the property the fixture exists for. `build/` is git-ignored
through the existing `tests/fixtures/*/build/` pattern and `FixtureProject::copy` skips it, so
no test ever builds the fixture in place.

The generated entry point is `hello_ffi@@main:run/1`, and the `.app` file lists an empty
`applications`, so the closure over this shipment is `hello_ffi`, `kernel` and `stdlib` — three
applications, of which one is staged as `lib/hello_ffi` and two as `lib/<name>-<vsn>`.

The launch contract in `tests/common/erl.rs` was checked by hand against the *unstaged*
shipment and the host OTP root before it was written down, so a `tests/stage_run.rs` failure
in GREEN will be a failure of `stage` and not of the helper:

```console
$ env -i ROOTDIR=$OTP BINDIR=$OTP/erts-17.0.5/bin EMU=beam PROGNAME=hello_ffi \
      HOME=$T/home PATH=$T/emptypath ERL_CRASH_DUMP=$T/home/erl_crash.dump \
      $OTP/erts-17.0.5/bin/erlexec -boot $OTP/bin/no_dot_erlang -noshell +B \
      -start_epmd false -pa $SHIP/hello_ffi/ebin \
      -eval "'hello_ffi@@main':run('hello_ffi')" -extra 3 a b
args=3 a b
hello from priv
cwd=/tmp/tmp.FDrK1S9SKl/cwd
$ echo $?
3
```

and with `-extra --crash`, exit 1, `runtime error: Erlang error` on standard error, and no
`erl_crash.dump` in either the working directory or `HOME`.

### Failing tests

44 new tests fail, every one of them on an assertion or on an `Err` that reached the test —
none on a compile error. `cargo build`, `cargo fmt --all -- --check`,
`cargo clippy --all-targets --all-features -- -D warnings`,
`RUSTDOCFLAGS=-Dwarnings cargo doc --no-deps` and `cargo deny check` are all clean, so the
suite is red for one reason only.

`tests/assemble.rs` — 33 of 33, all through `assemble::stage`:

```
a_boot_file_naming_a_version_that_is_not_staged_is_an_error
a_dangling_symlink_is_refused
a_failed_staging_leaves_neither_an_output_nor_a_temporary_directory
a_missing_required_erts_binary_is_an_error
a_non_empty_output_directory_is_an_error
a_priv_file_keeps_the_mode_it_had_in_the_source_tree
a_symlink_inside_the_application_directory_is_copied_as_a_file
a_symlink_pointing_out_of_the_application_directory_is_refused
an_appup_beside_an_app_file_is_not_staged
an_empty_output_directory_is_accepted
an_extra_binary_is_not_also_reported_as_excluded
an_extra_binary_the_runtime_does_not_have_is_an_error
every_erts_binary_that_was_not_staged_is_listed_with_a_reason
every_file_is_put_in_the_category_the_report_will_add_up
every_staged_erts_binary_stays_executable
explain_reports_the_sizes_the_applications_the_exclusions_and_the_junk
force_replaces_a_non_empty_output_directory
keep_junk_leaves_the_junk_in_place_and_records_nothing
staging_the_same_inputs_twice_produces_an_identical_listing
staging_the_same_inputs_twice_produces_identical_trees
the_boot_references_that_were_checked_are_reported
the_category_totals_sum_to_the_total_bytes
the_extra_binaries_are_staged_beside_the_required_four
the_junk_files_are_removed_and_recorded_with_their_sizes
the_listing_lists_every_file_sorted_by_path_and_never_itself
the_listing_names_the_erts_version_the_release_and_the_otp_version
the_listing_on_disk_round_trips_through_serde
the_listing_records_the_mode_of_every_file_it_wrote
the_source_include_doc_examples_c_src_and_mibs_directories_are_not_staged
the_staged_applications_name_their_version_source_and_directory
the_staged_root_is_the_directory_that_was_asked_for
the_staged_tree_holds_exactly_the_expected_paths
the_total_bytes_are_the_size_of_the_tree_the_listing_aside
```

`tests/stage_run.rs` — 6 of 6, gated on `gleam` and `erl`, both present on this machine, so
every one of them really ran a `gleam export erlang-shipment` before it failed:

```
a_crash_exits_one_and_leaves_no_dump_in_the_working_directory
a_staged_hello_ffi_exits_zero_when_the_first_argument_is_zero
a_staged_hello_ffi_prints_its_arguments_and_its_priv_file
running_the_staged_root_does_not_change_a_byte_of_it
the_staged_hello_ffi_has_bytes_in_every_category_an_artifact_needs
the_staged_root_holds_no_sources_and_the_kernel_the_boot_file_names
```

`tests/cli.rs` — 5 of the 7 added:

```
stage_explain_names_the_binaries_it_left_out
stage_json_carries_the_documented_keys
stage_refuses_a_non_empty_output_directory_and_exits_one
stage_with_force_replaces_a_non_empty_output_directory
stage_writes_the_tree_and_prints_the_totals
```

The other two — `the_help_lists_the_stage_command` and
`stage_without_an_out_directory_is_a_usage_error` — pass, and are recorded here rather than
weakened. They pin the clap surface, and the clap surface is what had to exist for the other
five to be assertion failures instead of compile errors: the subcommand parses, `--out` is
required, and neither fact says anything about whether staging works. Both are kept because
the flag set is a contract of its own.

Representative failures, one per file:

```
---- the_staged_tree_holds_exactly_the_expected_paths stdout ----
thread 'the_staged_tree_holds_exactly_the_expected_paths' panicked at tests/assemble.rs:143:27:
staging should succeed: staging is not implemented yet

---- a_boot_file_naming_a_version_that_is_not_staged_is_an_error stdout ----
thread 'a_boot_file_naming_a_version_that_is_not_staged_is_an_error' panicked at tests/assemble.rs:473:18:
expected BootReferencesMissingApp, got NotImplemented

---- the_staged_hello_ffi_has_bytes_in_every_category_an_artifact_needs stdout ----
thread 'the_staged_hello_ffi_has_bytes_in_every_category_an_artifact_needs' panicked at tests/stage_run.rs:71:6:
the fixture stages: NotImplemented

---- stage_refuses_a_non_empty_output_directory_and_exits_one stdout ----
thread 'stage_refuses_a_non_empty_output_directory_and_exits_one' panicked at tests/cli.rs:625:5:
error: staging is not implemented yet
```

### Suite totals

| target | result |
| --- | --- |
| `src` unit tests | 95 passed |
| `tests/appfile.rs` | 51 passed |
| `tests/assemble.rs` | **0 passed, 33 failed** |
| `tests/cli.rs` | 20 passed, **5 failed** |
| `tests/closure.rs` | 34 passed |
| `tests/otp.rs` | 29 passed |
| `tests/regressions.rs` | 14 passed |
| `tests/smoke_cli.rs` | 6 passed |
| `tests/stage_run.rs` | **0 passed, 6 failed** |
| doctests | 1 passed |

No previously passing test regressed.

### What the RED phase deliberately did not produce

Two things the milestone owes are only writable once `stage` returns an `Ok`, and inventing
them now would be recording an output nobody has seen:

- **`tests/snapshots/assemble__stage_explain_table.snap`.** The `explain` test exists and
  fails at `stage`, before `insta` is reached, so no `.snap` and no `.snap.new` is written.
  The snapshot is reviewed and committed in GREEN, and until then the format of the table is
  pinned only by the assertions in `tests/cli.rs` that read the first line and the section
  headings.
- **The size table for `hello_ffi`.** The spec asks for the project's first real size number,
  by category, before any stripping. `the_staged_hello_ffi_has_bytes_in_every_category_an_artifact_needs`
  asserts every category is non-empty and prints `StagedRoot::explain()` on standard error for
  exactly this purpose; the numbers go into this log in GREEN.

Every other assertion is exact now: the twenty-two staged paths and their order, the three junk
removals and their byte counts, the six excluded ERTS binaries and their reasons, the category
of nine named files, the six staged applications with their versions, sources and directories,
and the two boot references. None of them was written to be easy to satisfy.

## GREEN

`assemble::stage` is implemented. The one RED stub and `AssembleError::NotImplemented` are
gone, and every test written in RED passes unchanged except for one corrected assertion,
recorded below.

### What `stage` does

The function is a validation, a build and a rename, in that order.

1. **`prepare_output`** — `out` may be absent or an empty directory; anything else is
   `OutputNotEmpty` unless `force`, which removes it. Neither path touches the filesystem when
   the answer is "acceptable", so a staging that fails later has destroyed nothing.
2. **`build`** into `<out>.tmp-<pid>` — the four required ERTS binaries and the `--extra-bin`
   names (a missing one is `MissingErtsBinary` / `MissingExtraBinary`, never a skip);
   `bin/no_dot_erlang.boot`; then every application's `ebin` and `priv`; then the boot
   cross-check; then junk removal; then `ginary.stage.json`.
3. **`publish`** — `rename` onto `out`. Every error path between step 2 and here removes the
   temporary tree, which is what `a_failed_staging_leaves_neither_an_output_nor_a_temporary_directory`
   pins.

The copy is `std::fs::copy`, which carries the source's permission bits, so an executable NIF
stays executable and a data file does not become one without a single explicit `chmod`.
Directory entries are visited in sorted order, so two stagings of the same input walk the same
tree in the same order.

Symlinks are resolved with `canonicalize` against the canonical application directory: inside
it, the file is dereferenced and copied as a plain file; outside it, or dangling, it is
`UnsafeSymlink`. The staged tree therefore holds no symlinks at all, which is what the payload
format needs and what the launcher's extract step will assume.

### Two policy decisions taken in GREEN

Both are narrower than a first reading of the spec, and both are narrower on purpose.

**`EXCLUDED_APP_DIRS` is not a filter that runs at every depth.** `src`, `include`, `doc`,
`examples`, `c_src` and `mibs` are siblings of `ebin` and `priv` in an application directory,
and staging copies `ebin` and `priv` only, so they are left behind structurally. An earlier
draft also pruned those names *inside* `ebin` and `priv`, and that is wrong: `snmp` ships its
compiled MIBs as `priv/mibs/*.bin` and loads them at run time, so a name filter at depth would
have produced an artifact that failed only when the application looked for one. The constant
now documents what the allowlist leaves behind rather than naming a delete pass, and its
rustdoc says why nothing under `priv` is pruned by name.

**Junk removal matches the plan's globs exactly, at the top of `priv`.** `priv/obj/` and
`priv/lib/*.a` are direct children, as `*/priv/obj/**` and `*/priv/lib/*.a` are in the plan,
and `otp_test_engine.so` is removed for `crypto` alone — it is OpenSSL's test engine, and a
file of that name under another application would be somebody's own. A recursive hunt for any
directory called `obj` would delete more than the plan sanctions and would do it silently.

### GREEN / test corrections

One assertion, in `tests/cli.rs::stage_writes_the_tree_and_prints_the_totals`.

It read

```rust
assert!(stdout.starts_with("category  bytes  files\n"), ...);
```

which cannot pass for any staged tree. The table comes from `closure::render_table`, which pads
every column but the last to its widest cell; `erts_binary` (11) and `app_resource` (12) are
both wider than `category` (8), so the header line is always `category      bytes  files`. The
assertion pinned a two-space padding that the shared renderer can never produce, and no change
to `assemble` could have satisfied it. It now pins the same fact — the first line of the
default output is the three-column header, in that order — without pinning the padding:

```rust
assert_eq!(
    stdout
        .lines()
        .next()
        .map(|line| line.split_whitespace().collect::<Vec<_>>()),
    Some(vec!["category", "bytes", "files"]),
    "the default output is the per-category table:\n{stdout}"
);
```

This is the same tolerance `appfile_parse_prints_one_labelled_block_per_file` already uses for
its own table (`stdout.starts_with("name ")`). Nothing else was changed, weakened or deleted:
the other 43 tests written in RED pass against the implementation as written.

### The snapshot RED could not produce

`tests/snapshots/assemble__stage_explain_table.snap` is now committed. It was generated from
the six-application scenario and read before being accepted: the category totals sum to the
`total` row (1319 bytes over 21 files), the six applications carry the versions and sources the
closure resolved, the six excluded programs carry the reasons `excluded_reason` gives, the three
junk removals carry their exact byte counts, and the two boot references are the ones checked.

### The first real size number

`hello_ffi`, staged against the host OTP 29.0.5 (ERTS 17.0.5), with default options and **no
stripping of any kind** — this is the number every later milestone is measured against.

| category | bytes | files |
|---|---:|---:|
| `erts_binary` | 56,602,456 | 4 |
| `boot` | 7,060 | 1 |
| `otp_beam` | 10,147,388 | 202 |
| `gleam_beam` | 11,508 | 3 |
| `priv` | 16 | 1 |
| `app_resource` | 7,140 | 3 |
| **total** | **66,775,568** | **214** |

| app | vsn | source | files | bytes |
|---|---|---|---:|---:|
| `hello_ffi` | 0.1.0 | shipment | 5 | 11,733 |
| `kernel` | 11.0.3 | otp | 105 | 3,033,060 |
| `stdlib` | 8.0.3 | otp | 99 | 7,121,259 |

63.7 MiB for an application whose own code is 11.5 KB. Two facts stand out and both are A2's
work:

- `beam.smp` alone is 56,169,608 bytes of the 56,602,456 in `erts_binary` — 99.2 % of it, and
  84 % of the whole tree. The other three (`erlexec` 174,824, `inet_gethost` 186,848,
  `erl_child_setup` 71,176) are noise beside it. It is an unstripped ELF, so `strip --strip-all`
  is the single largest lever the project has.
- `kernel` and `stdlib` contribute 202 `.beam` files and 10.1 MB, unstripped, with their debug
  and literal chunks intact. `beam_lib:strip_release` is the second lever.

Sixteen programs in the runtime's `bin` were left behind, each with its reason, and the tree
holds no `src` at any depth — the assertions in `tests/stage_run.rs` that say so ran against
this staged root.

### Gates

| Gate | Result |
| --- | --- |
| `cargo fmt --all -- --check` | clean |
| `cargo clippy --all-targets --all-features -- -D warnings` | clean |
| `cargo test` | 294 passed, 0 failed |
| `RUSTDOCFLAGS=-Dwarnings cargo doc --no-deps` | clean |
| `cargo deny check` | advisories ok, bans ok, licenses ok, sources ok |

Per target, with `GINARY_REQUIRE_TOOLCHAIN=1` so the gated file cannot skip silently:

```
src unit tests        95 passed
tests/appfile.rs      51 passed
tests/assemble.rs     33 passed
tests/cli.rs          25 passed
tests/closure.rs      34 passed
tests/otp.rs          29 passed
tests/regressions.rs  14 passed
tests/smoke_cli.rs     6 passed
tests/stage_run.rs     6 passed
doctests               1 passed
```

`cargo deny` still emits `license-not-encountered` warnings for the unused `ISC` and `Zlib`
allowances in `deny.toml`; they are allowances no current dependency uses, not violations, and
are left as they were in A1a and A1b.

## Fix round 1

The first adversarial review of the milestone returned thirteen findings: two high, four medium
and seven low. Four are behavioural, and each of those got a failing test before anything was
fixed. The three that are defects in `src/assemble.rs` have a file under `tests/regressions/`;
the fourth is a gap in the command line's coverage rather than a defect, so its tests went to
`tests/cli.rs` and were watched failing against a deliberately broken wiring instead.

All three product defects have the same shape. Assembly checks what it finds *inside* a
directory and trusted the directory it was given, or turned a path into text and treated the
failure as nothing to do. Both are the same rule from CLAUDE.md read twice: skipping is a
reported decision or an error, never a default.

### 1 — a symlinked `ebin` or `priv` was followed out of the application (high)

`copy_tree` applied the `UnsafeSymlink` boundary check to every entry it read out of a
directory, and to none of the two directories `stage_apps` called it with. It opened with
`create_dir(to)` and `entries_of(from)`, and `read_dir` follows a symlink, so an application
whose `priv` was a link to somewhere else on the build machine had that directory copied into
the artifact whole — the exact outcome the variant's own rustdoc says cannot happen.

`tests/regressions/a1c_a_symlinked_ebin_or_priv_escaped_the_app.rs`, before the fix:

```
---- a1c_..._escaped_the_app::a_priv_that_is_a_symlink_out_of_the_application_is_refused stdout ----
panicked at tests/regressions/a1c_a_symlinked_ebin_or_priv_escaped_the_app.rs:116:10:
a priv that leaves the application is refused rather than followed: StagedRoot { ...
  StagedFile { path: "lib/notify/priv/secrets.txt", size: 20, mode: 436, category: Priv }, ... }

---- a1c_..._escaped_the_app::an_ebin_that_is_a_symlink_out_of_the_application_is_refused stdout ----
panicked at tests/regressions/a1c_a_symlinked_ebin_or_priv_escaped_the_app.rs:145:10:
an ebin that leaves the application is refused rather than followed: StagedRoot { ...
  StagedFile { path: "lib/notify/ebin/secrets.txt", size: 20, mode: 436, category: Other }, ... }
```

`secrets.txt` was never inside the application, and both stagings exited zero.

`copy_tree` is now `copy_subtree`, which stats `from` with `symlink_metadata` before it reads
anything out of it and puts a symlinked root through the same `resolve_link` as a symlinked
child.

### 2 — a file whose name is not valid UTF-8 vanished from the tree (high)

Three places turned a path into text with `to_str()` and treated `None` as nothing to do: the
recursive copy skipped the entry, `excluded_bins` dropped the program from `--explain`, and
`slash_path` filtered the component out of the path it recorded. Staging reported success and
the listing never mentioned the file.

`tests/regressions/a1c_a_non_utf8_file_name_was_dropped.rs`, before the fix — a `priv` holding
`caf\xe9.dat` staged as if it were empty:

```
panicked at tests/regressions/a1c_a_non_utf8_file_name_was_dropped.rs:80:10:
a file that cannot be named is refused rather than dropped: StagedRoot { ...
  StagedFile { path: "lib/notify/priv/greeting.txt", size: 16, mode: 436, category: Priv }, ... }
```

`AssembleError::NonUtf8Name` is the new answer, raised from a single `file_name_of` helper that
the copy, the exclusion list and junk removal all go through, and from `walk` when
`slash_path` — now returning `Option<String>` — cannot spell a component. The rule it enforces
is worth stating plainly: the listing is text, and an artifact holding a file its own index
cannot name is worse than one that was not built.

### 3 — a symlinked directory looped, and stepped around the exclusion (medium)

The copy resolved a link, asked `resolved.is_dir()` and recursed, with no record of the
directories it had already entered and no second look at the structural exclusion.
`priv/loop -> .` therefore described a tree of infinite depth. The reviewer reproduced an
`ENAMETOOLONG` naming an innocent file; on this machine it does not get that far:

```
test a1c_a_symlinked_directory_looped_or_leaked::a_symlink_that_points_at_its_own_directory_is_refused_by_name ...
thread '...' (671624) has overflowed its stack
fatal runtime error: stack overflow, aborting
error: test failed, to rerun pass `--test regressions`
Caused by:
  process didn't exit successfully: ... (signal: 6, SIGABRT: process abort signal)
```

A crash that takes the whole test binary with it is the strongest possible RED, and it is what
`ginary stage` would have done to a user with a looping link in a `priv`.

The second half of the finding is the exclusion bypass. `ebin/sources -> ../src` resolved to a
directory that is under the *application* root, which was the only boundary, so the sources went
into the artifact:

```
panicked at tests/regressions/a1c_a_symlinked_directory_looped_or_leaked.rs:135:10:
a link into the application's sources is refused rather than followed: StagedRoot { ...
  StagedFile { path: "lib/notify/ebin/sources/leak.gleam", size: 22, ... category: Other }, ... }
```

`docs/dev/architecture.md`, `tests/assemble.rs` and `tests/stage_run.rs` all assert that `src`
never travels, and none of the three used a symlink.

The copy is now a `TreeCopy` carrying two boundaries and a stack. A link to a *file* may still
point anywhere inside the application; a link to a *directory* may not leave the `ebin` or
`priv` being copied, because a structural rule a symlink can step around is not a rule. The
stack of canonical directories the copy is inside makes a link back to an ancestor
`AssembleError::SymlinkCycle` naming the link, rather than a recursion that ends when the
machine runs out of stack or the filesystem runs out of path. And `copy_file` now raises
`AssembleError::Copy`, which names the destination as well as the source: the reviewer's
original `ENAMETOOLONG` blamed `greeting.txt`, whose name was fine, because the failure was on
the path being written.

### 4 — `--keep-junk` and `--extra` had no test at any level (medium)

Both flags are named CLI deliverables and neither was exercised through clap. Four tests went
into `tests/cli.rs`, and the fake `crypto` grew a `priv/lib/libcrypto_static.a` and the runtime
an unreferenced `sasl` so that there was something to keep and something to add. They were
watched failing against an inverted wiring — `remove_junk: *keep_junk` and `extra: &[]` in
`src/cli.rs`, reverted immediately after:

```
---- stage_with_keep_junk_keeps_the_files_the_default_deletes stdout ----
panicked at tests/cli.rs:672:5:
--keep-junk has to reach StageOptions::remove_junk, or the file is gone

---- stage_without_keep_junk_removes_the_same_files_and_says_so stdout ----
panicked at tests/cli.rs:690:5:
the default is to remove the junk

---- stage_with_an_extra_application_stages_it_beside_the_closure stdout ----
panicked at tests/cli.rs:711:5:
--extra has to reach the closure the staging is built from:
category      bytes  files
...
staged 18 files, 1133 bytes, into /tmp/.tmpBmtxYN/out
```

The fourth, `stage_without_the_extra_application_leaves_it_out`, passes either way by design: it
is the negative control that says nothing reaches the tree the closure did not ask for.

`src/cli.rs` itself is unchanged — the wiring was right, and now it cannot be broken silently.

### 5 — the two helpers that spawn a real program had no timeout (medium)

`run_staged` and `FixtureProject::export_shipment` both called `Command::output()`: no deadline
and an inherited stdin. The one place in the suite that boots a whole BEAM was the only place
with no bound at all, in a project that already owns `process::run_with_timeout` for exactly
this hazard.

`src/process.rs` cannot be called from either: it takes a program and `&str` arguments and
returns captured text, and both callers need an environment built from nothing, a working
directory, `OsString` arguments and the child's exit code. `tests/common/bounded.rs` is the
test-side counterpart, and it borrows the discipline rather than the signature — stdin on the
null device, both pipes drained by threads of their own, and a deadline after which the child is
killed and named. The budgets are `fixture::EXPORT_BUDGET` (180 s) and `erl::RUN_BUDGET` (60 s),
against a fixture that exports in under a second and runs in a tenth of one, and both are in the
`docs/dev/testing.md` table beside every other external process the suite starts.

### 6 — `run_staged` is a subset of ADR 0003, not its specification (medium)

The helper claimed to be "the executable specification of the launch contract ADR 0003 records"
and contradicted it twice: the ADR describes a launcher that inherits the environment and
removes a denylist (`ERL_LIBS`, `ERL_FLAGS`, `ERL_AFLAGS`, `ERL_ZFLAGS`, `ERL_OTP*_FLAGS`,
`ERL_ROOTDIR`, `ERL_EPMD_PORT`) and that sets `HOME` and `ERL_CRASH_DUMP` only when the user has
not, where the helper clears the environment and sets both unconditionally. The ADR's `+fnu`,
`-args_file` and `-config` are absent from it too.

The ADR is not amended: a test that inherited the developer's environment could not assert on
anything, so clearing it is right *for a test*, and the launcher's rule is right for a launcher.
What changes is the claim. `tests/common/erl.rs`, `tests/stage_run.rs` and `docs/dev/testing.md`
now call it a hermetic subset and say what is outside the overlap: a `LaunchPlan` that agrees
with this function still needs its own tests over an inherited environment before A3 can call
the contract covered.

### 7 to 13 — the low findings

- `the_source_include_doc_examples_c_src_and_mibs_directories_are_not_staged` rejected the six
  names at *any* depth, which is a rule `src/assemble.rs` deliberately does not have and which
  a realistic `priv/mibs/*.bin` would have broken. It now asserts the exclusion where the module
  applies it — the top level of an application, whose entries must be `ebin` and `priv` and
  nothing else — and a new test stages a `priv/mibs/OTP-CRYPTO.bin` to pin the depth policy from
  the other side. `tests/stage_run.rs` had the same over-broad assertion over `src` and is
  scoped the same way, over all six names rather than one.
- `docs/dev/testing.md` and `tests/common/fixture.rs` both said `hello_ffi` has no committed
  `manifest.toml`. It has one, and it locks zero packages, which is the stronger claim: nothing
  has to be resolved from hex. Both now say that.
- This log claimed staging reads `AppSet::skipped_optional`. It does not, and nothing in the
  staged tree or in `explain()` reports skipped optional applications. The housekeeping section
  now records that as owed work rather than as shipped behaviour.
- `tests/fixtures/hello_ffi/src/hello_ffi.gleam` had no SPDX header, alone among the fixture's
  files. `REUSE.toml`'s aggregate annotation is why no gate caught it.
- The crash test asserted on the relative path `erl_crash.dump`, which resolves in the shared
  crate root rather than in anything the test owns; a stray dump from anything else would have
  failed it. The assertion above it already covers the meaningful case with an owned absolute
  path, so the relative one is gone.
- "no temporary directory left behind" walked files only, so an *empty* `<out>.tmp-<pid>` would
  have passed it. It now asserts on the work directory's entries as well, and its comment names
  `inet_gethost` — the last of `otp::REQUIRED_ERTS_BINARIES` and the file the test removes —
  rather than `beam.smp`.
- `docs/dev/architecture.md` had the `inspect_root` paragraph stranded below the new staging
  section, where it read as its conclusion. It is back in the runtime-discovery discussion it
  belongs to, and the ragged wrap in the junk-removal paragraph is fixed. The staging section's
  symlink sentence now states both boundaries and the two new error variants.

### Gates after fix round 1

| Gate | Result |
| --- | --- |
| `cargo fmt --all -- --check` | clean |
| `cargo clippy --all-targets --all-features -- -D warnings` | clean |
| `cargo test` | 305 passed, 0 failed |
| `RUSTDOCFLAGS=-Dwarnings cargo doc --no-deps` | clean |
| `cargo deny check` | advisories ok, bans ok, licenses ok, sources ok |

Per target, with `GINARY_REQUIRE_TOOLCHAIN=1` so the gated file cannot skip silently:

```
src unit tests        95 passed
tests/appfile.rs      51 passed
tests/assemble.rs     34 passed   (+1: the priv/mibs positive case)
tests/cli.rs          29 passed   (+4: --keep-junk, the default, --extra, and its control)
tests/closure.rs      34 passed
tests/otp.rs          29 passed
tests/regressions.rs  20 passed   (+6: the three A1c files)
tests/smoke_cli.rs     6 passed
tests/stage_run.rs     6 passed
doctests               1 passed
```

The two `license-not-encountered` warnings for the unused `ISC` and `Zlib` allowances in
`deny.toml` are unchanged from A1a, A1b and the GREEN run above.

## Final gate

Independent re-run of every gate on the staged A1c tree (nothing committed), toolchain
present: `gleam` 1.18.1 at `~/.local/share/mise/installs/gleam/latest/gleam`, `erl` at
`~/.local/share/mise/installs/erlang/latest/bin/erl`.

| Gate | Command | Result |
| --- | --- | --- |
| fmt | `cargo fmt --all -- --check` | pass, no diff |
| clippy | `cargo clippy --all-targets --all-features -- -D warnings` | pass, no warnings |
| test | `cargo test` | pass, 305 tests, 0 failed, 0 ignored |
| doc | `RUSTDOCFLAGS=-Dwarnings cargo doc --no-deps` | pass |
| deny | `cargo deny check` | advisories ok, bans ok, licenses ok, sources ok |
| test (toolchain required) | `GINARY_REQUIRE_TOOLCHAIN=1 cargo test` | pass, same 305 tests, 0 skipped |

Per-binary summaries, identical for the plain and the `GINARY_REQUIRE_TOOLCHAIN=1` run:

```
unittests src/lib.rs    95 passed
unittests src/main.rs    0 passed
tests/appfile.rs        51 passed
tests/assemble.rs       34 passed
tests/cli.rs            29 passed
tests/closure.rs        34 passed
tests/otp.rs            29 passed
tests/regressions.rs    20 passed
tests/smoke_cli.rs       6 passed
tests/stage_run.rs       6 passed
doctests                 1 passed
```

The six toolchain-gated `tests/stage_run.rs` tests really execute rather than skip: under
`--nocapture` the run prints the staged `explain()` table with the live OTP 29.0.5 apps
(`kernel-11.0.3`, `stdlib-8.0.3`, `hello_ffi-0.1.0` from the exported shipment) and the boot
references it checked, and `require_tools` emits no `skipping:` line in either run.

`cargo deny check` still reports only the three `license-not-encountered` warnings for the
unused `CDLA-Permissive-2.0`, `ISC` and `Zlib` allowances in `deny.toml`, unchanged since A1a.

`git status --short` lists 25 paths, all staged, none of them sandbox shim files, and
`--untracked-files=all` reports nothing untracked:

```
M  docs/dev/architecture.md
A  docs/dev/log/A1c.md
M  docs/dev/testing.md
A  src/assemble.rs
M  src/cli.rs
M  src/closure.rs
M  src/lib.rs
A  tests/assemble.rs
M  tests/cli.rs
A  tests/common/bounded.rs
A  tests/common/erl.rs
M  tests/common/fake_otp.rs
A  tests/common/fixture.rs
M  tests/common/mod.rs
M  tests/regressions.rs
A  tests/regressions/a1c_a_non_utf8_file_name_was_dropped.rs
A  tests/regressions/a1c_a_symlinked_directory_looped_or_leaked.rs
A  tests/regressions/a1c_a_symlinked_ebin_or_priv_escaped_the_app.rs
A  tests/snapshots/assemble__stage_explain_table.snap
A  tests/stage_run.rs
A  tests/fixtures/hello_ffi/gleam.toml
A  tests/fixtures/hello_ffi/manifest.toml
A  tests/fixtures/hello_ffi/priv/greeting.txt
A  tests/fixtures/hello_ffi/src/hello_ffi.gleam
A  tests/fixtures/hello_ffi/src/hello_ffi_ffi.erl
```
