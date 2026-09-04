// SPDX-License-Identifier: MIT OR Apache-2.0
//! The Windows exit-code probe proved its point and then failed the job with
//! the very number it had just proved right.
//!
//! **What went wrong.** `.github/workflows/ci.yml` asserted the one platform
//! fact D2 said needed a real Windows host — that `erl.exe` propagates
//! `halt(N)` — with three lines under `shell: pwsh`:
//!
//! ```text
//! erl -noshell -eval "halt(3)"
//! if ($LASTEXITCODE -ne 3) { throw "expected ERRORLEVEL 3, got $LASTEXITCODE" }
//! ```
//!
//! The first time that step ever ran it failed, and it failed *silently*:
//! `##[error]Process completed with exit code 1.` was the only line it
//! produced, no Erlang banner, no `expected ERRORLEVEL 3, got …` message, in
//! 0.54 s. (`Windows build and exit-code propagation`,
//! <https://github.com/P4suta/ginary/actions/runs/33864729638/job/100996872499>.)
//!
//! Silence is the whole diagnosis. A `pwsh` step whose command cannot be found
//! prints the `The term 'erl' is not recognized as a name of a cmdlet …`
//! block; one whose `throw` fires prints the thrown message under a `Line |`
//! gutter. The log holds neither, so `erl.exe` *was* found, it ran, and it
//! left exactly `3` behind — the platform fact holds. What failed the step is
//! what GitHub appends to every `pwsh` script:
//! `if ((Test-Path -LiteralPath variable:\LASTEXITCODE)) { exit $LASTEXITCODE }`.
//! The probe deliberately ends with `$LASTEXITCODE` at 3, the epilogue exits 3,
//! and under `pwsh -command ". '<file>'"` any non-zero exit from the sourced
//! script comes back as 1. So the assertion passed and the step reported a
//! failure with nothing to read — the exact shape measured below, which
//! reproduces on this machine's own `pwsh` with an `erl` that does propagate.
//!
//! **The input.** Any `shell: pwsh` step that inspects `$LASTEXITCODE` and
//! ends while it is non-zero. The step need not be about Erlang: the deliberate
//! non-zero exit is the point of every such probe, and the epilogue turns each
//! one into a failed job.
//!
//! **The correct behaviour.** A `pwsh` step that asserts an exit code captures
//! it, says what it saw, and ends with a status of its own — `exit 0` when the
//! assertion held, `exit 1` (after an `::error::` line) when it did not. The
//! number under test is then evidence rather than the step's own fate, and a
//! failure is a sentence rather than silence.

use std::process::Command;

use crate::common::bounded::run_bounded;
use crate::common::repo::{WorkflowStep, composite_action_steps, workflow_steps, yaml_files_under};
use crate::common::tools::{PWSH_BUDGET, require_working_pwsh};

/// What GitHub appends to every `shell: pwsh` script, verbatim.
///
/// Documented under "Exit codes and error action preference"; it is what makes
/// a failing native command fail the step, and what fails a step that ends
/// with a code it only meant to assert.
const EPILOGUE: &str =
    r"if ((Test-Path -LiteralPath variable:\LASTEXITCODE)) { exit $LASTEXITCODE }";

/// The step's script as the failing run ran it, minus the epilogue.
const AS_IT_FAILED: &str = "erl -noshell -eval \"halt(3)\"\n\
                            if ($LASTEXITCODE -ne 3) { throw \"expected ERRORLEVEL 3, got \
                            $LASTEXITCODE\" }\n";

/// The same assertion, ending with a status of its own.
const AS_IT_SHOULD_BE: &str = "& $erl -noshell -eval \"halt(3)\"\n\
                               $code = $LASTEXITCODE\n\
                               Write-Host \"erl halt(3) left exit code $code\"\n\
                               if ($code -ne 3) { Write-Host \"::error::expected 3, got $code\"; \
                               exit 1 }\n\
                               exit 0\n";

/// The lines of `script` that do something, blanks and comments dropped.
///
/// Pure, and the reason the rule below reads the *last* one: PowerShell's
/// `$LASTEXITCODE` survives `Write-Host` and an `if` that does not run, so
/// what a step leaves behind is decided by the last line that acts, not by the
/// last line in the file.
fn effective_lines(script: &str) -> Vec<&str> {
    script
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect()
}

