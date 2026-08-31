// SPDX-License-Identifier: MIT OR Apache-2.0
//! The `ginary` command line interface.
//!
//! Only the commands that are actually implemented appear here. The interface
//! is meant to grow (`verify`, `otp` and the cross-target flags are all
//! planned), so the derive layout keeps one variant per command with its own
//! flags rather than a shared flag bag.

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use serde::Serialize;

use crate::appfile::{self, AppResource};
use crate::assemble::{self, StageOptions, StagedRoot};
use crate::beam;
use crate::bundle::{self, BuildReport};
use crate::cache;
use crate::closure::{self, AppSet};
use crate::config::{
    BuildFlags, BuildOptions, MAX_COMPRESSION_LEVEL, MIN_COMPRESSION_LEVEL, ProjectConfig,
};
use crate::crashdump::{self, CrashDump};
use crate::diag::{self, Diag};
use crate::doctor;
use crate::elf::{self, ElfInfo};
use crate::gleam;
use crate::inspect::{self, InspectReport, LaunchPlanReport};
use crate::otp;
use crate::report::{self, SizeReport};
use crate::sbom::{self, SbomDocument};
use crate::strip::{self, StripOptions, StripReport};
use crate::target::Target;
use crate::verify::{self, VerifyReport};

/// Version of the `version --json` schema.
pub const VERSION_FORMAT_VERSION: u32 = 1;

/// Version of the `appfile parse --json` schema.
pub const APPFILE_FORMAT_VERSION: u32 = 1;

/// Version of the `closure --json` schema.
pub const CLOSURE_FORMAT_VERSION: u32 = 1;

/// Version of the `stage --json` schema.
///
/// Two since A2: the object gained the `strip` and `report` members, which a
/// consumer written against version one would not find.
pub const STAGE_FORMAT_VERSION: u32 = 2;

/// Version of the `beam chunks --json` schema.
pub const BEAM_FORMAT_VERSION: u32 = 1;

/// Version of the `elf deps --json` schema.
pub const ELF_FORMAT_VERSION: u32 = 1;

/// Version of the `cache dir --json` and `cache clean --json` schemas.
pub const CACHE_FORMAT_VERSION: u32 = 1;

/// Version of the `stage --report json` schema.
pub const SIZE_REPORT_FORMAT_VERSION: u32 = 1;

/// Version of the `build --report json` schema.
pub const BUILD_FORMAT_VERSION: u32 = 1;

/// Width of the label column in the `elf deps` table.
///
/// `glibc_max` is the longest label, so every value starts in the same column.
const ELF_LABEL_WIDTH: usize = 10;

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

Run `ginary build` in a Gleam project to produce one, `ginary inspect` to read what a
packaged application holds, and `ginary doctor` to see what this machine can do. The
remaining commands — `appfile`, `closure`, `stage`, `beam`, `elf` and `cache` — are
windows onto the individual phases of a build.

