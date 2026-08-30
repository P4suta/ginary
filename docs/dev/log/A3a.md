<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# A3a — the payload format

Date: 2026-08-31 · Status: in progress

## Housekeeping

One test-coverage gap carried over from the A2 review, closed before any A3a product code was
written. No production code changed in this section; `src/` is byte-identical to `ec79070`.

### 1 — `ElfInfo::kind` was documented but asserted by no test (low)

`docs/dev/debugging.md` names `kind` in the `ginary elf deps --json` object and spells out its
four values, and `docs/dev/architecture.md` explains that `kind` is what decides whether `strip`
gets `--strip-unneeded` or `--strip-all`. Nothing in the suite read the key. Both the
`#[serde(rename_all = "snake_case")]` on `ElfKind` and the `e_type` mapping in `ElfKind::of`
could have been changed without a test noticing, and the JSON is a documented contract.

`elf_deps_json_carries_the_documented_keys` in `tests/cli.rs` is where the rest of that object is
asserted, so the two assertions go there rather than into a test of their own.

**The `ET_DYN` half** is the binary the test already runs against: a cargo binary is a
position-independent executable, so it must report `kind: "shared_object"` together with
`is_pie: true` — the pairing `docs/dev/debugging.md` calls out as the one that surprises people.
`is_pie` was previously asserted only to *be* a boolean; it is now asserted to be `true`.

**The non-`ET_DYN` half** needs a file the toolchain does not produce here, so the test makes
one: it copies the test binary's bytes and patches `e_type`, the two little-endian bytes at
offset 16 of the header, to `ET_EXEC`. That is exactly the one field `kind` reads, which keeps
the test honest about what it is covering — everything else about the file, `interp` included,
is unchanged, and `interp` is precisely the field that cannot tell the two apart. The patched
copy must report `kind: "executable"` and `is_pie: false`.

**RED evidence.** The test is a coverage fill, not a bug fix, so it passes against unmodified
`src/`. Each half was verified to be load-bearing by mutating the code it covers and watching it
fail:

`#[serde(rename_all = "snake_case")]` → `"camelCase"` on `ElfKind`:

```text
thread 'elf_deps_json_carries_the_documented_keys' panicked at tests/cli.rs:961:5:
assertion `left == right` failed: a cargo binary is a position-independent executable, and
`e_type` calls that an `ET_DYN` like any other shared object: {"files":[{"class":64,
"glibc_max":"2.39","interp":"/lib64/ld-linux-x86-64.so.2","is_pie":true,"kind":"sharedObject",
...}],"format_version":1}
  left: String("sharedObject")
 right: String("shared_object")
```

`object::elf::ET_EXEC => Self::Executable` → `Self::SharedObject` in `ElfKind::of`:

```text
thread 'elf_deps_json_carries_the_documented_keys' panicked at tests/cli.rs:985:5:
  left: String("shared_object")
 right: String("executable")
```

Both mutations were reverted; `src/elf.rs` is unchanged from `ec79070`.

**GREEN.** `cargo test`: 442 passed, 0 failed, across 15 binaries. `cargo fmt --all -- --check`,
`cargo clippy --all-targets --all-features -- -D warnings`, `RUSTDOCFLAGS=-Dwarnings cargo doc
--no-deps` and `cargo deny check` (advisories, bans, licenses, sources all ok) are clean.

### 2 — the working tree

`git status` is clean apart from the sandbox character-device shims `CLAUDE.md` lists, which are
not project files and are never staged. A2 is committed as `ec79070`. The only path staged for
this section is `tests/cli.rs`, plus this log.

## RED

The four modules of the payload format — `trailer`, `manifest`, `payload`, `diag` — exist in
`src/` with their whole public API declared and every body a stub marked `// RED: replaced in
GREEN`. A stub returns an explicit `Err(<Error>::NotImplemented)`, or the empty value its
signature admits when it cannot return an error (`Trailer::to_bytes` gives 64 zero bytes,
`Trailer::cache_key` an empty string). Nothing is `todo!()`, nothing panics, and the crate
compiles, so **every assertion below fails on an assertion or on an `Err`, not on a compile
error**. That is the whole point of the shape: a red suite that only fails to build proves
nothing about the API it is supposed to be pinning.

### Dependencies added

`tar`, `zstd`, `sha2` and `hex` join `[dependencies]`, each justified in a comment beside it in
`Cargo.toml`. `zstd` is `default-features = false` so that the multi-threaded encoder is not
compiled in: packing is single-threaded on purpose, because a thread count that varies with the
machine varies the output bytes and the format's first promise is that it does not.
`dev-dependencies` are unchanged; a `tests/` target may use a normal dependency, which is how the
hand-built archives are read back with `tar` and hashed with `sha2`.

`libc` is in the milestone's list and is **not** added: nothing in the RED tests or in the
planned GREEN bodies calls it, and `CLAUDE.md` forbids a dependency the milestone does not use.
If mode handling on extraction turns out to need `umask(2)`, it arrives with the code that calls
it.

### Three decisions where the code and `docs/format.md` had to be reconciled

Recorded here, and to be written into `docs/format.md` with the GREEN change.

1. **`launch.program` is a bare program name.** `docs/format.md` prints
   `"program": "erts-17.0.5/bin/erlexec"`, which duplicates `bindir`. The spec's shape —
   `program: "erlexec"` beside `bindir: "erts-17.0.5/bin"` — is what the tests pin, because the
   launcher needs the directory on its own anyway: it is what `BINDIR` is set to. The document
   is what changes.
2. **`otp_release` is a number.** `docs/format.md` prints `"otp_release": "29"`; `OtpInfo`,
   `StageListing` and `assemble` all carry it as a `u32` already, and a manifest that
   re-stringified it would be the only place in the crate that did. The document is what changes.
   `docs/format.md` also prints four `launch` keys that no milestone has yet produced — `vm_args`,
   `sys_config`, `distribution`, `filename_encoding` — and they are left out of `LaunchSpec`
   rather than added as fields nothing writes; an older document key that a newer launcher needs
   is a `format_version` bump, and a key nobody writes is a placeholder export.
3. **A trailer version byte this build does not know is an error, not the CLI.** The A3a spec
   says only "magic mismatch → `Ok(None)`"; `docs/format.md` rule 3 is more precise and says that
   `magic[0..7]` deciding *and* `magic[7]` being an unsupported version are different answers —
   `None` for the first, exit 122 for the second. The document is right and the tests pin it:
   `TrailerError::UnsupportedVersion { found, supported }`. A damaged application must never
   present ginary's help text.

One decision the spec left open and the tests close: `created_at` returns a `Result`.
`SOURCE_DATE_EPOCH` set to something that is not a second count is
`ManifestError::InvalidSourceDateEpoch` rather than a silent fall back to the clock. A build that
was asked to be reproducible and quietly was not is exactly the "no silent skipping" rule
`CLAUDE.md` states.

