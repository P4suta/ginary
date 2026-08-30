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

use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::cache_dir::{self, EnvSnapshot};
use crate::target::Target;

/// Version of the `doctor --json` schema.
pub const FORMAT_VERSION: u32 = 1;

/// How long a single tool probe may run before it is killed.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// How often a running probe is polled for completion.
const POLL_INTERVAL: Duration = Duration::from_millis(20);

/// The least time the output readers get after the child has been reaped.
///
/// Exiting closes the child's own ends of the pipes, so a reader that nothing
/// else is holding open reaches end of file at once. This is slack for that
/// thread to be scheduled, not a second budget: when the probe's own deadline
/// is further away, the deadline wins.
const DRAIN_GRACE: Duration = Duration::from_millis(500);

/// How much of a pipe the reader threads move per `read` call.
const DRAIN_CHUNK: usize = 8 * 1024;

/// The platform's bit bucket, used to keep probes from writing files.
#[cfg(windows)]
const NULL_DEVICE: &str = "nul";
/// The platform's bit bucket, used to keep probes from writing files.
#[cfg(not(windows))]
const NULL_DEVICE: &str = "/dev/null";

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
}

impl Report {
    /// Probes the current environment.
    ///
    /// Every probe is bounded by [`PROBE_TIMEOUT`]; a program that hangs is
    /// killed and reported as present but without a version. The bound covers
    /// reading the program's output as well as waiting for it, so a probe that
    /// leaves a background process holding the pipes cannot stall the report
    /// either — see [`run_with_timeout`].
    pub fn gather() -> Self {
        Self::gather_from(
            &PROBES,
            std::env::var_os("PATH").as_deref(),
            &EnvSnapshot::from_env(),
        )
    }

