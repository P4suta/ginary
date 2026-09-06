// SPDX-License-Identifier: MIT OR Apache-2.0
//! A workflow step that every local gate passed ran nothing at all.
//!
//! **What went wrong.** The `windows` job's packaging step printed the
//! arguments it had just run, and quoted them the way this repository's prose
//! quotes things:
//!
//! ```text
//! Write-Host "the artifact left exit code $ran for `0 hello world`"
//! ```
//!
//! A backtick is PowerShell's escape character. The trailing one escaped the
//! closing quote, the string ran on to the next `"` several lines later, and
//! `pwsh` refused the file:
//!
//! ```text
//! ParserError: ...ps1:56
//!   56 |   Write-Host "::error::expected 3 from the warm run, got $again"
//!      | The string is missing the terminator: ".
//! ```
//!
//! A parse error is for the whole script, so the step did not run its first
//! line either. It packaged nothing, started nothing and asserted nothing, and
//! reported a failure whose message named a line eleven lines past the fault.
//!
//! **Nothing local could see it.** `actionlint` reads workflow structure and
//! shell-checks `bash`; it does not parse `pwsh`, so the step was clean. The
//! `ci_matrix` rules read the script as *text* — they look for a captured exit
//! code and a comparison, and both were there in a script no shell would ever
//! execute. Every gate on the development host agreed, and the first runner
//! did not.
//!
//! **The input.** Every `shell: pwsh` script in the committed workflows.
//!
//! **The correct behaviour.** A step written in a language is checked by that
//! language's parser. PowerShell exposes its own — `Parser::ParseInput` — and
//! `pwsh` runs on Linux, macOS and Windows, so the rule holds wherever the
//! suite does. What it cannot do is run without a `pwsh`, and there the skip
//! is reported rather than silent, the way every other tool-gated claim here
//! is.

use crate::common::repo::{WorkflowStep, executed_yaml_files, workflow_steps};
use crate::common::tools::{PWSH_BUDGET, require_working_pwsh};

/// The shell whose steps this file parses.
const PWSH: &str = "pwsh";

/// The script that failed, reduced to the fault.
const AS_IT_FAILED: &str = "Write-Host \"code $ran for `0 hello world`\"\n\
                            Write-Host \"::error::and this line is inside that string\"\n";

/// The same line, quoted the way PowerShell quotes.
const AS_IT_SHOULD_BE: &str = "Write-Host \"code $ran for '0 hello world'\"\n\
                               Write-Host \"::error::and this line is its own statement\"\n";

/// Every `shell: pwsh` step of every workflow CI executes.
fn pwsh_steps() -> Vec<WorkflowStep> {
    let mut steps: Vec<WorkflowStep> = Vec::new();
    for workflow in executed_yaml_files() {
        steps.extend(
            workflow_steps(&workflow)
                .into_iter()
                .filter(|step| step.shell == PWSH && !step.run.trim().is_empty()),
        );
    }
    steps
}

/// The parse errors `pwsh` reports for one script, or `None` when there is no
/// usable PowerShell to ask.
///
/// PowerShell's own parser rather than a rule written here: the question is
/// whether `pwsh` will accept the file, and only `pwsh` answers that. The
/// script travels through a file rather than through `-Command`, because that
/// is how GitHub runs a step — `pwsh -command ". '<file>'"` — and quoting a
/// script into a command line would be a second thing to get wrong.
fn parse_errors(script: &str) -> Option<String> {
    let pwsh = require_working_pwsh()?;
    let dir = tempfile::tempdir().expect("a temporary directory");
    let file = dir.path().join("step.ps1");
    std::fs::write(&file, script).expect("write the step");

    // `ParseFile` fills `$errors` and returns the tree; an empty `$errors` is
    // a script PowerShell would run. `-NoProfile` for the reason
    // `require_working_pwsh` gives: a profile can prompt, and a prompt is a
    // hang.
    let probe = format!(
        "$errors = $null; \
         [System.Management.Automation.Language.Parser]::ParseFile('{}', [ref]$null, [ref]$errors) \
         | Out-Null; \
         if ($errors.Count) {{ $errors | ForEach-Object {{ $_.Message }} }}",
        file.display()
    );
    let mut command = std::process::Command::new(pwsh);
    command.args(["-NoProfile", "-NonInteractive", "-Command", &probe]);
    let output = crate::common::bounded::run_bounded(&mut command, PWSH_BUDGET, "the pwsh parser");
    Some(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

#[test]
fn the_parser_this_file_asks_tells_the_two_quotings_apart() {
    let Some(broken) = parse_errors(AS_IT_FAILED) else {
        eprintln!("skipping: no usable pwsh, so the parser cannot be asked");
        return;
    };
    assert!(
        !broken.is_empty(),
        "the line that stopped a runner has to be a parse error here too, or this file is asking \
         the wrong question"
    );

    let sound = parse_errors(AS_IT_SHOULD_BE).expect("the same pwsh answered once already");
    assert!(
        sound.is_empty(),
        "and the corrected quoting has to parse, or the rule is simply a refusal:\n{sound}"
    );
}

#[test]
fn every_committed_pwsh_step_parses() {
    let steps = pwsh_steps();
    assert!(
        steps.len() >= 3,
        "only {} `shell: {PWSH}` steps were found in the committed workflows; this scan has lost \
         its subject",
        steps.len()
    );

    let mut broken: Vec<String> = Vec::new();
    for step in &steps {
        let Some(errors) = parse_errors(&step.run) else {
            eprintln!("skipping: no usable pwsh, so the parser cannot be asked");
            return;
        };
        if !errors.is_empty() {
            broken.push(format!("{step}\n{errors}"));
        }
    }

    assert!(
        broken.is_empty(),
        "a `shell: {PWSH}` step PowerShell cannot parse runs none of itself — not its first line, \
         not the assertion it was written for — and fails naming a line past the fault. \
         `actionlint` does not read {PWSH}, so this is where that is caught:\n{}",
        broken.join("\n\n")
    );
}