### What was added

| path | what it holds |
|---|---|
| `src/trailer.rs` | `MAGIC`, `TRAILER_LEN`, `Trailer`, `TrailerError`; stub bodies |
| `src/manifest.rs` | `Manifest`, `LaunchSpec`, `Index`, `AppRef`, `NativeRef`, `created_at`, the two error types; stub bodies |
| `src/payload.rs` | `Packed`, `pack`, `unpack`, `read_manifest`, `read_index`, `PayloadError`; stub bodies |
| `src/diag.rs` | `Diag`, `PhaseGuard`, `EnvSnapshot`; `with_sinks` and `is_enabled` are real, the rest is stubbed |
| `tests/common/payload.rs` | `RawTar`/`RawEntry`, `staging_tree`, `CountingReader`, `SharedSink`, `tree_listing`, `sample_manifest` |
| `tests/trailer.rs` | 13 tests |
| `tests/manifest.rs` | 19 tests |
| `tests/payload.rs` | 33 tests |
| `tests/diag.rs` | 12 tests |
| `tests/snapshots/manifest__canonical_manifest_json.snap` | the manifest's wire field order |

`tests/common/payload.rs` writes tar headers a byte at a time rather than through the `tar`
crate, and that is the load-bearing choice of the whole RED phase: `tar` refuses to *write* most
of what `src/payload.rs` has to refuse to *read*, so an archive holding `../x`, an absolute path,
a symlink, a device node or a `ustar` prefix cannot be built with the library under test.
`the_hand_built_archives_read_back_as_the_entries_they_were_written_as` closes the loop by
reading all of them back with `tar` and asserting the paths, the entry types and one body, so a
header this file wrote wrongly fails there rather than making four other tests pass for the wrong
reason. It is the same rule `tests/appfile.rs` follows for the `.app` files `FakeOtp` generates.

### The 62 failing tests

`cargo test --no-fail-fast` runs 519 tests across 19 targets. The 442 that existed before this
section — the library's 102, the thirteen pre-existing integration targets and the one doc
test — all still pass. The four new targets fail as follows.

**`tests/trailer.rs` — 11 of 13 fail.**

```text
a_trailer_round_trips_through_its_sixty_four_bytes
the_encoding_is_the_byte_layout_the_format_document_prints
bytes_that_do_not_start_with_the_magic_are_not_a_trailer_at_all
a_trailer_version_this_build_cannot_read_is_an_error_rather_than_the_command_line
reserved_bytes_that_are_not_zero_are_refused
a_payload_of_no_bytes_is_refused
a_geometry_error_names_the_length_it_expected_and_the_length_the_file_has
an_offset_and_length_that_would_overflow_are_refused_rather_than_wrapping
a_file_shorter_than_the_trailer_holds_no_trailer
read_from_reads_the_last_sixty_four_bytes_of_a_file
the_cache_key_is_the_first_eight_bytes_of_the_digest_in_lower_case_hexadecimal
```

```text
thread 'the_encoding_is_the_byte_layout_the_format_document_prints' panicked at
tests/trailer.rs:51:5:
assertion `left == right` failed: the magic is the first eight bytes
  left: [0, 0, 0, 0, 0, 0, 0, 0]
 right: [71, 73, 78, 65, 82, 89, 0, 1]

thread 'a_geometry_error_names_the_length_it_expected_and_the_length_the_file_has' panicked at
tests/trailer.rs:145:18:
expected Geometry, got NotImplemented
```

The two that pass are the never-panic properties, `parse_never_panics_on_arbitrary_bytes` and
`parse_never_panics_on_the_magic_followed_by_rubbish`. A stub satisfies them, which is expected:
they are the policy `docs/dev/testing.md` states for every binary parser, and their job is to
keep satisfying it once there is code behind them.

**`tests/manifest.rs` — 14 of 19 fail.**

```text
check_version_accepts_the_version_this_build_writes
a_newer_format_version_parses_and_is_then_refused_by_check_version
the_launch_spec_of_a_manifest_this_build_writes_validates
an_absolute_launch_path_is_refused
a_launch_path_that_climbs_out_of_the_root_is_refused
a_launch_path_separated_by_backslashes_is_refused
an_empty_launch_path_is_refused
created_at_formats_the_seconds_it_is_given_as_rfc_3339_in_utc
created_at_gets_a_leap_day_right
created_at_honours_source_date_epoch_over_the_clock_it_is_given
a_source_date_epoch_that_is_not_a_second_count_is_an_error_rather_than_ignored
the_index_hashes_every_file_the_listing_names_and_keeps_its_category
a_file_the_listing_names_and_the_tree_no_longer_holds_is_an_error
an_index_round_trips_through_its_json
```

```text
thread 'a_launch_path_that_climbs_out_of_the_root_is_refused' panicked at tests/manifest.rs:129:5:
assertion `left == right` failed: the field name says which element of `pa` it was
  left: NotImplemented
 right: UnsafePath { field: "launch.pa[1]", value: "../../etc" }

thread 'created_at_gets_a_leap_day_right' panicked at tests/manifest.rs:184:66:
a plain second count: NotImplemented
```

Five pass, and each is a contract the derives alone already satisfy: the two round trips, the
`extra` map preserving an unknown `signature` key, the constant names, and the snapshot. The
snapshot is deliberately in that group — it exists to answer the spec's open question *"verify
serde_json preserves struct field order"*, and it does: the wire order is `format_version`,
`app`, `app_version`, `gleam_version`, `otp_release`, `otp_version`, `erts_version`, `target`,
`otp_applications`, `gleam_applications`, `launch`, `native`, `created_at`, `ginary_version`,
declaration order and not the alphabetical order `serde_json::Value` would impose.
`a_manifest_carrying_no_unknown_keys_writes_none` asserts the alphabetical order separately so
that the difference between the two is written down rather than discovered.

**`tests/payload.rs` — 29 of 33 fail.** Every `pack`, `unpack`, `read_manifest` and `read_index`
test, which is all of them but the three never-panic properties and the fixture check.

