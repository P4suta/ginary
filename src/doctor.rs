// SPDX-License-Identifier: MIT OR Apache-2.0
//! Environment diagnosis for `ginary doctor`.
//!
//! `doctor` answers one question: can this machine build a ginary artifact, and
//! if not, what is missing? It probes the external programs ginary shells out
//! to, reports the host target and the cache root, and states explicitly that a
//! Rust toolchain is *not* part of the answer — neither ginary nor the
//! executables it produces need `rustc` or `cargo` at run time.
//!
//! Probing never fails the command. A missing or broken tool is data, not an
//! error, so `doctor` always exits 0 and the caller reads the report.

use std::ffi::OsStr;
use std::path::PathBuf;
use std::time::Duration;

use serde::Serialize;

use crate::cache_dir::{self, EnvSnapshot};
use crate::otp;
use crate::process::{NULL_DEVICE, run_with_timeout};
use crate::target::Target;

/// Searching `PATH` for a program, re-exported from [`crate::process`].
///
/// `doctor` is where the search is visible to a user of the crate — it is what
/// the `gleam:`, `erl:`, `strip:` and `docker:` lines report — while the rule
/// itself is shared with [`crate::otp`].
pub use crate::process::find_in_path;

/// Version of the `doctor --json` schema.
pub const FORMAT_VERSION: u32 = 1;

/// How long a single tool probe may run before it is killed.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// One external program `doctor` knows how to probe.
struct Probe {
    /// Program name, looked up on `PATH`.
    name: &'static str,
    /// Arguments that make the program print its version and exit.
    args: &'static [&'static str],
    /// Turns the program's standard output into a human-readable version.
    parse: fn(&str) -> Option<String>,
}

/// The programs `doctor` probes, in report order.
const PROBES: [Probe; 4] = [
    Probe {
        name: "gleam",
        args: &["--version"],
        parse: parse_gleam_version,
    },
    Probe {
        name: "erl",
        args: &[
            "-noshell",
            // A broken OTP install can dump core on start-up. Without this the
            // probe would leave an `erl_crash.dump` in the user's working
            // directory, which is one of the UX problems ginary exists to fix.
            "-env",
            "ERL_CRASH_DUMP",
            NULL_DEVICE,
            "-eval",
            "io:format(\"~ts ~ts\",[erlang:system_info(otp_release),erlang:system_info(version)]),halt(0).",
        ],
        parse: parse_erl_version,
    },
    Probe {
        name: "strip",
        args: &["--version"],
        parse: parse_strip_version,
    },
    Probe {
        name: "docker",
        args: &["version", "--format", "{{.Server.Version}}"],
        parse: parse_docker_version,
    },
];

/// The state of one probed program.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ToolReport {
    /// Program name as spelled on `PATH`.
    pub name: String,
    /// Whether an executable of that name was found on `PATH`.
    pub found: bool,
    /// Version string, or `None` when the program is absent or did not answer.
    pub version: Option<String>,
    /// Absolute path of the executable, or `None` when it was not found.
    pub path: Option<PathBuf>,
}

impl ToolReport {
    /// Renders the one-line human form, for example `gleam: 1.18.1 (/usr/bin/gleam)`.
    fn render(&self) -> String {
        match (&self.version, &self.path) {
            (Some(version), Some(path)) => {
                format!("{}: {version} ({})", self.name, path.display())
            }
            (None, Some(path)) => {
                format!("{}: found, version unknown ({})", self.name, path.display())
            }
            (_, None) => format!("{}: not found", self.name),
        }
    }
}

/// What `doctor` says about the OTP installation it found.
///
/// A summary rather than the whole [`crate::otp::OtpInfo`]: `doctor` reports
/// what a person needs in order to recognise the installation, and the derived
/// paths are reconstructible from the root.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OtpReport {
    /// The code root the installation lives in.
    pub root: PathBuf,
    /// The major release, for example `29`.
    pub release: u32,
    /// The ERTS version, for example `17.0.5`.
    pub erts_vsn: String,
    /// The full version, for example `29.0.5`.
    pub otp_version: String,
}