/// Whether a line ends the step on a status of the step's own choosing.
///
/// Two spellings count, because both leave the epilogue nothing to fail on:
/// an `exit <digits>` of its own, and a reset of `$LASTEXITCODE` to zero.
fn ends_on_its_own_status(line: &str) -> bool {
    let line = line.trim();
    if let Some(rest) = line.strip_prefix("exit ") {
        return !rest.trim().is_empty() && rest.trim().chars().all(|c| c.is_ascii_digit());
    }
    let reset = line.replace(' ', "");
    reset == "$LASTEXITCODE=0" || reset == "$global:LASTEXITCODE=0"
}

/// Whether a script asserts an exit code and then leaves it as the step's own.
///
/// Syntactic on purpose: the question is about the text GitHub wraps, and the
/// scanner is calibrated below on the script that failed and on the shape that
/// replaces it, so a rule that answered "no defect" for everything could not
/// pass.
fn ends_with_the_code_it_asserts(script: &str) -> bool {
    if !script.contains("$LASTEXITCODE") {
        return false;
    }
    !effective_lines(script)
        .last()
        .is_some_and(|line| ends_on_its_own_status(line))
}

/// Runs `script` the way a `shell: pwsh` step is run, or `None` with a printed
/// skip on a machine with no working PowerShell.
///
/// The prologue and the epilogue are GitHub's, the invocation is GitHub's
/// `pwsh -command ". '<file>'"`, and the two streams come back merged because
/// the claim under test is that the step said *nothing at all*.
///
/// `-NoProfile` is this test's own and not GitHub's. Whether a profile is
/// loaded has no bearing on the collapse under test — what a sourced script's
/// non-zero `exit` becomes to the caller — and a profile that prints a banner,
/// a version notice or a module-load warning would make the emptiness
/// assertion below fail with a message accusing this repository of a defect on
/// somebody's workstation.
///
/// The child runs under [`PWSH_BUDGET`] through [`run_bounded`], for
/// `tests/common/bounded.rs`'s reason: `Command::output` waits forever and
/// inherits this process's standard input, so a `pwsh` that stopped for a
/// prompt would hang the whole test binary with no diagnosis.
fn run_as_a_pwsh_step(script: &str) -> Option<(Option<i32>, String)> {
    let pwsh = require_working_pwsh()?;
    let dir = tempfile::tempdir().expect("a temporary directory");
    let file = dir.path().join("step.ps1");
    std::fs::write(
        &file,
        format!("$ErrorActionPreference = 'stop'\n{script}{EPILOGUE}\n"),
    )
    .expect("the step script is written");
    let mut command = Command::new(&pwsh);
    command
        .arg("-NoProfile")
        .arg("-command")
        .arg(format!(". '{}'", single_quoted(&file.to_string_lossy())));
    let output = run_bounded(&mut command, PWSH_BUDGET, "the pwsh step");
    let mut said = String::from_utf8_lossy(&output.stdout).into_owned();
    said.push_str(&String::from_utf8_lossy(&output.stderr));
    Some((output.status.code(), said))
}

/// `text` as the body of a PowerShell single-quoted string.
///
/// An apostrophe is legal in a POSIX path and `tempfile::tempdir` honours
/// `TMPDIR`, so a temporary directory under `/home/o'brien` would otherwise
/// end the string early: pwsh would report a parse error, and the assertions
/// below would read that as the mechanism they are about having disappeared.
/// Doubling is PowerShell's own escape inside single quotes.
fn single_quoted(text: &str) -> String {
    text.replace('\'', "''")
}

/// A command that exits 3 without printing anything, for this host's shell.
///
/// The mechanism under test is PowerShell's, not Erlang's: what matters is
/// that the last native command in the step left 3 behind, and the shortest
/// program that does that is the honest stand-in for `erl -noshell -eval
/// "halt(3)"` on a machine that has no Windows Erlang.
fn exits_three() -> &'static str {
    if cfg!(windows) {
        "cmd /c \"exit 3\""
    } else {
        "/bin/sh -c \"exit 3\""
    }
}

/// Every step the runner executes, in file order: the workflows' and the
/// composite actions'.
///
/// A composite action's steps carry a `shell:` of their own and are wrapped by
/// the same runner in the same way, so a scan that reads `.github/workflows`
/// alone would let a `shell: pwsh` step added under `.github/actions` walk
/// into the identical trap unseen.
fn all_steps() -> Vec<WorkflowStep> {
    let mut out = Vec::new();
    for workflow in yaml_files_under(".github/workflows") {
        out.extend(workflow_steps(&workflow));
    }
    for action in yaml_files_under(".github/actions") {
        out.extend(composite_action_steps(&action));
    }
    out
}

