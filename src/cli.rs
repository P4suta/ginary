// SPDX-License-Identifier: MIT OR Apache-2.0
//! The `ginary` command line interface.
//!
//! Only the commands that are actually implemented appear here. The interface
//! is meant to grow (`build`, `stage`, `inspect`, `verify`, `cache`, `otp` are
//! all planned), so the derive layout keeps one variant per command with its
//! own flags rather than a shared flag bag.

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use clap::{Parser, Subcommand};
use serde::Serialize;

use crate::appfile::{self, AppResource};
use crate::closure::{self, AppSet};
use crate::doctor;
use crate::otp;
use crate::target::Target;

/// Version of the `version --json` schema.
pub const VERSION_FORMAT_VERSION: u32 = 1;

/// Version of the `appfile parse --json` schema.
pub const APPFILE_FORMAT_VERSION: u32 = 1;

/// Version of the `closure --json` schema.
pub const CLOSURE_FORMAT_VERSION: u32 = 1;

/// Width of the label column in the `appfile parse` table.
///
/// `included_applications` is the longest label, so every value starts in the
/// same column and the table stays readable when a file is missing a key.
const APPFILE_LABEL_WIDTH: usize = 21;

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
implemented yet; this version ships `version`, `doctor`, `appfile` and `closure`
only.",
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
    /// Read OTP application resource files.
    Appfile {
        /// What to do with the files.
        #[command(subcommand)]
        command: AppfileCommand,
    },
    /// Resolve every application a shipment needs, over the shipment and OTP.
    ///
    /// This is the debugging window into what `ginary build` will bundle: it
    /// takes the same two trees, the same roots and the same extras, and
    /// prints the applications it found, where each came from, and why.
    Closure {
        /// The directory `gleam export erlang-shipment` wrote.
        #[arg(value_name = "SHIPMENT")]
        shipment: PathBuf,
        /// The OTP installation to resolve against. Defaults to the one `erl`
        /// on `PATH` reports.
        #[arg(long, value_name = "PATH")]
        otp_root: Option<PathBuf>,
        /// An application to start from. Repeatable, and required: there is no
        /// reliable way to guess which application of a shipment is the one
        /// being packaged.
        #[arg(long = "root", value_name = "NAME", required = true)]
        roots: Vec<String>,
        /// An extra application to bundle, as `extra_applications` and
        /// `otp_applications` do. Repeatable.
        #[arg(long = "extra", value_name = "NAME")]
        extra: Vec<String>,
        /// Print a JSON object instead of a table.
        #[arg(long, conflicts_with = "explain")]
        json: bool,
        /// Print the origin of every application instead of its `ebin` path.
        #[arg(long)]
        explain: bool,
    },
}

/// A subcommand of `ginary appfile`.
#[derive(Debug, Subcommand)]
pub enum AppfileCommand {
    /// Parse `.app` files and print what ginary reads from them.
    ///
    /// This is the debugging window into the closure computation: when
    /// `ginary build` cannot find an application, this shows exactly which
    /// dependencies the `.app` files declare.
    Parse {
        /// The `.app` files to read.
        #[arg(required = true, value_name = "PATH")]
        paths: Vec<PathBuf>,
        /// Print a JSON object instead of a human-readable table.
        #[arg(long)]
        json: bool,
    },
}

/// The payload of `ginary appfile parse --json`.
#[derive(Debug, Serialize)]
pub struct AppfileReport {
    /// Version of this schema; see [`APPFILE_FORMAT_VERSION`].
    pub format_version: u32,
    /// One entry per file, in the order the files were given.
    pub apps: Vec<ParsedApp>,
}

/// One file's worth of [`AppfileReport`].
#[derive(Debug, Serialize)]
pub struct ParsedApp {
    /// The path as it was given on the command line.
    pub path: String,
    /// What the file declares.
    #[serde(flatten)]
    pub resource: AppResource,
}

/// The payload of `ginary closure --json`.
#[derive(Debug, Serialize)]
pub struct ClosureReport {
    /// Version of this schema; see [`CLOSURE_FORMAT_VERSION`].
    pub format_version: u32,
    /// The OTP `lib` directory the closure resolved against.
    pub otp_lib: String,
    /// The applications, the warnings and the skipped optional applications.
    #[serde(flatten)]
    pub apps: AppSet,
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
        Command::Appfile {
            command: AppfileCommand::Parse { paths, json },
        } => write_appfile(paths, *json, out),
        Command::Closure {
            shipment,
            otp_root,
            roots,
            extra,
            json,
            explain,
        } => write_closure(
            shipment,
            otp_root.as_deref(),
            roots,
            extra,
            *json,
            *explain,
            out,
        ),
    }
}