    /// Builds a report from an explicit environment.
    ///
    /// This is the half that is unit-tested: it reads neither `PATH` nor the
    /// process environment, so a test can hand it a temporary directory of fake
    /// programs and a fixed [`EnvSnapshot`]. [`Report::gather`] is the thin
    /// wrapper that captures the real ones.
    fn gather_from(probes: &[Probe], path_var: Option<&OsStr>, env: &EnvSnapshot) -> Self {
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

/// Searches `PATH` for an executable named `name`.
///
/// This is the `which(1)` rule: the first entry of `PATH` holding a regular
/// file with an execute bit wins. Empty `PATH` entries are skipped rather than
/// treated as the current directory, so a stray `:` cannot make ginary run a
/// program from the working directory.
pub fn find_in_path(name: &str, path_var: Option<&OsStr>) -> Option<PathBuf> {
    let path_var = path_var?;
    let file_name = with_exe_suffix(name);
    std::env::split_paths(path_var)
        .filter(|dir| !dir.as_os_str().is_empty())
        .map(|dir| dir.join(&file_name))
        .find(|candidate| is_executable_file(candidate))
}

/// Appends the host executable suffix (`.exe` on Windows) to a program name.
fn with_exe_suffix(name: &str) -> OsString {
    let mut file_name = OsString::from(name);
    file_name.push(std::env::consts::EXE_SUFFIX);
    file_name
}

/// Returns whether the path is a regular file the current user may execute.
fn is_executable_file(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// What a bounded child process produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProbeOutput {
    /// Whether the child exited with a success status.
    pub success: bool,
    /// Captured standard output, lossily decoded as UTF-8.
    pub stdout: String,
}

/// Why a probe produced no output.
#[derive(Debug, thiserror::Error)]
pub enum ProbeError {
    /// The program could not be spawned.
    #[error("cannot run `{program}`: {source}")]
    Spawn {
        /// The program that could not be spawned.
        program: String,
        /// The underlying operating system error.
        #[source]
        source: std::io::Error,
    },
    /// The spawned program could not be waited for.
    ///
    /// Distinct from [`ProbeError::Spawn`]: the program is running, or has run.
    /// Reporting this as a spawn failure would tell the user that a program
    /// they can see in the process table could not be started.
    #[error("cannot wait for `{program}`: {source}")]
    Wait {
        /// The program that could not be waited for.
        program: String,
        /// The underlying operating system error.
        #[source]
        source: std::io::Error,
    },
    /// The program did not exit within the timeout and was killed.
    #[error("`{program}` did not exit within {}s", .timeout.as_secs())]
    Timeout {
        /// The program that hung.
        program: String,
        /// The budget it exceeded.
        timeout: Duration,
    },
}

/// Runs a program with a hard timeout, capturing its standard output.
///
/// Standard output and standard error are drained by dedicated threads, so a
/// chatty child cannot deadlock on a full pipe while we are polling it. The
/// timeout bounds the whole call, not just the wait: the readers are never
/// joined, only given until the deadline (or half a second past the child's
/// exit, whichever is later) to reach end of file. A process the probe
/// backgrounded inherits the pipes and can hold them open long after the probe
/// itself has exited, so a join would wait on the grandchild instead of on the
/// budget.
///
/// The direct child never outlives the call: it is owned by a guard that kills
/// and reaps it on the way out, whether the probe timed out, could not be
/// waited for, or simply finished. Grandchildren are not killed — ginary does
/// not put probes in a process group — so on the timeout path the output is
/// whatever the readers had collected by the deadline, and the detached reader
/// threads end when the pipes finally close.
pub fn run_with_timeout(
    program: &Path,
    args: &[&str],
    timeout: Duration,
) -> Result<ProbeOutput, ProbeError> {
    let child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|source| ProbeError::Spawn {
            program: program.display().to_string(),
            source,
        })?;
    // From here on every exit runs the guard's destructor.
    let mut child = ChildGuard(child);

    let stdout = drain(child.0.stdout.take());
    // Standard error is drained but never read back: capturing it only keeps a
    // chatty child from blocking on a full pipe.
    let _stderr = drain(child.0.stderr.take());

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.0.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(POLL_INTERVAL),
            Ok(None) => break None,
            Err(source) => {
                return Err(ProbeError::Wait {
                    program: program.display().to_string(),
                    source,
                });
            }
        }
    };

    let Some(status) = status else {
        return Err(ProbeError::Timeout {
            program: program.display().to_string(),
            timeout,
        });
    };

    let stdout = stdout.take_until(deadline.max(Instant::now() + DRAIN_GRACE));
    Ok(ProbeOutput {
        success: status.success(),
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
    })
}

/// A pipe being read by a detached background thread.
///
/// The bytes are published as they arrive rather than returned at end of file,
/// so the caller can take what has been read so far without joining a thread
/// that may be blocked on a pipe nobody will close.
struct Drain {
    /// Everything read so far. The reader appends, the caller copies.
    buffer: Arc<Mutex<Vec<u8>>>,
    /// Signalled exactly once, when the reader reaches end of file.
    finished: Receiver<()>,
}

impl Drain {
    /// Returns the bytes read, waiting no later than `deadline` for the reader
    /// to reach end of file.
    ///
    /// A reader that is still blocked at the deadline is abandoned, and what it
    /// had already published is returned.
    fn take_until(self, deadline: Instant) -> Vec<u8> {
        let _ = self
            .finished
            .recv_timeout(deadline.saturating_duration_since(Instant::now()));
        let buffer = unpoison(self.buffer.lock());
        buffer.clone()
    }
}

/// Spawns a detached thread that reads a pipe to end of file.
fn drain<R: Read + Send + 'static>(pipe: Option<R>) -> Drain {
    let buffer = Arc::new(Mutex::new(Vec::new()));
    let (sender, finished) = mpsc::channel();
    let writer = Arc::clone(&buffer);
    std::thread::spawn(move || {
        if let Some(mut pipe) = pipe {
            let mut chunk = [0_u8; DRAIN_CHUNK];
            loop {
                match pipe.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(read) => unpoison(writer.lock()).extend_from_slice(&chunk[..read]),
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
        }
        // The receiver is gone once the caller has given up; that is not an
        // error, it is the timeout path.
        let _ = sender.send(());
    });
    Drain { buffer, finished }
}

