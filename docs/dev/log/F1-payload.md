<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# F1 — target names must survive inspection without host reinterpretation

The Windows review found that `destined_path_for` delegated Linux/macOS names to native
`Path::components`. A Linux index row `lib/data/nested.txt` and tar entry
`lib/data\nested.txt` therefore compared equal on Windows even though they identify different
Linux files. Deep verification reported no issue. Foreign extraction also accepted the tar
entry, wrote the slash-separated destination, and published its completion marker.

## Changes

- The additive target-aware API now parses target components lexically on every host. Unix
  backslashes and colons retain their literal meaning; Windows target paths retain separator,
  case-folding and invalid-name checks. The older native `destined_path` API is unchanged.
- Packing and extraction additionally require source/host components to mean the same thing
  as target components. Unrepresentable spellings fail with `UnsafePath`; they are never
  silently renamed by host path parsing.
- Extraction checks both archive and index destinations against host aliases. It detects
  file collisions and implicit parent-directory aliases, such as `Extra/a` and `extra/b` in
  a Linux payload extracted on Windows. Index rejection precedes writing its payload files.
- Verification remains an inspection of the declared target. A foreign artifact may be
  valid for its target while being impossible to extract on the current host. Both readers
  agree about the target identities; the extracting reader has the additional host check.

## Assertion evidence

Commands ran in the coordinated Windows Cargo window with `C:\Users\livec\.cargo\bin` on
PATH; all commands used `--locked --offline --features fault-injection`.

1. `cargo test --locked --offline --features fault-injection --test regressions
   f1_foreign_payload_paths_used_the_hosts_separators -- --nocapture` produced four assertion
   failures before production changes. It demonstrated host-dependent Unix normalization,
   false clean verification, successful extraction under the wrong name, and case collisions
   diagnosed only after writing a file. The legacy native API and Windows-target controls
   passed. Evidence: `.cache/assurance/F1/foreign-paths-red.log`.
2. The same command passed all six tests after the change, including both file aliases and
   implicit-directory aliases. Evidence: `.cache/assurance/F1/foreign-paths-green.log`.
3. `cargo test --locked --offline --features fault-injection --test payload --test verify`
   passed all 39 payload and 23 verify harness cases. Environment-backed artifact cases
   retain their explicit tool/environment gates; these counts are harness outcomes, not a
   claim that unavailable external runtimes were exercised. Evidence:
   `.cache/assurance/F1/foreign-paths-compatibility.log`.

The opposite extraction direction is compiled for Unix CI: a Windows backslash separator
must not become a literal Unix filename. This Windows session exercises the Windows direction
with real filesystem writes and keeps the Unix case for its native runner. No release,
upload, tag or push was performed.
