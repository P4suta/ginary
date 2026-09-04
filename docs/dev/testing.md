<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# Testing

## What exists now

| file | scope |
|---|---|
| `src/target.rs` unit tests | target names, parsing, round trips, the seven supported targets |
| `src/cache_dir.rs` unit tests | precedence, empty values, relative `XDG_CACHE_HOME`, no variable set |
| `src/doctor.rs` unit tests | version parsers, the probe list, tool reports, report rendering |
| `src/process.rs` unit tests | the `PATH` search, a bounded child, the timeout, a chatty pipe, a grandchild holding the pipes, child reaping, and `shell_quote` — the words it leaves alone, the ones it wraps, the single quote it closes and reopens, and a `/bin/sh` that reads eight of them back unchanged |
| `src/appfile.rs` unit tests | the internals no integration test can reach: atom quoting, escaping, float rendering, the nesting bound, the warning paths |
| `src/cli.rs` unit tests | clap definition validity, parsing, JSON and text command output |
| `tests/smoke_cli.rs` | the real binary: `--help`, `version`, `version --json`, no-argument exit 2, `doctor`, `doctor --json` |
| `tests/appfile.rs` | the `.app` reader: the term grammar, `Term`'s re-serialisation, `AppResource`, the error positions, and every fixture under `tests/fixtures/app/` |
| `tests/otp.rs` | `inspect_root` against fake roots that are whole and broken, `boot_lib_dirs`, and `discover` with and without an override |
| `tests/closure.rs` | the closure over fake shipment and OTP trees: seeds, edges, resolution order, determinism, the three errors, `explain` and `chain`, two property tests, and one gated run over a real shipment |
| `tests/cli.rs` | the real binary: `appfile parse` as a table and as JSON, `closure` as a table, JSON, `--explain` and its two footers, `stage` as a table, JSON, `--explain`, `--force` and its two usage errors, the `otp` field `doctor` now reports, and `cache dir`/`cache clean` over a cache root pinned with `GINARY_CACHE_DIR` |
| `tests/assemble.rs` | the staging root over fake trees: the exact layout, every exclusion, junk removal, modes, symlinks, the error paths, the listing, and determinism |
| `tests/stage_run.rs` | toolchain-gated: stage the `hello_ffi` fixture against the host OTP, strip it, measure it, and boot it through `erlexec` |
| `tests/beam.rs` | the IFF chunk reader: the grammar over hand-built bytes, the shape a compiler emits over three real modules, and the never-panic properties |
| `tests/elf.rs` | read-only ELF inspection, against the running test binary, a non-ELF file, truncations of a real binary, and the host `beam.smp` |
| `tests/strip.rs` | stripping a staged root: the exact `beam_lib` one-liner, the four verification failures, the three option shapes, idempotence, and `StagedRoot::refresh` |
| `tests/report.rs` | the size and dependency account: the rendered table and `needs:` line over a synthetic report, and the measurement over a real staged tree |
| `tests/trailer.rs` | the 64-byte trailer: the encoding, `None` against every error, the geometry arithmetic, and two never-panic properties |
| `tests/manifest.rs` | `ginary.json` and `ginary.index.json`: the wire field order, the unknown-key round trip, `check_version`, the `launch` path rules, `created_at`, and the index over a staging root |
| `tests/payload.rs` | the payload: deterministic packing, the round trip with modes, eight hand-built malicious archives, the two streaming reads, and three never-panic properties |
| `tests/macho.rs` | read-only Mach-O inspection (D3), against a committed real arm64 binary and hand-fabricated headers: the four thin magics and the two fat, `cputype` for x86_64 and arm64, a known section's file offset and size, an `LC_CODE_SIGNATURE` load command present and absent, a fat header's `is_fat` without an error, the typed refusals for a non-Mach-O and a truncated one, and two never-panic properties |
| `tests/payload_locate.rs` | `payload::locate` (D3, extended E9): the end-of-file trailer unchanged for a plain artifact, the eof trailer winning over a Mach-O section when both are present, a `__GINARY,__payload` section's absolute offset, `None` for a Mach-O with no section, and the three typed errors — a section too small for a trailer, one whose first bytes carry no trailer magic, and one whose declared length disagrees with the section's own size — plus `TrailerError::Fat` for a fat Mach-O. E9 adds the guard on the `LC_CODE_SIGNATURE` path a signed macOS artifact locates through: a Mach-O whose `dataoff` names an offset past the end of the file is not a read error, it falls through to the section lookup and returns `None`, rather than surfacing a spurious `TrailerError::Io` from reading past EOF |
| `tests/diag.rs` | the recorder through injected sinks: both output shapes, event order, elapsed time, and the four ways it stays off |
| `src/error.rs` unit tests | the five exit codes, the message of each variant, the `hint:` second line, and the panic-hook line |
| `src/selfexe.rs` unit tests | `/proc/self/exe` opens the running test binary, at offset zero, with the ELF magic; the magic and the route are `cfg(unix)`, the other two hold on either platform |
| `src/cache.rs` unit tests | the `Env` snapshot, the four resolution rules, the `TMPDIR` fallback and the entry path |
| `src/fault.rs` unit tests | the `<point>[:<action>]` grammar, the closed set of actions resolved through `armed_by`, and that nothing is armed without the feature |
| `src/launcher.rs` unit tests | the five `GINARY_CMD` values, that nothing near them is recognised, and the table `prune` and `uninstall` both print |
| `src/bundle.rs` unit tests | the three rules of a build with no seam an integration test can reach: the programs `distribution` and `heart` add to the ERTS bin set, the warning a distributed build with no `-name` earns, and the two ways a file `[tools.ginary]` names can fail to be read |
| `tests/launch.rs` | the pure plan: the argument vector in order, `GINARY_ERL_FLAGS`, non-UTF-8 arguments, the six set variables, the removal list and its `ERL_OTP*_FLAGS` family, the two refusals, and every preflight shape |
| `tests/cache.rs` | resolution and creation, the fallback warning, the ten extraction steps against a real payload, the sweep's three pid cases, and `clean` |
| `tests/launcher.rs` | the launcher contract on real processes: the environment, the argv, the exit code, the cache, the five failures, `GINARY_CMD`, `GINARY_DEBUG`, `GINARY_TRACE`, eight concurrent cold starts, the runtime settings, pruning on launch and the fault points |
| `tests/cache_lock.rs` | the two locks against util-linux `flock(1)`: a shared lock does not exclude a second shared lock and does exclude an exclusive one, `try_exclusive` answers `None` for an entry somebody holds, and the descriptor a `SharedLock` carries is not close-on-exec |
| `tests/artifact_real.rs` | toolchain-gated: one real artifact, assembled by hand out of the fixture and run with a cleared environment |
| `tests/config.rs` | `[tools.ginary]`: the defaults, every key, the five rules serde cannot state, the merge of the CLI flags over the table, the four shapes a `--out` can take, and the C4 native settings — a `[tools.ginary.native.<package>]` table read back against its package, and a target's own `native` map beside its hooks |
| `tests/gleam.rs` | the upward search for `gleam.toml`, what `--skip-export` reuses and what it says when there is nothing to reuse, the version line, and two gated runs of a real `gleam` |
| `tests/bundle.rs` | the parts of the build a machine with no toolchain can still hold: the refusal of a stub that already carries a trailer — through `check_stub` and through `build_with_stub`, which pins that the refusal comes *before* the export — the work directory's name, the report's two rendered forms, and (D3) the honest `BundleError::Stub` a `macos-aarch64` build gets with no `--stub` or `GINARY_STUB_DIR` on a host with no darwin toolchain |
| `tests/inspect.rs` | the text report and the launch plan over a hand-built `ArtifactInfo`, and a `SyntheticArtifact` opened, verified, and damaged in the two ways that matter |
| `tests/e2e_hello.rs` | toolchain-gated: `ginary build` in a copy of the `hello_ffi` fixture, and everything that follows — running the artifact with no Erlang on the machine, the warm cache, byte-identical rebuilds under `SOURCE_DATE_EPOCH`, `--report json` against the artifact's own size on disk, `--explain`, `-v` beside `GINARY_TRACE`, `inspect --verify`, `GINARY_CMD`, and the work directory |
| `tests/regressions.rs` | one module per fixed bug, `#[path]`-included from `tests/regressions/`; see the README there |
| `tests/verify.rs` | the deep check: a clean `SyntheticArtifact` raising nothing, the index findings over a payload laid out by hand so the index can disagree with the tree, a real ELF's object row, a machine mismatch, the allowlist and its injected-empty seam, an object refused by the injected size bound, a file that begins with the ELF magic and is not one, the rendered table, the command's two exit codes, the two ways `manifest.native` can lie about the artifact it describes — a row naming a file the index does not hold and a row recording a machine the object does not have — and a gated run over a real `ginary build` |
| `tests/sbom.rs` | the SPDX 2.3 document: the namespace derived from the payload digest, the whole document as a snapshot, the fields SPDX requires, the two relationship kinds, hex against `NOASSERTION`, a Gleam `manifest.toml` read, one refused and one absent, determinism over two runs, the command's `--out`, and two gated builds pinning `ginary build --sbom` and `--sbom-out` down to the report's last line |
| `tests/crashdump.rs` | a hand-written dump read field by field, a truncated one summarised rather than refused, a file that is not a dump, the `MAX_LINE_BYTES` bound, the rendered summary, the command's two forms, and a gated dump written by a real `erl` |
| `tests/doctor.rs` | what B2 added to `doctor`: the cache probe run honestly against a directory the test owns and rendered from hand-built values for the two failures no test may create, the project context — name, version, shipment age, `[tools.ginary]` status, native code under `priv`, a NIF installed as a symlink and a directory symlink the walk refuses to descend — and the `crypto` NIF, against a `FakeOtp` and against the host; C2 adds the targets table's host row through an injected resolution, resolving and refusing; C4 adds the per-target columns of the native table — the rendered table over one verdict of each kind, and the verdicts a project's own configuration reaches over an object under `priv` |
| `tests/target.rs` | what other modules ask the target model for: the container platform, `from_elf` over a glibc, a musl and a static binary, and `resolve_targets` — precedence, `host`, `all`, deduplication and the message an unknown selection earns |
| `tests/erts_source.rs` | the five ERTS source spellings and their four refusals, and the resolution through an injected ELF reader: a directory, a musl runtime, a static one, a target mismatch, a machine with no target, and a `FakeOtp` whose `beam.smp` is a shell script; three gated tests read the host's own emulator. The Windows arm has no injected reader — it reads a real PE header off a `FakeOtp::new().windows()` tree — and is covered in `tests/regressions/d2_a_windows_runtime_root_could_not_be_resolved.rs`. E7 adds the macOS arm on the same terms, over a `FakeOtp::new().macos()` tree whose `beam.smp` is a real thin Mach-O: a `cputype` resolved to `macos-aarch64`, a universal binary refused as more than one runtime, a `cputype` no target of ours names, and a header too short to be one |
| `tests/stubid.rs` | the identity marker: that this build's own binary carries exactly one, that the constant and the file scan to the same identity, the padding, and the scanner over bytes a test writes — none, two, a marker that runs past the end, an unterminated body, and each malformed field as its own typed error |
| `tests/stub.rs` | where a cross build's stub comes from and what it refuses: the four sources in order, both spellings in `GINARY_STUB_DIR`, the `.exe` suffix, the search that found nothing with every path in its message, and the seven gates of `verify` — the size cap, the marker, the version lock, the payload format, the target, the object header that disagrees with the marker, and a file that already carries a trailer. Two tests drive the real `ginary build`, and one gated test needs a cross-built musl stub. D3 adds three darwin cases over a hand-fabricated Mach-O carrying an appended marker, against the real Mach-O arm of `check_object`: a matching `cputype` accepted, a mismatched one refused by the header, and one already carrying a `__GINARY,__payload` section refused as an artifact. The RED-phase placeholder `a_darwin_stub_cannot_be_checked_here_yet`, which pinned the old `StubError::NotYetSupported` answer, is gone — it asserted the very behaviour these three replace |
| `tests/stub_flavor.rs` | the sentence a launcher-only build prints when it is run with no payload, asserted through `launcher::no_payload_line` in both flavors and through the process itself in whichever flavor the run compiled |
| `tests/sign_macos.rs` | `sign_macos::inject_and_sign` (D3, extended E8, reworked E9; `cli`-gated): E9 replaced the carve-a-new-section layout — which two real Macs proved verifies yet segfaults — with the append-inside-`__LINKEDIT` layout that also *runs*, so the tests move with it. The payload is appended after `__LINKEDIT`'s content and the segment grown to cover it, nothing slides, and `payload::locate` round-trips the exact bytes and digest injected — unsigned through `PayloadVia::EofTrailer` (its trailer is the last 64 bytes) and ad-hoc signed through `PayloadVia::MachOAppended` (its trailer sits just before the reused `LC_CODE_SIGNATURE`), each pinning the discriminant, `report.payload_offset`, the length and the bytes. Two E9 tests hold the run-AND-verify invariants a section layout broke: `an_injected_artifact_runs_the_stubs_own_entry_instructions` reads the bytes at the finished artifact's mapped entry (through `common::macho::entry_point`) and asserts they are the stub's own first instructions — the entry moved nowhere, because nothing moved — and `an_injected_artifact_does_not_claim_to_be_linker_signed` asserts the `CodeDirectory` `flags` carry `CS_ADHOC` and not the `CS_LINKER_SIGNED` a binary ginary rewrote must not claim. The typed refusals — a fat stub, a non-Mach-O stub, one already sectioned — stand, against the committed real Mach-O fixture standing in for a darwin stub, since none can be built on this host. E8's *validity* half through `tests/common/codesign.rs` stays: every code slot is the SHA-256 of the page it stands for (the signature covers the finished file, not the bytes before the last four fields were patched in), the signature begins on a 16-byte boundary and is the last thing in the file, the `CodeDirectory` describes the file it is attached to (`codeLimit`, one slot per page, `execSeg` naming `__TEXT` as finally laid out), and the appended payload stays inside what the signature covers and inside `__LINKEDIT`, which ends the file |
| `tests/download.rs` | one HTTPS fetch against a hand-rolled loopback server: the body written and the part file gone, a checksum and a length mismatch naming both values, a 500 retried and a 404 asked exactly once, a truncated body retried, three failures exhausting the attempts, the offline refusal that opens no socket, and the policy — the part file's name, the backoff schedule, the retryable statuses, one spelling of a digest, the base overrides and the two environment variables; and the same six questions asked of `get_text`, the release-API reader — a body back verbatim under the GitHub accept header, a 500 retried, a 404 asked once, a body over `MAX_TEXT_BYTES` refused rather than read into memory, the offline refusal naming no file, and a read that goes through the base override |
| `tests/catalog.rs` | the catalog: every field of schema 1 and an unknown key surviving at two levels, the schema and parse errors, the three sources with first-found winning the whole file, the selection rules — the host release, an exact version, the musl default, a named variant, ambiguity and each miss listing what is there — the version guard inside `select`, URL resolution against the catalog's own directory, and the cache: the completion marker, a warm cache needing no network, the whole cold path, a markerless extraction thrown away, the offline error travelling, a tarball keyed by its own digest, and the strict extractor's four refusals over hand-built archives — a symlink, a `..` path, an absolute path and a device node, each named and each leaving no runtime behind. D3 adds the `erlef/otp_builds` asset name for each arch, pinned against the real `OTP-29.0.5` release, and the commit guard that only admits an entry whose `otp_release` matches the host's own |
| `tests/otp_repack.rs` | the local pipeline: the six-row upstream asset table and four combinations it has none for, the selector grammar, the tag-to-version rule, the prune list against components rather than substrings, the dereference and the assertion that guards the strict extractor, and the pipeline itself over a fake upstream asset — the entry's fields, `SOURCE_DATE_EPOCH`, URLs relative to the catalog, a mislabelled asset refused before anything is written, and the injected ELF reader's error travelling; and the release API driven against a scripted server through `Net`'s base override — the digest it reported pinned into the entry, a body that does not match it refused, an asset carrying no digest refused rather than pinned to nothing, a release holding another architecture's asset, and a document that is not a release |
| `tests/erts_source_catalog.rs` | the two sources C3 adds, driven through the injected ELF reader: a catalog entry resolved out of a warm cache with its provenance, a claim about the machine and a claim about the linkage each denied by the emulator and named on both sides, the offline error crossing both layers, a selection error surfacing as itself, and a tarball extracted under its own digest |
| `tests/otp_cli.rs` | `ginary otp`: the list table and its `--target` and `--json` forms, an empty catalog explaining itself, `path` printing one line and never fetching, `fetch` refusing offline and naming what the catalog does hold, `update` copying bytes only after validating them, `GINARY_CATALOG`, and `repack --help` |
| `tests/e2e_cross.rs` | four-way gated: a real cross build out of the committed catalog for `linux-x86_64-musl`, `linux-aarch64-musl` and `linux-x86_64-gnu`, each artifact run in a container with no Erlang and no network, the aarch64 row behind a binfmt probe and the glibc row on the oldest Debian its own catalog entry allows |
| `tests/smoke_matrix.rs` | the C3 scaffolding held against the repository: the smoke-matrix script committed, executable, probing before it installs a binfmt handler and printing a PASS/FAIL table; the two mise tasks; `git check-ignore` proving the catalog committed and the tarballs not; and the four documents — the ADR and its index entry, the catalog schema in `docs/format.md`, the README's quickstart and caveats, and this table |
| `tests/native.rs` | the native half of a cross build, over fabricated objects: the scan — every object under `priv` in path order, the magic deciding rather than the extension, an ELF under `ebin` left alone, the format, machine and target of an ELF, a PE and a Mach-O, a library told from a program, the four files that begin like an object and are not one (a truncated ELF, a truncated Mach-O, a DOS `MZ`, an object past the size bound) and the directory a walk stopped at — and the reconciliation: an object already for the target kept, an override applied, verified, refused for the wrong machine and refused when it is not there, a static override accepted with a note, a hook's environment and working directory, a hook that writes nothing, a hook that fails, a hook that builds for the wrong machine, a hook that writes once and cannot answer for a second target, an override winning before a hook runs, the mismatch table and the same rows as an `--allow-native-mismatch` warning, the static-runtime refusal that the flag does not lift, `apply` over a staged tree, and the verdict of each artifact |
| `tests/e2e_native.rs` | four-way gated, the cross-built stub among the four: `ginary build` over a shipment with an object planted in its `priv` — a host build recording it in the manifest, a cross build refused with the table, the same build allowed through, a static runtime refusing a NIF it could not load, and a `native` override replacing one and saying so in the manifest |
| `tests/formal.rs` | the TLA+ model held against the repository: both files committed, every action and state named, the `.cfg` naming the four invariants, `mise run formal` pinning its checker by digest and passing `-deadlock` on no command line, and `docs/dev/formal.md` mapping the model onto `src/cache.rs`. It does not run TLC; `mise run formal` does |
| `tests/windows.rs` | the launcher half of Windows support, held to what a Linux machine can honestly check — every claim is a pure function: the cache root (`GINARY_CACHE_DIR`, `%LOCALAPPDATA%\ginary`, the `%TEMP%\ginary-<user>` fallback and its three bases, an empty variable counting as unset, the `%USERNAME%` that is not one path component) with the provenance table as a snapshot; the `\\?\` prefix over a drive-absolute path, forward slashes, UNC, an already-prefixed path, a relative one, and the identity that borrows on unix; the exit code a spawned child becomes, 256 and an access violation included; the two share modes the locks become — `FILE_SHARE_READ` for a runtime and `FILE_SHARE_DELETE` for a prune, which shares no reading and no writing and permits the rename the prune performs while holding the entry; `erl.exe` as the launch program of the Windows row of `target::ALL`; and that a Windows launch plan is the unix one with a different program name. Ungated, so the stub flavor asserts it too — the stub is the binary a Windows artifact is made of |
| `tests/windows_build.rs` | the build half and the D2 scaffolding: the data-driven required-file probe over a `FakeOtp::windows()` — `erl.exe`, `beam.smp.dll`, `inet_gethost.exe` and every DLL beside them, sorted, with `erl.ini`, `erlsrv.exe` and `werl.exe` left behind — the three refusals by name, the `erl.ini` removal and its size in the junk account, the four runtime sources a Windows build may not take its runtime from and the one it may, and five documents nothing else would notice going stale: the `build:windows` task, the README's `## Windows` section, the Windows half of `docs/dev/debugging.md`, ADR 0015 and its index entry |
| `tests/ci_matrix.rs` | the repository's own CI, held as data (E1, extended in E3): every job `ci.yml` promises and the fan-in's `needs:` list, the nightly and release workflows, the two committed CI scripts and their executable bits, the three security workflows — the CodeQL matrix parsed to `language: build-mode` rows, its weekly slot, Scorecard's publication and SARIF upload, dependency-review deferring to `deny.toml` — the dependabot policy parsed entry by entry and pinned as a snapshot, and the two hardening guards over *every* workflow: a top-level token that grants nothing but reads, a `permissions:` mapping on every job, and a full-SHA pin with a `# vX.Y.Z` comment behind every `uses:`; extended again in E4 with the toolchain matrix — the one `msrv` job that checks the declared floor and nothing else, its toolchain string held equal to `rust-version` in `Cargo.toml` so the two copies of the number cannot drift, and every other site across all seven workflows installing `stable`, `nightly.yml`'s `fuzz` excepted because cargo-fuzz has no stable equivalent; and the scope of `renovate.local.json5`, the one exception the local freshness gate is given — parsed with `serde_json` and held to a single `packageRules` entry over one datasource in one file, because a config that silences a gate is worth exactly its scope |
| `tests/repo_hardening.rs` | the half of a public repository that is not code (E3): the two rulesets parsed through `serde_json` and snapshotted in canonical form, the required status check compared against the `name:` of `ci.yml`'s `required:` job, CODEOWNERS, the pull-request template's `mise run check` and regression-test rows, the two issue forms and their config parsed as YAML — the target dropdown's own options, which fields are `required`, the private-advisory link first — a contact link tied to the repository setting it needs, and `SECURITY.md` |
| `tests/v1_readiness.rs` | the documents and metadata a v1 is judged by (E1): the README's structure and badges against the published slug, the licence files, the changelog, `CONTRIBUTING.md`, and the crate metadata `Cargo.toml` carries |
| `tests/deps.rs` | the committed dependency record, held to what the development machine's pre-push freshness gate reads (E4): `sha2` requested on the 0.11 line and `Cargo.lock` resolved onto it, one version each of `sha2`, `digest` and `block-buffer` — two `digest` majors are two incompatible `Digest` traits and that is what a half-finished migration looks like — and `sha2` and `hex`, the pair that computes and spells every digest, locked on the minor line their requirement names. Reads `Cargo.toml` and `Cargo.lock` through `tests/common/deps.rs`, a hand-rolled scanner rather than `toml`, because `toml` is behind the `cli` feature and these assertions hold for the stub flavor too |
| `tests/digest.rs` | SHA-256 is on-disk format, and this is the statement of it (E4): three published vectors — the empty input, `abc`, and one mebibyte of `index % 251` — hashed through `manifest::Index::from_staged` against hard-coded hex, the mebibyte pattern itself pinned so the vector above cannot become a test of nothing, the five committed `hello_ffi` fixture files snapshotted as `path size sha256`, `Packed.sha256` proved to be the digest of the bytes `payload::pack` wrote, and the unpack side recomputing exactly what the index recorded. Every constant was recorded before the 0.11 bump and checked against `sha256sum`, so a future swap of the hashing library that moves one byte fails loudly |

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

