// SPDX-License-Identifier: MIT OR Apache-2.0
//! Finding and running the external programs ginary shells out to.
//!
//! Two things live here, and both are needed by more than one caller:
//! [`find_in_path`], the `which(1)` rule, and [`run_with_timeout`], a bounded
//! child process whose output is captured. `doctor` probes four tools with
//! them; `otp` asks `erl` where it is installed. Neither wants a second,
//! subtly different implementation of "run this and do not hang".
//!
//! The hard part is the timeout. A child can outlive its own exit through a
//! grandchild that inherited the pipes, so the readers are detached threads
//! that publish what they have read rather than threads the caller joins. The
//! execution budget plus finite cleanup slack bounds the whole call — see
//! [`run_with_timeout`] for what that costs on the timeout path.
//!
//! Nothing in this module runs on the launcher path.

use std::borrow::Cow;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

mod capture;
pub use capture::{
    CAPTURE_LIMIT, CapturedOutput, ChildCleanup, ProcessReport, run_command, wait_child,
};

/// How often a running child is polled for completion.
const POLL_INTERVAL: Duration = Duration::from_millis(20);

/// The time output readers get after bounded child cleanup.
///
/// Exiting closes the child's own ends of the pipes, so a reader that nothing
/// else is holding open reaches end of file at once. This is slack for that
/// thread to be scheduled. Both readers share this deadline.
const DRAIN_GRACE: Duration = Duration::from_millis(500);

/// How much of a pipe the reader threads move per `read` call.
const DRAIN_CHUNK: usize = 8 * 1024;

/// The platform's bit bucket, used to keep child processes from writing files.
///
/// One `#[cfg]` pair became one rule: [`crate::platform::null_device`] states
/// which name each operating system has for it, so an expectation in a test
/// can compose the same answer rather than pinning one host's, and both
/// answers are asserted on whichever machine the suite runs on.
#[cfg(feature = "cli")]
pub(crate) const NULL_DEVICE: &str = crate::platform::null_device(crate::platform::HOST);

/// The bit bucket this build sends a child's crash dump to, readable from a
/// test.
///
/// The constant itself is `pub(crate)` because nothing outside the crate has
/// a use for it; this accessor exists so that
/// `tests/regressions/e11_the_beam_argv_named_the_unix_bit_bucket.rs` can hold
/// the production value and the rule against each other, which is the only
/// thing that keeps them from drifting apart.
#[cfg(feature = "cli")]
pub const fn null_device_here() -> &'static str {
    NULL_DEVICE
}

/// What a bounded child process produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessOutput {
    /// Whether the child exited with a success status.
    pub success: bool,
    /// Captured standard output, lossily decoded as UTF-8.
    pub stdout: String,
    /// Captured standard error, lossily decoded as UTF-8.
    ///
    /// A program that fails usually says why here and nothing at all on
    /// standard output, so a caller reporting a failure has to be able to
    /// quote it. Both pipes are drained either way, because a full one would
    /// block the child.
    pub stderr: String,
}

