// SPDX-License-Identifier: MIT OR Apache-2.0
//! The macOS job passed `ginary build` a `--erts` flag that has never existed,
//! so the job that closes the D3 Mac-runner gap died at argument parsing.
//!
//! **What went wrong.** E5 fixed the step ordering, so the macOS job finally
//! ran the *command line tool* rather than the launcher stub. It got one line
//! further:
//!
//! ```text
//! error: unexpected argument '--erts' found
//!
//!   tip: a similar argument exists: '--verbose'
//!
//! Usage: ginary build --target <TARGET> --verbose...
//! ##[error]Process completed with exit code 2.
//! ```
//!
//! (`macOS build, launch and signature (macos-14, macos-aarch64)`
//! <https://github.com/P4suta/ginary/actions/runs/33681144884/job/100417745900>).
//! There is no `--erts` on `ginary build` and there never was: an ERTS source
//! is a `gleam.toml` setting, `[tools.ginary] erts` or
//! `[tools.ginary.target."<target>"] erts`, and `scripts/smoke-matrix.sh`
//! appends exactly that to the fixture's config before it builds. The macOS
//! job invented a flag instead.
//!
//! So for the second run in a row, nothing the job exists for happened: no
//! artifact was packaged, none was run, `payload::locate` never looked for the
//! `__GINARY,__payload` section in a Mach-O a real macOS toolchain produced,
//! and `codesign --verify --strict` never saw the ad-hoc signature
//! `src/sign_macos.rs` writes. Whether that signature is acceptable to the
//! kernel is still unknown, and `docs/dev/log/D3.md` still awaits a Mac.
//!
//! **The input.** Any `run:` block that passes ginary a long flag the CLI does
//! not define. Nothing type-checks a workflow, so the first proof is the job
//! failing, twenty-five minutes of runner time after the mistake was made.
//!
//! **The correct behaviour.** Every long flag a committed workflow passes to
//! ginary is one that ginary's own `--help` for that subcommand lists. The
//! check is against the binary this test run built rather than against a
//! transcription of the CLI, so a flag that is renamed or withdrawn takes the
//! workflows with it.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::Command;

use crate::common::repo::{
    GinaryInvocation, ginary_invocations, parse_ginary_command, shell_scripts_under,
    yaml_files_under,
};