## The clean room

`cargo test` cannot answer the question the whole project is about, because it runs on a machine
that has Erlang installed and can only scrub the environment to pretend otherwise.
`scripts/smoke.sh` does not have to pretend: it packages the `hello_ffi` fixture and runs the
artifact inside `ubuntu:24.04`, which genuinely has no Erlang, with `--network none`, which
genuinely has no way to fetch one. Three checks, each a claim no in-repository test can make:

- `! command -v erl >/dev/null && /app 0 x y` — the machine has no Erlang and the application
  runs anyway, printing its arguments and reading its `priv`;
- `/app 7; test $? = 7` — the application's own exit code crosses the container boundary;
- `--read-only --tmpfs /tmp:rw,exec` with `HOME` on the read-only rootfs — the cache cannot be
  created where it would like to be and falls back to `${TMPDIR:-/tmp}/ginary-<uid>` with one
  warning, which is the path `src/cache.rs` has unit tests for and had never actually taken.

`mise run smoke` runs it. An unreachable docker daemon is a reported skip, the same rule
`require_tools` follows, and `GINARY_REQUIRE_TOOLCHAIN=1` turns that skip into a failure; the CI
`smoke` job sets it and is one of the jobs `required` waits on. `GINARY_BIN` points the script at
a release binary rather than `cargo run`, which is how the artifact size in
`docs/dev/log/A4.md` was measured — the debug stub is fifteen times the release one and the
number would say nothing.