```text
pack_writes_the_manifest_first_the_index_second_and_the_tree_sorted
pack_leaves_the_staging_listing_out_of_the_payload
packed_reports_the_length_and_digest_of_exactly_what_it_wrote
packing_the_same_tree_and_manifest_twice_produces_the_same_bytes
the_packed_headers_carry_no_time_and_no_owner
the_packed_headers_keep_the_mode_the_file_has_on_disk
pack_refuses_a_staging_root_that_holds_no_listing
a_packed_tree_unpacks_to_the_same_bytes_and_the_same_modes
unpack_returns_the_manifest_of_the_first_entry
a_payload_that_does_not_hash_to_the_trailer_is_refused_with_both_digests
a_payload_cut_in_half_is_an_error_rather_than_a_panic
an_empty_payload_is_refused_for_having_no_manifest
a_first_entry_that_is_not_the_manifest_is_refused
an_entry_that_climbs_out_of_the_destination_is_refused
an_absolute_entry_path_is_refused
a_ustar_prefix_that_climbs_out_of_the_destination_is_refused
a_long_name_entry_that_climbs_out_of_the_destination_is_refused
a_symlink_entry_is_refused
a_hard_link_entry_is_refused
a_device_entry_is_refused
a_fifo_entry_is_refused
a_directory_entry_is_unpacked_rather_than_refused
read_manifest_returns_the_first_entry
read_manifest_refuses_a_first_entry_that_is_not_the_manifest
read_manifest_stops_after_the_first_entry_of_a_large_payload
read_index_returns_both_front_entries
read_index_refuses_a_second_entry_that_is_not_the_index
a_payload_whose_manifest_is_not_json_is_refused
a_payload_whose_manifest_is_from_a_newer_format_is_refused
```

```text
thread 'a_symlink_entry_is_refused' panicked at tests/payload.rs:514:18:
expected UnsupportedEntry, got NotImplemented

thread 'a_payload_that_does_not_hash_to_the_trailer_is_refused_with_both_digests' panicked at
tests/payload.rs:366:18:
expected ChecksumMismatch, got NotImplemented

thread 'a_packed_tree_unpacks_to_the_same_bytes_and_the_same_modes' panicked at
tests/payload.rs:304:76:
pack: NotImplemented
```

Each of the eight malicious archives also asserts, through `Destination::assert_nothing_escaped`,
that the directory *containing* the destination still holds exactly its one sentinel file. A
rejection that had already written the file it rejected would satisfy the error assertion alone,
which is the failure mode the whole group exists to catch.

**`tests/diag.rs` — 8 of 12 fail.**

```text
a_phase_reaches_the_debug_sink_as_one_line_with_its_elapsed_time
key_values_reach_the_debug_sink_in_the_order_they_were_given
the_trace_sink_holds_one_json_object_per_line
trace_lines_are_written_in_the_order_the_events_happened
a_phase_that_takes_time_records_it
ginary_trace_writes_json_lines_to_the_file_it_names_and_creates_its_parents
ginary_debug_is_on_for_one_and_off_for_anything_else
both_sinks_get_the_same_events
```

```text
thread 'trace_lines_are_written_in_the_order_the_events_happened' panicked at tests/diag.rs:140:5:
assertion `left == right` failed
  left: []
 right: ["open_self", "read_trailer", "resolve_cache", "exec"]

thread 'ginary_trace_writes_json_lines_to_the_file_it_names_and_creates_its_parents' panicked at
tests/diag.rs:198:5:
a trace path turns the recorder on
```

Four pass: the disabled recorder writing nothing and creating no file, `with_sinks` reporting
itself enabled, and the two negative cases. `Diag::with_sinks` and `Diag::is_enabled` are the two
bodies that are *not* stubs, because they are the injection point every other test reaches the
recorder through and a stub there would make the whole file untestable rather than red.

### Gates

`cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features -- -D warnings`,
`RUSTDOCFLAGS=-Dwarnings cargo doc --no-deps` and `cargo deny check` (advisories, bans, licenses,
sources all ok) are clean over the stubs and the tests. `cargo test` is red by design, at 62
failures out of 519.

### Not in this section

`fuzz/` is not scaffolded yet. A fuzz target over `trailer::parse` or `payload::read_manifest`
while both are stubs measures the stub, so the four targets, the workspace exclusion that keeps
`cargo test` on stable, the `fuzz` and `fuzz:build` mise tasks and the 30-second session with its
execs/sec and crash count all belong to GREEN, where there is code under them. `docs/format.md`
and `docs/dev/testing.md` gain their fuzz sections there for the same reason.

## GREEN

Every stub marked `// RED: replaced in GREEN` is gone, and all 521 tests pass. `cargo test`:

```text
src/lib.rs         104   tests/elf.rs         16   tests/regressions.rs   30
src/main.rs          0   tests/manifest.rs    19   tests/report.rs        13
tests/appfile.rs    51   tests/otp.rs         29   tests/smoke_cli.rs      6
tests/assemble.rs   34   tests/payload.rs     33   tests/stage_run.rs     12
tests/beam.rs       32   tests/closure.rs     34   tests/strip.rs         29
tests/cli.rs        53   tests/diag.rs        12   tests/trailer.rs       13
doc tests            1

test result: ok. 521 passed; 0 failed; 0 ignored, across 19 targets
```

The 62 that were red are green, the 442 that existed before are unchanged, and the library gained
two unit tests (102 to 104) for the one piece of `src/diag.rs` no integration test can reach.

### What each module does now

**`src/trailer.rs`.** `to_bytes` lays the 64 bytes out; `parse` answers `Ok(None)` on
`magic[0..7]`, then `UnsupportedVersion` on `magic[7]`, then `Reserved`, then geometry. The
addition is `saturating_add`, so a `payload_offset` near `u64::MAX` cannot wrap into a length that
happens to match. A `payload_len` of zero reports `expected = payload_offset + 1 + 64`, the
shortest file a usable trailer can describe, which makes the "truncated" message the error already
carried the right one for it rather than a pair of equal numbers. `read_from` uses
`FileExt::read_exact_at`, and a file shorter than 64 bytes is `Ok(None)` through `checked_sub`.
The little-endian fields are folded out of the slice byte by byte rather than through
`try_into().expect(..)`: nothing on the launcher path may panic, and an infallible conversion that
is *written* as a fallible one is a `panic!` waiting for a refactor.

**`src/manifest.rs`.** `LaunchSpec::validate` checks `program` as a bare name — one component —
and `bindir`, `boot` and every `pa[i]` as root-relative paths, refusing empty, absolute,
backslash-bearing, and any `.`, `..` or empty component. `check_version` compares against
`FORMAT_VERSION`. `Index::from_staged` streams every listed file through a 64 KiB buffer for its
SHA-256, takes the mode from `symlink_metadata` and the category from the listing, sorts by path,
and turns a file it cannot read into `IndexError::Io` naming it. `created_at` is Howard Hinnant's
`civil_from_days` over `u64`, so no operation in it can overflow whatever second count it is
given, and no calendar table or leap-year special case appears; `SOURCE_DATE_EPOCH` overrides the
argument, an empty value is an unset one — the rule `cache_dir::resolve` already follows — and a
value that is not a second count is `InvalidSourceDateEpoch`.