/// Why a child process produced no output.
#[derive(Debug, thiserror::Error)]
pub enum ProcessError {
    /// A failed program's bounded output, retained with its original cause.
    #[error("{source}; {detail}; stdout: {stdout}; stderr: {stderr}")]
    Captured {
        /// Original execution failure.
        #[source]
        source: Box<ProcessError>,
        /// Standard output before failure.
        stdout: String,
        /// Standard error before failure.
        stderr: String,
        /// Output completeness and child cleanup details.
        detail: String,
    },
    /// The output cannot be used as a complete parse input.
    #[error("`{program}` output is incomplete: {detail}")]
    Incomplete {
        /// Executed program.
        program: String,
        /// Truncation, read error, or pipe deadline information.
        detail: String,
    },
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
    /// Distinct from [`ProcessError::Spawn`]: the program is running, or has
    /// run. Reporting this as a spawn failure would tell the user that a
    /// program they can see in the process table could not be started.
    #[error("cannot wait for `{program}`: {source}")]
    Wait {
        /// The program that could not be waited for.
        program: String,
        /// The underlying operating system error.
        #[source]
        source: std::io::Error,
    },
    /// The program did not exit within the timeout; cleanup was requested.
    #[error("`{program}` did not exit within {}ms", .timeout.as_millis())]
    Timeout {
        /// The program that hung.
        program: String,
        /// The budget it exceeded.
        timeout: Duration,
    },
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
        use std::os::unix::fs::PermissionsExt as _;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// Runs a program with an execution deadline and bounded output capture.
///
/// Standard output and standard error are drained by dedicated threads, so a
/// chatty child cannot deadlock on a full pipe while we are polling it, and
/// both are returned: what a failing program wrote to standard error is the
/// only explanation its caller has. The execution deadline is followed by at
/// most 500 ms of child cleanup and 500 ms of pipe observation, plus scheduling
/// overhead. Readers are detached because descendants can retain inherited
/// pipes after the direct child exits.
///
/// Cleanup requests termination of a running direct child and polls for its
/// status. If the operating system refuses termination or reaping, the failure
/// is reported and a background reaper owns the unfinished child. Descendants
/// are not terminated. [`run_command`] exposes exact retained bytes, omission
/// counts, EOF status and cleanup details for callers needing this evidence.
///
/// # Errors
///
/// [`ProcessError::Spawn`] when the program cannot be started,
/// [`ProcessError::Wait`] when it cannot be waited for, and
/// [`ProcessError::Timeout`] when it outlives `timeout`. Failure evidence wraps
/// the original cause in [`ProcessError::Captured`]; truncated or unobserved
/// output is refused as [`ProcessError::Incomplete`] instead of accepted for parsing.
pub fn run_with_timeout(
    program: &Path,
    args: &[&str],
    timeout: Duration,
) -> Result<ProcessOutput, ProcessError> {
    run_in_dir_with_timeout(program, args, None, timeout)
}

/// [`run_with_timeout`], started in a working directory of the caller's choice.
///
/// `dir` is [`None`] for "wherever this process is", which is what every
/// probe wants, and `Some` for a program whose *input* is the directory it
/// runs in. `gleam export erlang-shipment` is the only such caller today: it
/// compiles the project the working directory belongs to, so the build cannot
/// simply run it wherever `ginary build` was typed.
///
/// # Errors
///
/// As [`run_with_timeout`].
pub fn run_in_dir_with_timeout(
    program: &Path,
    args: &[&str],
    dir: Option<&Path>,
    timeout: Duration,
) -> Result<ProcessOutput, ProcessError> {
    run_env_in_dir_with_timeout(program, args, &[], dir, timeout)
}

/// [`run_in_dir_with_timeout`], with variables added to the environment.
///
/// `env` is *added to* what this process already has rather than replacing it,
/// because the one caller is `native::run_hook` and a build hook is a
/// developer's own command line: it needs the `PATH` that finds its compiler
/// and the `HOME` that finds its toolchain, and the variables named here are
/// the contract ginary adds on top. A name given twice takes the last value,
/// which is `Command`'s own rule.
///
/// # Errors
///
/// As [`run_with_timeout`].
pub fn run_env_in_dir_with_timeout(
    program: &Path,
    args: &[&str],
    env: &[(&str, OsString)],
    dir: Option<&Path>,
    timeout: Duration,
) -> Result<ProcessOutput, ProcessError> {
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (name, value) in env {
        command.env(name, value);
    }
    if let Some(dir) = dir {
        command.current_dir(dir);
    }
    let mut report = run_command(&mut command, timeout, CAPTURE_LIMIT);
    let detail = format!(
        "stdout omitted {} bytes, EOF={}, read error={:?}; stderr omitted {} bytes, EOF={}, read error={:?}; cleanup={:?}",
        report.stdout.omitted_bytes,
        report.stdout.complete,
        report.stdout.error,
        report.stderr.omitted_bytes,
        report.stderr.complete,
        report.stderr.error,
        report.cleanup
    );
    if let Some(source) = report.error.take() {
        if report.stdout.bytes.is_empty()
            && report.stderr.bytes.is_empty()
            && report
                .cleanup
                .as_ref()
                .is_none_or(|cleanup| cleanup.error.is_none())
        {
            return Err(source);
        }
        return Err(ProcessError::Captured {
            source: Box::new(source),
            stdout: report.stdout.text(),
            stderr: report.stderr.text(),
            detail,
        });
    }
    if !report.stdout.is_complete()
        || !report.stderr.is_complete()
        || report
            .cleanup
            .as_ref()
            .is_some_and(|cleanup| !cleanup.reaped || cleanup.error.is_some())
    {
        return Err(ProcessError::Incomplete {
            program: program.display().to_string(),
            detail: format!(
                "{detail}; stdout: {}; stderr: {}",
                report.stdout.text(),
                report.stderr.text()
            ),
        });
    }
    Ok(ProcessOutput {
        success: report.success(),
        stdout: report.stdout.text(),
        stderr: report.stderr.text(),
    })
}

/// Takes a lock result, treating poisoning as ordinary access.
///
/// The only data behind these locks is a byte buffer, and a reader thread that
/// panicked mid-append leaves it merely truncated, never inconsistent. Partial
/// output is exactly what the timeout path already returns.
fn unpoison<T>(result: std::sync::LockResult<T>) -> T {
    result.unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The characters a POSIX shell reads as themselves, so a word made only of
/// them needs no quoting.
///
/// Deliberately short: everything a version, a target, a variant or an
/// ordinary path is made of, and nothing whose meaning depends on where in the
/// word it sits. `~` is absent because a leading one is expanded, `=` and `:`
/// are here because they appear inside paths and options and neither is
/// special to `sh` in a word.
const SHELL_SAFE: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789._-/=:,+@%";

/// Renders one word so that a POSIX shell reads it back as this exact string.
///
/// Every command ginary *suggests* is a command a user pastes into a shell,
/// and a path is not a word: `~/My Documents/catalog.json` pasted bare is two
/// arguments and `$(id)` is a command substitution. A remedy that has to be
/// repaired before it runs is not a remedy, which is what
/// `catalog::fetch_command` found — see
/// `tests/regressions/c4_a_catalog_path_with_a_space_was_not_quoted.rs`.
///
/// A word of nothing but unremarkable characters — letters, digits and
/// `._-/=:,+@%` — is returned as it stands, because quoting `29.0.5` would
/// only make the line harder to read. Anything
/// else is wrapped in single quotes, inside which a shell expands nothing at
/// all; the one character that cannot appear there is the single quote itself,
/// which is written as `'\''` — close, escape one, reopen. An empty word
/// quotes to `''`, which is how a shell is told about an argument that is
/// there and empty.
///
/// This renders for `/bin/sh`. It is not an escaper for `cmd.exe`, and nothing
/// here builds a command line for one.
#[must_use]
pub fn shell_quote(word: &str) -> Cow<'_, str> {
    if !word.is_empty() && word.chars().all(|c| SHELL_SAFE.contains(c)) {
        return Cow::Borrowed(word);
    }
    let mut quoted = String::with_capacity(word.len() + 2);
    quoted.push('\'');
    for c in word.chars() {
        if c == '\'' {
            quoted.push_str("'\\''");
        } else {
            quoted.push(c);
        }
    }
    quoted.push('\'');
    Cow::Owned(quoted)
}

/// [`shell_quote`] over a path, which is what most suggested commands carry.
///
/// A path that is not UTF-8 is rendered lossily, the same way every message in
/// this crate renders one: the remedy is already about a path the user can see
/// and a replacement character is more use than no line at all.
#[must_use]
pub fn shell_quote_path(path: &Path) -> String {
    shell_quote(&path.to_string_lossy()).into_owned()
}

/// A child process whose bounded cleanup starts when it goes out of scope.
///
/// The obligation belongs to the value rather than to each `return`, so a new
/// error path cannot forget it: the A0 review found exactly that, a `try_wait`
/// failure that abandoned a running child.
struct ChildGuard(Option<Child>);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(child) = self.0.take() {
            let _ = capture::finish_child(child);
        }
    }
}