Only Linux x86_64 host packaging is implemented; cross-target builds are not.",
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
    /// Package the Gleam project in this directory into one executable.
    ///
    /// Everything from `gleam export erlang-shipment` to the artifact: the
    /// application closure, a trimmed BEAM runtime, the payload and the
    /// launcher, in one file that runs on a machine with no Erlang.
    Build {
        /// Where to write the artifact.
        ///
        /// A directory — an existing one, or a value ending in a separator —
        /// has the application name appended to it; anything else is the
        /// artifact's own path. Defaults to `[tools.ginary] output`, which
        /// itself defaults to `build/ginary`.
        #[arg(long, value_name = "PATH")]
        out: Option<PathBuf>,
        /// Keep the debug information the default removes.
        #[arg(long)]
        no_strip: bool,
        /// Strip only the native binaries, leaving the `.beam` files alone.
        #[arg(long, conflicts_with = "strip_beams_only", conflicts_with = "no_strip")]
        strip_elf_only: bool,
        /// Strip only the `.beam` files, leaving the native binaries alone.
        #[arg(long, conflicts_with = "no_strip")]
        strip_beams_only: bool,
        /// A target to build for. Repeatable.
        ///
        /// `host`, `all` or a canonical name such as `linux-x86_64-gnu`.
        /// Defaults to `[tools.ginary] targets`, which itself defaults to
        /// `host`. A canonical name puts `-<target>` in the artifact's file
        /// name, whichever target it is; `host` selects this machine and
        /// keeps the plain name a bare build writes.
        #[arg(long = "target", value_name = "TARGET")]
        targets: Vec<String>,
        /// The OTP installation to bundle. Defaults to the one `erl` reports.
        #[arg(long, value_name = "PATH")]
        otp_root: Option<PathBuf>,
        /// Reuse the existing `build/erlang-shipment` instead of exporting.
        #[arg(long)]
        skip_export: bool,
        /// Keep the staging work directory and print where it is.
        #[arg(long)]
        keep_staging: bool,
        /// The zstd level the payload is packed at.
        #[arg(
            long,
            value_name = "N",
            value_parser = clap::value_parser!(i32).range(
                MIN_COMPRESSION_LEVEL as i64..=MAX_COMPRESSION_LEVEL as i64,
            ),
        )]
        compression_level: Option<i32>,
        /// An extra application to bundle without starting it. Repeatable.
        #[arg(long = "extra-otp-app", value_name = "NAME")]
        extra_otp_apps: Vec<String>,
        /// A program to stage from the runtime's `bin`. Repeatable.
        #[arg(long = "extra-bin", value_name = "NAME")]
        extra_bins: Vec<String>,
        /// Bundle `epmd` and start the runtime distributed.
        ///
        /// Turns `[tools.ginary] distribution` on; there is no flag that turns
        /// it off, because the absence of a switch is not an instruction.
        #[arg(long)]
        distribution: bool,
        /// An `erl -args_file` to copy into the artifact.
        ///
        /// Overrides `[tools.ginary] vm_args`, and is relative to the working
        /// directory rather than to the project.
        #[arg(long = "vm-args", value_name = "PATH")]
        vm_args: Option<PathBuf>,
        /// A `sys.config` to copy into the artifact.
        ///
        /// Overrides `[tools.ginary] sys_config`, and is relative to the
        /// working directory rather than to the project.
        #[arg(long = "sys-config", value_name = "PATH")]
        sys_config: Option<PathBuf>,
        /// The form the build report takes.
        #[arg(long, value_name = "FORMAT", default_value = "text")]
        report: ReportFormat,
        /// Print the closure and staging accounts before the report.
        #[arg(long)]
        explain: bool,
        /// Write an SPDX 2.3 bill of materials beside the artifact.
        #[arg(long)]
        sbom: bool,
        /// Where the bill of materials goes. Implies `--sbom`.
        #[arg(long = "sbom-out", value_name = "PATH")]
        sbom_out: Option<PathBuf>,
        /// Say what each phase is doing, on standard error.
        #[arg(short = 'v', long = "verbose", action = clap::ArgAction::Count)]
        verbose: u8,
    },
    /// Read a packaged application: its manifest, its size and its integrity.
    Inspect {
        /// The artifact to read.
        #[arg(value_name = "EXE")]
        path: PathBuf,
        /// Re-hash the payload and compare it with the trailer.
        #[arg(long)]
        verify: bool,
        /// Print the argument vector and environment the launcher would use.
        #[arg(long)]
        launch_plan: bool,
        /// Print a JSON object instead of a table.
        #[arg(long)]
        json: bool,
    },
    /// Check a packaged application deeply: every file, and every binary.
    ///
    /// `inspect --verify` re-hashes the payload against the trailer and stops
    /// there. This streams the payload a second time, checks every file
    /// against `ginary.index.json`, and inspects every native binary in it:
    /// the machine it was built for and the libraries it expects the target
    /// machine to already have. Exits 1 when it finds anything.
    Verify {
        /// The artifact to check.
        #[arg(value_name = "EXE")]
        path: PathBuf,
        /// Print a JSON object instead of a table.
        #[arg(long)]
        json: bool,
    },
    /// Write an SPDX 2.3 bill of materials for a packaged application.
    Sbom {
        /// The artifact to describe.
        #[arg(value_name = "EXE")]
        path: PathBuf,
        /// Where to write the document. Defaults to `<app>.spdx.json` beside
        /// the artifact.
        #[arg(long, value_name = "PATH")]
        out: Option<PathBuf>,
    },
    /// Summarise an `erl_crash.dump`.
    ///
    /// The slogan, the system version and the largest processes, read as a
    /// stream: a crash dump can be larger than the machine's memory.
    Crashdump {
        /// The dump to read.
        #[arg(value_name = "PATH")]
        path: PathBuf,
        /// Print a JSON object instead of a table.
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
    /// Build the staging root: the exact tree an artifact is made of.
    ///
    /// The same closure `ginary closure` prints, copied into a directory
    /// alongside the four ERTS binaries and the boot file. It is the debugging
    /// window onto what `ginary build` will pack, and it is what the launcher
    /// will later extract into its cache.
    Stage {
        /// The directory `gleam export erlang-shipment` wrote.
        #[arg(value_name = "SHIPMENT")]
        shipment: PathBuf,
        /// The OTP installation to stage from. Defaults to the one `erl` on
        /// `PATH` reports.
        #[arg(long, value_name = "PATH")]
        otp_root: Option<PathBuf>,
        /// An application to start from. Repeatable, and required.
        #[arg(long = "root", value_name = "NAME", required = true)]
        roots: Vec<String>,
        /// An extra application to bundle. Repeatable.
        #[arg(long = "extra", value_name = "NAME")]
        extra: Vec<String>,
        /// Where to write the staging root. It must not exist, or must be an
        /// empty directory, unless `--force` is given.
        #[arg(long, required = true, value_name = "DIR")]
        out: PathBuf,
        /// Replace the output directory if it exists and is not empty.
        #[arg(long)]
        force: bool,
        /// A program to stage from the runtime's `bin` beyond the required
        /// four, such as `heart` or `epmd`. Repeatable.
        #[arg(long = "extra-bin", value_name = "NAME")]
        extra_bins: Vec<String>,
        /// Keep the files staging would otherwise delete as known-useless.
        #[arg(long)]
        keep_junk: bool,
        /// Remove debug information from the staged tree. On by default.
        #[arg(long, overrides_with = "no_strip")]
        strip: bool,
        /// Keep the debug information the default removes.
        #[arg(long, overrides_with = "strip")]
        no_strip: bool,
        /// Strip only the native binaries, leaving the `.beam` files alone.
        #[arg(long, conflicts_with = "strip_beams_only", conflicts_with = "no_strip")]
        strip_elf_only: bool,
        /// Strip only the `.beam` files, leaving the native binaries alone.
        #[arg(long, conflicts_with = "no_strip")]
        strip_beams_only: bool,
        /// The form the size report takes.
        ///
        /// `text` prints it under the staging tables; `json` prints the report
        /// alone, so that it can be piped. Only the `json` *value* conflicts
        /// with `--json` and `--explain`, which is a rule about the value and
        /// not about the flag, so [`value_conflict`] enforces it rather than
        /// clap: `--report text --json` asks for the documented default and
        /// must not be a usage error.
        #[arg(long, value_name = "FORMAT", default_value = "text")]
        report: ReportFormat,
        /// Print a JSON object instead of a table.
        #[arg(long, conflicts_with = "explain")]
        json: bool,
        /// Print the whole account: sizes, applications, exclusions, junk.
        #[arg(long)]
        explain: bool,
    },
    /// Read the chunk table of compiled BEAM modules.
    Beam {
        /// What to do with the modules.
        #[command(subcommand)]
        command: BeamCommand,
    },
    /// Read what native binaries need from the machine that runs them.
    Elf {
        /// What to do with the binaries.
        #[command(subcommand)]
        command: ElfCommand,
    },
    /// Inspect and empty the directory packaged applications extract into.
    Cache {
        /// What to do with the cache.
        #[command(subcommand)]
        command: CacheCommand,
    },
}