**`src/payload.rs`.** `pack` reads `ginary.stage.json`, builds the index from it, serialises both
front entries, checks that the tree holds nothing the index does not describe, and writes
`manifest, index, then every listed file in path order` through a `HashingWriter` into a
single-threaded zstd encoder. `unpack` wraps the source in `Take` then `HashingReader` then the
zstd decoder then `tar::Archive` with `preserve_permissions(true)`, `preserve_mtime(false)`,
`unpack_xattrs(false)` and `overwrite(false)`; each entry is checked for type, then for path,
then — at position 0 — for name, and everything else goes through `unpack_in`, whose `false` is
`PathEscape`. After the last entry the rest of the stream is drained into `io::sink()` so that the
digest covers the whole payload, and only then compared. `read_manifest` and `read_index` share
one `front_entry` helper and stop where they are named to stop.

**`src/diag.rs`.** `from_env` turns the stderr sink on for exactly `GINARY_DEBUG=1`, opens the
`GINARY_TRACE` file for appending after creating its parents, and degrades a file it cannot open
to one warning. `record` is the single writer both sinks go through, and it returns immediately
when `origin` is `None`, so a disabled recorder reads no clock. `t_us` is measured when the record
is *written* rather than when the event began, which is what keeps the timestamps non-decreasing
in the order the lines appear even when phases nest; a phase's start is `t_us - elapsed_us`.

### Test corrections

One, and it is the RED scaffold rather than a contract:

- **`tests/payload.rs::a_payload_cut_in_half_is_an_error_rather_than_a_panic`** asserted
  `!matches!(error, PayloadError::NotImplemented)`, a variant that exists only while the module is
  a stub and that GREEN deletes. The assertion is now `matches!(error, PayloadError::Io(_))`,
  which says what half a zstd stream actually produces, and the test also gained
  `destination.assert_nothing_escaped()` so that it follows the same rule as the eight malicious
  archives beside it. Strictly stronger than what it replaced.

Nothing else in the four test files changed. In particular
`tests/manifest.rs::a_file_the_listing_names_and_the_tree_no_longer_holds_is_an_error` kept its
`other => panic!(..)` arm: with `NotImplemented` gone, `IndexError` has one variant and that arm
became an `unreachable_patterns` warning, which `-D warnings` fails on. The fix is on the
production side — `IndexError` is now `#[non_exhaustive]`, which is true of it (reading a staged
tree will not be the only way to fail an index for long) and which keeps a wildcard arm reachable
and required in every crate but this one.

### Decisions the implementation added

1. **Three new `PayloadError` variants.** `Serialise` (a document ginary writes itself did not
   serialise — unreachable today, and not a place for an `expect`), `FrontEntryTooLarge` (entries
   0 and 1 are read whole, and a few kilobytes of zstd can claim a terabyte of tar entry;
   `MAX_FRONT_ENTRY_BYTES` is 8 MiB against an index of a few hundred kilobytes), and `Unlisted`
   (the staging tree holds a file `ginary.stage.json` does not name). The last is the "no silent
   skipping" rule at this layer: packing it would put a file in the artifact the index does not
   describe, and leaving it out would drop a file without a word.
2. **`mtime` is set to 0 explicitly, after `HeaderMode::Deterministic`.** tar-rs 0.4.46's
   deterministic mode writes `DETERMINISTIC_TIMESTAMP` — 1153704088, a fixed *non-zero* value — as
   a workaround for tools that mishandle a zero one. `docs/format.md` and
   `the_packed_headers_carry_no_time_and_no_owner` both say 0, so ginary sets that one field
   itself. Found by the test, not by reading the crate.
3. **Entry modes are the normalised `0644`/`0755` that `Deterministic` produces**, and the index
   records the staged file's own bits. They agree for every tree assembly produces, and the
   normalisation is what keeps a umask, an ACL or a set-user-ID bit out of an artifact.
   `docs/format.md` states both halves.
4. **Entry 0 is written to `<root>/ginary.json` by the reader.** It is read into memory because it
   is parsed, so it cannot also be `unpack_in`-ed; it must still exist on disk, because
   `docs/dev/architecture.md` makes its presence the proof that a cache entry is complete.
5. **`Diag` writes JSON by hand.** The module is meant to be dependency-free on the launcher path,
   so `push_json_string` does the RFC 8259 escaping — quote, backslash, and every code point below
   `0x20`. Two unit tests in `src/diag.rs` cover it, one of them by parsing the line back with
   `serde_json` after feeding a value holding a quote, a backslash, a newline, a tab and a `\u{1}`.

### `fuzz/`

Four targets, a workspace of its own (`[workspace]` in `fuzz/Cargo.toml`, and the root manifest
does not mention the directory), so `cargo test`, `cargo clippy --all-targets` and `cargo deny` at
the root stay on the stable toolchain. `mise run fuzz:build` builds them; `mise run fuzz` runs each
for 30 seconds in turn.

The target the spec calls `payload_unpack` is `payload_read_manifest`, for the reason the spec
gives: `unpack` writes to disk and creates directories, so a fuzzer would spend its time in the
kernel and leave a tree behind after every crash. The name says what it fuzzes.

`trailer_parse` takes `&[u8]` and splits it itself — 64 trailer bytes then eight of file length —
rather than deriving `Arbitrary` for a `([u8; 64], u64)`. A seed file is then exactly what the end
of a real artifact holds, instead of whatever encoding `Arbitrary` happens to use for a tuple.

**Session, 30 s per target, `-rss_limit_mb=4096`. No crashes, no timeouts, no OOMs.**

| target | runs | exec/s | edges | corpus |
|---|---|---|---|---|
| `trailer_parse` | 21,585,723 | 696,313 | 55 | 7 / 433 B |
| `appfile_terms` | 1,314,448 | 42,401 | 693 | 759 / 109 KB |
| `beam_chunks` | 1,304,478 | 42,079 | 916 | 381 / 174 KB |
| `payload_read_manifest` | 759,699 | 24,506 | 1,483 | 302 / 118 KB |

`fuzz/artifacts/` is empty for all four; nothing was minimised and no regression test was added,
because nothing crashed.

**The seeds are the finding.** The first session ran without them, and
`payload_read_manifest` reported `cov: 112 corp: 2/2b` after 945,497 runs: a random vector is
never a zstd frame, so the target measured the first `if` in the decoder and nothing else. Three
of the four parsers start with a magic — `GINARY\0`, `FOR1`/`BEAM`, a zstd frame header — and
`fuzz/seeds/<target>/` now holds one small real input each, passed as a second corpus directory by
the task. With the seed the same target reaches 1,483 edges. The seeds are committed; the
generated `fuzz/corpus/` is not.

### Documentation

