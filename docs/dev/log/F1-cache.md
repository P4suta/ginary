<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# F1 cache maintenance and selftest

Cache maintenance must preserve active runtimes, extraction work, crash evidence and unrelated
files. Selftest must exercise the environment a normal launch uses and protect its runtime for
the lifetime of its child process. No release, upload, push, tag or hosted action is part of this
work.

## Evidence and changes

- `clean` recursively removed entire application directories, despite describing the contents
  as regenerable runtime data. `uninstall` accepted any directory containing `ginary.json` and
  removed temporary trees while their owning process was alive.
- The existing `clean` API and `CleanReport` fields remain available with safe removal behavior.
  `clean_detailed` and `DetailedCleanReport` expose removed entry paths, freed bytes and retained
  paths with their reason. Dumps, unrelated paths and links remain; live residue and locked
  runtime entries remain. Emptied application directories are removed after reclaiming owned
  entries, while the cache root stays.
- The baseline B1 vanished-entry regression used a Unix shell runtime and an environment that
  isolated only Unix cache variables. An additive native runtime fixture uses the existing
  portable `ShimStep` implementation and explicitly isolates `GINARY_CACHE_DIR`, retaining real
  fault-recovery coverage on Windows.

## TDD record

All Cargo commands prepend `C:\Users\livec\.cargo\bin` to PATH and use the existing offline cache.

1. RED: `cargo test --locked --offline --features fault-injection --test regressions
   f1_cache_maintenance -- --nocapture` ran four existing-API regression tests before production
   edits. All four failed assertions: deleted crash evidence, deleted an unrelated manifest
   directory, deleted live extraction residue, and a locked entry causing a cleaning error.
2. Initial GREEN attempts were interrupted by unrelated concurrent compile transitions in
   `sbom`/`output` and `bundle`/`BuildFailure`. These compile failures provide no behavioral evidence
   about the cache changes; focused checks are rerun after the library is coherent.

3. RED: `cargo test --locked --offline --features fault-injection --test regressions
   f1_selftest -- --nocapture` ran two real native-runtime selftests on Windows. Both failed
   assertions before the selftest production change: the real `ERL_OTP29_FLAGS` was absent from
   the scrub list, and the running selftest held no entry lock.
4. GREEN: both selftest cases passed after using the captured environment and the same
   lock/recheck path as normal launch. Their trace explicitly requests sensitive detail inside
   an isolated test directory so argv/default/scrub assertions remain intentional.
5. GREEN: `cargo test --locked --offline --features fault-injection --test cache -- --nocapture`
   passed all 30 compiled cases on Windows; two external-`flock` cases reported their missing
   tool skip. The new portable shared-lock regression does execute on Windows.
6. GREEN: `cargo test --locked --offline --features fault-injection --test regressions
   f1_cache_ -- --nocapture` passed all nine Windows cases, including a live artifact paused
   after extraction, uninstalled concurrently, and then resumed to its fixture runtime's exit
   code. Empty live trash and existing rename destinations are preserved too. The symlink
   case is Unix-only and was not executed on this Windows host.
7. GREEN: `cargo test --locked --offline --features fault-injection --test regressions
   b1_the_entry_could_vanish_between -- --nocapture` passed the repaired baseline test on a real
   Windows process: exactly two extractions, followed by runtime exit 7.
8. `cargo clippy --locked --offline --all-targets --all-features -- -D warnings` reached no
   cache-specific findings but was blocked by 29 `result_large_err` findings propagated through
   concurrent `ConfigError`/`BundleError` changes. The root run owns the final combined gate.
9. Adversarial RED: `cargo test --locked --offline --test regressions
   f1_clean_preserves_a_key_shaped -- --nocapture` showed that a 16-hex directory plus `{}` still
   caused unrelated user data to be deleted. Clean and uninstall now require a bounded,
   nonsymlink, supported and valid manifest matching the containing application. A bare key
   without that proof remains `unowned`; a dead-owner temporary residue may precede its marker,
   but an existing invalid marker is retained. The proof is checked before and after locking.
   This is structural ownership, not cryptographic authentication: the payload digest is not
   stored in the extracted manifest and cannot be recomputed from it.
10. GREEN after this stronger ownership check: all ten Windows `f1_cache_` cases passed with
    fault injection, including the actual concurrently paused extraction. Fixtures representing
    owned entries now contain valid matching-app manifests; byte accounting measures those
    real marker bytes rather than a two-byte placeholder.
11. Launcher lifecycle RED: `cargo test --locked --offline --test regressions
    f1_launcher_records_maintenance -- --nocapture` failed because successful directory lookup
    had no launcher start/end events. `run` now starts a `launcher` operation and emits end or
    failure for returned maintenance/runtime outcomes; known launcher errors include their
    exit code. Unix exec emits a `handoff` fact because a replaced process cannot report its
    own completion. Existing argv selftest/launch tests explicitly opt into sensitive traces;
    the default crash-slogan trace test expects redaction.