impl OtpReport {
    /// Summarises a discovered installation.
    pub fn of(info: &otp::OtpInfo) -> Self {
        Self {
            root: info.root.clone(),
            release: info.release,
            erts_vsn: info.erts_vsn.clone(),
            otp_version: info.otp_version.clone(),
        }
    }

    /// Renders the two `otp` lines of the human-readable report.
    fn render(&self) -> String {
        format!(
            "otp: {} (release {}, erts {})\notp root: {}",
            self.otp_version,
            self.release,
            self.erts_vsn,
            self.root.display()
        )
    }
}

/// The full `doctor` result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Report {
    /// Version of this schema; see [`FORMAT_VERSION`].
    pub format_version: u32,
    /// The target ginary itself runs on.
    pub host_target: Target,
    /// Always `false`: no Rust toolchain is needed to run ginary or its output.
    pub rustc_required: bool,
    /// The resolved cache root, or `None` when no variable located one.
    pub cache_dir: Option<PathBuf>,
    /// The environment variable the cache root came from.
    pub cache_dir_source: Option<&'static str>,
    /// Why the cache root could not be resolved, when it could not.
    pub cache_dir_error: Option<String>,
    /// One entry per probed program: `gleam`, `erl`, `strip`, `docker`, in that
    /// order.
    pub tools: Vec<ToolReport>,
    /// The OTP installation [`crate::otp::discover`] found, or `None` when
    /// there is none to report.
    pub otp: Option<OtpReport>,
    /// Why there is none, when there is none.
    ///
    /// A machine with no Erlang and a machine whose Erlang cannot be packaged
    /// are both `otp: null`, and only this field tells them apart. Discovery
    /// failing is a reported decision, never a silent one, so it is `None`
    /// exactly when [`Report::otp`] is `Some`.
    pub otp_error: Option<String>,
}

impl Report {
    /// Probes the current environment.
    ///
    /// Every probe is bounded by [`PROBE_TIMEOUT`]; a program that hangs is
    /// killed and reported as present but without a version. The bound covers
    /// reading the program's output as well as waiting for it, so a probe that
    /// leaves a background process holding the pipes cannot stall the report
    /// either — see [`crate::process::run_with_timeout`].
    pub fn gather() -> Self {
        Self::gather_from(
            &PROBES,
            std::env::var_os("PATH").as_deref(),
            &EnvSnapshot::from_env(),
            otp::discover(None)
                .map(|info| OtpReport::of(&info))
                .map_err(|error| error.to_string()),
        )
    }

    /// Builds a report from an explicit environment.
    ///
    /// This is the half that is unit-tested: it reads neither `PATH` nor the
    /// process environment, so a test can hand it a temporary directory of fake
    /// programs and a fixed [`EnvSnapshot`]. [`Report::gather`] is the thin
    /// wrapper that captures the real ones.
    fn gather_from(
        probes: &[Probe],
        path_var: Option<&OsStr>,
        env: &EnvSnapshot,
        otp: Result<OtpReport, String>,
    ) -> Self {
        let (otp, otp_error) = match otp {
            Ok(report) => (Some(report), None),
            Err(reason) => (None, Some(reason)),
        };
        let tools = probes
            .iter()
            .map(|probe| probe_tool(probe, path_var))
            .collect();

        let (cache_dir, cache_dir_source, cache_dir_error) = match cache_dir::resolve(env) {
            Ok(resolved) => (Some(resolved.path), Some(resolved.source.variable()), None),
            Err(error) => (None, None, Some(error.to_string())),
        };

        Self {
            format_version: FORMAT_VERSION,
            host_target: Target::host(),
            rustc_required: false,
            cache_dir,
            cache_dir_source,
            cache_dir_error,
            tools,
            otp,
            otp_error,
        }
    }

