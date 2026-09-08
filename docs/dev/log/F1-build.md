<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# F1 — isolated builds, safe output publication, and retained process evidence

Date: 2026-09-08. Local implementation evidence; no release, tag, push, or upload.

## Product behavior

A nonblocking project lock covers export, shipment consumption, target publication, and
auxiliary finalization. A competing build reports the lock and project immediately. Each
invocation owns a fresh `.work-<pid>-<random>` directory, and each target has its own staging
root inside it. Retained successful and failed builds name this directory; cleanup warnings
preserve the directory in the result as well as the warning text.

The additive `build_detailed` and `build_with_stub_detailed` APIs retain completed target
rows, the failing target, unattempted targets, cleanup warnings and retained staging.
Existing entrypoints keep their result types; failures after publication carry the richer
failure as their typed source. `build_finalized` and `build_with_stub_finalized` invoke an
auxiliary writer under the project lock, including for targets published before a later
failure. Early failures invoke the writer once with an empty slice. This closes the race
between releasing the build lock and reading an executable to produce its SBOM.

Outputs are checked against one another and actual input identities before publication.
The checks cover canonical path aliases, existing hard links, Windows case aliases,
shipment/runtime trees, project source/test/priv trees, dependency and project manifests,
explicit configuration/native overrides, and selected stubs, including environment/cache
stub lookup. Auxiliary output plans use `validate_output_paths`. Existing input trees are
scanned by identity, with cyclic directories visited once and an explicit one-million-entry
limit. Unreadable or special entries fail closed. A compiler export is followed by another
input check while the shipment is locked.

Manifest copies and SBOMs are written through synced same-directory temporary files.
macOS artifact signing also takes place on a temporary destination before publication.
Artifact lengths come from the completed file, including macOS signature/padding overhead.
Target-specific SBOM names derive from the complete artifact filename. Embedding callers
must provide a verified ginary stub; an arbitrary host executable is refused before export.

## Process evidence and bounded cleanup

The configured-command `run_command` and already-spawned `wait_child` APIs retain exact
stdout/stderr bytes, observed exit status, the primary failure, elapsed time, EOF/read
errors, omitted-byte counts and child cleanup status. Each stream retains at most 1 MiB by
default, keeping its tail. Cleanup uses nonblocking polls with 500 ms slack. If the OS
refuses termination or reaping, the report says so and an unfinished child is handed to a
background reaper; the caller never enters an unconditional blocking wait after kill fails.
Both readers share another 500 ms deadline. Descendants are not process-group terminated;
a descendant holding a pipe produces an explicit incomplete observation.

Legacy string-returning process APIs preserve failure output and refuse incomplete parse
input. The integration harness now uses the same executor. With
`GINARY_TEST_EVIDENCE_DIR`, timeout, nonzero exit, incomplete capture and cleanup failures
save a unique directory containing bounded `stdout.bin`, `stderr.bin`, and versioned
`report.json` metadata. A failed evidence write is reported without claiming files were saved.

## Assertion RED evidence

The regression assertions were exercised against the preceding behavior before fixes:

- Target SBOM names compared equal; publishing SBOM/macOS output truncated a hard-link peer.
- Duplicate target output names reached shipment lookup; a held project lock did not stop
  another build before shipment lookup.
- Repeated builds reused retained staging, failure results omitted its location, and a
  manifest publication failure lost the executable already published.
- An unrelated file supplied through the embedding entrypoint reached shipment lookup.
- Shipment/runtime input hard links were accepted as outputs.
- `exporting_a_project_cannot_replace_its_sources_or_dependency_manifest` returned `Ok(())`
  for an output at `src/hello.gleam` (one failed assertion, zero compilation failures).
- Timeout reports discarded the status collected while reaping, and the old test harness
  panic omitted `last-progress-before-hang`. Both assertions failed before shared capture
  and cleanup wiring. Root's earlier timeout-output RED is retained in
  `.cache/assurance/F1/process-red.log`.
