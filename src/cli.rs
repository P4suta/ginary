// SPDX-License-Identifier: MIT OR Apache-2.0
//! The `ginary` command line interface.
//!
//! Only the commands that are actually implemented appear here. The interface
//! is meant to grow (`build`, `stage`, `inspect`, `verify`, `cache`, `otp` are
//! all planned), so the derive layout keeps one variant per command with its
//! own flags rather than a shared flag bag.

use std::io::Write;

use anyhow::Context as _;
use clap::{Parser, Subcommand};
use serde::Serialize;

use crate::doctor;
use crate::target::Target;

/// Version of the `version --json` schema.
pub const VERSION_FORMAT_VERSION: u32 = 1;

/// The ginary command line.
#[derive(Debug, Parser)]
#[command(
    name = "ginary",
    version,
    about = "Package a Gleam application and a trimmed BEAM runtime into one executable",
    long_about = "\
ginary packages the output of `gleam export erlang-shipment` together with a trimmed
BEAM runtime into a single executable, so that the people who run a Gleam program do
not need Erlang installed.

Status: pre-alpha. The `build` command that produces those executables is not
implemented yet; this version ships `version` and `doctor` only.",
    arg_required_else_help = true
)]
pub struct Cli {
    /// The command to run.
    #[command(subcommand)]
    pub command: Command,
}

/// An implemented ginary command.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Print the ginary version and the target it was built for.
    Version {
        /// Print a JSON object instead of a human-readable line.
        #[arg(long)]
        json: bool,
    },
    /// Report the toolchain, host target and cache directory ginary found.
    Doctor {
        /// Print a JSON object instead of human-readable lines.
        #[arg(long)]
        json: bool,
    },
}

/// The payload of `ginary version --json`.
#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct VersionReport {
    /// The ginary crate version.
    pub version: &'static str,
    /// The target ginary was built for.
    pub target: Target,
    /// Version of this schema; see [`VERSION_FORMAT_VERSION`].
    pub format_version: u32,
}

impl VersionReport {
    /// Builds the report for the running binary.
    pub fn current() -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION"),
            target: Target::host(),
            format_version: VERSION_FORMAT_VERSION,
        }
    }

    /// Renders the human-readable line, `ginary <version> (<target>)`.
    pub fn render_text(&self) -> String {
        format!("ginary {} ({})\n", self.version, self.target)
    }
}

/// Parses the process arguments and runs the requested command.
///
/// Argument errors are reported by clap itself, which exits with status 2.
pub fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let stdout = std::io::stdout();
    dispatch(&cli.command, &mut stdout.lock())
}

/// Runs one command, writing its output to `out`.
///
/// Taking the writer as a parameter keeps the commands testable without
/// capturing the process' standard output.
pub fn dispatch(command: &Command, out: &mut impl Write) -> anyhow::Result<()> {
    match command {
        Command::Version { json } => write_version(&VersionReport::current(), *json, out),
        Command::Doctor { json } => write_doctor(&doctor::Report::gather(), *json, out),
    }
}

/// Writes a version report in the requested form.
///
/// Rendering is separated from [`VersionReport::current`] so that tests can
/// assert on the output of a report they built themselves.
fn write_version(report: &VersionReport, json: bool, out: &mut impl Write) -> anyhow::Result<()> {
    if json {
        write_json(out, report)
    } else {
        out.write_all(report.render_text().as_bytes())
            .context("cannot write the version to standard output")
    }
}

/// Writes a diagnosis in the requested form.
///
/// Rendering is separated from [`doctor::Report::gather`] so that tests can
/// assert on the output of a report they built themselves, without running the
/// external programs `gather` probes.
fn write_doctor(report: &doctor::Report, json: bool, out: &mut impl Write) -> anyhow::Result<()> {
    if json {
        write_json(out, report)
    } else {
        out.write_all(report.render_text().as_bytes())
            .context("cannot write the diagnosis to standard output")
    }
}