/// Whether a step's script is wrapped and appended to by PowerShell.
///
/// `shell: pwsh` is what the repository writes and `powershell` is the Windows
/// PowerShell spelling of the same wrapper; a step with neither is scanned too
/// when its script reads `$LASTEXITCODE`, because a variable only PowerShell
/// defines is the surest sign of a script that will one day be given the shell
/// that defines it.
fn is_powershell(step: &WorkflowStep) -> bool {
    matches!(step.shell.as_str(), "pwsh" | "powershell")
}

#[test]
fn no_pwsh_step_ends_with_the_exit_code_it_asserts() {
    // The scanner, calibrated on the two shapes this bug is about.
    assert!(
        ends_with_the_code_it_asserts(AS_IT_FAILED),
        "the scanner has to see the script that failed run 33864729638 as a defect, or it passes \
         on the tree that produced it"
    );
    assert!(
        !ends_with_the_code_it_asserts(AS_IT_SHOULD_BE),
        "and it has to leave alone a probe that ends on a status of its own"
    );
    assert!(
        !ends_with_the_code_it_asserts("cargo test --locked --no-fail-fast\n"),
        "a step that never inspects $LASTEXITCODE is not asserting an exit code, and its own \
         non-zero exit is exactly what should fail the job"
    );

    // Why the rule exists, measured rather than argued. `pwsh` is on the
    // hosted Linux and Windows runners, so this runs in CI; a machine without
    // one prints a skip and the rule below is still checked.
    if let Some((code, said)) = run_as_a_pwsh_step(&format!(
        "{}\nif ($LASTEXITCODE -ne 3) {{ throw \"expected 3, got $LASTEXITCODE\" }}\n",
        exits_three()
    )) {
        assert_ne!(
            code,
            Some(0),
            "the mechanism this rule is about is gone: a pwsh step whose assertion held and \
             whose $LASTEXITCODE is 3 now succeeds. Observed exit {code:?}, output {said:?}"
        );
        assert!(
            said.is_empty(),
            "and it fails with nothing to read, which is why the log of job 100996872499 was \
             empty. Observed output: {said:?}"
        );
    }
    if let Some((code, said)) = run_as_a_pwsh_step(&format!(
        "{}\n$code = $LASTEXITCODE\nWrite-Host \"left exit code $code\"\nif ($code -ne 3) \
         {{ Write-Host \"::error::expected 3, got $code\"; exit 1 }}\nexit 0\n",
        exits_three()
    )) {
        assert_eq!(
            code,
            Some(0),
            "a probe that captures the code, says it and ends on `exit 0` passes the step it \
             just proved right. Output: {said:?}"
        );
        assert!(
            said.contains("left exit code 3"),
            "and it says the number it saw, so a future failure is a sentence rather than \
             silence. Output: {said:?}"
        );
    }

    // The tree: every workflow step and every composite-action step that
    // PowerShell will wrap, plus any step that reads `$LASTEXITCODE` whatever
    // shell it names.
    let steps = all_steps();
    assert!(
        steps
            .iter()
            .any(|step| step.workflow.starts_with(".github/actions/")),
        "the scan reads no composite action, so `.github/actions` is either empty or is being \
         parsed as a workflow — and a `shell: pwsh` step added there would walk into this trap \
         unwatched"
    );
    let mut offenders = Vec::new();
    let mut wrong_shell = Vec::new();
    let mut checked = 0usize;
    for step in steps {
        if !step.run.contains("$LASTEXITCODE") {
            continue;
        }
        checked += 1;
        if !step.shell.is_empty() && !is_powershell(&step) {
            wrong_shell.push(format!("{step} (shell `{}`):\n{}", step.shell, step.run));
            continue;
        }
        if ends_with_the_code_it_asserts(&step.run) {
            offenders.push(format!("{step} (shell `{}`):\n{}", step.shell, step.run));
        }
    }
    assert!(
        checked > 0,
        "no workflow asserts an exit code any more, so this rule is measuring nothing — and the \
         Windows exit-code contract is the one thing this repository proves end to end"
    );
    assert!(
        wrong_shell.is_empty(),
        "`$LASTEXITCODE` is PowerShell's, and a step that names another shell expands it to \
         nothing: the comparison passes or fails on the empty string rather than on the code the \
         step meant to check:\n{}",
        wrong_shell.join("\n")
    );
    assert!(
        offenders.is_empty(),
        "a `shell: pwsh` step that ends while $LASTEXITCODE is non-zero is failed by the \
         `{EPILOGUE}` GitHub appends, whatever its own assertions concluded, and it fails with \
         nothing on either stream:\n{}",
        offenders.join("\n")
    );
}