The library and binary targets spawn only fake shell scripts in temporary directories, never a
program from the machine's `PATH`. Four integration targets do reach it, each for a stated
reason:

| target | what reaches `PATH` | how it is bounded |
|---|---|---|
| `tests/smoke_cli.rs` | `ginary doctor` probes whatever `gleam`, `erl`, `strip` and `docker` are there | none has to be present or to succeed; a hanging probe costs `doctor::PROBE_TIMEOUT` (10 s) before it is killed |
| `tests/cli.rs` | the same, plus the `otp` field, which runs the ambient `erl` | the two `otp` assertions are gated on `require_tools(&["erl"])` |
| `tests/otp.rs`, `tests/appfile.rs`, `tests/closure.rs` | `otp::discover(None)` and the host OTP tree it names | every one of those tests is gated on `require_tools` |
| `tests/stage_run.rs` | `gleam export erlang-shipment`, `otp::discover(None)`, and the `erlexec` of the staged tree | every test is gated on `require_tools(&["gleam", "erl"])`; the launched runtime gets `env_clear()`, an empty `PATH` directory and a `HOME` inside the test's temporary tree, and both children run under a deadline — `fixture::EXPORT_BUDGET` (180 s) and `erl::RUN_BUDGET` (60 s) — with stdin on the null device |
| `tests/gleam.rs`, `tests/e2e_hello.rs` | the real `gleam`, and through `ginary build` the real `erl` and `strip` | every one of those tests is gated on `require_tools`; the build runs under `built::BUILD_BUDGET` (900 s) and each run of the artifact under `built::RUN_BUDGET` (120 s), and the artifact itself is run with `env_clear()` and an empty-directory `PATH`, so nothing ambient reaches the packaged application |
| `tests/regressions.rs` | nothing ambient: it *replaces* `PATH` with a temporary directory holding stub scripts | the stubs exit at once |
| `tests/cache_lock.rs`, `tests/launcher.rs`, `tests/regressions.rs` | util-linux `flock(1)`, and `sleep(1)` for the ADR 0010 proof | every one of those tests is gated on `require_tools(&["flock"])` — the lock has to be observed by a program that is not ginary, or it proves nothing about the kernel — and `GINARY_REQUIRE_TOOLCHAIN=1` turns the skip into a failure, which is how CI keeps them from quietly not running |
| `tests/crashdump.rs` | one `erl` run that writes a real `erl_crash.dump`, so the parser is held against a file its author did not write | gated on `require_tools(&["erl"])`; the recipe is `erl -noshell -env ERL_CRASH_DUMP <tmp>/dump -eval 'spawn(fun() -> exit(kaboom) end), timer:sleep(100), erlang:halt("kaboom", [{flush,true}]).'`, which exits 1 and leaves a whole dump ending in `=end` |
| `tests/doctor.rs`, `tests/verify.rs` | the host OTP's `crypto` NIF, and one real `ginary build` verified end to end | both gated on `require_tools`; `tests/verify.rs` also reads a shipment named by `GINARY_TEST_ARTIFACT` when one is set, and reports a skip when it is not |
| `tests/strip.rs`, `tests/elf.rs` | the one `strip` run and the two `beam.smp` reads | both gated on `require_tools`; everything else in the two files runs against the test binary, a temporary tree, or a stub `erl` written by the builder |
| `tests/e2e_cross.rs` | the real `gleam` and `erl` through `ginary build --target`, and `docker run` for three images | gated four ways, each absence a printed skip naming the task that produces it: `require_tools(&["gleam", "erl", "docker"])`, `dist/otp/catalog.json` (`mise run otp:repack`), a cross-built stub (`mise run stubs:build`), and for the aarch64 row a `docker run --platform linux/arm64` probe. The build runs under `BUILD_BUDGET` (900 s) and each container under `RUN_BUDGET` (180 s), with `--network none` so an artifact that fetched anything at run time would fail rather than pass |
| `tests/smoke_matrix.rs` | `git check-ignore`, and nothing else | gated on `require_tools(&["git"])`; every other test in the file reads committed files |
| `tests/stub.rs` | one gated test runs the real `gleam` and `erl` and needs a cross-built stub | gated on `require_tools(&["gleam", "erl"])` *and* on `stubfile::cross_stub`, which looks in `$GINARY_STUB_DIR` and then `target/stubs` and reports `skipping: no ginary-stub-<version>-<target>` when there is none; `GINARY_REQUIRE_STUBS=1` turns that skip into a failure, and `GINARY_REQUIRE_TOOLCHAIN` deliberately does not. Every other test in the file runs `ginary build` with `GINARY_STUB_DIR` and `GINARY_CACHE_DIR` pointed at empty directories the test owns, so a stub on the developer's machine cannot change the answer |
| `tests/e2e_native.rs`, `tests/regressions/c2_the_artifact_never_had_to_use_the_stub.rs` | five more tests that need a cross-built stub | the same `stubfile::cross_stub` gate as the row above, for `linux-aarch64-musl`, `linux-x86_64-gnu` and `Target::host()`; they are named here because they are the five that ran in no CI job while the `smoke-matrix` step listed only two files. The workflow derives the list from the tree now: `tests/regressions/e6_five_stub_gated_tests_ran_in_no_ci_job.rs` |