/// Writing throwaway executables for the tests of this crate.
///
/// It lives here rather than in each test module because `doctor` probes
/// programs it must first create, and two copies of the `ETXTBSY` retry loop
/// below would be two chances to get it wrong.
#[cfg(all(test, unix))]
pub(crate) mod test_support {
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::time::Duration;

    /// The argument that makes a script written by [`script`] exit before its
    /// body runs, so exec-ability can be probed without any side effect.
    const EXEC_PROBE: &str = "--ginary-exec-probe";

    /// Creates an executable shell script and returns its path.
    ///
    /// The script is not returned until it has actually been exec'd once; see
    /// [`wait_until_executable`].
    ///
    /// # Panics
    ///
    /// If the script cannot be written, marked executable, or exec'd.
    pub(crate) fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt as _;
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
    ///
    /// # Panics
    ///
    /// If the script is still not executable after the retry budget.
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
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::time::Instant;

    #[cfg(unix)]
    use super::test_support::script;

    /// A bound long enough for a healthy child and short enough for a test.
    const TEST_TIMEOUT: Duration = Duration::from_secs(10);

    #[test]
    fn an_ordinary_word_is_left_alone() {
        for word in [
            "29.0.5",
            "linux-x86_64-musl",
            "/home/u/dist/otp/catalog.json",
            "static",
        ] {
            assert_eq!(shell_quote(word), word, "{word} needs no quoting");
            assert!(
                matches!(shell_quote(word), Cow::Borrowed(_)),
                "and is not copied"
            );
        }
    }