- Root's CLI regression proved an SBOM could replace the implicit cross-target stub;
  `.cache/assurance/F1/implicit-stub-red.log` records successful exit before the fix.

The finalization-under-lock API is covered by a nested real build attempt: it fails busy
inside the callback and succeeds after finalization returns. The callback's early-failure
case is separately checked to run exactly once with no artifacts.

## Local verification

Windows x86_64, Rust 1.98; Cargo uses the installed toolchain at
`C:\Users\livec\.cargo\bin`, with offline dependency resolution.

- `cargo test --offline --lib process::`: 15 passed, including exact non-UTF-8 tail,
  zero capture limit, injected read failure, failed kill, failed wait, and exit/kill race.
- `cargo test --offline --test regressions f1_build_outputs_were_not_isolated`:
  12 passed, including real filesystem identities, retained work and finalization locking.
- Earlier bundle unit suite: 21 passed; bundle/SBOM integration suites: 17 + 23 passed.
- Combined F1 regression run: 55/56 passed in
  `.cache/assurance/F1/build-process-green.log`; every build/process/evidence regression
  passed. The sole failure belonged to a launcher trace test expecting the superseded
  event name `begin` instead of `start`, sent to its owner for correction.
- Nested harness regression verifies retained timeout bytes and JSON cleanup/elapsed/
  omission fields without mutating this test process's environment.

The Windows keep-staging expectation now follows the target-specific staging root. The
Windows README guard tracks the current runtime contract and remaining MAX_PATH
qualification instead of historical roadmap keywords. Tests needing real Erlang/Gleam or
native macOS execution remain separately gated; synthetic packaging does not establish
native runtime execution or signing qualification. The root task records the final
consolidated suite after concurrent edits stop; interim doctest failures caused by another
Cargo invocation replacing its referenced rlib are not counted as product regressions.

## Atomic publication completion checks

The mandatory publication failure matrix was expanded after the initial build changes.
`src/fault.rs` now registers `output-write:fail` (partial temporary write),
`output-persist:fail` (complete temporary file before replacement), and
`artifact-sign:fail` / `artifact-sign:corrupt` (partial signing write / altered finished
signature). All are process-local, opt-in `fault-injection` points; release builds do not
read the selector.

`.cache/assurance/F1/atomic-red.log` records three assertion failures when real CLI SBOM,
native build and macOS build commands returned success despite the requested failures.
After injection was implemented, `.cache/assurance/F1/signature-red.log` records two green
atomic tests and the separate assertion RED: a corrupted finished macOS signature was still
published successfully. The corrected builder reads its completed temporary artifact using
`inspect::open`, checks `inspect::verify`, and calls the independent
`sign_macos::verify_ad_hoc` before publication. The signer validator was implemented by the
assurance workstream and verifies the actual generated signature, page hashes and layout.

`cargo test --offline --features fault-injection --test regressions atomic_publication_`
then passed all three tests, recorded in `.cache/assurance/F1/atomic-green.log`. Within those
tests, SBOM and native artifact write/replacement failures preserve existing bytes; macOS
write/replacement/signing/corruption failures preserve existing bytes. Every injected failure
is followed by a successful real CLI rerun and complete JSON or artifact/payload validation.
This is portable verification over a committed real Mach-O fixture, not execution of that
artifact on macOS; native code-signature qualification remains a separate CI obligation.

A final CLI assertion exposed the remaining human-readable size mismatch:
`BuildReport::artifact_line` still printed the simple appended-file arithmetic even though
`TargetBuild` and JSON held measured lengths. `.cache/assurance/F1/mac-size-red.log` records
that assertion failure. Both renderers now call one measured-size renderer. The existing
ELF/PE text is preserved when the arithmetic agrees; macOS overhead is represented by the
actual completed file length. The full atomic matrix was rerun after this correction.