    /// Renders the human-readable report, one subject per line.
    pub fn render_text(&self) -> String {
        let mut lines = vec![
            format!("host target: {}", self.host_target),
            "rustc/cargo: not required (neither ginary nor its artifacts need a Rust toolchain)"
                .to_owned(),
            match (&self.cache_dir, self.cache_dir_source) {
                (Some(path), Some(source)) => {
                    format!("cache dir: {} (from {source})", path.display())
                }
                _ => format!(
                    "cache dir: unresolved ({})",
                    self.cache_dir_error.as_deref().unwrap_or("unknown reason")
                ),
            },
        ];
        lines.extend(self.tools.iter().map(ToolReport::render));
        lines.push(match (&self.otp, &self.otp_error) {
            (Some(otp), _) => otp.render(),
            (None, Some(reason)) => format!("otp: unusable ({reason})"),
            (None, None) => "otp: not found".to_owned(),
        });
        lines.push(String::new());
        lines.join("\n")
    }
}

/// Looks a program up on `PATH` and, if present, asks it for its version.
fn probe_tool(probe: &Probe, path_var: Option<&OsStr>) -> ToolReport {
    let Some(path) = find_in_path(probe.name, path_var) else {
        return ToolReport {
            name: probe.name.to_owned(),
            found: false,
            version: None,
            path: None,
        };
    };

    let version = match run_with_timeout(&path, probe.args, PROBE_TIMEOUT) {
        Ok(output) if output.success => (probe.parse)(&output.stdout),
        Ok(_) | Err(_) => None,
    };

    ToolReport {
        name: probe.name.to_owned(),
        found: true,
        version,
        path: Some(path),
    }
}

/// Parses `gleam --version`, which prints `gleam <semver>`.
fn parse_gleam_version(stdout: &str) -> Option<String> {
    last_token_of_first_line(stdout)
}

/// Parses the OTP release and ERTS version printed by the `erl` probe.
///
/// The probe prints exactly `<otp_release> <erts_version>`, for example
/// `29 17.0.5`.
fn parse_erl_version(stdout: &str) -> Option<String> {
    let line = stdout.lines().next()?;
    let mut tokens = line.split_whitespace();
    let release = tokens.next()?;
    let erts = tokens.next()?;
    Some(format!("OTP {release}, erts {erts}"))
}

/// Parses `strip --version`, whose first line ends with the binutils version.
fn parse_strip_version(stdout: &str) -> Option<String> {
    last_token_of_first_line(stdout)
}

/// Parses `docker version --format {{.Server.Version}}`, a bare version.
fn parse_docker_version(stdout: &str) -> Option<String> {
    let line = stdout.lines().next()?.trim();
    (!line.is_empty()).then(|| line.to_owned())
}