Those bounds are what keeps `test:fast` fast; they are not a claim that nothing external runs.

## The two flavors of the suite

C2 split the crate with the `cli` feature, so there are now two binaries to hold to a contract
and the suite runs twice:

| task | build | what it covers |
|---|---|---|
| `mise run test` | `cargo test --features fault-injection` | everything, against the full command line tool |
| `mise run test:stub` | `cargo test --no-default-features` | the launcher half, against a binary with no clap, no TOML reader and no commands |
| `mise run lint:stub` | `cargo clippy --no-default-features --all-targets -- -D warnings` | that the stub build compiles clean, tests included |

`check` runs all three. Without `test:stub` and `lint:stub` the stub configuration would be
compiled by nothing until `stubs:build` failed on a developer's machine.

**How a test target chooses.** A file whose every claim is about a module the `cli` feature
carries opens with `#![cfg(feature = "cli")]` under its module documentation, so a stub-flavor
run compiles it to an empty test binary rather than to an error. Twenty-five of the integration
targets are in that group. What is left runs in both flavors: `tests/launcher.rs`,
`tests/launch.rs`, `tests/cache.rs`, `tests/cache_lock.rs`, `tests/payload.rs`,
`tests/manifest.rs`, `tests/trailer.rs`, `tests/target.rs`, `tests/diag.rs`, `tests/formal.rs`,
`tests/stubid.rs`, `tests/stub_flavor.rs`, `tests/windows.rs` and 19 of the 74 modules of
`tests/regressions.rs`, which are gated one `mod` line at a time.

That set is not an accident: it is exactly the modules a stub carries. `SyntheticArtifact` is
built from `payload::pack`, `manifest::Index` and the staging *listing* types, and those are the
items `assemble.rs` keeps outside its own `cli` gate, so the whole launcher contract — the
argument vector, the environment difference, the cache, the lock, the five numbered exit codes —
is asserted against the stub build as well as the full one.

**A test that differs between the flavors asserts both branches rather than one.**
`tests/stub_flavor.rs` is the pattern: the sentence a payloadless stub prints lives in
`launcher::no_payload_line`, so both flavors can assert the string, and the *process* assertion
branches on `cfg!(feature = "cli")` — a full build must print `Usage:` and must not claim to have
no CLI, a stub build must print the sentence and nothing else.
`launcher::the_magic_is_what_decides_the_mode` does the same where it breaks the magic: the claim
is that the magic decided the mode, and what the other half turns out to *be* depends on which
binary the suite built.

**Two test helpers are gated.** `common::built` drives `bundle` and `common::fake_otp` drives
`otp` and `beam`, so both carry `#[cfg(feature = "cli")]` in `tests/common/mod.rs`;
`stubfile::cross_stub` names `stub::STUB_DIR_VAR` and is gated in place. `common::tools` calls
`process::find_in_path` rather than the `doctor` re-export of it, which is what lets the
launcher-side files that gate on `require_tools` compile without the feature.

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
`GINARY_TEST_SHIPMENT` — and there is no default, because a path is not a program. The rule is
`tests/common/shipment.rs::choose_shipment`: unset — or set to nothing at all, which is what an
unset `${{ vars.… }}` expands to and what `var_os` reports as `Some("")` — is a reported skip
*however* `GINARY_REQUIRE_TOOLCHAIN` is set, because that variable is a claim about programs the
machine installs and cannot be a claim that somebody exported a Gleam project here; a non-empty
value that is not a directory is a failure *however* it is set, because the caller asked for a
run and mistyped the path. A fixture a gated test needs from outside the repository is named by
the caller or it is not used.

A test can also need a *file this repository builds and does not commit*, and that is a third
question again. The five cross-built stubs under `target/stubs` come from `mise run stubs:build`,
which needs `cross`, a running docker daemon and minutes per target; no amount of Erlang on the
machine produces one. So `tests/common/stubfile.rs::choose_cross_stub` reads a switch of its own,
`GINARY_REQUIRE_STUBS`, and reads `GINARY_REQUIRE_TOOLCHAIN` not at all: a missing stub is a
printed skip that names `mise run stubs:build`, and only a job that *obtains* the stubs sets
`GINARY_REQUIRE_STUBS=1` to turn that skip into a failure — where a miss means the step that
built or downloaded them produced nothing rather than that the machine never had one. Four
tracked files ask for one, and between them they hold nine tests: `tests/e2e_cross.rs` (three),
`tests/e2e_native.rs` (four), `tests/stub.rs` (one) and
`tests/regressions/c2_the_artifact_never_had_to_use_the_stub.rs` (one, in the `regressions`
target). In CI two jobs have the stubs and run all four targets: `smoke-matrix`, which
cross-builds three of them, and `coverage`, which downloads all five from `cross-build` because
the 90% line floor it enforces was measured with those nine tests running.

The stubs are half of what those nine need. Seven of them — every test in `tests/e2e_cross.rs`
and `tests/e2e_native.rs` — write `erts = "catalog"` into the fixture and build against
`dist/otp/catalog.json`, and the repository commits the catalog while `.gitignore` keeps every
tarball it names out of the tree. A job holding the stubs but not the runtimes does not skip
those seven: it runs them, and each dies in the runtime resolver with `cannot use the catalog:
... No such file or directory`. So both jobs also run `ginary otp repack --out dist/otp` before
the tests, and the rule that ties the two artifacts together is asserted over every job of
`ci.yml` in
`tests/regressions/e6_the_coverage_floor_measured_a_stubless_subset.rs`.
The `test` job builds and downloads none and skips them, loudly. Conflating the two questions is
what failed `test` and `coverage` on the first pull-request run, and counting the four files by
hand in a comment is what left five of the nine running in no job at all; see
`tests/regressions/e6_the_toolchain_flag_required_a_cross_stub_nobody_built.rs` and
`tests/regressions/e6_five_stub_gated_tests_ran_in_no_ci_job.rs`, which derives the list from the
tree so a fifth caller cannot be added without the workflow learning about it.

A fourth question, and the third gate: a program that is **not** part of that toolchain at all.
`actionlint` lints the workflow files. It has nothing to do with whether a runtime can be
packaged, no hosted runner ships it, and `mise` installs it on a developer machine — so
`require_tools(&["actionlint"])` was a claim `GINARY_REQUIRE_TOOLCHAIN` could not make true, and
the three jobs that set that variable and run the `regressions` target all panicked on runners
whose toolchain was complete. `tests/common/tools.rs::require_actionlint` reads
`GINARY_REQUIRE_ACTIONLINT` instead, and exactly one job sets it: `lint` in
`.github/workflows/ci.yml`, which installs the tool from its own release with a pinned digest and
then *runs the test by name*. Both halves are asserted, because a check moved out of three jobs
and into none is a check that was deleted rather than fixed; see
`tests/regressions/e7_actionlint_was_required_of_every_toolchain_job.rs`.

E11 adds two gates that are not variables at all, and that is the point of them. Both live in
`tests/common/tools.rs` beside the three above. `require_posix_shell` answers with `/bin/sh` by
absolute path, or a printed skip: the claim those tests make is about what a POSIX shell does
with a line, so a machine without one cannot answer it. It is a program the toolchain jobs do
install, so it escalates under `GINARY_REQUIRE_TOOLCHAIN` exactly like `require_tools`, and a
name looked up on `PATH` would not do — `bash` resolves on a Windows runner to the Windows
Subsystem for Linux launcher, which exits `1` with nothing on either stream. What the gate
answers is held equal to what the hook rule names by
`tests/regressions/e11_a_shell_script_test_ran_on_a_host_with_no_posix_shell.rs`, so the two
cannot drift.

`require_elf_stripper` is the new *kind*. It asks two things: that `strip` is on `PATH`, which is
the ordinary gate, and that the host's own executables are ELF files, which nobody can install.
The fixture every ELF-stripping test plants is a real binary this machine wrote, and
`ginary::strip`'s ELF phase reads what a linker put there rather than a header written by hand,
so on a Windows runner the first condition holds and the second cannot. That half therefore
escalates under **no** variable: `GINARY_REQUIRE_TOOLCHAIN=1` on a Windows job is a true claim
about the toolchain and would be a false claim about the object format, and a gate that panicked
there would be demanding a machine nobody can provide. The skip is printed and names the format
the host writes, so the reason is in the log rather than in a reader's head.

The rule the five gates share is worth stating once. A gate is a claim somebody has to be able to
make true, so it belongs to whichever job installs the thing it is about:
`GINARY_REQUIRE_TOOLCHAIN` to the jobs that install Erlang, Gleam and a POSIX shell,
`GINARY_REQUIRE_STUBS` to the jobs that build or download the cross stubs,
`GINARY_REQUIRE_ACTIONLINT` to the job that installs actionlint. A sixth variable is warranted
exactly when a sixth kind of thing is promised by a different job — and *no* variable is
warranted when the thing is a property of the platform, because then there is no job that could
set it honestly.

A skipped test must say so. A silent skip is indistinguishable from a passing test and is treated
as a defect.

## Fake trees