## Manifest-specific failures and final input-order review

The `fail-document` action on `output-write` and `output-persist` lets the executable
finish before failing its manifest's atomic writer. The CLI regression checks the previous
manifest bytes survive, the JSON failure report retains the published artifact row without
claiming a new manifest, and an ordinary rerun replaces the manifest with complete JSON.
The initial fixture used `--target host`, which deliberately produces no target manifest;
that fixture was corrected to the canonical host target. The valid assertion RED was then
captured with document-only injection disabled in
`.cache/assurance/F1/manifest-specific-red.log`, before restoring the behavior under test.

The independent output-order review found two additional concrete input risks. A catalog
runtime is selected after CLI preflight, so its files could previously be named as SBOM
outputs. Preflight now protects the entire selected OTP cache tree for catalog/tarball
sources, compares existing hard-link identities, and protects an explicit catalog document.
The regression checks both a runtime path and its alias are rejected during preflight.
The source tree's existing `gleam.toml` could also be modified by trace append before it was
parsed. Root's trace-sink protection fixes that separate ordering issue; the build regression
asserts the original configuration bytes survive and the valid build still succeeds.

Final focused command:
`cargo test --offline --features fault-injection --test regressions f1_build_outputs_were_not_isolated`
passed all 18 tests (zero failed/ignored), in
`.cache/assurance/F1/manifest-catalog-green.log`. Input-identity scans remain deliberately
fail-closed: an input tree that cannot be inspected, including mandatory Windows sharing
restrictions from another writer, is reported instead of silently assumed safe.

## Pinned native tool discovery

Targeted prerequisite discovery found the already installed tools outside PATH:

- `C:\Users\livec\AppData\Local\mise\installs\gleam\1.18.1\gleam.exe`
  reports `gleam 1.18.1`.
- `C:\Users\livec\AppData\Local\mise\installs\erlang\29.0.5\bin\erl.exe`
  reports BEAM emulator `17.0.5`; the installation's `releases\29\OTP_VERSION`
  contains `29.0.5`.

The default sandbox cannot read/execute these installed tools (AccessDenied was reproduced).
Supported `require_escalated` read-only listings and version checks were approved and passed.
No package download or global installation was necessary. The root task can prepend the two
installation paths and run authorized real integration tests with the same supported
escalation route. This discovery itself launched no Cargo build or test run.

## Real-runtime E2E fixture updates

After enabling the installed pinned toolchain, the root's first real `e2e_hello` run
completed 21/26 tests successfully, including executing the packaged application with an
empty PATH and propagating its exit code. The five failures were diagnosed from
`.cache/assurance/F1/final-faults-driver.log` as outdated fixture expectations:

- A canonical retained staging path used the Windows verbatim prefix and native separators;
  the fixture compared its text with a differently spelled directory. It now parses the
  `staging:` line and compares directory identity.
- JSON expected build-report schema 1 instead of the accepted schema 2. The fixture now
  checks schema 2 and its status/target/SBOM fields. Size fields are checked against the
  input stub and completed artifact, avoiding ELF-only arithmetic on signed macOS files.
- Two trace assertions expected sensitive argv/environment values under default redaction.
  Those inspections now explicitly opt into `GINARY_TRACE_SENSITIVE=1`; argv is decoded
  from JSON and path suffixes are compared using host path semantics.
- The duplicate-target fixture's exact directory listing omitted the persistent project
  lock. The expected listing now includes that control directory alongside one artifact
  and one manifest.

Only `tests/e2e_hello.rs` changed for these failures; no product behavior was relaxed.
The root owns the subsequent real-runtime rerun after the full suite completes. No concurrent
Cargo invocation was started during these fixture corrections.