/// Writes a value as pretty JSON followed by a newline.
fn write_json(out: &mut impl Write, value: &impl Serialize) -> anyhow::Result<()> {
    let mut json = serde_json::to_vec_pretty(value).context("cannot serialise the report")?;
    json.push(b'\n');
    out.write_all(&json)
        .context("cannot write JSON to standard output")
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory as _;

    use super::*;

    #[test]
    fn the_command_line_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn the_long_help_mentions_the_planned_build_command() {
        let help = Cli::command().render_long_help().to_string();
        assert!(help.contains("build"), "long help:\n{help}");
    }

    #[test]
    fn no_arguments_is_an_error_that_exits_with_two() {
        let error = Cli::try_parse_from(["ginary"]).expect_err("a subcommand is required");
        assert_eq!(error.exit_code(), 2, "{error}");
    }

    #[test]
    fn an_unknown_subcommand_is_an_error() {
        assert!(Cli::try_parse_from(["ginary", "build"]).is_err());
    }

    #[test]
    fn version_and_doctor_parse_with_and_without_json() {
        assert!(matches!(
            Cli::try_parse_from(["ginary", "version"])
                .expect("parses")
                .command,
            Command::Version { json: false }
        ));
        assert!(matches!(
            Cli::try_parse_from(["ginary", "version", "--json"])
                .expect("parses")
                .command,
            Command::Version { json: true }
        ));
        assert!(matches!(
            Cli::try_parse_from(["ginary", "doctor"])
                .expect("parses")
                .command,
            Command::Doctor { json: false }
        ));
        assert!(matches!(
            Cli::try_parse_from(["ginary", "doctor", "--json"])
                .expect("parses")
                .command,
            Command::Doctor { json: true }
        ));
    }

    #[test]
    fn the_version_line_starts_with_the_program_name_and_a_semver() {
        let text = VersionReport::current().render_text();
        let version = text
            .strip_prefix("ginary ")
            .and_then(|rest| rest.split_whitespace().next())
            .expect("a version token");
        assert_eq!(version.split('.').count(), 3, "not a semver: {version}");
        assert!(text.ends_with(&format!("({})\n", Target::host())), "{text}");
    }

    #[test]
    fn version_json_carries_the_documented_keys() {
        let mut out = Vec::new();
        write_version(&VersionReport::current(), true, &mut out).expect("writes");
        let value: serde_json::Value = serde_json::from_slice(&out).expect("valid JSON");
        assert_eq!(value["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(value["target"], Target::host().name());
        assert_eq!(value["format_version"], 1);
        assert!(out.ends_with(b"\n"));
    }

    /// A diagnosis with one entry per probed program, built by hand.
    ///
    /// These tests assert on rendering only. Probing the real machine is the
    /// job of `tests/smoke_cli.rs`, which drives the built binary; a unit test
    /// that shelled out to `gleam`, `erl`, `strip` and `docker` would depend on
    /// the developer's `PATH` and could spend the whole probe budget waiting
    /// for an unresponsive daemon.
    fn sample_doctor_report() -> doctor::Report {
        doctor::Report {
            format_version: doctor::FORMAT_VERSION,
            host_target: Target::host(),
            rustc_required: false,
            cache_dir: Some(std::path::PathBuf::from("/home/u/.cache/ginary")),
            cache_dir_source: Some("HOME"),
            cache_dir_error: None,
            tools: ["gleam", "erl", "strip", "docker"]
                .into_iter()
                .map(|name| doctor::ToolReport {
                    name: name.to_owned(),
                    found: false,
                    version: None,
                    path: None,
                })
                .collect(),
        }
    }

    #[test]
    fn doctor_json_carries_the_documented_keys() {
        let mut out = Vec::new();
        write_doctor(&sample_doctor_report(), true, &mut out).expect("writes");
        let value: serde_json::Value = serde_json::from_slice(&out).expect("valid JSON");
        assert_eq!(value["format_version"], 1);
        assert_eq!(value["rustc_required"], false);
        assert_eq!(value["host_target"], Target::host().name());
        assert_eq!(value["tools"].as_array().expect("tools").len(), 4);
        assert!(out.ends_with(b"\n"));
    }

    #[test]
    fn doctor_text_names_every_subject() {
        let mut out = Vec::new();
        write_doctor(&sample_doctor_report(), false, &mut out).expect("writes");
        let text = String::from_utf8(out).expect("utf-8");
        for subject in [
            "host target:",
            "rustc/cargo: not required",
            "cache dir:",
            "gleam:",
            "erl:",
            "strip:",
            "docker:",
        ] {
            assert!(text.contains(subject), "missing {subject} in:\n{text}");
        }
    }
}