`docs/format.md` is reconciled with the code and gained four sections: the header-field table and
the reading rules under **Payload**, a manifest **Fields** table (with the note that the wire order
is the struct's declaration order, which the snapshot pins), **Index: `ginary.index.json`**,
**Determinism**, and a **Changes** section recording the seven format decisions and why each one
went the way it did. `docs/dev/testing.md` gained a **Fuzzing** section — the four targets, the
seed-corpus rule and the measurement behind it, why `unpack` is not a target, why `fuzz/` is its
own workspace, and what to do with a crash — and its two stale "planned" lines now point at it.

### Gates

```text
cargo fmt --all -- --check                          clean
cargo clippy --all-targets --all-features -D warnings   clean
cargo test                                          521 passed, 0 failed
RUSTDOCFLAGS=-Dwarnings cargo doc --no-deps         clean
cargo deny check                                    advisories/bans/licenses/sources ok
```

All five were re-run with `fuzz/` present, to confirm the root workspace does not reach into it.

### Staged

`Cargo.toml`, `Cargo.lock`, `mise.toml`, `src/{lib,trailer,manifest,payload,diag}.rs`,
`tests/{trailer,manifest,payload,diag}.rs`, `tests/common/{mod,payload}.rs`, `tests/cli.rs`,
`tests/snapshots/manifest__canonical_manifest_json.snap`, `fuzz/` (manifest, lock, `.gitignore`,
the four targets and the four seed directories), `docs/format.md`, `docs/dev/testing.md` and this
log. Nothing is committed.

## Fix round 1

Thirteen review findings: two high, five medium, six low. All thirteen are addressed. Five of
them were defects in behaviour and each has a file under `tests/regressions/`; the rest were
tests that asserted less than their names promised, and each of those was strengthened and then
watched failing against a deliberate mutation of the line it is supposed to pin, because a test
that has never failed proves nothing whichever way it was written.

### The five behavioural defects

`cargo test --test regressions a3a`, before any production change. Six of the seven new tests
fail; the seventh (`a_payload_that_passes_its_digest_still_writes_the_manifest`) passes both
before and after, and is there so that moving the write cannot quietly drop it.

```text
running 7 tests
test a3a_a_zero_length_payload_looked_truncated::a_trailer_that_claims_no_payload_says_so_rather_than_naming_a_missing_byte ... FAILED
test a3a_the_second_payload_entry_was_never_checked::a_second_entry_that_is_not_the_index_is_refused ... FAILED
test a3a_a_contiguous_entry_was_extracted::a_contiguous_file_entry_is_refused_like_every_other_type_that_is_not_a_file ... FAILED
test a3a_a_rejected_payload_left_its_manifest_behind::a_payload_that_fails_its_digest_leaves_no_manifest_behind ... FAILED
test a3a_a_rejected_payload_left_its_manifest_behind::unpacking_twice_into_one_destination_does_not_rewrite_the_manifest ... FAILED
test a3a_the_second_payload_entry_was_never_checked::a_payload_that_stops_after_the_manifest_is_refused ... FAILED
test a3a_a_rejected_payload_left_its_manifest_behind::a_payload_that_passes_its_digest_still_writes_the_manifest ... ok

---- a3a_a_rejected_payload_left_its_manifest_behind::a_payload_that_fails_its_digest_leaves_no_manifest_behind ----
panicked at tests/regressions/a3a_a_rejected_payload_left_its_manifest_behind.rs:64:5:
a payload that was refused left the file that says the tree is complete:
Ok(["/tmp/.tmpq669bR/dest/ginary.index.json", "/tmp/.tmpq669bR/dest/ginary.json",
    "/tmp/.tmpq669bR/dest/lib"])

---- a3a_a_rejected_payload_left_its_manifest_behind::unpacking_twice_into_one_destination_does_not_rewrite_the_manifest ----
panicked at tests/regressions/a3a_a_rejected_payload_left_its_manifest_behind.rs:104:5:
assertion `left == right` failed: the second unpack rewrote a manifest it was not allowed to
overwrite
  left: [123, 34, 102, 111, 114, 109, 97, 116, ...]   (the packed `ginary.json`, 600 bytes)
 right: [97, 32, 115, 101, 110, 116, ...]             (`a sentinel nobody may overwrite`)

---- a3a_a_contiguous_entry_was_extracted::a_contiguous_file_entry_is_refused_like_every_other_type_that_is_not_a_file ----
panicked at tests/regressions/a3a_a_contiguous_entry_was_extracted.rs:45:10:
a contiguous file is not one of the two legal entry types: Manifest { format_version: 1, app:
"hello", ... }

---- a3a_the_second_payload_entry_was_never_checked::a_second_entry_that_is_not_the_index_is_refused ----
panicked at tests/regressions/a3a_the_second_payload_entry_was_never_checked.rs:54:10:
entry 1 is fixed by the format for every reader, not only the streaming one: Manifest {
format_version: 1, app: "hello", ... }

---- a3a_the_second_payload_entry_was_never_checked::a_payload_that_stops_after_the_manifest_is_refused ----
panicked at tests/regressions/a3a_the_second_payload_entry_was_never_checked.rs:80:10:
an artifact with no index is not an artifact this ginary wrote: Manifest { format_version: 1,
app: "hello", ... }

---- a3a_a_zero_length_payload_looked_truncated::a_trailer_that_claims_no_payload_says_so_rather_than_naming_a_missing_byte ----
panicked at tests/regressions/a3a_a_zero_length_payload_looked_truncated.rs:40:5:
the message says what is wrong: the trailer says the file is 4161 bytes long and it is 4160, so
it was truncated or something was appended to it

test result: FAILED. 1 passed; 6 failed
```

The zero-payload test was run in that RED without its
`matches!(error, TrailerError::EmptyPayload)` line, and the line was added with the fix: naming a
variant that does not exist yet is a compile error, and a compile error is not RED. The message
assertion above is the one that failed, and it failed on exactly the wrong diagnosis the finding
described.

**Finding 2 and finding 11 — `ginary.json` was written before the payload was trusted, and with a
call that overwrites.** `unpack` wrote entry 0 as soon as it had parsed it. `docs/format.md` says
that file's presence is what a cache entry's completeness is judged by, so a payload that then
failed its SHA-256 check left a directory announcing itself as finished; and `std::fs::write`
overwrites, while every other entry was unpacked under `set_overwrite(false)`, so unpacking twice
into one destination replaced the manifest and only then failed on entry 1. Both are one fix.
Entry 0 is now held in a small `FrontMatter` value through the entry loop and written after the
digest comparison, by a `create_file` that opens with `create_new`. A rejected payload leaves a
partial tree *without* the marker, which is what the cache in A3b will treat as incomplete, and a
populated destination is refused before anything in it is modified.

**Finding 13, first half — `unpack` did not enforce entry 1.** `read_index` did and `unpack` did
not, so an artifact whose index was misnamed or missing extracted happily into a cache directory
`ginary verify` could not read. The loop now applies `expect_name(1, INDEX_NAME, ..)` and refuses
a payload that ends before entry 1 with `MissingEntry { position: 1, .. }`. The format's front
order is a property of the artifact, not of one reader.

**Finding 6 — a contiguous file was extracted.** The allowlist read
`Regular | Continuous | Directory` while `docs/format.md` said `Regular` and `Directory`. `pack`
has never written a `'7'` entry, so the extra arm widened only what a hostile archive could
contain. `Continuous` now falls into the reject arm as `contiguous file`.

**Finding 10 — a zero-length payload was reported as a one-byte truncation.** `parse` computed
`payload_offset + payload_len.max(1) + 64`, so a well-formed file whose trailer claimed no
payload produced "the trailer says the file is 4161 bytes long and it is 4160, so it was
truncated". Nothing had been truncated and the missing byte was an artefact of the `.max(1)`.
`TrailerError::EmptyPayload` is now its own variant with its own message, and the geometry sum is
the plain `payload_offset + payload_len + TRAILER_LEN` again.

### The tests that asserted less than they claimed

Each of these pins behaviour that was already correct, so the RED is a mutation of the exact line
the test exists to hold. The mutation was reverted immediately after the run in every case.

**Finding 1 — the overflow test was vacuous.** It fixed `payload_offset: u64::MAX,
payload_len: 64` and called `parse(&raw, 128)`; `u64::MAX.wrapping_add(64).wrapping_add(64)` is
127, so `Geometry` came back whether the arithmetic saturated or wrapped. The test now uses the
wrapped length itself, and asserts the saturation point:

```text
# src/trailer.rs: saturating_add -> wrapping_add
panicked at tests/trailer.rs:170:47:
an overflowing geometry is refused: Some(Trailer { payload_offset: 18446744073709551615,
payload_len: 64, payload_sha256: [1, 35, 69, ...] })
```

That is the failure the test's name promised all along: a wrapping parser hands the launcher an
offset of `u64::MAX` and calls the file valid.

**Finding 3 — `PayloadError::Unlisted` had no test.** Two now: a stray
`lib/hello/ebin/stray.beam` written into the staging root is refused by name, and
`the_staging_listing_is_the_one_file_pack_does_not_need_named` pins the single exemption.

```text
# src/payload.rs: check_tree_is_listed(staging, &index)? removed
panicked at tests/payload.rs:305:10:
a file the index does not describe is neither packed nor dropped: Packed { len: 1026, sha256: [...] }
```

**Finding 4 — the zstd-bomb guard had no test.** A manifest padded to exactly
`MAX_FRONT_ENTRY_BYTES + 1` bytes compresses to under 64 KB, which is the attack in one line, and
a second test packs one of exactly `MAX_FRONT_ENTRY_BYTES` and requires it to be read whole. The
mutation is the one the finding named:

```text
# src/payload.rs: .take(MAX_FRONT_ENTRY_BYTES + 1) -> .take(MAX_FRONT_ENTRY_BYTES)
panicked at tests/payload.rs:771:18:
expected FrontEntryTooLarge, got ManifestFormat { source: Error("EOF while parsing an object",
line: 1, column: 8388608) }
```

Without the `+ 1` an oversized manifest is silently truncated into a parse error, which is a
different diagnosis for a different fault; the boundary test is what stops the guard being fixed
by moving it one byte the other way.

**Finding 13, second half — `ListingFormat` and `IndexFormat` had no test.** A
`ginary.stage.json` that exists and is not JSON is the state an interrupted staging run leaves,
and a payload whose index entry is correctly named and unparseable is a corrupt artifact. Both
are now tested, and both were watched failing against a mutant that maps the parse error to the
neighbouring variant:

```text
# src/payload.rs: ListingFormat / IndexFormat -> ManifestFormat
panicked at tests/payload.rs:343:18:
expected ListingFormat, got ManifestFormat { source: Error("key must be a string", line: 1, column: 2) }
panicked at tests/payload.rs:799:5:
expected IndexFormat, got ManifestFormat { source: Error("key must be a string", line: 1, column: 2) }
```

**Finding 7 — two `LaunchSpec` rules `docs/format.md` states had no test.**

```text
# src/manifest.rs: the value.contains('/') block removed from check_name
panicked at tests/manifest.rs:184:10:
`program` is a name inside `bindir`, not a path: ()

# src/manifest.rs: matches!(component, "" | "." | "..") -> matches!(component, "..")
panicked at tests/manifest.rs:204:33:
a `.` component is refused: ()
(and a_launch_path_with_an_empty_component_is_refused, on `erts-17.0.5//bin`)
```

`program: "erts-17.0.5/bin/erlexec"` is exactly the shape the v1 format decision moved away from,
so it is the regression that matters rather than a synthetic `../`.

**Finding 8 — an empty `SOURCE_DATE_EPOCH` is an unset one, and nothing said so.**

```text
# src/manifest.rs: the Some(value) if value.is_empty() arm removed
panicked at tests/manifest.rs:270:10:
an exported-but-empty variable did not ask for a fixed timestamp: InvalidSourceDateEpoch { value: "" }
```

### Findings fixed without a new behaviour

**Finding 5 — `PathEscape` was unreachable, undocumented as such, and untested.** It is kept:
deleting it would turn a future `false` from the tar crate into a silently skipped file, which is
the one outcome this format may not have. What was wrong is that nothing said so. The call site
is now a `refuse_skip(entry.unpack_in(dest)?, name)` helper whose documentation states that
`check_entry_path` has already refused everything the tar crate answers `false` for, a unit test
in `src/payload.rs` pins the `false -> PathEscape` mapping — the one place a `bool` can be handed
to the code directly — and `docs/format.md` and `docs/dev/testing.md` both record the decision
rather than describing a rejection no archive can produce.

**Finding 9 — a `Diag` test's file assertion was over a path the recorder was never given.**
`nothing_set_records_nothing_and_creates_no_file` counted a temporary directory that no
`EnvSnapshot` mentioned, so the count could not have changed under any implementation. It now
names a trace path, proves with the *same* path that a recorder asked for one creates it and its
parent, removes it, and only then asserts that the recorder built from an empty environment
creates nothing. A second test covers the empty-value rule, `GINARY_TRACE=` being an unset one,
which matches the `SOURCE_DATE_EPOCH` rule one module over.

**Finding 12 — `sample_manifest`'s comment claimed to fill `extra` and did not.** The comment now
says what the value is: every *declared* field filled, `extra` deliberately empty because this is
the manifest `docs/format.md` prints and the snapshot pins, with the flattened-key round trip
asserted by `a_key_this_build_does_not_know_survives_a_round_trip`, which adds one. Populating it
instead would have made the canonical snapshot non-canonical.

### Documentation

`docs/format.md`: trailer validation rule 5 is `EmptyPayload` rather than a geometry error; the
**Reading** section gains the fixed front order as a rule every reader applies, the contiguous
type in the refused list, the no-overwrite rule, the marker-last rule with what a rejected
payload leaves behind, and the paragraph saying `PathEscape` is unreachable defence in depth; and
a **v1, milestone A3a, review round 1** entry in **Changes** records the five decisions.
`docs/dev/testing.md`: the malicious-archive policy says why every escape test asserts
`UnsafePath` and never `PathEscape`, and gains the completeness-marker rule and why the manifest
is created with `create_new`.

### Gates

```text
cargo fmt --all -- --check                              clean
cargo clippy --all-targets --all-features -D warnings   clean
cargo test                                              541 passed, 0 failed
RUSTDOCFLAGS=-Dwarnings cargo doc --no-deps             clean
cargo deny check                                        advisories/bans/licenses/sources ok
```

541 is 521 plus twenty: two unit tests in `src/payload.rs`, seven in the four new regression
files, six in `tests/payload.rs`, four in `tests/manifest.rs` and one in `tests/diag.rs`. The
three `license-not-encountered` warnings from `cargo deny` are the pre-existing unmatched
allowances and are not new to this round.

### Staged

`src/{trailer,payload}.rs`, `tests/{trailer,manifest,payload,diag}.rs`,
`tests/common/payload.rs`, `tests/regressions.rs`, the four new files under `tests/regressions/`,
`docs/format.md`, `docs/dev/testing.md` and this log. `src/manifest.rs` and `src/diag.rs` are
unchanged: both findings against them were missing tests, not wrong code. Nothing is committed.

## Fix round 2

One review finding, medium, behavioural. It is the round-1 fix's own shadow: holding entry 0 back
until the digest matched took `ginary.json` out of the unpack loop, and `set_overwrite(false)` —
the rule that had been refusing a second entry of that name — went with it.

### RED

`tests/regressions/a3a_a_repeated_front_entry_forged_the_marker.rs` written first, with
`PayloadError::DuplicateEntry` and `PayloadError::ReservedName` added as variants nothing
produces yet: naming a variant that does not exist is a compile error, and a compile error is not
RED. `cargo test --test regressions a3a_a_repeated`, before any change to `pack` or `unpack`:

```text
running 5 tests
test a3a_a_repeated_front_entry_forged_the_marker::packing_a_staged_file_with_a_reserved_name_is_refused ... FAILED
test a3a_a_repeated_front_entry_forged_the_marker::a_second_manifest_entry_is_refused_and_plants_nothing ... FAILED
test a3a_a_repeated_front_entry_forged_the_marker::a_second_index_entry_is_refused ... FAILED
test a3a_a_repeated_front_entry_forged_the_marker::a_directory_entry_named_like_the_manifest_is_refused ... FAILED
test a3a_a_repeated_front_entry_forged_the_marker::a_second_manifest_entry_behind_a_current_directory_component_is_refused ... FAILED

---- a_second_manifest_entry_is_refused_and_plants_nothing ----
panicked at tests/regressions/a3a_a_repeated_front_entry_forged_the_marker.rs:89:18:
expected DuplicateEntry, got Io(Os { code: 17, kind: AlreadyExists, message: "File exists" })

---- a_second_manifest_entry_behind_a_current_directory_component_is_refused ----
panicked at tests/regressions/a3a_a_repeated_front_entry_forged_the_marker.rs:89:18:
expected DuplicateEntry, got Io(Os { code: 17, kind: AlreadyExists, message: "File exists" })

---- a_directory_entry_named_like_the_manifest_is_refused ----
panicked at tests/regressions/a3a_a_repeated_front_entry_forged_the_marker.rs:89:18:
expected DuplicateEntry, got Io(Os { code: 17, kind: AlreadyExists, message: "File exists" })

---- a_second_index_entry_is_refused ----
panicked at tests/regressions/a3a_a_repeated_front_entry_forged_the_marker.rs:89:18:
expected DuplicateEntry, got Io(Custom { kind: AlreadyExists, error: TarError { desc: "failed to
unpack `/tmp/.tmpwEVU85/dest/ginary.index.json`", io: Custom { kind: AlreadyExists, error:
TarError { desc: "failed to unpack `ginary.index.json` into
`/tmp/.tmpwEVU85/dest/ginary.index.json`", io: Os { code: 17, kind: AlreadyExists, message: "File
exists" } } } } })

---- packing_a_staged_file_with_a_reserved_name_is_refused ----
panicked at tests/regressions/a3a_a_repeated_front_entry_forged_the_marker.rs:156:10:
ginary may not write an artifact its own reader refuses: Packed { len: 1117, sha256: [68, 174,
252, 140, 28, 183, 61, 32, ...] }

test result: FAILED. 0 passed; 5 failed
```

Three things are visible in that output beyond "the error is wrong".

The three manifest cases fail with a **bare** `AlreadyExists` from the final `create_new`, not
from the loop — which is the finding exactly: the loop wrote `<dest>/ginary.json` happily, and
the only thing that noticed was ginary trying to write its own marker over the archive's. The
file it collided with held `ATTACKER MARKER`. The `./ginary.json` case fails identically, which
is why the fix compares the path the entry *lands* on and not the raw header field: the tar crate
drops a `.` component, so the two names are one destination.

The index case fails differently — `set_overwrite(false)` does catch it, because entry 1 *is*
unpacked — so it was already refused, just not by name and not with a message anyone could act
on. It is in the file so that the two reserved names are covered by the same rule rather than one
by a rule and one by an accident of which entries happen to be unpacked.

The pack case does not fail with a wrong error at all: it **succeeds**. `check_tree_is_listed`
exempted `ginary.stage.json` and nothing else, so a staging root holding a file called
`ginary.json` produced a well-formed artifact with a duplicate entry 5 — a build that succeeds
and a launcher that cannot start, on some other machine, later.

### The fix

`RESERVED_NAMES` in `src/payload.rs` is the single table both ends read: `[(MANIFEST_NAME, 0),
(INDEX_NAME, 1)]`, the name and the position the format fixes it at.

`check_entry_path` now *returns* the destination-relative path — the `/`-joined `Normal`
components, which is what tar's `unpack_in` builds — instead of only answering yes or no. Every
caller had that path available and none of them had it in a comparable form. `unpack` passes it
to `check_not_reserved` for every entry at position 2 or later, so a repeat is
`DuplicateEntry { position, name, fixed }` before the entry is written rather than an
`AlreadyExists` after it. Positions 0 and 1 keep their `expect_name` check against the raw name:
the format fixes those two exactly, and `./ginary.json` at position 0 is still `UnexpectedEntry`.

`pack` calls `check_no_reserved_names` on the staging listing before it builds the index, so the
reserved name is caught before any file is hashed, and it is `ReservedName { path, fixed }`
rather than a successful build. An *unlisted* file of that name is still `Unlisted`, which is the
older and equally correct refusal.

The reason both ends need an explicit check, rather than one of them relying on the file system:
an entry a reader handles specially is an entry the reader's generic defences have stopped
covering. `set_overwrite(false)` protects every name that goes through `unpack_in`, and the two
front-matter names are precisely the ones that do not.

### Documentation

`docs/format.md`: the **Payload** section states the two reserved names and that `pack` refuses a
listing naming one; **Reading** gains the `DuplicateEntry` rule, says the comparison is against
the landing path rather than the header field, and says why it is not redundant with the
overwrite rule; **Changes** gains a **v1, milestone A3a, review round 2** entry.
`docs/dev/testing.md`: the malicious-archive policy gains the paragraph on what taking an entry
out of the loop costs, and the four shapes the new regression file covers.

### Gates

```text
cargo fmt --all -- --check                              clean
cargo clippy --all-targets --all-features -D warnings   clean
cargo test                                              546 passed, 0 failed
RUSTDOCFLAGS=-Dwarnings cargo doc --no-deps             clean
cargo deny check                                        advisories/bans/licenses/sources ok
```

546 is 541 plus the five tests of the new regression file. The three `license-not-encountered`
warnings from `cargo deny` are the pre-existing unmatched allowances.

### Staged

`src/payload.rs`, `tests/regressions/a3a_a_repeated_front_entry_forged_the_marker.rs`,
`tests/regressions.rs`, `docs/format.md`, `docs/dev/testing.md` and this log. Nothing is
committed.

## Final gate

An independent run of every gate against the working tree, changing nothing but this section.
Toolchain: rustc/cargo 1.97.1, gleam 1.18.1, Erlang/OTP 29.0.5 (erts-17.0.5).

| Gate | Command | Result |
| --- | --- | --- |
| fmt | `cargo fmt --all -- --check` | exit 0, no output |
| clippy | `cargo clippy --all-targets --all-features -- -D warnings` | exit 0, no warnings; re-run after `cargo clean -p ginary` so nothing was served from cache |
| test | `cargo test` | exit 0, 546 passed, 0 failed, 0 ignored |
| doc | `RUSTDOCFLAGS=-Dwarnings cargo doc --no-deps` | exit 0, no warnings |
| deny | `cargo deny check` | exit 0, advisories ok, bans ok, licenses ok, sources ok |
| test (toolchain required) | `GINARY_REQUIRE_TOOLCHAIN=1 cargo test` | exit 0, the same 546 passed, 0 failed, 0 ignored |

Per binary, identical for the plain and the `GINARY_REQUIRE_TOOLCHAIN=1` run:

```text
unittests src/lib.rs    106 passed
unittests src/main.rs     0 passed
tests/appfile.rs         51 passed
tests/assemble.rs        34 passed
tests/beam.rs            32 passed
tests/cli.rs             53 passed
tests/closure.rs         34 passed
tests/diag.rs            13 passed
tests/elf.rs             16 passed
tests/manifest.rs        23 passed
tests/otp.rs             29 passed
tests/payload.rs         39 passed
tests/regressions.rs     42 passed
tests/report.rs          13 passed
tests/smoke_cli.rs        6 passed
tests/stage_run.rs       12 passed
tests/strip.rs           29 passed
tests/trailer.rs         13 passed
doctests                  1 passed
```

The two runs differ only in the `finished in` timings; every `Running` line and every
`test result:` count matches line for line. The four files new in A3a — `tests/trailer.rs`,
`tests/manifest.rs`, `tests/payload.rs` and `tests/diag.rs` — contribute 88 of the 546, and
`tests/regressions.rs` carries the five A3a regression files.

The gated tests ran rather than skipped. `require_tools` in `tests/common/tools.rs` asserts
rather than returning `None` when `GINARY_REQUIRE_TOOLCHAIN=1`, so a skipped gated test under
that variable is a panicking test, not a passing one. The gated run exits 0 with 0 failed and 0
ignored in all nineteen binaries, which is only possible if `tests/stage_run.rs` (12) and
`tests/closure.rs` (34) each found `gleam` and `erl` and executed against a real tree; the
captured output holds no `skipping: <tool> not on PATH` line either.

`cargo deny check` reports `advisories ok, bans ok, licenses ok, sources ok` and exits 0. It
also emits three advisory `license-not-encountered` warnings for allowances in `deny.toml` that
no crate in the current graph uses (`BSD-3-Clause`, `CDLA-Permissive-2.0`, `ISC`). Those are
pre-existing and are not gate failures.

The root gates stayed on stable. `fuzz/Cargo.toml` declares its own `[workspace]` and the root
`Cargo.toml` has no `[workspace]` table naming it, so `cargo test`, `cargo clippy
--all-targets` and `cargo deny` at the root never reach the nightly-only libFuzzer targets. The
final gate did not invoke `cargo +nightly fuzz`; the fuzz session results are recorded in the
GREEN section above.

`git status --short` lists 36 paths, nothing committed. All are staged; this log reads `AM`
because this section was appended to it after it was staged, which is the only change this
final gate made:

```text
M  Cargo.lock
M  Cargo.toml
AM docs/dev/log/A3a.md
M  docs/dev/testing.md
M  docs/format.md
A  fuzz/.gitignore
A  fuzz/Cargo.lock
A  fuzz/Cargo.toml
A  fuzz/fuzz_targets/appfile_terms.rs
A  fuzz/fuzz_targets/beam_chunks.rs
A  fuzz/fuzz_targets/payload_read_manifest.rs
A  fuzz/fuzz_targets/trailer_parse.rs
A  fuzz/seeds/appfile_terms/nested.app
A  fuzz/seeds/beam_chunks/gleam_bool.beam
A  fuzz/seeds/payload_read_manifest/front_matter.zst
A  fuzz/seeds/trailer_parse/valid_trailer
M  mise.toml
A  src/diag.rs
M  src/lib.rs
A  src/manifest.rs
A  src/payload.rs
A  src/trailer.rs
M  tests/cli.rs
M  tests/common/mod.rs
A  tests/common/payload.rs
A  tests/diag.rs
A  tests/manifest.rs
A  tests/payload.rs
M  tests/regressions.rs
A  tests/regressions/a3a_a_contiguous_entry_was_extracted.rs
A  tests/regressions/a3a_a_rejected_payload_left_its_manifest_behind.rs
A  tests/regressions/a3a_a_repeated_front_entry_forged_the_marker.rs
A  tests/regressions/a3a_a_zero_length_payload_looked_truncated.rs
A  tests/regressions/a3a_the_second_payload_entry_was_never_checked.rs
A  tests/snapshots/manifest__canonical_manifest_json.snap
A  tests/trailer.rs
```

No sandbox shim name (`.bashrc`, `.zshrc`, `.idea`, `.vscode`, `.gitconfig`, `.gitmodules`,
`.mcp.json`, `.profile`, `.ripgreprc`, `.bash_profile`, `.zprofile`) appears in the index or in
the untracked listing, and `git status --short --untracked-files=all` adds no untracked path.
A3a is green on all six gate runs.