/// Takes a lock result, treating poisoning as ordinary access.
///
/// The only data behind these locks is a byte buffer, and a reader thread that
/// panicked mid-append leaves it merely truncated, never inconsistent. Partial
/// output is exactly what the timeout path already returns.
fn unpoison<T>(result: std::sync::LockResult<T>) -> T {
    result.unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// A child process that is killed and reaped when it goes out of scope.
///
/// The obligation belongs to the value rather than to each `return`, so a new
/// error path cannot forget it: the A0 review found exactly that, a `try_wait`
/// failure that abandoned a running child.
struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        // Both calls are harmless once the child has exited and been waited
        // for: `kill` reports an invalid argument and `wait` returns the status
        // the standard library already cached.
        let _ = self.0.kill();
        let _ = self.0.wait();
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
    use super::*;

    /// The argument that makes a script written by [`script`] exit before its
    /// body runs, so exec-ability can be probed without any side effect.
    #[cfg(unix)]
    const EXEC_PROBE: &str = "--ginary-exec-probe";

    /// Creates an executable shell script and returns its path.
    ///
    /// The script is not returned until it has actually been exec'd once; see
    /// [`wait_until_executable`].
    #[cfg(unix)]
    fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        std::fs::write(
            &path,
            format!("#!/bin/sh\ncase \"$1\" in {EXEC_PROBE}) exit 0;; esac\n{body}\n"),
        )
        .expect("writes script");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("marks script executable");
        wait_until_executable(&path);
        path
    }

    /// Blocks until the freshly written script can be exec'd.
    ///
    /// Cargo runs these tests as threads of a single process. While one thread
    /// holds a write descriptor on a new file, a sibling thread's
    /// `Command::spawn` forks; the forked child inherits a duplicate of that
    /// descriptor until it execs, and any exec of the inode inside that window
    /// fails with `ETXTBSY`. The window is microseconds long and cannot reopen
    /// once no descriptor is left, so one bounded retry loop closes it for good.
    ///
    /// This belongs in the test helper, not in `run_with_timeout`: production
    /// code must report `ETXTBSY` rather than paper over it.
    #[cfg(unix)]
    fn wait_until_executable(path: &Path) {
        for _ in 0..500 {
            match Command::new(path)
                .arg(EXEC_PROBE)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            {
                Ok(mut child) => {
                    let _ = child.wait();
                    return;
                }
                Err(error) if error.kind() == std::io::ErrorKind::ExecutableFileBusy => {
                    std::thread::sleep(Duration::from_millis(2));
                }
                Err(error) => panic!("cannot exec {}: {error}", path.display()),
            }
        }
        panic!("{} is still not executable", path.display());
    }

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

    #[test]
    fn an_absent_path_variable_finds_nothing() {
        assert_eq!(find_in_path("gleam", None), None);
    }

    #[cfg(unix)]
    #[test]
    fn the_first_matching_path_entry_wins() {
        let first = tempfile::tempdir().expect("tempdir");
        let second = tempfile::tempdir().expect("tempdir");
        let expected = script(first.path(), "gleam", "echo first");
        script(second.path(), "gleam", "echo second");

        let path_var = std::env::join_paths([first.path(), second.path()]).expect("join paths");
        assert_eq!(find_in_path("gleam", Some(&path_var)), Some(expected));
    }

    #[cfg(unix)]
    #[test]
    fn non_executable_and_directory_entries_are_skipped() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("gleam"), "not executable").expect("writes file");
        std::fs::create_dir(dir.path().join("erl")).expect("creates directory");

        let path_var = std::env::join_paths([dir.path()]).expect("join paths");
        assert_eq!(find_in_path("gleam", Some(&path_var)), None);
        assert_eq!(find_in_path("erl", Some(&path_var)), None);
    }

    #[cfg(unix)]
    #[test]
    fn empty_path_entries_are_not_the_working_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        script(dir.path(), "gleam", "echo hi");
        // A leading `:` must not be read as "look in `.`".
        let path_var = OsString::from(format!(":{}", dir.path().display()));
        assert!(find_in_path("gleam", Some(&path_var)).is_some());
        assert_eq!(find_in_path("nope", Some(&path_var)), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_successful_probe_returns_its_stdout() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = script(dir.path(), "prog", "echo gleam 9.9.9");
        let output = run_with_timeout(&path, &[], PROBE_TIMEOUT).expect("runs");
        assert!(output.success);
        assert_eq!(
            parse_gleam_version(&output.stdout).as_deref(),
            Some("9.9.9")
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_failing_probe_reports_failure_rather_than_erroring() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = script(dir.path(), "prog", "echo boom >&2; exit 1");
        let output = run_with_timeout(&path, &[], PROBE_TIMEOUT).expect("runs");
        assert!(!output.success);
    }

    #[cfg(unix)]
    #[test]
    fn a_hanging_probe_times_out_and_is_killed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = script(dir.path(), "prog", "sleep 60");
        let started = Instant::now();
        let error =
            run_with_timeout(&path, &[], Duration::from_millis(200)).expect_err("should time out");
        assert!(matches!(error, ProbeError::Timeout { .. }), "{error}");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the timeout must not wait for the child"
        );
    }

    /// Regression for the A0 review, round 2: the success path joined the
    /// reader threads, and a grandchild that inherited the pipes held them open
    /// long past the deadline. A 200 ms budget waited out a 30 s `sleep`.
    #[cfg(unix)]
    #[test]
    fn a_grandchild_holding_the_pipes_cannot_outlast_the_timeout() {
        let dir = tempfile::tempdir().expect("tempdir");
        // `sleep` inherits stdout and stderr and keeps the write ends open
        // after the shell that spawned it has already exited. Ten seconds is
        // twice the window this test allows, so the assertion below cannot pass
        // merely because the grandchild happened to finish first; it is short
        // enough that the detached process is gone soon after the suite.
        let path = script(dir.path(), "prog", "sleep 10 & echo gleam 1.2.3");
        let started = Instant::now();
        let output = run_with_timeout(&path, &[], Duration::from_millis(200)).expect("runs");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "waited {:?} on a 200ms budget",
            started.elapsed()
        );
        assert!(output.success);
        assert_eq!(
            parse_gleam_version(&output.stdout).as_deref(),
            Some("1.2.3"),
            "the output written before the deadline must still be reported"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_chatty_probe_does_not_deadlock_on_a_full_pipe() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Far more than a pipe buffer, on both stdout and stderr.
        let path = script(
            dir.path(),
            "prog",
            "i=0; while [ $i -lt 4000 ]; do echo aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa; \
             echo bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb >&2; i=$((i+1)); done; echo gleam 1.2.3",
        );
        let output = run_with_timeout(&path, &[], PROBE_TIMEOUT).expect("runs");
        assert!(output.success);
        assert!(output.stdout.ends_with("gleam 1.2.3\n"));
    }

    /// Regression for the A0 review: `cargo test` flaked with `ETXTBSY` about
    /// once in thirteen runs. A thread that had just written a script exec'd it
    /// while a sibling thread's `Command::spawn` still sat between `fork` and
    /// `exec`, holding an inherited duplicate of the write descriptor.
    #[cfg(unix)]
    #[test]
    fn scripts_written_and_run_in_parallel_are_never_text_file_busy() {
        let dir = tempfile::tempdir().expect("tempdir");
        let workers: Vec<_> = (0..8)
            .map(|worker| {
                let root = dir.path().to_owned();
                std::thread::spawn(move || {
                    for round in 0..25 {
                        let own = root.join(format!("{worker}-{round}"));
                        std::fs::create_dir(&own).expect("creates directory");
                        let path = script(&own, "prog", "echo gleam 0.0.1");
                        let output = run_with_timeout(&path, &[], PROBE_TIMEOUT)
                            .unwrap_or_else(|error| panic!("{worker}-{round}: {error}"));
                        assert!(output.success, "{worker}-{round}");
                    }
                })
            })
            .collect();
        for worker in workers {
            worker.join().expect("worker thread");
        }
    }

    /// Regression for the A0 review, round 2: a `waitpid` failure used to be
    /// reported as [`ProbeError::Spawn`], telling the user a program that is
    /// running could not be started. The arm needs fault injection to reach, so
    /// the variant's own wording is what this pins down.
    #[test]
    fn a_wait_failure_does_not_claim_the_program_could_not_be_run() {
        let wait = ProbeError::Wait {
            program: "/usr/bin/gleam".to_owned(),
            source: std::io::Error::from(std::io::ErrorKind::PermissionDenied),
        };
        assert!(
            wait.to_string()
                .starts_with("cannot wait for `/usr/bin/gleam`"),
            "{wait}"
        );

        let spawn = ProbeError::Spawn {
            program: "/usr/bin/gleam".to_owned(),
            source: std::io::Error::from(std::io::ErrorKind::NotFound),
        };
        assert!(
            spawn.to_string().starts_with("cannot run `/usr/bin/gleam`"),
            "{spawn}"
        );
    }

    #[test]
    fn spawning_a_missing_program_is_a_spawn_error() {
        let error = run_with_timeout(Path::new("/nonexistent/ginary-probe"), &[], PROBE_TIMEOUT)
            .expect_err("should fail to spawn");
        assert!(matches!(error, ProbeError::Spawn { .. }), "{error}");
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
        assert!(text.ends_with("gleam: not found\n"));
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
        let report = Report::gather_from(&[], None, &cache_snapshot("/srv/ginary-cache"));
        assert_eq!(report.cache_dir, Some(PathBuf::from("/srv/ginary-cache")));
        assert_eq!(report.cache_dir_source, Some("GINARY_CACHE_DIR"));
        assert_eq!(report.cache_dir_error, None);
        assert!(report.tools.is_empty());
    }

    #[test]
    fn gathering_records_why_the_cache_directory_is_unresolved() {
        let report = Report::gather_from(&[], None, &EnvSnapshot::default());
        assert_eq!(report.cache_dir, None);
        assert_eq!(report.cache_dir_source, None);
        assert!(report.cache_dir_error.is_some(), "{report:?}");
    }

    /// Regression for the A0 review: the `try_wait` error path returned without
    /// killing or reaping, abandoning a running child. The obligation now lives
    /// in the guard's destructor, which is what this test pins down; the error
    /// path itself cannot be induced without fault injection.
    #[cfg(unix)]
    #[test]
    fn dropping_the_child_guard_kills_and_reaps_the_child() {
        let dir = tempfile::tempdir().expect("tempdir");
        let beat = dir.path().join("beat");
        let path = script(
            dir.path(),
            "prog",
            &format!(
                "while true; do echo tick >> {}; sleep 0.02; done",
                beat.display()
            ),
        );
        let child = Command::new(&path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawns");
        let pid = child.id();
        let guard = ChildGuard(child);

        // Wait until the child is demonstrably running.
        let started = Instant::now();
        while std::fs::metadata(&beat).map(|meta| meta.len()).unwrap_or(0) == 0 {
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "child never ran"
            );
            std::thread::sleep(Duration::from_millis(10));
        }

        drop(guard);

        let after_drop = std::fs::metadata(&beat).expect("beat file").len();
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(
            std::fs::metadata(&beat).expect("beat file").len(),
            after_drop,
            "the child kept running after its guard was dropped"
        );

        #[cfg(target_os = "linux")]
        assert!(
            !Path::new(&format!("/proc/{pid}")).exists(),
            "process {pid} was killed but not reaped"
        );
        let _ = pid;
    }
}