`tests/common/fake_otp.rs` builds the two directory layouts every build-side module reads, in a
temporary directory, in milliseconds, with no Erlang installed. `tests/common/script.rs` is the
third builder: `program` plants a throwaway `erl` on a `PATH` of its own, described as a list of
`ShimStep` rather than a line of shell, because the same behaviour has to exist in two forms.
`shim_form` and `shim_file_name` are the two rules that decide which: on unix an executable
`/bin/sh` file called `erl`, and on Windows the compiled `examples/ginary_test_shim.rs` copied
to `erl.exe`, because nothing there reads a shebang — `CreateProcess` looks for `MZ`, finds
`#!`, and refuses the file, which is how thirty-six targets failed on the first Windows runner
inside the fixture builder itself. The shim reads its steps from `<program>.steps` and writes
`<program>.argv`, and `shim_sidecar` is the one naming rule both forms use for those files.
`tests/common/snapshot.rs` is the fourth helper, and exists because those
trees live in a `tempfile` directory whose name changes on every run: `scrub` replaces each root
with a placeholder, longest path first, and respells every separator as `/` through
`tests/common/hostpath.rs`, so a snapshot pins the sentence and the shape of the path rather
than the machine or the slash it writes between two components. `hostpath` holds seven more
rules of the same kind. `is_absolute_for` decides absoluteness per platform, over drive-absolute,
UNC and verbatim spellings, and `strip_dir` removes a fixture directory whichever separator
joined it to the name behind it. E11 added five: `separator_for` names a platform's separator;
`joined_for` joins a `/`-separated listing path onto a root the way a named platform spells one,
respelling every separator of the relative half and leaving the root and every backslash in a
unix file name alone, with `joined` the same rule asked about this machine; `json_escaped`
spells a path the way a JSON document carries it, so a test that looks for a path inside a trace
looks for what is actually written there; and `same_path` compares two paths as the host's file
system does, which is not string equality on a platform whose names are case-insensitive. The
join rules exist because `Path::join` spells one join with the host separator and leaves the rest
alone, which on Windows produces the mixed spelling nothing writes — see
`tests/regressions/e11_a_listing_path_was_joined_the_way_the_host_spells_one.rs`.

`script` grew three helpers and a step alongside the shim rules above: `ShimStep::Sleep` is a
program that stays alive for a while, expressed in milliseconds and rendered as whole seconds by
the `/bin/sh` form and as milliseconds by the compiled one; `live_process` plants and spawns one,
which is how a test gets a process it can observe without a `sleep` binary; and `recorded_argv`
reads the argument vector a planted program wrote, through `argv_log_path`, which names the
sidecar under the platform's own spelling of the program rather than under its unix name.
`tests/common/native.rs` is the object-fixture builder: `object_for` writes a shared object for a
named target in that target's own container format, `host_native_object` is what a test plants
where the host's own native code goes — the committed x86-64 glibc ELF fixture on the one host it
is really for, a fabricated header everywhere else — and `host_writes_elf` is the question a test
asks before reaching for the running executable as a fixture. `tests/common/http.rs` gained
`REPLY_SHUTDOWN`, `DRAIN_BUDGET` and `answer_one`: the first two are the close rule for a served
connection, half-closed and then drained under a bound rather than dropped, because a fixture
that tore down a connection it had just answered raced its own client; the third serves exactly
one request and hands back what was asked.
`tests/common/fixture.rs` and `tests/common/erl.rs` are the two A1c added, and they work on real
trees rather than fake ones: the first copies a fixture Gleam project and exports it, the second
boots what assembly wrote. `tests/common/bounded.rs` is what both of them spawn through, so that
neither can hang the suite; it is the test-side counterpart of `src/process.rs`, which it cannot
call because that function takes neither an environment nor a working directory.
`tests/common/artifact.rs` is what A3b added, and it is the only helper that builds a whole
*artifact*: a staging root whose `erts-<vsn>/bin` programs are `/bin/sh` scripts, the real
`payload::pack` over it, and this test run's own `ginary` binary with that payload and a real
trailer appended. The launch program's stub prints one `env:<NAME>=<VALUE>` line for every
variable the launch contract names — `<unset>` for one that is absent, so an absent `ERL_LIBS`
cannot be confused with a stub that never ran — then one `argv:` line per argument, and exits 7.
Seven is not zero on purpose: "the exit code is mirrored" has to be a claim about a number
nothing else in the system produces. The stub also answers `--exit N`, `--signal N` (it kills
itself, so a supervised run has a signal to turn into `128 + signo`), `--dump` (it writes
`$ERL_CRASH_DUMP` with a `Slogan:` line in it, which is all `launch::supervise` reads) and
`--sleep N` (it runs `sleep N` as a separate process, which is what gives ADR 0010's proof a
runtime to observe and a grandchild to inherit the lock). It exits 0 rather than 7 when its
`-eval` is `erlang:halt(0)`, so that `GINARY_CMD=selftest` exercises the whole path on a machine
with no Erlang, and the `env:` lines it prints cover `HEART_COMMAND` and the manifest's own
`launch.env` names alongside the six the launch contract fixes. Everything the launcher decides is therefore readable on
standard output, and the launcher's whole contract is testable on a machine with no Erlang at
all. `SyntheticArtifact` also carries the ways an artifact can be broken — `break_magic`,
`break_geometry`, `break_payload`, `break_payload_tail`, `truncate` — because each one is a
different numbered exit code and a test that broke the file by hand would be a test that broke it
slightly differently each time. The two payload ones differ in *where*, and A4 added the second:
the launcher only needs a digest that no longer matches, but `ginary inspect` has to keep
answering "what was this file supposed to be" about a file that fails `--verify`, so its tests
damage the payload sixteen bytes before the end, past both front entries, which is the same place
`tests/e2e_hello.rs` damages a real artifact by hand. `truncate` takes its bytes out of the
*payload* and leaves the trailer at the end, which is deliberate and was corrected during A3b:
shortening the file from its end takes the trailer with it, and `docs/format.md` rule 2 makes
what is left the ginary command line tool rather than a damaged artifact. The fault worth a test
is the one that still carries a trailer and no longer matches it.

C1 adds no builder and one seam. `erts_source::resolve_with` takes the function that reads the
emulator, so a `FakeOtp` root plus a hand-written `ElfFacts` is a whole musl or static runtime as
far as the plumbing above the ELF reader is concerned — the provenance strings, the target
mismatch and the `nif_loading` rule are all reachable on a machine with no cross-built `beam.smp`
on it. The fake's own `beam.smp` is a `/bin/sh` stub, which is why the *unseamed*
`erts_source::resolve` over one is the test for `NotAnElfRuntime`: the mistake that error exists
for is a runtime tree assembled by hand, and the builder writes exactly that.

C2 adds a second seam of the same shape one layer up. `doctor::probe_targets_with` takes the
function that resolves a runtime, because the one row `doctor` actually reads is the host's own
and reading it needs an Erlang on the machine. Both halves of that branch are pinned without a
toolchain — a resolution that succeeds is a `yes` row carrying the provenance, the linkage and
the minimum libc it was handed, and an `ErlNotFound` is a `not yet` row carrying the whole error
chain — and the machine that *has* an `erl` is covered on top of that by two gated tests. It was
the fix for a C1 defect: two ungated tests asserted that `resolvable` was true, so a machine with
no Erlang on it saw a failure where this document promises a reported skip.

`tests/common/repack.rs` is what B2 added, and it is the one helper that writes a tar archive
itself rather than calling `payload::pack`. It has to: `pack` computes `ginary.index.json` from
the same walk it packs, so an index that disagrees with the tree it describes cannot be produced
by the code that writes both, and those disagreements are exactly what `ginary verify` exists to
find. `repack::build` stages the same tree `SyntheticArtifact` does, builds the same index, takes
the digests, and only then applies `RepackOptions`: `corrupt` rewrites a file's bytes behind the
index, `drop_from_index` deletes a row and leaves the file, `ghost_index_rows` invents a row for a
file nobody packed, `appended` writes entries the packer never would — a second `ginary.json`, a
directory entry, a symlink — and `target` makes the manifest claim another architecture. What
comes out is a whole artifact whose *trailer digest matches its payload*, which is the point:
`inspect --verify` passes on every one of them. It also carries the real ELF the synthetic tree
deliberately has not got — `with_native_object` plants `test_binary()` at `NATIVE_PATH`, and
`patch_elf_machine` rewrites two bytes of its header so a test on one architecture has a binary
for another with no cross toolchain. E7 pointed `test_binary()` at this test run's own binary; E9
repointed it at the committed `tests/fixtures/elf/` ELF (read directly, so `repack` still builds
under `--no-default-features`), because a test that plants "a real ELF" was planting a PE on
Windows and a Mach-O on macOS, where `elf::inspect_bytes` refused it. With the fixture the plant
is a genuine `x86_64` Linux ELF on every host, and the expectations read the object's own machine
— `native_machine`/`native_target` off the fixture's `e_machine`, not `Target::host()` — so the
row a healthy artifact lists is the one the payload really carries; `foreign_machine`/
`foreign_target` are that value's opposite, the machine to rewrite the header to for the mismatch
tests.

`tests/common/cachefs.rs` is what B1 added, and it exists because pruning turns on two things a
test cannot fake for itself: how old an entry is, and whether anybody is using it. `plant_entry`
writes `<app>/<key>/ginary.json` and back-dates it with `set_mtime`, so an entry can be thirty
days old in a test that takes a millisecond. `is_unlocked` asks `flock -n -x` whether a lock file
is free and `wait_until_unlocked` polls that answer until it matches, bounded by `LOCK_BUDGET`
(10 s), so a claim about a lock is an assertion rather than a hang. `HeldLock` takes an exclusive
lock from *outside* ginary — `flock -x <lock> sh -c 'read line'` with a pipe on its standard
input — and releases it by closing the pipe rather than by killing the process: `flock(1)` forks,
and the grandchild is the one holding the inherited descriptor, which is ADR 0010's own mechanism
seen from the other side. Killing the `flock` process would leave the lock held and the test
watching the wrong thing.