/// Returns the last whitespace-separated token of the first non-empty line.
fn last_token_of_first_line(stdout: &str) -> Option<String> {
    stdout
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())?
        .split_whitespace()
        .next_back()
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::*;

    #[cfg(unix)]
    use crate::process::test_support::script;

    #[test]
    fn gleam_version_is_the_trailing_token() {
        assert_eq!(
            parse_gleam_version("gleam 1.18.1\n").as_deref(),
            Some("1.18.1")
        );
    }

    #[test]
    fn strip_version_is_the_trailing_token_of_the_banner() {
        assert_eq!(
            parse_strip_version("GNU strip (GNU Binutils for Ubuntu) 2.42\nCopyright (C) 2024\n")
                .as_deref(),
            Some("2.42")
        );
    }

    #[test]
    fn erl_version_combines_release_and_erts() {
        assert_eq!(
            parse_erl_version("29 17.0.5").as_deref(),
            Some("OTP 29, erts 17.0.5")
        );
    }

    #[test]
    fn erl_version_needs_both_fields() {
        assert_eq!(parse_erl_version("29"), None);
        assert_eq!(parse_erl_version(""), None);
    }

    #[test]
    fn docker_version_is_the_bare_line() {
        assert_eq!(parse_docker_version("29.7.2\n").as_deref(), Some("29.7.2"));
    }

    #[test]
    fn empty_output_yields_no_version() {
        assert_eq!(parse_docker_version("\n"), None);
        assert_eq!(parse_gleam_version(""), None);
        assert_eq!(parse_strip_version("   \n"), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_missing_tool_is_reported_as_not_found() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path_var = std::env::join_paths([dir.path()]).expect("join paths");
        let report = probe_tool(&PROBES[0], Some(&path_var));
        assert_eq!(
            report,
            ToolReport {
                name: "gleam".to_owned(),
                found: false,
                version: None,
                path: None,
            }
        );
        assert_eq!(report.render(), "gleam: not found");
    }

    #[cfg(unix)]
    #[test]
    fn a_tool_that_fails_is_found_without_a_version() {
        let dir = tempfile::tempdir().expect("tempdir");
        script(dir.path(), "gleam", "exit 3");
        let path_var = std::env::join_paths([dir.path()]).expect("join paths");
        let report = probe_tool(&PROBES[0], Some(&path_var));
        assert!(report.found);
        assert_eq!(report.version, None);
        assert!(
            report
                .render()
                .starts_with("gleam: found, version unknown (")
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_working_tool_reports_its_version_and_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        script(dir.path(), "gleam", "echo gleam 1.18.1");
        let path_var = std::env::join_paths([dir.path()]).expect("join paths");
        let report = probe_tool(&PROBES[0], Some(&path_var));
        assert!(report.found);
        assert_eq!(report.version.as_deref(), Some("1.18.1"));
        assert!(report.render().starts_with("gleam: 1.18.1 ("));
    }

    #[test]
    fn the_erl_probe_cannot_drop_a_crash_dump_in_the_working_directory() {
        let erl = PROBES
            .iter()
            .find(|probe| probe.name == "erl")
            .expect("an erl probe");
        let guard = erl
            .args
            .iter()
            .position(|arg| *arg == "-env")
            .expect("the erl probe sets an emulator environment variable");
        assert_eq!(erl.args.get(guard + 1).copied(), Some("ERL_CRASH_DUMP"));
        assert_eq!(erl.args.get(guard + 2).copied(), Some(NULL_DEVICE));
    }

    #[test]
    fn the_probe_list_is_the_documented_one() {
        let names: Vec<&str> = PROBES.iter().map(|probe| probe.name).collect();
        assert_eq!(names, ["gleam", "erl", "strip", "docker"]);
    }

    #[test]
    fn the_text_report_has_one_line_per_subject() {
        let report = Report {
            format_version: FORMAT_VERSION,
            host_target: Target::host(),
            rustc_required: false,
            cache_dir: Some(PathBuf::from("/home/u/.cache/ginary")),
            cache_dir_source: Some("HOME"),
            cache_dir_error: None,
            otp: None,
            otp_error: None,
            tools: vec![ToolReport {
                name: "gleam".to_owned(),
                found: false,
                version: None,
                path: None,
            }],
        };
        let text = report.render_text();
        assert!(text.contains(&format!("host target: {}\n", Target::host())));
        assert!(text.contains("rustc/cargo: not required"));
        assert!(text.contains("cache dir: /home/u/.cache/ginary (from HOME)\n"));
        assert!(text.contains("gleam: not found\n"));
        assert!(text.ends_with("otp: not found\n"), "{text}");
    }

    /// Regression for the A1a review: `gather` dropped the `OtpError`, so an
    /// Erlang that is present but unusable rendered exactly like no Erlang at
    /// all and every actionable message the `otp` module carries was
    /// unreachable.
    #[test]
    fn a_failed_discovery_renders_the_reason_it_failed() {
        let report = Report::gather_from(
            &[],
            None,
            &cache_snapshot("/srv/ginary-cache"),
            Err("`/opt/broken` has no `erts-*` directory".to_owned()),
        );
        assert_eq!(report.otp, None);
        assert_eq!(
            report.otp_error.as_deref(),
            Some("`/opt/broken` has no `erts-*` directory")
        );
        assert!(
            report
                .render_text()
                .contains("otp: unusable (`/opt/broken` has no `erts-*` directory)"),
            "{}",
            report.render_text()
        );
    }

    #[test]
    fn a_successful_discovery_records_no_reason() {
        let report = Report::gather_from(
            &[],
            None,
            &cache_snapshot("/srv/ginary-cache"),
            Ok(OtpReport {
                root: PathBuf::from("/opt/otp"),
                release: 29,
                erts_vsn: "17.0.5".to_owned(),
                otp_version: "29.0.5".to_owned(),
            }),
        );
        assert_eq!(report.otp_error, None);
        let text = report.render_text();
        assert!(
            text.contains("otp: 29.0.5 (release 29, erts 17.0.5)"),
            "{text}"
        );
        assert!(text.contains("otp root: /opt/otp"), "{text}");
    }

    #[test]
    fn an_unresolved_cache_dir_still_renders_a_line() {
        let report = Report {
            format_version: FORMAT_VERSION,
            host_target: Target::host(),
            rustc_required: false,
            cache_dir: None,
            cache_dir_source: None,
            cache_dir_error: Some("no HOME".to_owned()),
            otp: None,
            otp_error: None,
            tools: Vec::new(),
        };
        assert!(
            report
                .render_text()
                .contains("cache dir: unresolved (no HOME)")
        );
    }

    /// A snapshot that resolves to `dir` through `GINARY_CACHE_DIR`.
    fn cache_snapshot(dir: &str) -> EnvSnapshot {
        EnvSnapshot {
            ginary_cache_dir: Some(OsString::from(dir)),
            ..EnvSnapshot::default()
        }
    }

    #[cfg(unix)]
    #[test]
    fn gathering_probes_the_given_path_and_never_needs_rustc() {
        let dir = tempfile::tempdir().expect("tempdir");
        script(dir.path(), "gleam", "echo gleam 4.5.6");
        let path_var = std::env::join_paths([dir.path()]).expect("join paths");

        let report = Report::gather_from(
            &PROBES[..1],
            Some(&path_var),
            &cache_snapshot("/srv/ginary-cache"),
            Err("no OTP was looked for".to_owned()),
        );

        assert!(!report.rustc_required);
        assert_eq!(report.format_version, FORMAT_VERSION);
        assert_eq!(report.host_target, Target::host());
        assert_eq!(report.tools.len(), 1);
        assert_eq!(report.tools[0].name, "gleam");
        assert_eq!(report.tools[0].version.as_deref(), Some("4.5.6"));
    }

    #[test]
    fn gathering_takes_the_cache_directory_from_the_snapshot() {
        let report = Report::gather_from(
            &[],
            None,
            &cache_snapshot("/srv/ginary-cache"),
            Err("no OTP was looked for".to_owned()),
        );
        assert_eq!(report.cache_dir, Some(PathBuf::from("/srv/ginary-cache")));
        assert_eq!(report.cache_dir_source, Some("GINARY_CACHE_DIR"));
        assert_eq!(report.cache_dir_error, None);
        assert!(report.tools.is_empty());
    }

    #[test]
    fn gathering_records_why_the_cache_directory_is_unresolved() {
        let report =
            Report::gather_from(&[], None, &EnvSnapshot::default(), Err("no OTP".to_owned()));
        assert_eq!(report.cache_dir, None);
        assert_eq!(report.cache_dir_source, None);
        assert!(report.cache_dir_error.is_some(), "{report:?}");
    }
}
