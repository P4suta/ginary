<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# F1 Windows BEAM argument limits

The real native OTP repack spent 613 seconds preparing the installed OTP 29.0.5 runtime,
then exited 1 because starting its `bin/erl.exe` failed with Windows OS error 206.
`.cache/assurance/F1/native-otp-repack/repack-driver.log` retains the complete bounded process
evidence. The ordinary error path removed its owned temporary tree; the installed source was
not modified. Most elapsed time belonged to preparation and source hashing, so the minimized
acceptance below does not repeat that whole operation.

`strip::strip_beams` used `MAX_ARGUMENT_BYTES = 256 * 1024` on every host, counting only
unquoted module path bytes. Windows limits the entire CreateProcessW command line to 32,767
UTF-16 characters including its terminator. The executable path, Erlang expression, separators
and quoting also consume that limit. The authoritative contract is Microsoft's
[CreateProcessW documentation](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-createprocessw).

`f1_beam_batches_exceeded_windows_command_lines` reproduces the boundary with 400 small
modules under paths containing spaces. Its native shim rewrites every file, making lost
batches observable through independent BEAM chunk verification. Its actual Erlang case uses
400 copies of the committed 4,664-byte `gleam@bool.beam` in this initial Windows run, checks that every copy loses debug
chunks while retaining code, and checks that the source fixture remains byte-identical.
The failure case requires the failed batch's file range and the tool's original error text.
Each spawned runtime retains the existing bounded execution/capture/cleanup behavior.

The root's focused RED run failed all three assertions with actual OS error 206, including
the installed-Erlang case, in 0.59 seconds. The complete output is
`.cache/assurance/F1/beam-arguments-red.log`. Only after that assertion RED was `src/strip.rs`
changed. It now accounts for quoted UTF-16 executable and argument lengths, including
backslashes before quotes, trailing backslashes, separators and the terminator. Its 30,720-unit
budget leaves room below the OS limit for the Erlang launcher's extra arguments. Fixed
arguments are subtracted before modules are batched, and every module is preflighted before
the first invocation. An individually oversized path is identified with an existing
`StripError::Io`/`InvalidInput` result and a shorter-staging-directory remedy.

The public constants and error layouts remain unchanged. A multi-batch runtime failure adds
its batch number and file range to the existing `BeamStripFailed.stderr`, retaining the tool's
error text; single-batch error text and underlying process errors retain their established
form. Portable unit cases additionally check UTF-16 surrogate pairs, quoting, backslash
expansion, fixed/executable overhead and an oversized single path. The implementing agent ran
rustfmt and `git diff --check`; the root owns coordinated Cargo validation.

The coordinated focused GREEN passed all eight private strip unit tests, all three new
minimized regressions, and the existing strip compatibility target. That target reports
29 harness successes: 27 executed cases plus two explicit host skips (the Windows test
binary is PE, and there is no host ELF shared object to stage). Those skips are not Linux
ELF validation. The three new regressions completed in
1.52 seconds, including the installed Erlang/OTP 29.0.5 process stripping every one of the
400 real modules. `.cache/assurance/F1/beam-validation.json` records zero exit codes for all
three validation groups. This demonstrates the repaired native process path directly;
the separate full installed-runtime repack retry remains the root's broader acceptance run.
The detailed logs are `beam-budget-units-green.log`, `beam-strip-compatibility-green.log`
and `beam-arguments-green.log` in that same evidence directory. The Windows command-line
rules also have portable unit coverage, while actual Unix execution belongs to Unix runners.
Source and tests are frozen after that GREEN result.

## Actual OTP 27 fixture compatibility

The later Linux qualification ran the unchanged implementation against real OTP 27.3.4.16.
Four execution regressions failed: the module under `priv`, the bracket and star directory
names, and the 400-module batch. Their inputs were the committed OTP 29 fixtures; the
failures occurred in OTP 27's `beam_lib:extract_atom` / `binary_to_atom`, before stripping.
The exact assertion RED is retained in `.cache/assurance/F1/linux/coverage-full.log` (the
first three at lines 7438–7464). The fixture README records their OTP 29.0.5 provenance.

This is a test input compatibility defect. The official Erlang compiler's
[atom-table implementation](https://github.com/erlang/otp/blob/master/lib/compiler/src/beam_asm.erl)
uses a compact `AtU8` representation for OTP 28 and later, with the `no_long_atoms` option
selecting the OTP 27-compatible encoding. A new runtime's valid byte-parser fixture cannot
be assumed readable by an older runtime's stripping tool.

The execution tests now compile a tiny documented module using the discovered OTP's own
absolute `erl` path, in a fresh owned directory outside the staged tree. The existing bounded
test executor gives compilation 60 seconds and preserves failure streams. Before copying
the compiled module, the helper requires `Code`, `Dbgi`, and `Docs`; the tests retain their
existing assertions on debug/documentation removal, code preservation, every file in the
400-module set, unchanged compiler output, and untouched sibling directories. No production
code or committed parser fixture was changed.

An independent, network-disabled OTP 27 container comparison is retained as
`.cache/assurance/F1/linux/beam-fixture-probe.sh` and `beam-fixture-probe.log`. Within a
45-second process bound, it compiled the same source, observed both debug/documentation
chunks, stripped it successfully, and observed `Code` and `Line` remaining while `Dbgi`
and `Docs` were absent. The original OTP 29 fixture reproduced the atom decoding failure
with exit 1 in the same container; the comparison command exited 0. An earlier probe
reader forgot that `beam_lib` emits gzip and is retained separately in
`beam-fixture-probe-reader-error.log`; correcting that evidence reader required no product
change. Coordinated Linux Cargo validation of the four corrected regressions belongs to
the developer agent's subsequent qualification run, not this standalone probe.