/// One parser case: a command line, and the subcommand path and long flags it
/// is expected to yield — or `None` when the line does not run ginary at all.
type Case = (&'static str, Option<(Vec<&'static str>, Vec<&'static str>)>);

#[test]
fn a_ginary_command_line_yields_its_subcommand_path_and_its_long_flags() {
    let cases: [Case; 6] = [
        (
            // The macOS step, as `WorkflowStep::commands` joins it.
            "\"$GITHUB_WORKSPACE/target/release/ginary\" build --target \"macos-aarch64\" \
             --erts \"dir:${otp_root}\" )",
            Some((vec!["build"], vec!["--target", "--erts"])),
        ),
        (
            // A nested subcommand behind `cargo run`.
            "cargo run --quiet --locked -- otp repack --upstream-tag OTP-29.0.5 --out dist/otp",
            Some((vec!["otp", "repack"], vec!["--upstream-tag", "--out"])),
        ),
        (
            "\"$GITHUB_WORKSPACE/target/release/ginary\" inspect --json=pretty artifact",
            Some((vec!["inspect"], vec!["--json"])),
        ),
        // Copying the binary is not running it.
        ("cp target/release/ginary dist/stubs/ginary-linux", None),
        // Neither is building something else with a flag ginary happens to have.
        (
            "cross build --release --locked --no-default-features --target x86_64-unknown-linux-musl",
            None,
        ),
        // Nor is running a packaged artifact.
        ("\"$artifact\" 0 hello world", None),
    ];

    for (line, expected) in cases {
        let parsed = parse_ginary_command(line);
        let expected = expected.map(|(path, flags)| {
            (
                path.into_iter().map(str::to_owned).collect::<Vec<_>>(),
                flags.into_iter().map(str::to_owned).collect::<Vec<_>>(),
            )
        });
        assert_eq!(parsed, expected, "parsing `{line}`");
    }
}

/// The parser cases a committed shell script contributes: a command behind
/// `if !`, behind a subshell, behind a variable assignment, and one whose
/// program word is an interpolation.
type ScriptCase = (&'static str, Option<(Vec<&'static str>, Vec<&'static str>)>);

#[test]
fn a_command_behind_if_a_negation_or_an_assignment_still_names_ginary() {
    // Every one of these is a line CI actually executes:
    // `scripts/smoke-matrix.sh` and `scripts/smoke.sh` are `run:` steps of
    // `smoke-matrix` and `smoke`. A parser that only recognises a command
    // whose *first* word is the program sees none of them, so the flags they
    // pass were unguarded by the very test that exists for a bad flag.
    let cases: [ScriptCase; 5] = [
        (
            "if ! (cd \"$work/$app\" && GINARY_CATALOG=\"$catalog\" ginary build \
             --target \"$target\" --stub \"$stub\") > \"$work/$target.build.log\" 2>&1; then",
            Some((vec!["build"], vec!["--target", "--stub"])),
        ),
        (
            "if ! ginary verify \"$artifact\" > \"$work/$target.verify.log\" 2>&1; then",
            Some((vec!["verify"], vec![])),
        ),
        (
            "  (cd \"$work/$app\" && \"$GINARY_BIN\" build)",
            Some((vec!["build"], vec![])),
        ),
        // A diagnostic that quotes the command it is about runs nothing.
        (
            "    fail \"$target: \\`ginary verify\\` found something; see $log\"",
            None,
        ),
        // Neither does a comment that names it.
        ("# ginary build --target linux-x86_64-musl", None),
    ];

    for (line, expected) in cases {
        let parsed = parse_ginary_command(line);
        let expected = expected.map(|(path, flags)| {
            (
                path.into_iter().map(str::to_owned).collect::<Vec<_>>(),
                flags.into_iter().map(str::to_owned).collect::<Vec<_>>(),
            )
        });
        assert_eq!(parsed, expected, "parsing `{line}`");
    }
}

#[test]
fn every_long_flag_a_workflow_or_a_script_passes_to_ginary_is_one_the_cli_accepts() {
    let mut invocations = Vec::new();
    for file in yaml_files_under(".github/workflows")
        .into_iter()
        .chain(shell_scripts_under("scripts"))
    {
        invocations.extend(ginary_invocations(&file));
    }

    // Two sentinels rather than a count. `invocations.len() >= 2` was
    // satisfied by four unrelated `cargo run -- otp repack` lines, so a parser
    // change that stopped recognising the very line this test was written for
    // would have shrunk the subject silently. These two are the lines that
    // package an artifact — the macOS step whose invented `--erts` is the
    // defect above, and the matrix script's build, which no scan read at all
    // until the scripts were added.
    for (source, needle) in [
        (".github/workflows/ci.yml", "release/ginary"),
        ("scripts/smoke-matrix.sh", "ginary build"),
    ] {
        assert!(
            invocations
                .iter()
                .any(|found| found.source == source && found.line.contains(needle)),
            "the scan found no `{needle}` in {source}, so whatever it is checking is not the \
             command that packages an artifact. It found:\n{}",
            invocations
                .iter()
                .map(GinaryInvocation::to_string)
                .collect::<Vec<_>>()
                .join("\n")
        );
    }

    let mut offenders = Vec::new();
    for invocation in &invocations {
        let accepted = accepted_long_flags(&invocation.path);
        for flag in &invocation.long_flags {
            if !accepted.contains(flag) {
                offenders.push(format!(
                    "{invocation}\n  `{flag}` is not a flag of `ginary {}`; it accepts {:?}",
                    invocation.path.join(" "),
                    accepted
                ));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "a workflow or a script passes ginary a flag the CLI does not define. Nothing \
         type-checks a `run:` block or a `.sh`, so the first proof is a job that dies at \
         argument parsing:\n{}",
        offenders.join("\n")
    );
}

/// Every long flag `ginary <path..> --help` lists.
///
/// The help of the binary this test run built, not a transcription of it: a
/// flag that is renamed has to take the workflows with it, and a list written
/// down here would be one more thing to keep in step.
///
/// # Panics
///
/// If the binary cannot be run, or if the subcommand path is not one it has —
/// both of which are the assertion this helper exists for.
fn accepted_long_flags(path: &[String]) -> BTreeSet<String> {
    let binary = PathBuf::from(env!("CARGO_BIN_EXE_ginary"));
    let output = Command::new(&binary)
        .args(path)
        .arg("--help")
        .output()
        .unwrap_or_else(|error| panic!("cannot run {}: {error}", binary.display()));
    assert!(
        output.status.success(),
        "`ginary {} --help` failed: there is no such subcommand\n{}",
        path.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    let help = String::from_utf8_lossy(&output.stdout);
    help.split(|c: char| c.is_whitespace() || c == ',' || c == '<')
        .filter(|word| word.starts_with("--") && word.len() > 2)
        .map(|word| {
            word.trim_end_matches(['.', ')', ']'])
                .split('=')
                .next()
                .unwrap_or(word)
                .to_owned()
        })
        .collect()
}
