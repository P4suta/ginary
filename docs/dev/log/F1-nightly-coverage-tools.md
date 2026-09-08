<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# F1 — matching LLVM tools for local nightly branch coverage

The installed Windows nightly was `rustc 1.98.0-nightly (4c9d2bfe4 2026-07-01)`, host
`x86_64-pc-windows-msvc`, LLVM `22.1.8`. Its installed component list did not contain LLVM
tools. The current rustc binary was:

```text
C:\Users\livec\.rustup\toolchains\nightly-x86_64-pc-windows-msvc\bin\rustc.exe
```

The authorized prerequisite was installed through rustup's component operation, using
`require_escalated` because the toolchain is outside the workspace:

```powershell
& 'C:\Users\livec\.cargo\bin\rustup.exe' component add llvm-tools-preview --toolchain nightly
```

The operation exited 0. Automatic approval review did not reject it. A subsequent
`rustup component list --toolchain nightly --installed` included
`llvm-tools-x86_64-pc-windows-msvc`. The exact installed tools are:

```text
C:\Users\livec\.rustup\toolchains\nightly-x86_64-pc-windows-msvc\lib\rustlib\x86_64-pc-windows-msvc\bin\llvm-cov.exe
C:\Users\livec\.rustup\toolchains\nightly-x86_64-pc-windows-msvc\lib\rustlib\x86_64-pc-windows-msvc\bin\llvm-profdata.exe
```

Both `--version` probes exited 0 in the ordinary workspace sandbox and reported
`LLVM version 22.1.8-rust-1.98.0-nightly`, matching the existing nightly compiler.
No alternate LLVM installation or replacement toolchain is needed.

No Cargo build, test or coverage measurement was run as part of this prerequisite task.
The default toolchain was not changed. The required 90% line and 80% branch coverage gates
were left unchanged; successful tool installation is not coverage evidence.

## Verified runner contract and evidence boundary

The installed `cargo-llvm-cov` is 0.8.7; CI pins 0.9.0. Its report help misleadingly lists
`--all-features`, which the real parser rejects. The supported commands are
`show-env --pwsh --doctests` and `report --doctests --lcov --output-path <file> --locked --offline`.
Keep `--all-features` on the instrumented `cargo test` command only; reporting reads the
compiled objects and profiles. Its `show-env` and `report` subcommands do not accept
`--branch`. The installed nightly compiler supports `-Zcoverage-options=branch`, which is
appended to both Rust and rustdoc flags after environment generation. Encoded flags, when
present, use ASCII unit separator rather than a space. `--doctests` belongs on both environment
generation and reporting; this nightly no longer lists the old Cargo
`-Z doctest-in-workspace` option.

`CARGO_TARGET_DIR` and `CARGO_LLVM_COV_TARGET_DIR` name the same fresh directory. The outcome
adapter first enumerates tests and then executes the instrumented suite. Reporting runs even
after test failure, and the original test exit remains a separate required verdict. Line and
branch gates run independently against the resulting LCOV. Neither a successful report nor
a passing floor can hide a failed or incomplete test run.

The exact portable PowerShell recipe is in `docs/dev/testing.md`. The root prepared the
workspace-local `.cache/assurance/F1/run-nightly-coverage.ps1` for the final measurement, with
matched explicit `LLVM_COV` and `LLVM_PROFDATA` paths. That script's existence is not execution
evidence. When run, its intended evidence directory is
`.cache/assurance/F1/qualification-coverage-nightly/`: `run.json` keeps separate test/report/
line/branch exits, `tests/outcomes.json` classifies planned tests, `tests-driver.log` and
`report.log` preserve command output, and `branch.lcov`, `line-gate.log`, `branch-gate.log`
record the actual measurement. The root `F1.md` owns the final observed result.

The assurance task separately executed 15 coverage-gate fixtures and the coverage helper's
five mocked command-failure scenarios. Those passed; they validate evidence retention and
gate refusal behavior, without claiming any measured percentage for this project. Local
Windows coverage measures Windows-compiled code and cannot substitute for the Linux job's
required cross stubs and native runtime prerequisites.

## Actual report-option failure and correction

The real all-feature nightly test run completed with 1,983 successful tests, 31 reported
skips and no not-run tests. Its first report command then exited 1 with
`invalid option '--all-features' for subcommand 'report'`. The original report log and raw
profiles were preserved. A fresh-shell retry with no `CARGO_LLVM_COV_SHOW_ENV` value produced
the same rejection, so this was not an inherited instrumentation-variable defect.

The [0.8.7 parser](https://github.com/taiki-e/cargo-llvm-cov/blob/v0.8.7/src/cli.rs#L1126)
and [CI's 0.9.0 parser](https://github.com/taiki-e/cargo-llvm-cov/blob/v0.9.0/src/cli.rs#L1126)
accept feature selection only for commands that build or run tests. Read-only local probes
with the option before `--help` reproduced rejection for SHOW_ENV values 0 and 1, while
`--locked` and `--offline` were accepted. Evidence is
`.cache/assurance/F1/coverage-report-option-probe.json`. The earlier help-only interpretation
was insufficient and has been corrected in the documented recipe and CI helper.

The helper mock now enforces this report-parser contract while still requiring all features
on the test command. All five coverage scenarios failed against the old helper before the
report-only flag was removed. No Cargo build or instrumented test was repeated for this
script correction; the root rerenders the preserved profiles and records actual gate results.
The corrected helper passed all eleven Python tests in `coverage-report-contract-green.log`
and actual ShellCheck in `actionlint-coverage-report-contract-shellcheck.json`; the five
before-fix failures are in `coverage-report-contract-red.log`, all below
`.cache/assurance/F1/`. Its SHA-256 changed from
`9f714c6c46e2a839b6f78b69e85a85f38659b8747d6b9605122568b5e5f0e6c5` to
`d38c0912e0f0bcc3187e60fd0c64a7c6e6928c2074ef231736bae9eb3a88b465`.