    #[test]
    fn a_word_with_a_space_becomes_one_argument() {
        assert_eq!(
            shell_quote("/home/u/My Documents/catalog.json"),
            "'/home/u/My Documents/catalog.json'"
        );
    }

    #[test]
    fn the_characters_a_shell_would_act_on_are_quoted_rather_than_dropped() {
        for word in [
            "a$(id)b", "a;b", "a|b", "a&b", "a>b", "a*b", "~/x", "a\\b", "a\nb", "a\"b",
        ] {
            let quoted = shell_quote(word);
            assert!(
                quoted.starts_with('\'') && quoted.ends_with('\''),
                "{word} is quoted"
            );
            assert!(
                quoted.contains(word),
                "and its bytes survive verbatim: {quoted}"
            );
        }
    }

    #[test]
    fn a_single_quote_closes_escapes_and_reopens() {
        assert_eq!(shell_quote("it's"), r#"'it'\''s'"#);
    }

    #[test]
    fn an_empty_word_quotes_to_a_pair_of_quotes() {
        assert_eq!(shell_quote(""), "''");
    }

    #[cfg(unix)]
    #[test]
    fn a_shell_reads_back_exactly_what_was_quoted() {
        for word in [
            "plain",
            "two words",
            "it's",
            "a$(echo no)b",
            "a;b",
            "",
            "~/x",
            "a\\b",
        ] {
            let output = Command::new("/bin/sh")
                .args(["-c", &format!("printf %s {}", shell_quote(word))])
                .output()
                .expect("a shell");
            assert_eq!(
                String::from_utf8_lossy(&output.stdout),
                word,
                "a shell reads `{word}` back unchanged"
            );
        }
    }

    #[test]
    fn a_path_quotes_the_way_a_word_does() {
        assert_eq!(
            shell_quote_path(Path::new("/tmp/my catalogs/c.json")),
            "'/tmp/my catalogs/c.json'"
        );
        assert_eq!(shell_quote_path(Path::new("/tmp/c.json")), "/tmp/c.json");
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
    fn a_successful_run_returns_its_stdout() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = script(dir.path(), "prog", "echo gleam 9.9.9");
        let output = run_with_timeout(&path, &[], TEST_TIMEOUT).expect("runs");
        assert!(output.success);
        assert_eq!(output.stdout, "gleam 9.9.9\n");
    }

    /// Regression for the A1a review: standard error was drained to keep the
    /// pipe from filling and then dropped, so a child that failed with its
    /// diagnosis on standard error was reported with an empty explanation.
    #[cfg(unix)]
    #[test]
    fn a_failing_run_reports_failure_and_keeps_what_it_wrote_to_standard_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = script(dir.path(), "prog", "echo boom >&2; exit 1");
        let output = run_with_timeout(&path, &[], TEST_TIMEOUT).expect("runs");
        assert!(!output.success);
        assert_eq!(output.stdout, "");
        assert_eq!(output.stderr, "boom\n");
    }

