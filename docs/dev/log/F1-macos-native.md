<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# F1 — the suite on a macOS host, and what it found there

This record is the counterpart of [F1.md](F1.md), which was worked on a Windows host, and of
[F1-host-tool-gates.md](F1-host-tool-gates.md), which fixed the first tool gate that assumed one.
F1 said plainly that native macOS execution had not been qualified locally; this session ran the
suite on a Mac for the first time and this is what that cost and what it bought.

No release, tag, publish, attestation or version change was made. One pull request was merged
(#13, a green Dependabot SHA pin) and nothing else was pushed.

## The host

macOS 26 on `aarch64-apple-darwin`, Rust 1.98.1, Erlang 29.0.5 and Gleam 1.18.1 — the same
versions `erlef/setup-beam` pins in `.github/workflows/ci.yml` — plus Docker, `codesign`,
`otool`, `shellcheck`, `actionlint`, `cargo-nextest`, `cargo-mutants`, `cargo-llvm-cov`,
`cargo-deny` and a Temurin 25 JDK.

One measurement caveat belongs at the top rather than in a footnote: for most of this session the
machine was running another repository's mutation campaign, and its load average sat near 27 on
18 cores. Every wall-clock number below was taken under that load, and one class of failure —
recorded under "not a defect" — is a consequence of it.

## The baseline

`cargo fmt --all -- --check` and all three clippy flavors passed unchanged. The suite did not:

```console
$ GINARY_REQUIRE_TOOLCHAIN=1 cargo test --features fault-injection --no-fail-fast
… 2106 passed; 226 failed
```

226 failures across 22 of 48 targets. They were not 226 problems. One production defect accounted
for most of them.

## The defect that made ginary unusable on macOS

```console
$ ./target/debug/ginary version
ginary: the Mach-O `__GINARY,__payload` section could not be read: cannot parse the Mach-O file:
Invalid Mach-O symbol table offset or size
$ echo $?
122
```

Every command of a debug build exited 122 — `version`, `doctor`, `build`, all of them. `main()`
decides mode by reading the running executable, and on macOS that question goes to
`payload::locate`, which reads at most `MACHO_HEAD_CAP` (16 MiB) of the front of the file because
"a Mach-O header and every one of its load commands sit at the front". The premise is right and
the implementation did not keep to it: `locate` handed the capped head to `macho::read`, which
hands it to `object::File::parse`, and `object` validates more than the load commands — it
resolves `LC_SYMTAB`, whose `symoff` points into `__LINKEDIT` at the *end* of the file. A debug
build of ginary for this target is 32 MB, so the head stopped before `__LINKEDIT` and a file that
simply had no payload was reported as a broken artifact.

`macho::code_signature` was already written the right way and says so in its own comment — it
walks the load commands by hand and "never needs a byte past the last load command it reads".
`macho::section` claimed the same rationale in its documentation and then delegated to `read`.
The documentation was right and the implementation was not.

The fix is that `section` walks the load commands the way `code_signature` does, and `locate`
reads fat-versus-thin off the four magic bytes it has already matched instead of parsing the
object to ask. `tests/regressions/f1_reading_a_macho_head_needed_the_whole_file.rs` is the
regression: the committed `aarch64-apple-darwin` fixture truncated at its own `__LINKEDIT` offset
is the same file in 64 KB instead of 32 MB and produces the identical `object` error for the
identical reason. It failed on two of its three claims before the change and passes on all three
after, with the third — `code_signature` answering the same from head and whole file — as the
control that the fix did not move what already worked.

The release build is under the cap, which is why the `macos` CI job, which builds `--release`,
never saw this. It is also why `tests/smoke_cli.rs` had never once run green on a Mac.

## The defect in the first diagnostic a user reaches for

`doctor`'s native-code table answered its `matches_host` column with `info.machine == host`,
where `host` is `Target::host().arch`. The scan lists ELF and nothing else, so on an `aarch64`
Mac an `aarch64` *Linux* shared object agreed about the CPU and was reported as matching a
machine that cannot load it by any means. That is the defect F1 fixed for `ginary verify` —
"checks object format against OS as well as CPU" — left standing in the table `ginary doctor`
prints. The column now asks both halves;
`tests/regressions/f1_the_native_table_matched_an_elf_to_a_host_that_cannot_load_one.rs` pins it,
and the claim it makes is true on every host rather than on Linux: an ELF matches if and only if
this host's own objects are ELF and the CPU agrees.

## Seven test defects, none of them a production defect

Each of these is a test that could only ever have passed on the host it was written on. They are
listed rather than absorbed, because "the suite went green" is worth nothing if the reason is
that the claims were loosened.

- **Paths compared as text on a host whose temporary directory is behind a symlink.** macOS puts
  `TMPDIR` under `/var/folders/…` and `/var` is a link to `/private/var`. `ginary::assemble`
  canonicalises the application root once and walks from there, so every path it reports is
  canonical and every path a test joined onto `TempDir::path` is not. Six sites in five files.
  `tests/common/hostpath.rs::resolved` is the rule they now share: it resolves the directories
  *above* the last component and carries the last one, because two of these tests plant a
  dangling link and one plants a link that escapes — resolving the leaf would follow the very
  link under test.
- **`flock(1)` required of a host that cannot have it.** Eleven lock tests take the other side of
  the exclusion with util-linux `flock(1)`, which is the right design — a lock proved with the
  code that takes it proves nothing — and `GINARY_REQUIRE_TOOLCHAIN` escalated its absence into a
  panic on a Mac. That variable is a claim about the toolchain an artifact is built with, and no
  macOS job can install this program. `tools::require_flock` now escalates on Linux and reports a
  printed skip elsewhere, exactly as `require_elf_stripper` does for a host whose objects are not
  ELF. The other half of the same contract — that a shared lock survives `execve` — *is* proved
  on macOS, by `tests/launcher.rs`.
- **A file name that is not UTF-8, on a filesystem that refuses one.** APFS rejects
  `caf\xe9.dat` outright (`EILSEQ`), so two tests failed in their own setup rather than in their
  claim. `tools::filesystem_holds_non_utf8_names` asks by doing it, in the caller's own
  directory, and reports a skip when the answer is no.
- **A unit test that asked the host what platform it was.** `selfexe::tests::open_self_opens_the_test_binary`
  asserted the ELF magic under `#[cfg(unix)]`, which is true on macOS. It is the fourteenth of
  the thirteen tests `platform::object_format` was written for, and it only ever ran on Linux. It
  now asserts this host's own object magic.
- **A fabricated cross stub built from the running binary.** `c2_a_target_sub_table_with_no_erts_passed_the_guard`
  rewrote `e_machine` in a copy of this test run's own `ginary`, which is an ELF only where the
  host is Linux; on macOS it wrote a foreign machine into the middle of a Mach-O header and
  `stub::verify` refused the result before the guard under test was ever reached. It uses the
  committed dynamic-gnu ELF fixture now, which is what `common::repack::test_binary` already
  carries in its own documentation for the same reason.
- **A corruption offset measured from the end of the file.** `a_flipped_payload_byte_fails_verify_and_still_prints_the_manifest`
  damaged a byte at `len - TRAILER_LEN - 16`, which is inside the payload on Linux and Windows and
  inside the *code signature* on macOS, where the payload sits in `__LINKEDIT` with the signature
  after it. Verification then found nothing wrong and the test failed for the right reason about
  the wrong byte. The offset comes from `payload::locate` now, which is where the payload is on
  every platform.
- **"The artifact begins with the stub" is one layout's spelling of it.**
  `c2_the_artifact_never_had_to_use_the_stub` asserts that a build given `--stub` used *that
  file*, by comparing the artifact's leading bytes against the stub's. It had never run on a Mac,
  because it is gated on `target/stubs` holding a stub for the host and `mise run stubs:build`
  produced no darwin one — until this session made it. With one there, it failed: "the build used
  some other file", about a build that used precisely the file it was given. A macOS artifact
  carries its payload inside `__LINKEDIT` and is then ad-hoc signed, so its load commands hold a
  grown `__LINKEDIT` and an `LC_CODE_SIGNATURE` the stub had other values for. What is
  byte-for-byte identical is everything between the end of the load commands and `__LINKEDIT` —
  which is exactly the region E9's writer is defined to move nothing in, and
  `common::macho::content_before_linkedit` is now the rule that reads it. This is a test that
  *only a new capability could reach*: making the darwin stub buildable here is what turned a
  permanent skip into a real claim.
- **A darwin target that is this host.** `a_macos_build_with_no_darwin_stub_gets_the_honest_stub_search_error`
  named `macos-aarch64`, which on an `aarch64` Mac is the host target: `stub::locate` answers it
  with the running binary and the build reaches the export. It asks about whichever darwin target
  is *not* this host.

## One script that only ran on Linux

`scripts/ci/publish-distribution.sh` compared its uploaded and re-downloaded asset lists with
`find … -printf '%f\n'`, which is GNU find's and no other's, so the rehearsal test that runs that
script could not run off Linux (`find: -printf: unknown primary or operator`). The `./` prefix is
removed with `sed` now — every POSIX host has it, and the list is the same. ShellCheck passes and
the rehearsal runs here.

## What a Mac proved that no machine here had proved before

- **`mise run stubs:build` builds the darwin stubs.** The task carried a Linux-host comment
  saying macOS stubs "come from the CI release build on a macOS runner and from nowhere else".
  On a Mac they are not cross builds at all. The task now builds both natively, and reports a
  missing `rustup` target as a named skip rather than as an absence:
  `ginary-stub-0.1.0-macos-aarch64` is 969,168 bytes and `ginary-stub-0.1.0-macos-x86_64` is
  1,085,848 bytes.
- **`mise run smoke:macos` packages, runs and checks a real artifact.** The task runs the
  committed `scripts/ci/macos-smoke.sh` — the same file the `macos` CI job runs, not a copy —
  with this host's target. It passed:

  ```json
  {"status":"successful","complete":true,"exit_code":0,"command_exit_code":0,"log_exit_code":0}
  {"expected":3,"observed":3}
  ```

  `codesign --verify --strict` reported the artifact "valid on disk" and "satisfies its
  Designated Requirement" **before and after** the launch — the second check being the one that
  proves extracting a payload did not rewrite the file it came out of — and the packaged
  application printed its arguments, `hello from priv` and the caller's working directory, and
  propagated exit 0 and exit 3. Evidence is under `target/assurance/macos-aarch64/`.

## The documents that had stopped being true

`docs/dev/v1-readiness.md` was last touched at `2e19b9d` (E23) and F1 did not move it, so the
fail-closed checklist went on owing evidence it had already produced: the macOS launch was
`CI-gated` in the Phase D table and first in the deferred list, while
[F1-integration.md](F1-integration.md) and run
[34281075949](https://github.com/P4suta/ginary/actions/runs/34281075949) recorded both native
macOS jobs building, launching and verifying real artifacts. The sweep now carries a Phase F
table, the real coverage figures (90.75% lines, 83.80% branches), the real nightly fuzz budget
(600 seconds, not 30), and a deferred list that no longer claims a remote does not exist.

E23's regression forbids a deferred bullet that *says* it is no longer deferred. What it could
not see is the case that actually happened, where nobody edits the bullet at all.
`tests/regressions/f1_the_deferred_list_outlived_its_own_evidence.rs` closes that: it crosses the
deferred bullets with the evidence tables and reports any bullet whose subject a table already
marks done. It is calibrated on
`tests/fixtures/readiness/settled_item_in_the_deferred_list.md`, which is the sweep as it stood
on 2026-09-13 — one offender and three near misses that share words with a finished row and must
not be reported. The bullet parsing both rules use moved to `tests/common/readiness.rs` so there
is one implementation of it.

Two smaller ones. `docs/adr/0016` ended on an open question — whether what the writer produces
maps, runs and verifies on a real Mac — and now carries the entry that answers it, without
rewriting the dated records that asked it. `docs/dev/testing.md` listed as "still to come" an
`Artifact` helper that is `tests/common/built.rs`, and as "planned" the determinism, concurrency
and property tests that all exist; the per-phase trace time bounds really do not exist, and that
one says so and names the milestone it waits on.

## `toml` 1.x, and a comment that described a deferral already taken

`Cargo.toml` said the reader was "held at 0.9 deliberately" and that taking 1.0 "wants a change
of its own that re-reads the config tests against the new behaviour". Dependabot `6ffb2be` took
it to 1.1.4 and changed `Cargo.toml` and `Cargo.lock` and nothing else, so the comment went on
describing a deferral that had already happened and `docs/dev/log/E4.md` went on listing it as
pending.

The re-read is done. What a 1.x reader gives a user is now pinned rather than assumed:
`an_unknown_key_in_the_tools_table_names_the_key_and_the_file` asserts the line the key is on and
that the message lists the keys the table does accept, and
`a_manifest_that_is_not_toml_names_the_file_and_where_it_stops_parsing` asserts the line, the
column and what was expected there. Those positions come from the dependency, so a future major
that drops them fails a test instead of quietly degrading the error.

## `doc:stub`

E23 recorded, out of subject, that `mise run doc` never documents the stub flavor: `cargo doc
--no-default-features` fails, so a `missing_docs` or an invalid tag in code the `cli` feature
gates *out* was checked by nothing at all. It fails on 73 unresolved intra-doc links, and all 73
are the crate-level module history linking to modules this build does not contain. `doc:stub`
denies every rustdoc warning and allows that one lint, with the argument written down: the stub
build's items are a subset of the full build's, so every link in code that compiles there is also
checked by `doc`. The task is in `mise run check` and the step is in the `lint` CI job.

## The mutation-plan tool on syn 3

Dependabot #15 bumps `syn` from 2.0.119 to 3.0.4 in `tools/mutation-plan` and breaks it: syn 3
renamed `BareFnArg`/`BareVariadic` to `NamedArg`/`FnPtrVariadic` and moved a match arm's guard out
of `Arm` and into the pattern as `Pat::Guard(PatGuard)`. Seven compile errors, and the lint and
MSRV jobs fail on the pull request.

The port is small — two visitor names and one method — but the thing that has to be true is not
that it compiles. The planner maps source spans to `cfg` scopes so that each mutation candidate is
assigned to a runner that can compile it, and a plan whose sites moved would stop matching the
campaign it plans. So the acceptance evidence is the plan itself. For each of the seven campaign
modules a `cargo mutants --list --json --diff` array was enriched by a planner built against syn 2
and by one built against syn 3, and the outputs compared:

| module | candidates | plans |
|---|---|---|
| `trailer` | 27 | identical |
| `payload` | 121 | identical |
| `closure` | 98 | identical |
| `appfile` | 207 | identical |
| `launch` | 104 | identical |
| `verify` | 121 | identical |
| `cache` | 282 | identical |

960 candidates, byte-identical on both sides. The tool's own 18 tests pass and clippy is clean
over it with warnings denied.

## The assurance tasks, on this host

- **`mise run formal`** — passed. TLC generated 31,939 states, found 7,860 distinct ones and
  searched to depth 29, checking four temporal properties with no error. Those are the same
  numbers [F1-integration.md](F1-integration.md) recorded from the hosted `formal` job, so the
  model checks identically on a Mac.
- **`mise run smoke:macos`** — passed; see above.
- **`mise run smoke`** — not run. The Docker daemon, reachable at the start of this session, was
  unreachable by the time the task ran, and `scripts/smoke.sh` reported the skip rather than
  taking it silently. The Linux clean-room claim is unmeasured on this host and is not claimed.

  **Since, with the daemon up:** it does not skip, it *fails* — `3 of 3 checks failed`, every one
  of them `Exec format error`. The task builds for the host and runs the result in `ubuntu:24.04`,
  so on a Mac the artifact is a Mach-O and no check could have run. A clean-room claim that was
  never tested, reported as a clean-room claim that failed. It asks the artifact's own first four
  bytes now — what the container needs is an ELF, and nothing else about the host decides that —
  and says so:

  ```text
  skipping: the artifact is not an ELF (cffaedfe), so ubuntu:24.04 cannot execute it
            the macOS clean room is `mise run smoke:macos`
  ```

  `GINARY_REQUIRE_TOOLCHAIN=1` turns it into a failure, which is what the Linux CI job sets. The
  Linux clean-room claim is still unmeasured here; the difference is that the task now says that
  rather than the opposite.
- **`mise run mutants`** — started, not finished, and at this load it cannot be: one mutant of
  `maintenance_owns` took about five minutes. What it did report is recorded in the next section.

  **Since:** the whole campaign is still not finishable here, and it does not have to be. The
  question a local run has to answer is "did this change resurrect anything?", and
  `cargo mutants --in-diff` answers exactly that over the mutants in the changed lines. Thirty of
  them, against the branch that closed the campaign's last fourteen: **28 caught, 2 unviable, 0
  missed, 0 timeout**, in 21 minutes.

  Two things had to be taken off `PATH` first, and both are recorded above rather than worked
  around: `erl` and `gleam`, so the end-to-end tests skip instead of failing on first-exec
  assessment, and `f1_distribution_executed_a_tag_in_the_default_branch_context`, which this
  machine's `git` policy wrapper refuses. cargo-mutants requires a green baseline, and a baseline
  green for the wrong reason is worse than a red one.
- **`mise run cov`** and **`mise run fuzz`** — not run. Coverage is a full instrumented suite run
  and fuzzing needs a nightly toolchain this host does not have; neither is claimed here.

  **Since, both run.**

  Coverage, with `--no-fail-fast` because the default stops at the first failing target and
  reports the coverage of everything that did not run: **89.81% lines, 85.57% functions, 90.59%
  regions**, against a gate of 90% lines. Five end-to-end tests did not finish, every one of them
  the `incomplete process observation` shape this record already accounts for, so the assertions
  after their failure point never ran and the lines those reach are uncovered. The number is a
  floor for this host rather than a measurement of the tree, and the tree's own figure is the one
  CI reports.

  Fuzzing, with the nightly toolchain this host has since acquired: four targets, 601 seconds
  each, **30,034,873 / 11,487,024 / 24,241,642 / 11,039,175 runs**, no crash and nothing in
  `fuzz/artifacts/`.

  It found one real defect, and not in the fuzzers. Building them updated `fuzz/Cargo.lock`, which
  turned out to be **stale on `main`**: #21 bumped `zstd` to 0.14 in the root lockfile and left the
  fuzz workspace's own at 0.13.3, where `cargo metadata --locked` refuses to resolve. Nothing
  caught it — dependabot watches the two ecosystems separately, and the nightly `fuzz` job was the
  only thing that had ever read that lockfile, so the failure would have arrived in a different
  workflow hours after the pull request that caused it. The `lint` job resolves it now.

## Code scanning, triaged and not dismissed

Ten alerts are open. None was dismissed, because dismissing one is a decision about this
repository's security posture rather than a code change, and because two of them are questions
rather than findings. The triage:

| # | rule | where | reading |
|---|---|---|---|
| 15 | `rust/cleartext-logging` | `tests/regressions/f1_trace_could_not_be_shared_or_correlated.rs:47` | False positive, and pointing the wrong way: this is the *redaction* test. It asserts the trace does **not** contain `secret-argument-123` and friends, and prints the needle it searched for when the assertion fails. The literals are fixtures. |
| 9 | `rust/log-injection` | `tests/regressions/e13_…:151` | False positive. An assertion message quoting a reply the test's own fixture server wrote. Not a log, and not a peer's data reaching one. |
| 6 | `rust/cleartext-logging` | `src/sbom.rs:497` | False positive. A `#[test]` asserting the RFC 4122 variant nibble of a UUID derived from a content digest, printing the UUID when it does not hold. |
| 8 | Scorecard `PinnedDependencies` | `scripts/ci/wincheck.Dockerfile:15` | Real, and the fix is not obviously an improvement. `FROM rust:1-bookworm` is a *developer* image (`mise run check:windows`) and nothing in CI builds it. Pinning it by digest with no updater watching that digest trades "gets patches" for "never changes", and `.github/dependabot.yml` has no `docker` ecosystem entry. Pinning **and** adding that entry is the honest pair; it is a decision, so it is recorded rather than taken. |
| 7, 2 | Scorecard `BinaryArtifacts` | `tests/fixtures/{elf,macho}/inet_gethost-*` | Working as intended. These are the committed real ELF and Mach-O a linker wrote, whose provenance each fixture's `README.md` records, and E9 replaced `current_exe()` with them *because* a real object is what those tests need. Removing them removes the tests. |
| 5, 4, 3, 1 | Scorecard `CIIBestPractices`, `CodeReview`, `Maintained`, `BranchProtection` | repository | Not code. Repository-administration questions for the maintainer. |

`docs/dev/log/F1-integration.md` says the follow-up hosted run must confirm both ordinary CI and
the CodeQL result before auto-merge is restored. The distribution cache-poisoning alerts it names
(10–14) are fixed and closed; what is above is what is left, and auto-merge is still off.

## A diagnostic that sent the reader to the wrong place

Running `ginary doctor` on this host produced:

```text
cache writable: yes
cache executable: no (mounted noexec?)
cache detail: `child 84206` did not exit within 10000ms
hint: set GINARY_CACHE_DIR to a directory this user can write to on a filesystem that is not
      mounted `noexec`
```

The `detail` line is honest and the two lines around it are wrong. Under `noexec` the exec fails
*immediately*, with `EACCES`; a program that started and did not return within the probe's
ten-second budget is the opposite observation, and the machine described above is exactly the one
that produces it — the probe writes a *new* executable every run, so it pays first-exec assessment
every run. A user reading that output goes to `mount(8)` about a directory that is fine.

The same defect had a second instance the regression found on its way past: a directory nothing
could be written to printed `no (nothing could be written to run)` — a line whose own comment
argues that naming `noexec` there would mislead — and was then handed the `noexec` hint anyway.
The line and the hint under it disagreed.

`CacheProbe` now carries `timed_out`, set from a `std::io::ErrorKind::TimedOut` that `run_probe`
attaches to a `ProcessError::Timeout` rather than from the shape of a message, and the three
causes get three sentences and three hints:

```text
cache executable: no (the probe did not finish in time)
cache detail: `child 21071` did not exit within 10000ms
hint: the probe started and did not return in time, so this says the machine is busy rather than
      anything about the directory; some systems assess a newly written program on its first run,
      and the probe writes a new one each time. Run `ginary doctor` again when the machine is idle.
```

`tests/regressions/f1_a_probe_that_timed_out_was_reported_as_a_noexec_mount.rs` pins all three,
and `doctor --json` carries `timed_out` beside `writable` and `executable`. One existing
assertion changed: `tests/doctor.rs` asserted that a read-only directory earns the `noexec`
hint, which is the second instance above, and it now asserts the write hint and that the word
`noexec` does not appear.

## One more gap, found by running the task

`mise run mutants` writes `mutants.out/` into the project root and `.gitignore` did not mention
it, so the documented assurance task left untracked output in `git status` — next to a rule that
forbids `git add -A`. `fuzz/.gitignore` already did the equivalent for the fuzz targets. Added.

## What this leaves

- **The nightly mutation campaign is still red.** It has failed every night since 2026-09-06;
  run [34747498271](https://github.com/P4suta/ginary/actions/runs/34747498271) reconciles to
  `caught 733, unviable 81, missed 101, timeout 30, not_run 15` with 56 of 95 shards failing. The
  101 survivors are `cache` 57, `payload` 15, `appfile` 14, `verify` 12, `closure` 2 and `launch`
  1. Seven tests were written here against the two largest clusters in `cache` —
  `maintenance_owns` and `sweep` — and ten more against `appfile`, whose fourteen survivors are
  now **thirteen killed and one equivalent**. A campaign over `maintenance_owns` reports twelve
  of its twenty mutants caught, five equivalent (argued and checked by hand) and two undecided
  because the run was restricted to one test target; the two sections at the end of this record
  have the numbers. That leaves `payload` 15, `verify` 12, `closure` 2 and `launch` 1 untouched,
  along with the rest of `cache`, and the campaign is still red.
- **The Windows console control event.** Unchanged, and still the one mechanism of `docs/adr/0015`
  resting on argument rather than on a run. It needs a Windows host and an ADR for a second
  `#[allow(unsafe_code)]`, and this session had neither.
- **Open pull requests.** #13 is merged. #15 is ported and verified here but not pushed. #10
  carries E23 work that is already on `main` as `d43c619` and conflicts; the editorial question
  `docs/dev/log/E23.md` raises about which record is the canonical E23 is still open. #7 is the
  release-please pull request and is out of scope by instruction.
- **Code scanning.** Ten alerts remain open (two `rust/cleartext-logging`, one
  `rust/log-injection`, and Scorecard's `PinnedDependencies`, `BinaryArtifacts` and repository
  ones). None was triaged here and none was dismissed.

## The local suite, finally measured

`GINARY_REQUIRE_TOOLCHAIN=1 cargo test --features fault-injection --no-fail-fast` on this host:
**2,587 passed, 20 failed**, against 226 failures at the baseline. Every one of the twenty is
accounted for and none is a claim about ginary:

| how many | what | why |
|---|---|---|
| 18 | `did not exit within …`, `incomplete process observation`, a cache probe that timed out, and one temporary tree that never appeared in time | first-exec assessment, measured above |
| 1 | `doctor_renders_truncated_tool_evidence…` | the same, probing a freshly built `ginary` |
| 1 | `f1_distribution_executed_a_tag_in_the_default_branch_context` | this machine's `git` policy wrapper, not worked around |

A twenty-first was a real miss and is fixed: `tests/regressions/b1_a_locked_entry_blocked_the_launch.rs`
still asked for `flock` through `require_tools`. The sweep that introduced `require_flock` grepped
`tests/*.rs` and not `tests/regressions/`, and no CI job could catch it — the Linux jobs that set
`GINARY_REQUIRE_TOOLCHAIN` have `flock`, and the `macos` job runs the smoke script rather than
the suite. Only a macOS host running the whole suite finds a gate like that, which is the point
of this record.

## Two things that are not defects

- **`syspolicyd`, and what it does to a suite that writes executables.** This one deserves its
  own paragraph, because it explains every timing failure above and it is the single most useful
  thing this session learned about running ginary's suite on a Mac.

  macOS assesses a newly written executable the first time it is exec'd — Gatekeeper's policy
  daemon, `syspolicyd`. The assessment is cached against the file, so the *second* exec is free.
  Measured on this host, in the state described at the top of this record:

  ```text
  first exec 33.815s   second exec 0.024s
  first exec 59.921s   second exec 0.024s
  ```

  with `syspolicyd` sitting at 80–90% of a core. Three orders of magnitude, on the first exec
  only.

  ginary's suite is unusually exposed to this. `process::test_support::script` writes a fresh
  executable and execs it; `scripts_written_and_run_in_parallel_are_never_text_file_busy` does it
  two hundred times on purpose, because writing and exec'ing in parallel is the whole subject of
  that test; and every end-to-end test builds a fresh artifact and runs it. On an idle Mac none of
  this is visible. On this one the `--lib` target advanced about one round of that test every
  twenty seconds, and the full run was abandoned rather than left to finish in hours.

  It is also the answer to the eleven end-to-end tests that reported `incomplete process
  observation` with a status present, every byte captured and `EOF=false`: the reader had not seen
  the pipe close within `DRAIN_GRACE` (500 ms) of the child being reaped, and `elapsed` on those
  runs was 23–27 seconds for work that takes about five. Raising the grace to five seconds turned
  all eleven green, and the same runtime measured on an idle pipe closed it in 34 ms.
  `DRAIN_GRACE` was **not** changed: loosening a bound to make a gate pass on a machine in this
  state is the thing `CLAUDE.md` forbids. The honest record is that those eleven are *unmeasured*
  here, not failing.

  Nothing in ginary should change for this. What should change is what a developer is told, and
  `docs/dev/testing.md` now says it: the suite's cost on macOS is dominated by first-exec
  assessment, the second run of the same tree is fast because the assessments are cached, and a
  machine already executing thousands of fresh binaries — another mutation campaign, say — will
  make the first run look like a hang.
- **A `git` wrapper.** `f1_distribution_executed_a_tag_in_the_default_branch_context` builds a
  private git fixture repository with signing and hooks turned off, and this machine's
  `~/.local/bin/git` is a policy wrapper that refuses exactly those options. The test is right,
  the wrapper is right, and nothing was worked around: it does not run on this host and says so.

## Killing the survivors: `appfile`

`appfile` is the other cluster worth starting on, and for a reason that has nothing to do with
its size: it parses text and starts no processes, so on this host — where a freshly written
executable costs thirty seconds — its whole test target runs in 0.15 seconds and a mutant can be
checked in the time a rebuild takes. Fourteen of the campaign's 101 survivors are in it.

Ten tests were added to `tests/appfile.rs`, each one edge of one boundary: the arithmetic of a
`\NNN` octal escape, the two halves of the `\x{...}` guard, the `u8::MAX` bound on a binary's
bytes (`ÿ` is codepoint 255 and belongs in one; 256 does not), both of the `>>` tests a binary
has — the `&&` that opens an empty one and the `||` that closes a full one — a float that parses
to infinity rather than to an error, the two lookaheads in `parse_term` that decide *which parser
runs* for a lone `<` or a lone `-`, the `#` that opens no map, and the nesting counter's
decrement, which is what makes a hundred siblings two levels deep rather than a hundred.

Each of the fourteen mutations was then applied to `src/appfile.rs` by hand and the target run,
which is cheaper here than a campaign and is what caught the error in this record's other
section. **Thirteen are killed.**

The fourteenth is equivalent: `render_float`'s `None if text.contains('.') => text`. The arm
under it, `None => format!("{text}.0")`, cannot be reached, because `text` is the `Debug` form of
a *finite* `f64` with no `e` in it, and Rust writes a decimal point for every such value. The
guard is therefore always true and no test can tell the two readings apart. That is an assumption
about the standard library rather than about ginary, so it is now pinned by a `proptest` over
normal floats and a case list of the subnormal and zero ones, instead of being argued in a
comment: if a future Rust writes `1` for a float, the property fails and the fallback arm stops
being unreachable — which is the moment somebody needs to know.

## Killing the survivors: `maintenance_owns`

`cache::maintenance_owns` decides whether a key-shaped directory is this application's before
anything removes it, and it reads the completeness marker under two bounds — `symlink_metadata`
refuses what is not a plain file, and `MAX_FRONT_ENTRY_BYTES` refuses what is too large to be a
manifest. Both bounds are off-by-one sensitive and neither edge was asserted, which is why the
campaign left survivors across four of its lines: `>` traded for `>=` and for `==`, `+ 1` traded
for `- 1` and for `* 1`, and two `||` traded for `&&`.

Four tests in `tests/cache.rs`, each one edge of one bound and each written so the two sides of
the edge have *different outcomes* — an entry removed, or an entry kept and reported `Unowned`:

- a marker of exactly `MAX_FRONT_ENTRY_BYTES` is this application's, so the bound is the largest
  marker there is and not the first one too large;
- a marker one byte over it is refused *however well its first bytes parse* — the padding is
  after the closing brace, so a reader that stopped one byte early would accept a complete valid
  manifest and delete the entry, which is exactly what `take(MAX)` instead of `take(MAX + 1)`
  does;
- a marker that is a symlink to an otherwise valid manifest is not read through;
- a marker that cannot be stated at all (a residue directory at mode `0o000`) is not ours, which
  is a different thing from a marker that is *absent* — the case an unfinished residue is allowed
  to be in.

`cargo mutants --file src/cache.rs --re maintenance_owns --features fault-injection -- --test
cache` was run against the result: twenty mutants, **twelve caught, seven missed, one
unviable**.

```text
caught: 1955:5  replace maintenance_owns -> bool with true / with false
caught: 1958:23 replace match guard error.kind() == NotFound with true / with false
caught: 1958:36 replace == with !=
caught: 1963:8  delete !
caught: 1965:27 replace > with == / with < / with >=
caught: 1977:31 replace > with == / with >=
caught: 1984:49 replace == with !=
missed: 1964:9  1965:9  1977:9   replace || with &&
missed: 1974:53 replace + with - / with *
missed: 1985:9  1986:9  replace && with ||
```

The mode-`0o000` test kills all three `1958` mutants, the symlink test kills `delete !`, and the
two bound tests kill every `>` on both length checks — which is what they were written for.

Two caveats on the seven, and the first is about the measurement rather than the code. **This run
gave each mutant only `tests/cache.rs`**, because `-- --test cache` is what makes twenty mutants
take an hour instead of a day on this host. `1985:9` and `1986:9` are the final
`app == manifest.app && check_version().is_ok() && validate().is_ok()`, and the fixtures that
separate those three terms live in `tests/regressions/f1_cache_maintenance_discarded_live_work.rs`
— a target this run never compiled. They are not evidence of a gap; they are evidence of the
restriction, and the nightly campaign, which runs the whole suite, is the thing that decides them.

The other five are **equivalent mutants**, and the argument was checked by hand rather than
reasoned at: each mutation was applied to `src/cache.rs`, the tests were run, and the outcome
compared. `metadata` comes from `symlink_metadata`, which does not follow links, and the guard is

```rust
if !metadata.is_file()
    || metadata.file_type().is_symlink()
    || metadata.len() > crate::payload::MAX_FRONT_ENTRY_BYTES
```

followed later by a second check of the same bound over the bytes actually read.

- `1964:9` makes the guard `(!is_file() && is_symlink()) || len > MAX`. The readings differ only
  when `!is_file()` holds and `is_symlink()` does not — a directory, a FIFO or a socket named
  `ginary.json`. The original refuses at once; the mutant falls through to `File::open`, which
  fails outright for a socket and succeeds for a directory whose `read_to_end` then fails with
  `EISDIR`. Both arms `return false`. Note that `is_symlink()` cannot hold while `is_file()`
  does, so that term is *documentation* rather than a decision: it is what tells a future reader
  that changing `symlink_metadata` to `metadata` would break this.
- `1965:9`, `1974:53` (both) and `1977:9` are all one fact about the function: **it checks the
  same bound twice**, and the first check makes the second unreachable as a decision. A marker
  larger than `MAX_FRONT_ENTRY_BYTES` is refused by `metadata.len()` before a byte is read, so
  the `take(MAX + 1)` below it and the `bytes.len() > MAX` after it can never see an oversized
  file. Turning `+ 1` into `- 1` and running the suite proves it: the bound tests stay green,
  because for a marker of *exactly* the bound the extra byte changes nothing (the manifest is 741
  bytes and the rest is padding, so `MAX - 1` still carries a complete document) and a marker over
  the bound never reaches the read at all.

  The second check is not dead code — `metadata.len()` is a `stat` at one instant and the file
  could grow before the read — but that is a race no test can arrange, which is exactly why these
  three are equivalent under any deterministic suite.

So five of the seven cannot be killed by any test, and two more need a target this run did not
compile. This repository has no mechanism for the first category: there is no `mutants.toml`, no
`#[mutants::skip]` anywhere in `src/`, and `docs/dev/v1-readiness.md` says flatly that "a
surviving mutant fails its shard". The campaign therefore cannot go green while they exist, and
there are three ways out — a documented per-mutant exclusion carrying the argument, a
restructuring that collapses the duplicated bound (at the cost of the TOCTOU re-check and of the
`is_symlink()` documentation), or a permanently red shard. That is a decision about this
project's assurance policy rather than a change to make quietly, and it is left to be made.