/// Computes a closure and writes it in the requested form.
///
/// The OTP root is resolved through [`otp::discover`], so `--otp-root` gets
/// the same validation the host installation does: a directory that is not an
/// OTP root is refused here rather than producing an empty library listing and
/// a confusing `AppNotFound` three applications later.
fn write_closure(
    shipment: &Path,
    otp_root: Option<&Path>,
    roots: &[String],
    extra: &[String],
    json: bool,
    explain: bool,
    out: &mut impl Write,
) -> anyhow::Result<()> {
    let otp = otp::discover(otp_root)?;
    let apps = closure::app_dependency_closure(shipment, &otp.lib, roots, extra)?;

    if json {
        return write_json(
            out,
            &ClosureReport {
                format_version: CLOSURE_FORMAT_VERSION,
                otp_lib: otp.lib.display().to_string(),
                apps,
            },
        );
    }

    let mut text = if explain {
        closure::explain(&apps)
    } else {
        render_closure_table(&apps)
    };
    text.push_str(&render_closure_notes(&apps));
    out.write_all(text.as_bytes())
        .context("cannot write the closure to standard output")
}

/// Renders the default table: name, version, source and `ebin` directory.
fn render_closure_table(apps: &AppSet) -> String {
    let rows: Vec<[String; 4]> = apps
        .iter()
        .map(|app| {
            [
                app.name.clone(),
                app.vsn.clone(),
                closure::source_label(&app.source).to_owned(),
                app.ebin.display().to_string(),
            ]
        })
        .collect();
    closure::render_table(["name", "vsn", "source", "ebin"], &rows)
}

/// Renders the warnings and the skipped optional applications, if any.
///
/// Both are silent when empty: a heading with nothing under it reads as a
/// finding rather than as its absence.
fn render_closure_notes(apps: &AppSet) -> String {
    let mut text = String::new();
    if !apps.warnings.is_empty() {
        text.push_str("\nwarnings:\n");
        for warning in &apps.warnings {
            text.push_str(&format!("  {warning}\n"));
        }
    }
    if !apps.skipped_optional.is_empty() {
        text.push_str("\nskipped optional applications:\n");
        for (name, requested_by) in &apps.skipped_optional {
            text.push_str(&format!("  {name}, requested by {requested_by}\n"));
        }
    }
    text
}

/// Reads every `.app` file and writes the result in the requested form.
///
/// The first unreadable file stops the command: a partial table followed by an
/// error would be read as a complete answer.
fn write_appfile(paths: &[PathBuf], json: bool, out: &mut impl Write) -> anyhow::Result<()> {
    let mut apps = Vec::with_capacity(paths.len());
    for path in paths {
        let resource = appfile::parse_app_file(path)
            .with_context(|| format!("cannot read the application file `{}`", path.display()))?;
        apps.push(ParsedApp {
            path: path.display().to_string(),
            resource,
        });
    }

    if json {
        return write_json(
            out,
            &AppfileReport {
                format_version: APPFILE_FORMAT_VERSION,
                apps,
            },
        );
    }

    let text: String = apps.iter().map(render_app).collect::<Vec<_>>().join("\n");
    out.write_all(text.as_bytes())
        .context("cannot write the application files to standard output")
}

/// Renders one file as a labelled block ending in a newline.
fn render_app(app: &ParsedApp) -> String {
    let resource = &app.resource;
    let mut text = format!("{}\n", app.path);
    let mut row = |label: &str, value: String| {
        text.push_str(&format!("  {label:APPFILE_LABEL_WIDTH$} {value}\n"));
    };
    row("name", resource.name.clone());
    row("vsn", resource.vsn.clone());
    row(
        "description",
        resource.description.clone().unwrap_or_else(none),
    );
    row("applications", list(&resource.applications));
    row(
        "optional_applications",
        list(&resource.optional_applications),
    );
    row(
        "included_applications",
        list(&resource.included_applications),
    );
    row("modules", list(&resource.modules));
    row("registered", list(&resource.registered));
    row(
        "mod",
        if resource.has_mod { "yes" } else { "no" }.to_owned(),
    );
    row("env keys", list(&resource.env_keys));
    for warning in &resource.warnings {
        text.push_str(&format!("  warning: {warning}\n"));
    }
    text
}

/// Renders a list of names, or `(none)` when it is empty.
fn list(names: &[String]) -> String {
    if names.is_empty() {
        none()
    } else {
        names.join(", ")
    }
}

/// The placeholder for an absent value.
fn none() -> String {
    "(none)".to_owned()
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
            otp: None,
            otp_error: Some("`erl` is not on PATH".to_owned()),
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