    #[cfg(unix)]
    #[test]
    fn the_two_streams_are_captured_separately() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = script(dir.path(), "prog", "echo out; echo err >&2");
        let output = run_with_timeout(&path, &[], TEST_TIMEOUT).expect("runs");
        assert_eq!(output.stdout, "out\n");
        assert_eq!(output.stderr, "err\n");
    }

    #[cfg(unix)]
    #[test]
    fn arguments_reach_the_program() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = script(dir.path(), "prog", "echo \"$1/$2\"");
        let output = run_with_timeout(&path, &["one", "two"], TEST_TIMEOUT).expect("runs");
        assert_eq!(output.stdout, "one/two\n");
    }

    #[cfg(unix)]
    #[test]
    fn a_hanging_program_times_out_and_is_killed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = script(dir.path(), "prog", "sleep 60");
        let started = Instant::now();
        let error =
            run_with_timeout(&path, &[], Duration::from_millis(200)).expect_err("should time out");
        assert!(matches!(error, ProcessError::Timeout { .. }), "{error}");
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
        let output = run_command(
            &mut Command::new(&path),
            Duration::from_millis(200),
            CAPTURE_LIMIT,
        );
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "waited {:?} on a 200ms budget",
            started.elapsed()
        );
        assert!(output.success());
        assert!(
            !output.stdout.is_complete(),
            "the inherited pipe did not reach EOF"
        );
        assert!(
            output.stdout.text().contains("gleam 1.2.3"),
            "the output written before the deadline must still be reported: {:?}",
            output.stdout
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_chatty_program_does_not_deadlock_on_a_full_pipe() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Far more than a pipe buffer, on both stdout and stderr.
        let path = script(
            dir.path(),
            "prog",
            "i=0; while [ $i -lt 4000 ]; do echo aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa; \
             echo bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb >&2; i=$((i+1)); done; echo gleam 1.2.3",
        );
        let output = run_with_timeout(&path, &[], TEST_TIMEOUT).expect("runs");
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
                        let output = run_with_timeout(&path, &[], TEST_TIMEOUT)
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
    /// reported as [`ProcessError::Spawn`], telling the user a program that is
    /// running could not be started. The arm needs fault injection to reach, so
    /// the variant's own wording is what this pins down.
    #[test]
    fn a_wait_failure_does_not_claim_the_program_could_not_be_run() {
        let wait = ProcessError::Wait {
            program: "/usr/bin/gleam".to_owned(),
            source: std::io::Error::from(std::io::ErrorKind::PermissionDenied),
        };
        assert!(
            wait.to_string()
                .starts_with("cannot wait for `/usr/bin/gleam`"),
            "{wait}"
        );

        let spawn = ProcessError::Spawn {
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
        let error = run_with_timeout(Path::new("/nonexistent/ginary-probe"), &[], TEST_TIMEOUT)
            .expect_err("should fail to spawn");
        assert!(matches!(error, ProcessError::Spawn { .. }), "{error}");
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
        let guard = ChildGuard(Some(child));

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