/// A subcommand of `ginary cache`.
#[derive(Debug, Subcommand)]
pub enum CacheCommand {
    /// Print the cache root and the rule that produced it.
    ///
    /// The same resolution a packaged application makes, so this is the
    /// answer to "where did my artifact put its runtime": `GINARY_CACHE_DIR`,
    /// then `XDG_CACHE_HOME/ginary`, then `HOME/.cache/ginary`, and
    /// `${TMPDIR:-/tmp}/ginary-<uid>` when none of those is set.
    Dir {
        /// Print a JSON object instead of a line.
        #[arg(long)]
        json: bool,
    },
    /// Remove extracted runtimes, freeing the space they take.
    ///
    /// Nothing is lost: the next run of an artifact extracts its runtime
    /// again. The cache root itself stays.
    Clean {
        /// Empty one application's directory instead of all of them.
        #[arg(long, value_name = "NAME")]
        app: Option<String>,
        /// Print a JSON object instead of a table.
        #[arg(long)]
        json: bool,
    },
    /// Remove the extracted runtimes nothing has used for a while.
    ///
    /// The same housekeeping every packaged application does for itself as it
    /// starts, on demand and over the whole cache. An entry a process is
    /// running out of is never removed, whatever its age: the lock decides,
    /// and `--all` is "whatever its age", not "whatever is using it".
    Prune {
        /// How many days an unused entry may live. Defaults to
        /// `GINARY_PRUNE_DAYS`, or 14.
        #[arg(long, value_name = "N")]
        days: Option<u64>,
        /// Consider every entry, whatever its age. Locks are still honoured.
        #[arg(long)]
        all: bool,
        /// Prune one application's directory instead of all of them.
        #[arg(long, value_name = "NAME")]
        app: Option<String>,
    },
}

/// The payload of `ginary cache dir --json`.
#[derive(Debug, Serialize)]
pub struct CacheDirReport {
    /// Version of this schema; see [`CACHE_FORMAT_VERSION`].
    pub format_version: u32,
    /// The resolved cache root.
    pub path: String,
    /// The rule that produced it, as [`cache::Origin::describe`] names it.
    pub origin: &'static str,
    /// Whether this is the temporary-directory fallback.
    pub is_fallback: bool,
}

/// The payload of `ginary cache clean --json`.
#[derive(Debug, Serialize)]
pub struct CacheCleanReport {
    /// Version of this schema; see [`CACHE_FORMAT_VERSION`].
    pub format_version: u32,
    /// The cache root the removal happened under.
    pub root: String,
    /// The application that was emptied, or [`None`] for all of them.
    pub app: Option<String>,
    /// The directories that were removed, sorted.
    pub removed: Vec<String>,
    /// The total size of what was removed, in bytes.
    pub bytes: u64,
}

/// The form `ginary stage` prints its size report in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub enum ReportFormat {
    /// An aligned table and the `needs:` line, under the staging output.
    #[default]
    Text,
    /// The report alone, as one JSON object, with nothing else printed.
    Json,
}

/// A subcommand of `ginary beam`.
#[derive(Debug, Subcommand)]
pub enum BeamCommand {
    /// List the chunks of each module, and whether it holds debug information.
    ///
    /// This is the debugging window into stripping: a module that is still big
    /// after a build shows here exactly which chunk it is big because of.
    Chunks {
        /// The `.beam` files to read.
        #[arg(required = true, value_name = "PATH")]
        paths: Vec<PathBuf>,
        /// Print a JSON object instead of a table.
        #[arg(long)]
        json: bool,
    },
}