`Runner::spawn` and `Runner::output` retry while the kernel answers `ETXTBSY`. That is a
property of the harness and not of the launcher: cargo runs these tests as threads of one
process, a `fork` for one test's spawn inherits the descriptor another thread is writing the
next artifact through, and until that child reaches `execve` the file is still open for writing
(rust-lang/rust#39189). Serialising the suite would hide a race the launcher does not have; the
retry is bounded at ten seconds, after which `ETXTBSY` is reported like any other failure.

The *wait* is bounded too. `Runner::output` and the eight-way concurrency test go through
`bounded::wait_bounded`, the half of `run_bounded` that takes an already-spawned child, with a
budget of `artifact::RUN_BUDGET`. A launcher that deadlocks on the cache is precisely what these
tests exist to catch, and an unbounded `wait_with_output` would report it as a stalled job with
no diagnosis rather than as a failed test.

`tests/common/project.rs` and `tests/common/built.rs` are the two A4 added, and they sit at the
two ends of the build. The first writes a `gleam.toml` in a temporary directory with a tree around
it, which is all the upward search, `--skip-export` and "not in a Gleam project" need; it never
writes Gleam source, because a project that has to *compile* is `tests/fixtures/hello_ffi`. The
second drives the real command: `BuiltProject` copies a fixture, runs this test run's own `ginary
build` in it under a deadline and lists the `.work-<pid>` directories the build did or did not
leave, and `ArtifactRun` runs what it produced under `env_clear()`, a `PATH` that is an empty
directory, and a `HOME` and `XDG_CACHE_HOME` inside the test's own tree — the same scrubbing
`tests/common/artifact.rs` applies, for the same reason: an artifact that only ran because the
developer had Erlang installed has proved nothing.

`tests/common/payload.rs` is what A3a added, and it builds no tree at all in the `FakeOtp` sense:
it writes tar headers byte by byte (`RawTar`), the smallest staging root the format tests need
(`staging_tree`), and the two instruments those tests read through, `CountingReader` and
`SharedSink`. The two policy sections below say why each exists. E8 added `recorded_mode(requested,
is_dir)`: the mode a staging fixture records for a file — the value asked for where the host has
permission bits, and `platform::modeless_mode` where it does not — so a fixture built on a Windows
host records the same `0o644`/`0o755` its filesystem and the `tar` header do, rather than a mode
the filesystem discarded. It was the fixture-side half of A1 (the `0o755` a no-op `set_mode` left
in the listing was what `ginary verify` reported five mismatches over), and `staging_tree` now
routes every file's mode through it.

`tests/common/native.rs` gained `host_object_target` in E8's Fix round 2: the target an object
built with `host_machine()` and `host_interp()` actually describes. It is a Linux ELF with a
glibc or musl `PT_INTERP` whatever machine wrote it, and a test that expected `Target::host()`
out of `native::scan_shipment` was reading the two as one value because on a Linux host they are.

`tests/common/stubfile.rs` is what C2 added, and it builds the two shapes of fixture the stub
half needs. `Marker` is the four fields of an identity marker held as *text*, so a test can write
a version, a target, a format or a flavor no parser would ever produce, and `Marker::bytes`
renders the whole 128-byte record with its NUL and its zero padding; `marker_from_body` goes one
level lower and takes bytes, for a body that is not UTF-8. `with_markers` plants any number of
those in `noise`, which is a seeded xorshift rather than anything random — a scanner bug that
turns on one byte in ten thousand has to be a failure that reproduces, not a flake — and asserts
its own bytes hold no needle, which is what makes it a negative fixture rather than an accident.
The needle itself is stored *masked* and unmasked at run time, for the same reason `stubid`
masks its own: a helper holding `GINARY-STUB-ID\0` contiguously would put a second marker into
every test binary that links it, and `tests/stubid.rs` would be scanning itself. Splitting it in
two halves — which is what both did until E10 — is not enough, because a linker may lay two
constants out side by side and then the file holds the needle after all; that is exactly what a
Windows `ginary.exe` was found doing. `fragments` exposes the stored images so the invariant is
checkable rather than argued: no two of them, in any order, may spell the needle. `stubid::scan`
counts whole *records* for the same reason, so fifteen bytes of unrelated data are not a second
identity.

`stub_copy` is the other shape: `stub::verify` reads a *file* and looks at its object header, so
its fixtures have to be real executables. It copies this test run's own `ginary` and rewrites the
marker in place — rather than fabricating an ELF — because the claim under test is what the gates
do with a header a linker actually wrote; `stub_copy_without_marker` zeroes it instead, and
`text_with_marker` writes a shell script carrying a perfectly good marker, which is what tells
the marker gates and the object gate apart. `pe_bytes` and `pe_with_marker` are the Windows
counterpart and are written by hand, the way `payload.rs` writes tar headers by hand: there is no
Windows toolchain here, and the only fields `check_object` reads out of a PE are the format and
the COFF machine. `cross_stub` is the gated lookup — `GINARY_STUB_DIR`, then `target/stubs` — for
a real cross-built stub, and its rule is `stubfile::choose_cross_stub`: a printed skip, unless
`GINARY_REQUIRE_STUBS=1` says the file was supposed to be there.

`tests/common/http.rs` is one of the two C3 added, and it exists because four of the claims about
`src/download.rs` are properties of a *server* and none of them can be written down as a file: a
body that hashes to the wrong digest, a 500 that becomes a 200 on the second ask, a 404 that must
*not* be asked again, and a connection that dies mid-body. `TestServer::start` takes a map of path
to a list of `Reply` values, answers them in order and then repeats the last one for ever, and
records every request — so a test asserts on *how many times* the client asked as readily as on
what it got back, which is the only way to state "a 4xx is asked exactly once". `Reply` has three
shapes: `Body` with a status and a `Content-Length` that matches, `Truncated` with a
`Content-Length` that promises more than is written before the close, and `Hangup`, which accepts
the connection and writes nothing. It binds `127.0.0.1:0` and reports the port it was given, so
any number of tests run in parallel without agreeing on anything, and `wait_for_requests` is
bounded by `WAIT_BUDGET` (10 s) so a stalled client is a failed assertion rather than a hung test
binary. It is hand-rolled rather than a dependency and it is the smallest server those claims
need: HTTP/1.1, `GET` only, one connection at a time, no chunking, no ranges, no keep-alive — and
**no read timeout**, so a client that connects and never sends a request line stalls the serving
thread until the test binary exits. Nothing ginary sends does that; a helper that grew one would
need one.

`tests/common/catalog.rs` is the other, and it builds the three fixtures the catalogue half needs.
`CatalogBuilder` assembles a `catalog.json` out of the schema types and serialises it with
`serde_json` — deliberately **not** through `Catalog::to_json`, because a test that wrote its
fixture with the writer it is checking would pass whatever the writer did; it is the same rule
`tests/common/payload.rs` follows when it writes tar headers by hand. `static_variant` and
`gnu_variant` are the two filled-in entries, taking the digest and the length from the caller
because those have to be the tarball's real ones for anything downstream to verify.
`runtime_tarball` packs a `FakeOtp` root as one zstd stream, which is what `ensure_otp` is handed;
the pipeline's own packing rules — path order, `mtime` 0, `uid`/`gid` 0 — are asserted against
`catalog::pack_runtime` itself in
`tests/regressions/c3_a_repacked_runtime_carried_a_non_zero_mtime.rs` rather than reproduced in
the fixture. `FakeUpstream` is what `ginary otp repack` reads: a `FakeOtp` tree wrapped in a
top-level directory and gzipped, which is the shape `gleam-community/erlang-linux-builds`
publishes, with `extras` for planting the fat a prune is supposed to strip. `plant_cached_otp` is
the other end — an extraction that already happened, `.meta.json` and all — so a test can assert
that a warm cache is used rather than re-fetched. The runtime inside every one of them is a
`FakeOtp`, whose `beam.smp` is a shell script, which is exactly the shape the *unseamed*
inspection refuses and is why the catalogue tests drive `resolve_in_with` and `repack_with` with
the ELF reader injected.

`tests/common/portability.rs` is what E6 added, and it is not a fixture builder at all: it is the
rule that the *test tree itself* has to compile on all three operating systems. `unix_sites` is a
pure function over one file's text — it strips comments and literals with a small lexer that
carries block comments and raw strings across lines, then tracks `cfg(unix)` gates through the
brace stack — and it answers, for every mention of `os::unix`, whether a gate covers it. The rule
it enforces is that every such mention sits under one: an inner attribute on a file that is wholly
about unix, an outer attribute on the item, or an attribute on an enclosing block, whichever fits.
`tests/regressions/e6_the_test_helpers_did_not_compile_on_windows.rs` asserts the scanner against
source it is handed and then turns it loose on every `.rs` file `git` tracks under `tests/`.

E7 added `unmet_needs` to the same file, which is not a scanner: it is the `DT_NEEDED` names
of an emulator that `ginary::verify::NEEDED_ALLOWLIST` does not admit, sorted and deduplicated.
It exists because the portability promise is about the *host's* Erlang and not about ginary,
so a test that asserted a real artifact verifies with no findings at all was asserting a
property of one machine's OTP build. The expectation is computed from the installation, which
makes the two sides of that assertion two different files.

E8's Fix round 2 added the rule that decides when a test may be scoped to one platform at all,
because the Windows runner made the difference matter. **A test may be scoped to a platform when
its subject only exists there; it may not be scoped to a platform to avoid a failure that is
about the product.** The first kind is a claim whose fixture the other platform cannot supply —
`tests/elf.rs`'s reads of `current_exe` and of the host OTP tree's `beam.smp` (only a Linux host
links an ELF and ships an ELF emulator), `tests/cli.rs`'s three `elf deps` claims about the
binary this run built, and `tests/native.rs`'s seven hook claims (`native::HOOK_SHELL` is
`/bin/sh` on every host by decision, so a host without a POSIX shell gets the documented
`NativeError::HookProcess` instead). Every one of those leaves an ungated test that holds the
contract on all three platforms — the not-an-ELF path, the format-blind half of `tests/elf.rs`,
and `tests/regressions/c4_the_hook_shell_was_cmd_on_a_windows_host.rs`. The second kind is what
`src/platform.rs` is for: a fact about an operating system, written once and asserted for every
`Os` on the machine ginary is developed on. `docs/dev/log/E8.md` §16 keeps the ledger of which
Windows failures are which.

A scan is a proxy, and a better check exists on any machine with docker. `mingw-w64` is all a
Linux host needs to type-check the whole tree for Windows, which is what the C sources of
`zstd-sys` had made look impossible:

```console
$ mise run check:windows
```

The image is `scripts/ci/wincheck.Dockerfile` — `rust:1-bookworm` plus `mingw-w64` plus
`rustup target add x86_64-pc-windows-gnu` — and the task builds it and runs the check inside it,
against a target directory of its own so a foreign libc's objects never land in `target/`:

```console
$ docker build -t ginary-wincheck:1 -f scripts/ci/wincheck.Dockerfile .
$ docker run --rm -v "$PWD":/w -w /w -e CARGO_TARGET_DIR=/tmp/t ginary-wincheck:1 \
    cargo check --all-targets --locked --keep-going --target x86_64-pc-windows-gnu
```

Forty-five seconds warm, and it catches what the scan cannot: a call to something *already*
gated — `cache::prepare` is `cfg(unix)`, and an ungated call site of it mentions no `os::unix`
for a scan to find. Run it before changing a shared test helper. The gnu triple and not the msvc
one, because `zstd-sys` compiles C and `mingw-w64` is the C compiler a Linux host can have.

`tests/common/native.rs` is what C4 added, and it is the fixture half of a milestone that has no
cross toolchain to build a real fixture with. Three kinds of object, by the same rule the earlier
helpers follow: `elf_bytes` writes a whole ELF64 by hand — the class, the endianness, `e_type`,
`e_machine` and, when one is asked for, a single `PT_INTERP` program header pointing at the
string behind it. That is the one shape no rewriting of a host binary can produce, and it is two
shapes rather than one: an object whose interpreter names *musl* on a machine with no musl
toolchain, and an object with no interpreter at all, which is what a musl NIF built `-static` is
and the case `reconcile` accepts with a note rather than a guess. It is also a hundred and fifty
bytes where a copy of this test run's own binary is thirteen megabytes on a disk the whole suite
shares; `repack::patch_elf_machine` keeps the other technique, because its claims are about a
file a linker actually wrote and these are about header fields. `pe_bytes` builds on
`stubfile::pe_bytes` and rewrites the COFF `Characteristics` field, because the only thing that
tells a `.dll` from a `.exe` is one bit and a test that could not set it could not tell a library
from a program. `macho_bytes` is written by hand, the way the PE helper is: eight fields, no load
commands, `ncmds` zero — a whole if empty object — and `macho_magic_only` is the four bytes that
are *not* one, which is the `Unknown` row and the warning beside it. `dos_stub` is the other
half of that pair for PE: the `MZ` magic with no `PE\0\0` signature where its DOS header points,
which is a file that begins like an object and is not one. `plant` and `plant_executable` write a
fixture into a tree the test owns.

One shape the file cannot fabricate is a position-independent *program*, because `DF_1_PIE` lives
in a `DT_FLAGS_1` entry of a real dynamic section. The C4 tests that need one used
`repack::test_binary` — this test run's own binary, which `cargo` links `-pie`. E9 turned that
same accessor into the committed ELF fixture (see `tests/common/repack.rs` above), because a real
ELF the *host* refuses is no ELF at all on Windows or macOS; `common::native::real_elf_bytes` /
`real_elf_path` are the accessors for `tests/fixtures/elf/inet_gethost-x86_64-linux-gnu`, the
committed `x86_64` Linux ELF that reads as one whatever host opens it. It parallels
`tests/fixtures/macho/` exactly: a real, unmodified binary, committed rather than downloaded at
test time, for the tests that must plant an object a real linker wrote rather than one this module
fabricated.

`tests/common/macho.rs` is what D3 added, and — like `tests/common/native.rs` before it — it has
no macOS toolchain to build a real fixture with, so almost everything in it is written field by
field: `thin_header` is the eight-field 64-bit header alone, `ncmds` zero, the shape `macho::read`
and `sign_macos::inject_and_sign` both have to accept as a whole (if empty) object rather than a
truncated one; `fat_header` writes the big-endian `FAT_MAGIC` layout naming any number of
architectures with no thin data behind their offsets, since `macho::read` refuses a fat binary
before it would need to follow one. `with_section` is the one that matters most: a thin Mach-O
carrying one segment and one section, laid out so the load commands end exactly where the
section's own file offset begins — the property a real linker's output has, and a test fixture
has to have too, for the geometry claims under test to mean anything — with an
`LC_CODE_SIGNATURE` load command over a trailing blob added when asked, mirroring where
`__LINKEDIT` sits in a real binary. `with_payload_section` and `payload_section_body` build on it
for the one section shape `src/payload.rs::locate` and `src/sign_macos.rs` are written against
ahead of their own implementation: the 64-byte trailer struct at the section's own start,
`payload_offset` fixed at `TRAILER_LEN` because the payload immediately follows it, and nothing
else in the section — see "Payload section geometry" below for why that fixed layout, and not
just the equation `Trailer::parse` checks, is what `locate` itself enforces. E9 adds
`entry_point`, a by-hand reader of `LC_MAIN` plus `__TEXT` that resolves the file offset a
Mach-O's entry point maps to and returns the bytes there, so a test can assert the finished
artifact's mapped entry still holds the stub's own first instructions — the invariant the
segfaulting section layout broke. The other half of the file is `tests/fixtures/macho/`: a real,
unmodified `aarch64-apple-darwin` binary (`tests/fixtures/macho/README.md` records its origin and
licence), committed rather than downloaded at test time, for the tests — `inject_and_sign`'s
among them — that have to hold against load commands and segment geometry a real linker wrote
rather than one this module fabricated.

`tests/common/codesign.rs` is what E8 added, and it is the reading half of the ad-hoc signature
`src/sign_macos.rs` writes — the counterpart to `macho.rs`, and, crucially, one that goes nowhere
near `src/sign_macos.rs`, so a test written against it checks the signer rather than restating
it. It walks the load commands to find `LC_CODE_SIGNATURE`, parses the `CSMAGIC_EMBEDDED_SIGNATURE`
superblob and then the `CodeDirectory` field by field from Apple's `cs_blobs.h` layout (`version`,
`flags`, `codeLimit`, `hashSize`/`hashType`/`pageSize`, `execSegBase`/`execSegLimit`, the code
slots), and recomputes the SHA-256 of every 4096-byte page of the file below the signature with
`sha2` — the value a kernel computes for itself as it faults each page in. `first_bad_slot` is the
whole point: it returns the first slot whose stored hash is not the page's own (and the count
disagreement first, so a directory claiming more slots than the file has pages is not read as
agreement), which is the state that gets a Mach-O `SIGKILL`ed before `main`. `segments` and
`segment` expose the load map for the page-alignment checks. It is not `cli`-gated, for the reason
`macho.rs` is not.

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

D2 added a fifth thing, and it keeps the rule above rather than stepping outside it:
`FakeOtp::windows()` writes an `erts-<vsn>/bin` holding `erl.exe`, `beam.smp.dll`,
`inet_gethost.exe` and the `erl.ini` beside them — the three names
`assemble::WINDOWS_REQUIRED_BINS` requires and the file assembly deletes — plus whatever
`extra_erts_bins` named. Everything else about the root is the unix builder's, because
everything else about a Windows runtime is the same.

Every `.exe` and `.dll` it writes is a real, if minimal, PE image: a DOS header whose
`e_lfanew` points at the PE signature, a COFF header naming the machine, and a PE32+ optional
header of the size that header declares. Nothing in one is executable and nothing needs to be —
this machine could not run a PE anyway — but the machine field is real, because it is the one
field a Windows runtime is read for. `FakeOtp::pe_machine` sets it, so a test about a runtime
for the wrong architecture changes that number and nothing else. `erl.ini` stays text, which is
what it is.

D3's Mach-O work needed the third flavour, and E7 added it on exactly the same terms.
`FakeOtp::macos()` writes the unix tree's names — nothing else about a macOS runtime differs —
with `beam.smp` written as a real thin 64-bit Mach-O rather than as a shell stub. As with the PE
images, nothing in it is executable and nothing needs to be; the `cputype` is real, because that
is the one field `erts_source::resolve` reads off a macOS runtime, and `FakeOtp::macho_cpu_type`
sets it and changes nothing else — the exact counterpart of `FakeOtp::pe_machine`.

So `otp::inspect_root` **accepts** a `FakeOtp::new().windows()` root, and the "no API for an
invalid tree" rule holds for both flavours. It reads the flavour off the tree —
`assemble::is_windows_erts_bin`, "does `erts-<vsn>/bin` hold `erl.exe`?" — and measures a
Windows tree against `assemble::WINDOWS_REQUIRED_BINS` with no execute bit asked for, since a
zip unpacked on this machine carries whatever the unzipper chose. `erts_source::resolve` reads
the PE header of that tree's `beam.smp.dll` and answers `windows-x86_64`. A test that needs an
incomplete tree removes a file from a whole one, which is the rule this section already states;
the older Windows staging tests still hand `assemble::stage` an `OtpInfo` built by hand, which
is four paths and two versions and every one of them is what the builder wrote.

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
is `GINARY_TEST_SHIPMENT` and nothing else: unset is a reported skip, and a value that is not a
directory is a failure. It used to fall back to a path on the author's machine, which read as a
default and was one machine's truth; the first live CI run failed the `test` and `coverage` jobs on
it, and `tests/regressions/e5_a_gated_test_defaulted_to_one_developers_machine.rs` now holds both
halves of the rule.

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

### `tests/fixtures/config/`

Nine whole `gleam.toml` files, one per rule, read by `tests/config.rs` through
`ProjectConfig::from_toml`, which takes the text and a path — so none of them has to be a real
project on disk. Two are valid and seven are invalid on purpose, and
`tests/fixtures/config/README.md` records which is which. The invalid seven each have a test
asserting the exact error variant, so editing one means editing its assertion with it.

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

**A name check that compares the whole path is a name check with a hole in it.** The first
version of that fix matched `ginary.json` exactly, and a *directory* of that name walked through
both ends: `pack` emitted `ginary.json/nested.txt`, and `unpack` created `<dest>/ginary.json` as
a directory on the way to the nested file, so the manifest's `create_new` failed on it with the
same unattributed `AlreadyExists` the reservation existed to end.
`tests/regressions/a3b_a_reserved_name_covered_only_the_exact_path.rs` pins the first-component
rule at both ends, including the `./` shape, and asserts the destination holds nothing but the
one front-matter entry that is legitimately unpacked.

## The repository as its own fixture

`tests/ci_matrix.rs`, `tests/repo_hardening.rs`, `tests/v1_readiness.rs`, `tests/formal.rs`,
`tests/smoke_matrix.rs` and `tests/deps.rs` have no fixture: the repository is the fixture. All
six read committed paths through `tests/common/repo.rs`, which is the one place that resolution
lives — `tests/deps.rs` adds `tests/common/deps.rs`, its own feature-free reader for
`Cargo.toml` and `Cargo.lock`:

- `root()` — the directory holding `Cargo.toml`, from `CARGO_MANIFEST_DIR`, so a test finds the
  same file whatever directory the run started in.
- `read(path)` — the file as text, panicking with the path when it is not there. For these
  targets that *is* the assertion: a workflow or a document the milestone promised and did not
  write is a failed test, named by where it was looked for.
- `read_opt(path)` / `exists(path)` — for a test that wants to make its own message, or that
  asserts a file's absence.
- `read_or_missing(path)` — the text, or the one-line marker `(missing <path>)`. Use it in a
  snapshot test rather than `read`: a panic reports only the path, while the marker makes the
  failure a diff between the record the milestone promised and the empty tree, so one run shows
  both the path and the whole expected content.
- `parse_yaml(text)` / `yaml(path)` — the document as YAML, through `saphyr`. GitHub loads the
  issue forms, `dependabot.yml` and every workflow with a YAML reader, and a substring assertion
  is just as happy with a file no reader will accept; parsing first makes that a test failure.
  `tests/regressions/e3_an_issue_form_was_not_valid_yaml.rs` is the bug that bought this helper,
  and it holds every `.github` record to it through `yaml_files_under(".github")`.
- `workflow_steps(path)` — every step of every job of one workflow, in file order, as
  `WorkflowStep { workflow, job, position, name, run, env }`: the job id it belongs to, its
  1-based position within that job, its `name:` (or its `uses:`), its `run:` script, and the
  job's `env:` overlaid with the step's own. `step.commands()` is that script as one command per
  line with backslash continuations joined, because a command wrapped for width is still one
  command and a cosmetic reflow must not change what a rule asserts. E5 bought it: three of that
  milestone's findings are about *order* and *environment* within a job — which build last wrote
  `target/release/ginary`, which target directory a second `cross` invocation reuses, which job a
  step lives in and therefore which `if:` decides whether it runs — and a substring search over
  the file text cannot answer any of them.
- `rust_toolchain_sites()` — every `dtolnay/rust-toolchain` step under `.github/`, as
  `ToolchainSite { workflow, job, toolchain }`, read out of the parsed workflow rather than
  grepped: the word `toolchain` also appears in comments, in `GINARY_REQUIRE_TOOLCHAIN` and in
  a job name, so a grep would answer a question nobody asked. E4 bought it, because every job
  had quietly pinned the MSRV and CI had therefore never once built this crate on stable.
- `workflow_jobs(path)` — every job of one workflow as
  `WorkflowJob { workflow, id, needs, env, commands, uses }`, with `runs(needle)` and
  `uses_action(needle)` over the last two. `workflow_steps` merges the job's `env:` into each
  step, which answers "what does this command run under"; two questions are about the job
  itself — what it `needs:`, and whether *the job* declares a variable — and neither survives
  that flattening. E6 bought it: the rule that a job may set `GINARY_REQUIRE_STUBS` exactly when
  it obtains the stubs is a statement about jobs.
- `parse_ginary_command(line)` / `ginary_invocations(path)` — one shell command line, and every
  ginary invocation in one committed file, as
  `GinaryInvocation { source, site, line, path, long_flags }`. `path` is the subcommand path
  (`["otp", "repack"]`), `long_flags` is every `--flag` it passes without its value. A `.yml` is
  read as a workflow step by step and anything else as a shell script line by line, so
  `scripts/smoke-matrix.sh` is scanned as well as the step that calls it. What the parser does
  *not* cover, deliberately: short flags are counted only as "a flag was seen", a flag's value
  is never read, and a program the scan cannot name — an interpolation other than a
  `GINARY_*BIN` variable — is not an invocation. `tests/regressions/e6_the_macos_job_passed_a_flag_the_cli_does_not_have.rs`
  holds every long flag found against the binary's own `--help`.
- `yaml_files_under(dir)` / `shell_scripts_under(dir)` — every `.yml`/`.yaml`, and every `.sh`,
  under a directory, recursively and sorted, so a failure names the same file on every machine.

`tests/common/digest.rs` is the other helper E4 added, and it is not a repository reader at all
— it is the fixture half of `tests/digest.rs`. It holds the three published SHA-256 vectors (the
empty input, `abc`, and one mebibyte of `index % 251`), the pattern generator behind the third,
and `vector_listing()`, which writes them into a directory and returns the staging listing over
them. The vectors go through `ginary::manifest::Index::from_staged` — the crate's own hashing
call site, reached through its own public API — rather than through `sha2` called from the test,
which would only prove `sha2` is `sha2`. The mebibyte matters for the same reason: every digest
in the format is computed incrementally, and an input smaller than one 64 KiB buffer never
reaches the second `update`. What makes the constants evidence rather than a recording is the
*order* they were taken in: each was recorded against sha2 0.10.9, before the 0.11 bump, and
checked against `sha256sum` by hand, so the suite reading them on the far side of the migration
is the proof that not one byte of any digest moved. A file written after a library swap records
whatever the new library produces and demonstrates nothing.

The same rule applies to all of them: assert on what the file *says*, not on where its lines
happen to wrap. `flowed()` in `tests/repo_hardening.rs` collapses whitespace before a prose
assertion, and the parsed helpers above are the equivalent for a record with structure.

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
not mention the directory, and the gates stay on stable. Nightly is installed by exactly one CI
job, `fuzz` in `nightly.yml`, and a numbered release by exactly one, `msrv` in `ci.yml`, which
proves the floor `rust-version` names and does nothing else.

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
| `bundle__build_report_targets_table.snap` | the six-column table a build of more than one target prints instead of one `artifact:` line |
| `doctor__doctor_targets_table.snap` | the targets table `doctor` prints, with one resolvable row and one that says which milestone it arrives with |
| `doctor__doctor_project_native_table.snap` | the project's native table with one column per configured target, one verdict of each kind |
| `native__native_mismatch_message.snap` | the refusal a cross build over foreign native code prints: the table, and one `fix:` line per row naming both `gleam.toml` keys and the flag |
| `native__native_mismatch_warning.snap` | the same rows, as the warning `--allow-native-mismatch` earns instead |
| `native__native_static_runtime_message.snap` | the refusal a static runtime earns for a shared object it could never load, and the setting that fixes it |

A snapshot is a contract, not a recording. `cargo insta review` is for reviewing a *deliberate*
change to output; accepting a snapshot to make a red test pass is the same defect as weakening an
assertion.

## Planned infrastructure

`tests/common/` already holds `tools.rs`, `fake_otp.rs`, `snapshot.rs`, `script.rs`,
`fixture.rs`, `erl.rs`, `bounded.rs`, `payload.rs`, `artifact.rs`, `built.rs`, `project.rs`,
`cachefs.rs`, `repack.rs`, `stubfile.rs`, `http.rs`, `catalog.rs`, `native.rs`, `macho.rs`,
`coverage.rs`, `repo.rs`, `deps.rs`, `digest.rs`, `shipment.rs`, `portability.rs`, `homepath.rs`
and `srcscan.rs`, described above. The last two are E7's, and both are pure scanners meant to be
reused: `homepath.rs` finds a person's absolute home path — `/home/<name>` or `/Users/<name>` —
in a file that has to run anywhere, reading each file in its own comment syntax (`//` opens one
in Rust and `#` opens an attribute; `#` opens one in YAML, TOML and shell; anything else is read
as code throughout) and exempting the fictional accounts this suite's own unit tests spell.
`srcscan.rs` holds the two scanners for defects only another platform can see: `calls_with`,
which finds a call to a named function that names the host in its arguments, and
`literal_sites`, which finds a literal in code rather than in prose. Still to come:

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
- **Fault injection** — `src/fault.rs`, behind the `fault-injection` Cargo feature. It is off by
  default, so a release artifact holds none of the points and never reads `GINARY_FAULT`;
  `mise run test`, `mise run test:fast`, `mise run test:nextest` and the CI test job all pass
  `--features fault-injection`. The points are `after-extract:pause` (sleep with the temporary
  tree on disk, so a test can `SIGKILL` the process and assert the next run sweeps it),
  `rename:eexist` (the losing side of the extraction race), `unpack:corrupt` (a payload that
  changes under the reader), `before-lock` (the cache entry is removed between the preflight and
  the shared lock, which is what a prune that won the race leaves behind), `launcher:panic` (a
  panic on the launcher path) and `pack:fail` (the *builder* stops between the stub and the
  payload). The first four are about *timing*, which is why no artifact a test can build reaches
  them, and each is paired with an assertion that the **next** run recovers: a fault that is only
  shown to fail is half a test. `launcher:panic` is about a promise: `main` installs a panic hook
  so that a bug in ginary is one attributed line and exit 121 rather than a Rust backtrace, and a
  hook nothing can trigger is a hook no test can check. `pack:fail` is the one point on the build
  side, and it is there so that a test can assert that a failed build leaves neither a work
  directory nor a half-written artifact. `FAULT_POINTS` in `src/fault.rs` is the list both this
  document and `debugging.md` are held against by unit test, so the three cannot drift apart.
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