The same real-runtime run passed 21/23 SBOM tests. The two failures were exact
text expectations for the now target-qualified `sbom: <path> (<target>)` line;
the tests now require the host target as well as the requested/default path.
The legacy B2 late-SBOM-failure regression used a directory destination, which
is now correctly refused before export. Its late-failure case now uses the
`output-persist:fail-document` selector under the fault-injection feature, and
requires both the published artifact's reported path and preservation of the
previous document. The missing-parent preflight regression remains unconditional.
These fixture updates await the root's coordinated real-runtime rerun.

## Windows publication alias review

The independent final review reproduced a real output collision with two initially
absent names: `artifact` and `artifact.` (also `artifact `). Preflight accepted both,
then Win32 replacement wrote the SBOM onto the executable and returned success.
`.cache/assurance/F1/alias-review.log` records the resulting document bytes at the
artifact path; `review_alias.rs` links the existing library for this reproduction.

Windows output destinations now reject trailing dots/spaces in every component,
alternate streams, reserved device names and device namespace prefixes before
identity inspection or temporary-file creation. This follows Microsoft's
[Windows filename rules](https://learn.microsoft.com/en-us/windows/win32/fileio/naming-a-file).
Ordinary canonical/verbatim disk and UNC paths remain supported; unusual verbatim
component names deliberately receive the same refusal. Drive-relative destinations
are refused because their working directory is otherwise implicit.

The new assertions failed before the guard (`windows-alias-red.log`: two failures,
one ordinary-path success), then all nine output unit tests passed after the guard
(`windows-alias-green.log`). These isolated rustc tests compile the actual `src/output.rs`
and existing dependency artifacts, with outputs entirely under `.cache/assurance/F1`;
no Cargo run or shared-target mutation occurred. An additional Windows CLI matrix
covers existing and absent artifacts, both artifact/SBOM arguments, ambiguous parent
components, devices and streams. The root owns that matrix's coordinated Cargo run.

The review also identified an unconfirmed portability gap: missing-name case comparison
currently follows the Windows host rule. Case-insensitive macOS volumes have not been
qualified locally for two differently cased, initially absent output names. This Windows
session supplies no native macOS filesystem evidence for that scenario.

## Native Windows catalog consumption and partial warnings

An owned copy of the current CLI was made under
`.cache/assurance/F1/native-otp-repack`; `binary-identity.json` records its SHA-256 equality
with the compiled source binary. A bounded driver retains exact stdout/stderr, EOF and
omission metadata, child status, elapsed time and cleanup state. Installed tool access
used the supported approved escalation route, and all generated data remained in the
workspace. Networking and publication were not used.

The real zero-dependency `hello_ffi` fixture exposed two defects:

- `catalog-build-red.log` records the native Windows build gate refusing the `catalog`
  source kind before runtime inspection. Windows hosts now permit catalog and tarball
  sources to reach the existing PE/header, required-file and catalog-claim validation.
  The final rule permits these explicit sources on every host: the resolver reads the
  runtime's bytes and cross-target BEAM stripping uses host Erlang. `host` still requires
  a Windows build host, and Docker remains unsupported. The F1 synthetic
  CLI test consumes both valid local sources, then rejects a Linux emulator and a runtime
  missing its required DLL while preserving the existing artifact. E8 and the Windows
  build documentation now describe the same rule.
- `warnings-red.log` records one published executable and a manifest-copy failure, but
  an empty warnings list despite distribution being enabled without a node name. Target
  warnings now accumulate as each finding is observed and are folded into both successful
  and partial reports. The existing manifest-failure regression requires the warning too.

The first real `otp repack --root` attempt failed after 613 seconds with Windows error 206
when the BEAM stripping command exceeded the native command-line length. It was an ordinary
exit 1, not a timeout: the direct child was reaped and both captured streams reached EOF
without omissions. `repack-driver.log`, `repack.status.txt` and the raw streams preserve
that failure. No catalog or runtime archive was published. The repacker's temporary source
snapshot was removed by its ordinary error cleanup, so no produced or source-snapshot
digest is claimed from this failed run. The source installation and original executable
were preserved. A coordinated retry after the separately owned strip fix will receive a
larger explicit budget; changing the provenance hash format to avoid the work is unnecessary.

The second real repack used a separately copied, hashed CLI containing the Windows argument
batching fix and an explicit 1,800-second budget. It completed in 674.575 seconds with exit 0,
both streams at EOF, no omitted bytes and a reaped direct child. Evidence remains under
`.cache/assurance/F1/native-otp-repack-rerun`; the original failed run remains intact. The
repacker stripped 1,322 modules from 51,639,548 to 9,610,639 bytes and produced a 46,202,598-byte
archive. Independent SHA-256 calculation matched the catalog:
`37c9c3747e62e79abbb278a44649c47ed21a48915fce67df504803b3592fd891`.
The catalog retains `local-runtime-root` provenance, tag `OTP-29.0.5`, and the source-tree
snapshot digest `2aace313a6237c17548530c68b8aab4244610eb343c089e5612fb54590570a22`.

The next actual consumer command exposed a separate CLI defect: `otp fetch --catalog` with
`GINARY_OFFLINE=1` rejected the existing adjacent archive in 49 milliseconds before verifying
it. `fetch-driver.log` and its raw streams preserve the failure. The library already verifies
and extracts local archives offline; `write_otp_fetch` incorrectly classified every cache
miss as a network request. The minimized CLI regression covers both adjacent and absolute
local archive paths, source preservation and digest verification, while retaining a true
remote-URL offline refusal case. This correction reuses the completed archive; another full
repack is not needed to test it.

The regression run recorded three actual local-source assertion failures and one remote
refusal success before the fix (`offline-local-catalog-assertions-red.log`); all 16 `otp_cli`
tests passed afterwards (`offline-local-catalog-green.log`). The CLI guard now refuses only
remote sources offline. The final plain CLI, SHA-256
`61861e74642de8f41c54210a3fead30c97ec8cf429a0c66742f155324a090a08`, then fetched the actual
46 MB adjacent archive into a separate, initially absent cache in 4.345 seconds with exit 0.
`fetch-green-driver.log` and the cache completion marker retain the command outcome and the
verified digest. This was an extraction, not a warmed-cache success.

The real `hello_ffi` consumer built offline with Gleam 1.18.1, the generated Windows catalog,
`--no-strip --target windows-x86_64 --sbom --report json`. It completed in 11.620 seconds and
produced an executable, manifest copy and SPDX 2.3 SBOM. The builder was the plain CLI before
the fetch-only guard correction; `builder-identity.json` records its separate SHA-256. Its
launcher, runtime resolver and artifact implementation contain the final product fixes.
The artifact's measured length is 19,755,853 bytes and its full-file SHA-256 is
`bb3fc1ac3f528053169fc743b4c27927ca25c024ca809e2addc9236d4b59304c`.
`verify --json` checked 214 files and four PE objects, with integrity and contents both passed
and no issues. Payload SHA-256 is
`331246a6b3b41f842dc400de1fc84c6880341ae55ccd21dc7cea42cc81cc0424`.

The packaged application then ran with an empty PATH, its own cache/home and working directory,
and no Gleam/Erlang installation paths in its environment. It printed exactly the expected
arguments (`3 a b`), `hello from priv`, and its caller's directory, and propagated exit 3.
That clean run retained Windows `SystemRoot` as operating-system context; stdout contains
three expected lines and stderr is empty. An earlier fully cleared environment without
`SystemRoot` also preserved the application behavior but Erlang emitted Winsock initialization
error 10106; its raw evidence remains under `hermetic.*` instead of being discarded. The clean
run is recorded under `hermetic-system.*`. A shell-only assertion initially expected CRLF
instead of Erlang's LF; the corrected logical-line assertions validated the already captured
bytes without rerunning the process (`hermetic-system-assertions.json`).

All successful native commands retained EOF, omission, status and direct-child cleanup
evidence. `acceptance-summary.json` records the artifact and SBOM hashes, verified archive
digest and real runtime behavior. The installed OTP version, launcher hash and emulator DLL
hash match their pre-repack values (`source-before.json`, `source-after.json`); the source
archive still matches its original produced digest. No network, global installation or
publication was used for this acceptance chain. These are Windows x86-64 native results;
they do not claim native Linux/macOS or ARM execution.

## Measured coverage follow-up

The first complete Windows branch-instrumented run measured 86.03% line coverage and
78.04% branch coverage, below the unchanged 90%/80% floors. Passing behavioral suites does
not make that coverage gate pass. LCOV identified missing catalog acceptance paths beyond
the original native happy path: gzip cache inputs, changed content under an existing archive
name, stale extraction recovery, blocked cache/install destinations, inconsistent declared
sizes/digests, malformed release metadata, dynamic libc provenance, and OpenSSL/JIT detection.
Additional integration tests exercise these inputs and assert source preservation, cache
completion boundaries, no premature downloads/publication, and the resulting catalog claims.

The bundle additions exercise both public current-executable wrappers, typed early errors,
corrupt packaged-stub refusal, cleanup diagnostics and legacy partial-publication evidence.
They do not add production-only coverage seams or change the instrumented source line map.
The root coordinates compilation and incremental measurement; no new gate percentage is
claimed before that measurement finishes.

A separate compact real OTP root was prepared under
`.cache/assurance/F1/coverage-native-repack/runtime` from the verified executable's extracted
runtime: 210 files and 8,009,032 bytes, plus the pinned release metadata and an original
`inet_db.beam` carrying debug information. `source.json` records exact provenance and the
restored module hash. Instrumented repacking of that input can reach actual Erlang stripping
without repeating the full installation's 674-second source snapshot. It supplements, rather
than replaces, the previously completed full native-runtime acceptance.

The coordinated incremental test run passed the updated catalog/repack/bundle harnesses
and the legacy partial-result regression. Before the additional real repack profiles were
merged, coverage rose to 86.86% lines and 79.31% branches; both remained below their floors.

The copied instrumented CLI then repacked the compact actual runtime in 12.258 seconds,
stripping 202 modules from 1,942,682 to 1,886,069 bytes. A second run with an empty PATH
completed in 10.645 seconds and explicitly reported that the one debug-bearing module was
not stripped because host Erlang was unavailable. Both commands succeeded, retained complete
bounded streams and reaped their direct children. Each generated archive was independently
hashed and consumed through an instrumented offline `otp fetch` into a fresh separate cache.
The first extracted `inet_db.beam` has no `Dbgi`; the unavailable-Erlang output retains it as
the report says. The source debug module's hash remains unchanged. `verified.json` and
`consumption-verified.json` under `coverage-native-repack` record the digests and observations.
The profile files were written to the root's agreed instrumented target pattern; the final
merged measurement, not the presence of these runs, determines whether the gate passes.

## Linux qualification and native catalog acceptance

An isolated Linux snapshot ran the repository's actual
`scripts/ci/coverage.sh branches` with matching Rust/LLVM 22.1.8, nightly
`2026-08-02`, cargo-llvm-cov 0.9.0, Gleam 1.18.1 and OTP 27.3.4.16. The composed
image used existing cached Rust, Erlang and Gleam images; exact identities,
tool-install provenance and profile-format smoke checks are retained under
`.cache/assurance/F1/linux`. The original workspace stayed read-only, project
commands ran offline with Docker networking disabled, and Linux Cargo state
used private named volumes. `GINARY_REQUIRE_TOOLCHAIN` was deliberately unset:
this environment has no nested Docker engine, cross stubs, actionlint or pwsh.

The first complete run took 1,037.3 seconds and recorded 2,279 successful tests,
15 failed tests and 18 explicit skips, with no unrun or interrupted tests.
The failures were retained and resolved separately: seven permission fixtures
needed an unprivileged user; four real stripping tests used an OTP 29 BEAM
fixture that OTP 27 cannot read; two nested-trace fixtures used an invalid
application name; and two prose contracts needed their existing new behavior
stated explicitly. The committed parser fixtures remain unchanged. Real
stripping regressions now compile a debug-bearing module with the discovered
runtime, then retain the original code/debug/docs and neighbor-preservation
assertions. The awkward JSON path is now in the cache parent, while its
application name remains valid for ownership checks.

Complete affected harnesses then ran as UID 1000. Final regression evidence
records 628 successful tests and five explicit skips; cache 38, launcher 64,
version consistency 10, macOS signing 24 and final verification 23 all passed.
The direct-binary runner's initially missing Cargo executable variable and
Git ownership configuration are preserved as a separate failed attempt.
`reconciled-outcomes.json` keys each result by harness and test and names the
run that supplies it; it does not add repeated executions to the test count.
Final qualification is **2,298 successful, 14 skipped, zero failed, zero
unrun and zero interrupted**. Skips identify unavailable cross prerequisites,
the named `notify` shipment, actionlint/pwsh and the intentional copied-tree
repository-gate cases. This is not full cross-platform CI equivalence.

The final instrumented CLI also repacked the complete installed Linux OTP
root, stripped 1,277 modules from 53,425,356 to 9,147,314 bytes, and produced a
26,194,956-byte archive. Its independently verified SHA-256 is
`1992d8110faa1a348f0a560c60d26d151e7f249701d18e117e249d683df1ea47`.
Offline fetch populated a new cache from the adjacent catalog archive. The
`hello_ffi` consumer then built from that catalog with default stripping,
retained staging and an SPDX SBOM. All original OTP file hashes and symlink
targets remained unchanged. The completed repack/fetch/build sequence and
deep verification took 282.88 seconds.

Deep verification authenticated 206 files and inspected four ELF objects.
It correctly returned exit 1 for the source emulator's external `libz.so.1`
dependency. Independent `readelf` output from the original emulator confirms
the finding. The allowlist was not widened: as documented by E7, a host's
extra system library is not a portability guarantee. The Linux artifact is
therefore recorded with that finding, not as a zero-issue portable artifact.
The final verifier harness separately inspected the already verified Windows
artifact from a read-only mount, preserving its named-input zero-issue contract.

The Linux application ran in the original cached Rust-only image, with no
Erlang installation, networking disabled, UID 1000 and an empty PATH. It printed
exactly `args=3 a b`, `hello from priv` and `cwd=/run-home/cwd`, propagated exit 3
and left stderr empty. That image supplies the declared zlib dependency; its
library hash is retained. An initial bind-mount evidence-permission failure
occurred before application execution and remains recorded separately. The
successful run used owned tmpfs for runtime tracing and profiles and retained
the raw streams, direct-child cleanup evidence and unchanged artifact hash:
`da44b9f0aa99a1f27d13ccce87ea2a14b453d846d3299556a9cef6d230560a84`
(51,566,837 bytes). This is an instrumented executable. Unix `exec` produced no
final raw launcher profile; the actual repack/fetch/build/verify profiles were
retained and merged without inventing runtime coverage.

Final measured coverage is **91.05% lines (18,202/19,991)** and **83.84% branches
(2,443/2,914)**, clearing the unchanged 90%/80% floors. The final source changes
kept line positions stable; the equivalent macOS section iteration was rebuilt
and exercised by the complete signing and regression harnesses. The final
LCOV, source hashes, 35 MB compressed profile archive, outcome provenance and
successful retention statuses are under `linux/final-coverage`. The exported
Linux executable, SBOM, catalog, verifier finding and Erlang-less runtime
evidence are under `linux/native-artifact`. No image, package or release was
published.