/// A subcommand of `ginary elf`.
#[derive(Debug, Subcommand)]
pub enum ElfCommand {
    /// List what each binary needs: its libraries, its glibc floor, its loader.
    Deps {
        /// The ELF files to read.
        #[arg(required = true, value_name = "PATH")]
        paths: Vec<PathBuf>,
        /// Print a JSON object instead of a table.
        #[arg(long)]
        json: bool,
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

/// The payload of `ginary stage --json`.
#[derive(Debug, Serialize)]
pub struct StageReport {
    /// Version of this schema; see [`STAGE_FORMAT_VERSION`].
    pub format_version: u32,
    /// The staged tree and the account of what went into it.
    ///
    /// The sizes are the ones the tree holds *after* stripping, because that is
    /// what the artifact will be made of.
    #[serde(flatten)]
    pub staged: StagedRoot,
    /// What the strip phase did.
    pub strip: StripReport,
    /// The size breakdown and the dependency summary.
    pub report: SizeReport,
}

/// The payload of `ginary stage --report json`.
#[derive(Debug, Serialize)]
pub struct SizeReportPayload {
    /// Version of this schema; see [`SIZE_REPORT_FORMAT_VERSION`].
    pub format_version: u32,
    /// What the strip phase did.
    pub strip: StripReport,
    /// The size breakdown and the dependency summary.
    #[serde(flatten)]
    pub report: SizeReport,
}

/// The payload of `ginary beam chunks --json`.
#[derive(Debug, Serialize)]
pub struct BeamReport {
    /// Version of this schema; see [`BEAM_FORMAT_VERSION`].
    pub format_version: u32,
    /// One entry per file, in the order the files were given.
    pub files: Vec<BeamFile>,
}

/// One file's worth of [`BeamReport`].
#[derive(Debug, Serialize)]
pub struct BeamFile {
    /// The path as it was given on the command line.
    pub path: String,
    /// The chunks, in the order the file holds them.
    pub chunks: Vec<BeamChunk>,
    /// Whether the file holds [`beam::DEBUG_INFO_CHUNK`].
    pub debug_info: bool,
}

/// One chunk of a [`BeamFile`].
#[derive(Debug, Serialize)]
pub struct BeamChunk {
    /// The four-byte identifier, as text.
    pub id: String,
    /// The offset of the chunk's data within the file.
    pub offset: usize,
    /// The length of the chunk's data.
    pub len: u32,
}

/// The payload of `ginary elf deps --json`.
#[derive(Debug, Serialize)]
pub struct ElfReport {
    /// Version of this schema; see [`ELF_FORMAT_VERSION`].
    pub format_version: u32,
    /// One entry per file, in the order the files were given.
    pub files: Vec<ElfFile>,
}

/// One file's worth of [`ElfReport`].
#[derive(Debug, Serialize)]
pub struct ElfFile {
    /// The path as it was given on the command line.
    pub path: String,
    /// What the file needs and what it is.
    #[serde(flatten)]
    pub info: ElfInfo,
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
/// Argument errors are reported by clap itself, which exits with status 2 —
/// including the ones [`value_conflict`] finds, which are handed back to clap
/// so that a conflict about a value reads exactly like a conflict about a flag.
pub fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();
    if let Some(message) = value_conflict(&cli.command) {
        Cli::command()
            .error(clap::error::ErrorKind::ArgumentConflict, message)
            .exit();
    }
    let stdout = std::io::stdout();
    dispatch(&cli.command, &mut stdout.lock())
}

/// The conflicts that are about an argument's *value* rather than its presence.
///
/// clap refuses a combination of flags, and `--report` is not a flag: the
/// conflict belongs to `--report json`, which prints the size report alone with
/// nothing else on standard output. That leaves nothing for `--explain` to add
/// and disagrees with `--json`, which prints the whole staging object. Attaching
/// the conflict to the argument instead would make `--report text --json` — the
/// documented default next to a flag it does not touch — a usage error.
///
/// `None` when the command line is usable; `Some(message)` is what the user is
/// told, in clap's own voice and with clap's exit status.
pub fn value_conflict(command: &Command) -> Option<String> {
    let Command::Stage {
        report: ReportFormat::Json,
        json,
        explain,
        ..
    } = command
    else {
        return None;
    };
    if *json {
        return Some(
            "the argument `--report json` cannot be used with `--json`: one prints the size \
             report, the other prints the whole staging object"
                .to_owned(),
        );
    }
    if *explain {
        return Some(
            "the argument `--report json` cannot be used with `--explain`: the report is \
             printed alone, so there is nothing for the account to appear beside"
                .to_owned(),
        );
    }
    None
}

/// Runs one command, writing its output to `out`.
///
/// Taking the writer as a parameter keeps the commands testable without
/// capturing the process' standard output.
pub fn dispatch(command: &Command, out: &mut impl Write) -> anyhow::Result<()> {
    match command {
        Command::Version { json } => write_version(&VersionReport::current(), *json, out),
        Command::Doctor { json } => write_doctor(&doctor::Report::gather(), *json, out),
        Command::Build {
            out: dir,
            no_strip,
            strip_elf_only,
            strip_beams_only,
            targets,
            otp_root,
            skip_export,
            keep_staging,
            compression_level,
            extra_otp_apps,
            extra_bins,
            distribution,
            vm_args,
            sys_config,
            report,
            explain,
            sbom,
            sbom_out,
            verbose,
        } => write_build(
            &BuildFlags {
                start: std::env::current_dir().context("cannot read the working directory")?,
                out: dir.clone(),
                no_strip: *no_strip,
                strip_elf_only: *strip_elf_only,
                strip_beams_only: *strip_beams_only,
                otp_root: otp_root.clone(),
                skip_export: *skip_export,
                keep_staging: *keep_staging,
                compression_level: *compression_level,
                extra_otp_apps: extra_otp_apps.clone(),
                extra_bins: extra_bins.clone(),
                distribution: *distribution,
                vm_args: vm_args.clone(),
                sys_config: sys_config.clone(),
                targets: targets.clone(),
                explain: *explain,
                verbose: *verbose,
            },
            *report,
            &SbomRequest {
                wanted: *sbom || sbom_out.is_some(),
                out: sbom_out.clone(),
            },
            out,
        ),
        Command::Inspect {
            path,
            verify,
            launch_plan,
            json,
        } => write_inspect(path, *verify, *launch_plan, *json, out),
        Command::Verify { path, json } => write_verify(path, *json, out),
        Command::Sbom {
            path,
            out: destination,
        } => write_sbom(path, destination.as_deref(), out),
        Command::Crashdump { path, json } => write_crashdump(path, *json, out),
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
        Command::Stage {
            shipment,
            otp_root,
            roots,
            extra,
            out: dir,
            force,
            extra_bins,
            keep_junk,
            strip: _,
            no_strip,
            strip_elf_only,
            strip_beams_only,
            report,
            json,
            explain,
        } => write_stage(
            &StageRequest {
                shipment,
                otp_root: otp_root.as_deref(),
                roots,
                extra,
                dir,
                options: StageOptions {
                    extra_bins: extra_bins.clone(),
                    remove_junk: !*keep_junk,
                    force: *force,
                },
                strip: strip_options(*no_strip, *strip_elf_only, *strip_beams_only),
                report: *report,
            },
            *json,
            *explain,
            out,
        ),
        Command::Beam {
            command: BeamCommand::Chunks { paths, json },
        } => write_beam_chunks(paths, *json, out),
        Command::Elf {
            command: ElfCommand::Deps { paths, json },
        } => write_elf_deps(paths, *json, out),
        Command::Cache {
            command: CacheCommand::Dir { json },
        } => {
            let dirs = cache::resolve(&cache::Env::from_env(), cache::current_uid());
            write_cache_dir(&dirs, *json, out)
        }
        Command::Cache {
            command: CacheCommand::Clean { app, json },
        } => {
            // Before the value is joined onto anything: `--app /home/u/work`
            // and `--app ..` both name a directory outside the cache, and what
            // this command does to a directory is remove it.
            if let Some(app) = app.as_deref()
                && !cache::is_app_name(app)
            {
                anyhow::bail!("{}", cache::AppNameRefusal(app));
            }
            let dirs = cache::resolve(&cache::Env::from_env(), cache::current_uid());
            let report = cache::clean(&dirs.root, app.as_deref())
                .with_context(|| format!("cannot clean the cache at {}", dirs.root.display()))?;
            write_cache_clean(&dirs, app.as_deref(), &report, *json, out)
        }
        Command::Cache {
            command: CacheCommand::Prune { days, all, app },
        } => {
            // For the reason `clean` checks it, and before anything is joined:
            // what pruning does to a directory is remove it.
            if let Some(app) = app.as_deref()
                && !cache::is_app_name(app)
            {
                anyhow::bail!("{}", cache::AppNameRefusal(app));
            }
            let env = cache::Env::from_env();
            let dirs = cache::resolve(&env, cache::current_uid());
            let options = cache::PruneOptions {
                days: days.unwrap_or_else(|| cache::prune_days(&env)),
                all: *all,
            };
            let report = cache::prune(
                &dirs.root,
                app.as_deref(),
                options,
                std::time::SystemTime::now(),
            )
            .with_context(|| format!("cannot prune the cache at {}", dirs.root.display()))?;
            write!(out, "{}", crate::launcher::render_prune(&report))
                .context("cannot write the prune report to standard output")
        }
    }
}

/// Writes the resolved cache root and its provenance.
fn write_cache_dir(
    dirs: &cache::CacheDirs,
    json: bool,
    out: &mut impl Write,
) -> anyhow::Result<()> {
    if json {
        return write_json(
            out,
            &CacheDirReport {
                format_version: CACHE_FORMAT_VERSION,
                path: dirs.root.display().to_string(),
                origin: dirs.origin.describe(),
                is_fallback: dirs.is_fallback,
            },
        );
    }
    writeln!(
        out,
        "cache dir: {} (from {})",
        dirs.root.display(),
        dirs.origin.describe()
    )
    .context("cannot write the cache directory to standard output")
}

/// Writes what one `ginary cache clean` removed.
fn write_cache_clean(
    dirs: &cache::CacheDirs,
    app: Option<&str>,
    report: &cache::CleanReport,
    json: bool,
    out: &mut impl Write,
) -> anyhow::Result<()> {
    if json {
        return write_json(
            out,
            &CacheCleanReport {
                format_version: CACHE_FORMAT_VERSION,
                root: dirs.root.display().to_string(),
                app: app.map(str::to_owned),
                removed: report
                    .removed
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect(),
                bytes: report.bytes,
            },
        );
    }

    for path in &report.removed {
        writeln!(out, "removed {}", path.display())
            .context("cannot write the removals to standard output")?;
    }
    let count = report.removed.len();
    let noun = if count == 1 {
        "directory"
    } else {
        "directories"
    };
    writeln!(out, "total: {count} {noun}, {} bytes", report.bytes)
        .context("cannot write the cache summary to standard output")
}

/// Turns the four stripping flags into the two booleans the module takes.
///
/// `--no-strip` turns both off. The two `--strip-*-only` flags turn the other
/// half off; clap already refuses them next to `--no-strip`, so there is no
/// combination here that means two things at once.
fn strip_options(no_strip: bool, elf_only: bool, beams_only: bool) -> StripOptions {
    if no_strip {
        return StripOptions {
            elf: false,
            beams: false,
        };
    }
    StripOptions {
        elf: !beams_only,
        beams: !elf_only,
    }
}

/// Everything `ginary stage` needs, gathered so the call is readable.
struct StageRequest<'a> {
    /// The shipment directory.
    shipment: &'a Path,
    /// The OTP root override, if one was given.
    otp_root: Option<&'a Path>,
    /// The applications to start from.
    roots: &'a [String],
    /// The extra applications to bundle.
    extra: &'a [String],
    /// Where the staging root goes.
    dir: &'a Path,
    /// How to build it.
    options: StageOptions,
    /// How much of it to strip.
    strip: StripOptions,
    /// The form the size report takes.
    report: ReportFormat,
}

/// Stages a shipment and writes the result in the requested form.
///
/// The closure is computed exactly as `ginary closure` computes it, so a
/// surprise in the staged tree can be traced back with the other command over
/// the same arguments.
fn write_stage(
    request: &StageRequest<'_>,
    json: bool,
    explain: bool,
    out: &mut impl Write,
) -> anyhow::Result<()> {
    let otp = otp::discover(request.otp_root)?;
    let apps =
        closure::app_dependency_closure(request.shipment, &otp.lib, request.roots, request.extra)?;
    let before = assemble::stage(&apps, &otp, &request.options, request.dir)?;

    let stripping = request.strip.elf || request.strip.beams;
    let strip_report = if stripping {
        strip::strip(before.root(), &otp, &request.strip)?
    } else {
        StripReport::disabled()
    };
    // The listing on disk still holds the pre-strip sizes, so the tree stops
    // describing itself the moment a byte is removed from it.
    let staged = if stripping {
        before.refresh()?
    } else {
        before.clone()
    };
    let size_report = report::measure(&before, &strip_report, staged.root())?;

    if json {
        return write_json(
            out,
            &StageReport {
                format_version: STAGE_FORMAT_VERSION,
                staged,
                strip: strip_report,
                report: size_report,
            },
        );
    }

    if request.report == ReportFormat::Json {
        return write_json(
            out,
            &SizeReportPayload {
                format_version: SIZE_REPORT_FORMAT_VERSION,
                strip: strip_report,
                report: size_report,
            },
        );
    }

    let mut text = if explain {
        staged.explain()
    } else {
        render_stage_table(&staged)
    };
    text.push('\n');
    text.push_str(&strip_report.to_string());
    text.push('\n');
    text.push_str(&size_report.render_text());
    text.push_str(&format!(
        "\nstaged {} files, {} bytes, into {}\n",
        staged.files().len(),
        staged.total_bytes(),
        staged.root().display()
    ));
    out.write_all(text.as_bytes())
        .context("cannot write the staging report to standard output")
}

/// Lists the chunks of every named module and writes them in the wanted form.
fn write_beam_chunks(paths: &[PathBuf], json: bool, out: &mut impl Write) -> anyhow::Result<()> {
    let mut files = Vec::with_capacity(paths.len());
    for path in paths {
        let bytes =
            std::fs::read(path).with_context(|| format!("cannot read `{}`", path.display()))?;
        let chunks = beam::chunks(&bytes)
            .with_context(|| format!("cannot read the chunks of `{}`", path.display()))?;
        files.push(BeamFile {
            path: path.display().to_string(),
            chunks: chunks
                .iter()
                .map(|chunk| BeamChunk {
                    id: chunk.id_str(),
                    offset: chunk.offset,
                    len: chunk.len,
                })
                .collect(),
            debug_info: beam::has_chunk(&bytes, &beam::DEBUG_INFO_CHUNK),
        });
    }

    if json {
        return write_json(
            out,
            &BeamReport {
                format_version: BEAM_FORMAT_VERSION,
                files,
            },
        );
    }

    let mut text = String::new();
    for (index, file) in files.iter().enumerate() {
        if index > 0 {
            text.push('\n');
        }
        text.push_str(&file.path);
        text.push('\n');
        let rows: Vec<[String; 3]> = file
            .chunks
            .iter()
            .map(|chunk| {
                [
                    chunk.id.clone(),
                    chunk.offset.to_string(),
                    chunk.len.to_string(),
                ]
            })
            .collect();
        text.push_str(&closure::render_table(["id", "offset", "len"], &rows));
        text.push_str(&format!(
            "debug_info: {}\n",
            if file.debug_info { "yes" } else { "no" }
        ));
    }
    out.write_all(text.as_bytes())
        .context("cannot write the chunk table to standard output")
}

/// Inspects every named binary and writes what it needs in the wanted form.
fn write_elf_deps(paths: &[PathBuf], json: bool, out: &mut impl Write) -> anyhow::Result<()> {
    let mut files = Vec::with_capacity(paths.len());
    for path in paths {
        let info =
            elf::inspect(path).with_context(|| format!("cannot inspect `{}`", path.display()))?;
        files.push(ElfFile {
            path: path.display().to_string(),
            info,
        });
    }

    if json {
        return write_json(
            out,
            &ElfReport {
                format_version: ELF_FORMAT_VERSION,
                files,
            },
        );
    }

    let mut text = String::new();
    for (index, file) in files.iter().enumerate() {
        if index > 0 {
            text.push('\n');
        }
        text.push_str(&file.path);
        text.push('\n');
        for (label, value) in [
            ("class", file.info.class.to_string()),
            ("machine", file.info.machine.clone()),
            ("interp", or_dash(file.info.interp.as_deref())),
            ("pie", yes_no(file.info.is_pie)),
            ("stripped", yes_no(file.info.stripped)),
            ("glibc_max", or_dash(file.info.glibc_max.as_deref())),
            ("needed", file.info.needed.join(", ")),
        ] {
            text.push_str(&format!("  {label:<ELF_LABEL_WIDTH$}{value}\n"));
        }
    }
    out.write_all(text.as_bytes())
        .context("cannot write the dependency table to standard output")
}

/// Renders an optional value, with a dash for the absent one.
fn or_dash(value: Option<&str>) -> String {
    value.unwrap_or("-").to_owned()
}

/// Renders a boolean as the word the tables print.
fn yes_no(value: bool) -> String {
    if value { "yes" } else { "no" }.to_owned()
}

/// Renders the default table: bytes and file count per category.
fn render_stage_table(staged: &StagedRoot) -> String {
    let rows: Vec<[String; 3]> = staged
        .bytes_by_category()
        .into_iter()
        .map(|(category, (bytes, files))| {
            [
                category.label().to_owned(),
                bytes.to_string(),
                files.to_string(),
            ]
        })
        .collect();
    closure::render_table(["category", "bytes", "files"], &rows)
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
/// The payload of `ginary build --report json`.
#[derive(Debug, Serialize)]
pub struct BuildJsonReport {
    /// Version of this schema; see [`BUILD_FORMAT_VERSION`].
    pub format_version: u32,
    /// What the build produced.
    #[serde(flatten)]
    pub report: BuildReport,
    /// Where the bill of materials was written, when one was asked for.
    ///
    /// Absent when neither `--sbom` nor `--sbom-out` was given, and absent
    /// when the document could not be written — in which case the command
    /// fails and says why on standard error. The text report's last line is
    /// `sbom: <path>`, so both forms name every file the command produced and
    /// a machine consumer never has to re-derive `<out dir>/<app>.spdx.json`
    /// or remember what it passed to `--sbom-out`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sbom: Option<String>,
}

/// Finds the project, merges the flags over its configuration, and builds.
fn write_build(
    flags: &BuildFlags,
    report: ReportFormat,
    sbom_request: &SbomRequest,
    out: &mut impl Write,
) -> anyhow::Result<()> {
    let project = gleam::find_project(&flags.start)?;
    let config = ProjectConfig::read(&project.manifest())?;
    let options = BuildOptions::merge(project.root(), &config, flags)?;
    // Layered rather than replaced: `-v` is a request for the phases on
    // standard error, and it may not take away a `GINARY_TRACE` file the user
    // asked for in the same breath. `-v` is exactly `GINARY_DEBUG=1` for the
    // length of this command.
    let mut env = diag::EnvSnapshot::from_env();
    if flags.verbose > 0 {
        env.ginary_debug = Some(std::ffi::OsString::from("1"));
    }
    let diag = Diag::from_env(&env);

    // Before the build and not after it: a `--sbom-out` in a directory that
    // does not exist is a mistake in the command line, and a build is minutes
    // of work to spend discovering one.
    if let Some(destination) = sbom_request.out.as_deref() {
        check_sbom_destination(destination)?;
    }

    let built = bundle::build(&options, &diag)?;
    let artifact = built.out.clone();

    // The document is written before the report, so that the report can name
    // it: `--report json` is one JSON document and a path appended after it
    // would not be in it. An artifact that is on disk still has to be named on
    // standard output whatever happens next, though — a caller that saw only
    // `cannot write the SBOM` could not tell this run from a build that
    // produced nothing — so a failure emits the report without the SBOM member
    // first and returns the error after.
    let written = if sbom_request.wanted {
        match write_sbom_for(&artifact, Some(project.root()), sbom_request.out.as_deref()) {
            Ok(written) => Some(written),
            Err(error) => {
                write_build_report(report, built, None, out)?;
                return Err(error);
            }
        }
    } else {
        None
    };
    write_build_report(report, built, written.as_deref(), out)
}

/// Writes one build's report in `report`'s form, naming `sbom` when there is
/// one.
fn write_build_report(
    report: ReportFormat,
    built: BuildReport,
    sbom: Option<&Path>,
    out: &mut impl Write,
) -> anyhow::Result<()> {
    if report == ReportFormat::Json {
        return write_json(
            out,
            &BuildJsonReport {
                format_version: BUILD_FORMAT_VERSION,
                report: built,
                sbom: sbom.map(|path| path.display().to_string()),
            },
        );
    }
    let mut text = String::new();
    if let Some(explain) = &built.explain {
        text.push_str(&explain.closure);
        text.push('\n');
        text.push_str(&explain.staged);
        text.push('\n');
    }
    text.push_str(&built.render_text());
    if let Some(staging) = &built.staging {
        text.push_str(&format!("staging: {}\n", staging.display()));
    }
    if let Some(sbom) = sbom {
        text.push_str(&format!("sbom: {}\n", sbom.display()));
    }
    out.write_all(text.as_bytes())
        .context("cannot write the build report to standard output")
}

/// Refuses a `--sbom-out` whose directory is not there.
///
/// Only the parent, and only its existence: everything else a write can fail
/// on — a permission, a full disk, a name that is already a directory — is
/// found by the write itself, and by then the build report has been printed.
fn check_sbom_destination(destination: &Path) -> anyhow::Result<()> {
    let parent = destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty());
    if let Some(parent) = parent
        && !parent.is_dir()
    {
        anyhow::bail!(
            "cannot write the SBOM to {}: {} is not a directory",
            destination.display(),
            parent.display()
        );
    }
    Ok(())
}

/// Reads one artifact and prints what it says about itself.
fn write_inspect(
    path: &Path,
    verify: bool,
    launch_plan: bool,
    json: bool,
    out: &mut impl Write,
) -> anyhow::Result<()> {
    let info = inspect::open(path)?;

    let verification = if verify {
        Some(inspect::verify(&info)?)
    } else {
        None
    };
    let plan = if launch_plan {
        Some(inspect::launch_plan(
            &info,
            Path::new(inspect::PLACEHOLDER_ROOT),
            Path::new(inspect::PLACEHOLDER_APP_DIR),
        )?)
    } else {
        None
    };

    if json {
        write_json(
            out,
            &InspectReport {
                format_version: crate::inspect::INSPECT_FORMAT_VERSION,
                path: path.display().to_string(),
                payload_offset: info.trailer.payload_offset,
                payload_len: info.payload_len,
                total_len: info.total_len,
                payload_sha256: hex::encode(info.trailer.payload_sha256),
                manifest: info.manifest.clone(),
                index: info.index.clone(),
                verify: verification.clone(),
                launch_plan: plan.as_ref().map(|plan| LaunchPlanReport {
                    program: plan.program.display().to_string(),
                    argv: plan
                        .args
                        .iter()
                        .map(|argument| argument.to_string_lossy().into_owned())
                        .collect(),
                    set: plan
                        .set
                        .iter()
                        .map(|(key, value)| {
                            format!("{}={}", key.to_string_lossy(), value.to_string_lossy())
                        })
                        .collect(),
                    remove: plan
                        .remove
                        .iter()
                        .map(|name| name.to_string_lossy().into_owned())
                        .collect(),
                }),
            },
        )?;
    } else {
        let mut text = info.render_text();
        if let Some(plan) = &plan {
            text.push('\n');
            text.push_str(&inspect::render_launch_plan(plan));
        }
        if let Some(verification) = &verification {
            text.push_str(&format!(
                "\nverify: {}\n  expected {}\n  actual   {}\n",
                if verification.ok() { "ok" } else { "MISMATCH" },
                verification.expected,
                verification.actual
            ));
        }
        out.write_all(text.as_bytes())
            .context("cannot write the inspection to standard output")?;
    }

    if let Some(verification) = &verification
        && !verification.ok()
    {
        anyhow::bail!(
            "{}: the payload does not match the trailer's digest",
            path.display()
        );
    }
    Ok(())
}

/// Whether `ginary build` was asked for a bill of materials, and where.
struct SbomRequest {
    /// Whether `--sbom` or `--sbom-out` was given.
    wanted: bool,
    /// The `--sbom-out` value, when there was one.
    out: Option<PathBuf>,
}

/// The payload of `ginary verify --json`.
#[derive(Debug, Serialize)]
pub struct VerifyJsonReport {
    /// The report, whose own `format_version` versions this schema.
    #[serde(flatten)]
    pub report: VerifyReport,
}

/// Checks one artifact deeply and writes what it found.
///
/// A finding is not an error until the report has been printed: the point of
/// the command is the table, and a caller that only saw the exit code would
/// not know which file was wrong.
fn write_verify(path: &Path, json: bool, out: &mut impl Write) -> anyhow::Result<()> {
    let report = verify::verify(path)?;

    if json {
        write_json(
            out,
            &VerifyJsonReport {
                report: report.clone(),
            },
        )?;
    } else {
        out.write_all(report.render_text().as_bytes())
            .context("cannot write the verification to standard output")?;
    }

    if report.ok() {
        Ok(())
    } else {
        anyhow::bail!("{}: {} issue(s) found", path.display(), report.issues.len())
    }
}

/// Writes an artifact's bill of materials and says where it went.
fn write_sbom(path: &Path, destination: Option<&Path>, out: &mut impl Write) -> anyhow::Result<()> {
    let written = write_sbom_for(path, None, destination)?;
    writeln!(out, "sbom: {}", written.display())
        .context("cannot write the SBOM path to standard output")
}

/// Builds the document for `artifact` and writes it, returning where it went.
fn write_sbom_for(
    artifact: &Path,
    project: Option<&Path>,
    destination: Option<&Path>,
) -> anyhow::Result<PathBuf> {
    let document: SbomDocument = sbom::for_artifact(artifact, project)?;
    let path = match destination {
        Some(path) => path.to_path_buf(),
        // The application's name, not the document's: the document is named
        // `<app>-<version>` and the file beside the artifact is named after
        // the artifact.
        None => sbom::out_path(artifact, sbom::application_name(&document)),
    };
    sbom::write(&document, &path)?;
    Ok(path)
}

/// Summarises one crash dump and writes it in the requested form.
fn write_crashdump(path: &Path, json: bool, out: &mut impl Write) -> anyhow::Result<()> {
    let dump: CrashDump = crashdump::read(path)?;
    if json {
        return write_json(out, &dump);
    }
    out.write_all(dump.render_text().as_bytes())
        .context("cannot write the crash dump summary to standard output")
}

fn write_json(out: &mut impl Write, value: &impl Serialize) -> anyhow::Result<()> {
    let mut json = serde_json::to_vec_pretty(value).context("cannot serialise the report")?;
    json.push(b'\n');
    out.write_all(&json)
        .context("cannot write JSON to standard output")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_line_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn the_long_help_names_the_command_a_reader_came_for() {
        let help = Cli::command().render_long_help().to_string();
        assert!(help.contains("build"), "long help:\n{help}");
        assert!(
            !help.contains("pre-alpha"),
            "the pre-alpha notice went when `build` landed:\n{help}"
        );
    }

    #[test]
    fn no_arguments_is_an_error_that_exits_with_two() {
        let error = Cli::try_parse_from(["ginary"]).expect_err("a subcommand is required");
        assert_eq!(error.exit_code(), 2, "{error}");
    }

    #[test]
    fn an_unknown_subcommand_is_an_error() {
        // A name no milestone plans, so that this test keeps asserting what it
        // is about when the interface grows. It used to be `build`, which A4
        // implemented.
        assert!(Cli::try_parse_from(["ginary", "frobnicate"]).is_err());
    }

    /// A `stage` command line, parsed.
    fn stage_command(extra: &[&str]) -> Command {
        let mut argv = vec![
            "ginary", "stage", "shipment", "--root", "notify", "--out", "out",
        ];
        argv.extend_from_slice(extra);
        Cli::try_parse_from(argv)
            .expect("the flags under test parse; the conflict is decided afterwards")
            .command
    }

    #[test]
    fn report_json_conflicts_with_the_two_flags_that_print_something_else() {
        assert!(
            value_conflict(&stage_command(&["--report", "json", "--json"]))
                .is_some_and(|message| message.contains("--json"))
        );
        assert!(
            value_conflict(&stage_command(&["--report", "json", "--explain"]))
                .is_some_and(|message| message.contains("--explain"))
        );
    }

    #[test]
    fn report_text_conflicts_with_nothing() {
        // The default, spelled out loud. A conflict attached to the argument
        // rather than to its value would make this a usage error.
        assert_eq!(
            value_conflict(&stage_command(&["--report", "text", "--json"])),
            None
        );
        assert_eq!(
            value_conflict(&stage_command(&["--report", "text", "--explain"])),
            None
        );
        assert_eq!(value_conflict(&stage_command(&["--report", "json"])), None);
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
            cache_probe: None,
            otp: None,
            otp_error: Some("`erl` is not on PATH".to_owned()),
            project: None,
            targets: Vec::new(),
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
