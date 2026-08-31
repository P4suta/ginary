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
//! budget then bounds the whole call, not just the wait — see
//! [`run_with_timeout`] for what that costs on the timeout path.
//!
//! Nothing in this module runs on the launcher path.

use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How often a running child is polled for completion.
const POLL_INTERVAL: Duration = Duration::from_millis(20);

/// The least time the output readers get after the child has been reaped.
///
/// Exiting closes the child's own ends of the pipes, so a reader that nothing
/// else is holding open reaches end of file at once. This is slack for that
/// thread to be scheduled, not a second budget: when the call's own deadline
/// is further away, the deadline wins.
const DRAIN_GRACE: Duration = Duration::from_millis(500);

/// How much of a pipe the reader threads move per `read` call.
const DRAIN_CHUNK: usize = 8 * 1024;

/// The platform's bit bucket, used to keep child processes from writing files.
#[cfg(windows)]
#[cfg(feature = "cli")]
pub(crate) const NULL_DEVICE: &str = "nul";
/// The platform's bit bucket, used to keep child processes from writing files.
#[cfg(not(windows))]
#[cfg(feature = "cli")]
pub(crate) const NULL_DEVICE: &str = "/dev/null";

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
    /// The program did not exit within the timeout and was killed.
    #[error("`{program}` did not exit within {}s", .timeout.as_secs())]
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

/// Runs a program with a hard timeout, capturing both of its output streams.
///
/// Standard output and standard error are drained by dedicated threads, so a
/// chatty child cannot deadlock on a full pipe while we are polling it, and
/// both are returned: what a failing program wrote to standard error is the
/// only explanation its caller has. The timeout bounds the whole call, not just
/// the wait: the readers are never joined, only given until the deadline (or
/// half a second past the child's exit, whichever is later) to reach end of
/// file. A process the child backgrounded inherits the pipes and can hold them
/// open long after the child itself has exited, so a join would wait on the
/// grandchild instead of on the budget.
///
/// The direct child never outlives the call: it is owned by a guard that kills
/// and reaps it on the way out, whether the call timed out, could not wait, or
/// simply finished. Grandchildren are not killed — ginary does not put these
/// processes in a process group — so on the timeout path the output is whatever
/// the readers had collected by the deadline, and the detached reader threads
/// end when the pipes finally close.
///
/// # Errors
///
/// [`ProcessError::Spawn`] when the program cannot be started,
/// [`ProcessError::Wait`] when it cannot be waited for, and
/// [`ProcessError::Timeout`] when it outlives `timeout`.
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
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(dir) = dir {
        command.current_dir(dir);
    }
    let child = command.spawn().map_err(|source| ProcessError::Spawn {
        program: program.display().to_string(),
        source,
    })?;
    // From here on every exit runs the guard's destructor.
    let mut child = ChildGuard(child);

    let stdout = drain(child.0.stdout.take());
    let stderr = drain(child.0.stderr.take());

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.0.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(POLL_INTERVAL),
            Ok(None) => break None,
            Err(source) => {
                return Err(ProcessError::Wait {
                    program: program.display().to_string(),
                    source,
                });
            }
        }
    };

    let Some(status) = status else {
        return Err(ProcessError::Timeout {
            program: program.display().to_string(),
            timeout,
        });
    };

    let drained_by = deadline.max(Instant::now() + DRAIN_GRACE);
    let stdout = stdout.take_until(drained_by);
    let stderr = stderr.take_until(drained_by);
    Ok(ProcessOutput {
        success: status.success(),
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
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
    use super::test_support::script;

    /// A bound long enough for a healthy child and short enough for a test.
    const TEST_TIMEOUT: Duration = Duration::from_secs(10);

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
        let output = run_with_timeout(&path, &[], Duration::from_millis(200)).expect("runs");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "waited {:?} on a 200ms budget",
            started.elapsed()
        );
        assert!(output.success);
        assert!(
            output.stdout.contains("gleam 1.2.3"),
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