12. One attempted build failed with Windows access denied while replacing `target/debug/ginary.exe`
    during another agent's running integration suite. This is not a behavioral test result;
    further Cargo launches are coordinated by the root task.
13. Prune ownership RED: `cargo test --locked --offline --test regressions
    f1_prune_preserves -- --nocapture` showed automatic prune also removed a foreign named
    directory and a key-shaped directory containing `{}`. The same manifest ownership checks
    now protect prune, before and after locking; application and entry symlinks are retained.
    `tests/common/cachefs::plant_entry` and older cache regression fixtures now identify their
    application with real valid manifests. Detailed retention reports intentionally include
    preserved crash dumps as `unowned`; removed paths are individual entries.
14. Final combined GREEN: `cargo test --locked --offline --features fault-injection --test
    cache --test doctor --test launcher --test smoke_cli --test regressions -- --nocapture`
    exited 0. Cache: 30 harness successes, including two missing-flock gates. Doctor: 36,
    including three missing-Erlang gates. Regressions: 550 harness successes, with external
    tool gates printed separately. Smoke CLI: 6. `tests/launcher.rs` is Unix-only and ran zero
    tests on Windows; the three portable F1 launcher/selftest regressions did execute and
    passed. The B1 vanished-entry fault regression also passed. An earlier concurrently
    compiled suite saw one extraction because a default-feature ginary binary replaced the
    fault-enabled one; the isolated final command retained fault injection throughout.
## Final CLI fixture migration

The root's real-toolchain full fault run exposed five remaining CLI fixture failures in
`.cache/assurance/F1/final-faults-driver.log`. Four prune cases still used the local
`tests/cli.rs::plant_aged` helper's `{}` marker and were correctly reported `unowned`.
That helper now delegates to the shared valid-manifest fixture while retaining its age.
The clean JSON case compared a mixed-separator `join("hello/key")` spelling against a
native Windows path; its expectation now joins the two components separately. The
single-application clean assertion also names the individual entry being reclaimed.

These are fixture migrations to the intended ownership/reporting contract. Production
cache behavior is unchanged. No Cargo command ran during the root's full-suite ownership;
the root records the coordinated follow-up result. `git diff --check` passed.

## Automatic sweep ownership review

The final independent review found that automatic `sweep` had retained the old permissive
name parser and the exception that deleted this process's own temporary tree. Consequently,
`.notes.tmp-0` could lose unrelated user data, and concurrent library calls in one process
could delete an extraction still in progress. Clean and prune's stronger checks did not cover
this separate launch-time path.

The sweep now requires the exact 16-lowercase-hex temporary/corrupt residue name, refuses
application and residue links, validates an existing manifest against its application, and
acquires an exclusive entry lock before deletion. A missing manifest is allowed only for an
unfinished, properly named residue. Every live PID is retained, including the caller's PID;
the existing `self_pid` argument remains accepted for source compatibility. Failed removals
are reported as retained instead of silently disappearing from the report.

The added F1 regressions distinguish unrelated names, live same-process work, dead partial
data, invalid manifests, held/released locks and Unix directory links. Existing sweep fixtures
now use real key shapes, and the unsafe own-PID deletion expectation became a preservation
assertion. No Cargo command was started by the implementing agent during the root's final
validation ownership; the root records the consolidated result.

## Invocation ownership during extraction

The final integration check found another writer of the same temporary path: extraction
still removed `.<key>.tmp-<pid>` unconditionally before creating it. That bypassed the sweep's
live-owner protection, deleted an unrelated preexisting tree, and let two library calls in
one process erase each other's work. The root's coordinated RED run records all three portable
failures in `.cache/assurance/F1/cache-concurrency-red.log`.

Each invocation now creates its own directory exclusively, using the PID plus twelve random
alphanumeric characters. It never removes a preexisting candidate. Cleanup removes only the
invocation's allocated directory; interrupted work retains its owner name for later cleanup.
The shared ownership parser recognizes the strict new shape and legacy PID-only residues,
preserves every live PID, and leaves invalid or unowned names alone. The paused-extraction
fixture discovers the child's actual unique directory instead of predicting a PID-only path.

The same concurrency regression also passes one open file to both threads. A cloned File
shares its cursor, so extraction now reads a bounded payload range with an explicit offset
for every read. Unix positional reads preserve the caller's cursor. Windows positional reads
set the cursor but do not depend on it, so simultaneous extractions still read their own
offsets correctly. The tests synchronize both real library calls before either starts unpacking,
require both to succeed with distinct temporary paths and one completed entry, and verify
that failure followed by retry preserves a planted earlier tree. The root owns final GREEN
execution; the implementing agent did not run Cargo concurrently.

Coordinated GREEN is recorded in `.cache/assurance/F1/cache-concurrency-green.log`: all
30 cache tests and all three portable concurrency/ownership tests passed. The Unix cursor
test awaits a Unix runner. A read-only final cache mutation enumeration finds 282 candidates,
so all twenty-four existing cache shards remain within the thirteen-mutant cap. The raw list
is `mutants-cache-invocations-list.json`; this enumeration executes no mutations.
