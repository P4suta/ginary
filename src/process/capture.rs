// SPDX-License-Identifier: MIT OR Apache-2.0
//! Bounded subprocess evidence, including the path that times out.
use std::collections::VecDeque;
use std::io::Read;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use super::{ChildGuard, DRAIN_CHUNK, DRAIN_GRACE, POLL_INTERVAL, ProcessError};

/// Default maximum retained bytes per output stream.
pub const CAPTURE_LIMIT: usize = 1024 * 1024;

/// The retained tail of one pipe and whether it is a complete observation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CapturedOutput {
    /// Exact retained bytes, including non-UTF-8 output.
    pub bytes: Vec<u8>,
    /// Bytes read but not retained because the capture limit was reached.
    pub omitted_bytes: u64,
    /// Whether the pipe reached EOF before its observation deadline.
    pub complete: bool,
    /// A read or reader-start error, if any.
    pub error: Option<String>,
}

impl CapturedOutput {
    /// Human-readable output; exact bytes remain available in `bytes`.
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.bytes).into_owned()
    }

    /// Whether every output byte was successfully observed and retained.
    pub fn is_complete(&self) -> bool {
        self.complete && self.omitted_bytes == 0 && self.error.is_none()
    }
}

/// Subprocess completion and evidence, returned even after spawn or wait failure.
#[derive(Debug)]
pub struct ProcessReport {
    /// Observed exit status, including the platform's signal information.
    pub status: Option<ExitStatus>,
    /// Why execution did not complete normally.
    pub error: Option<ProcessError>,
    /// Wall time spent executing and collecting bounded evidence.
    pub elapsed: Duration,
    /// Retained standard output.
    pub stdout: CapturedOutput,
    /// Retained standard error.
    pub stderr: CapturedOutput,
    /// What happened while stopping and reaping a spawned child.
    /// Absent if the command could not be spawned.
    pub cleanup: Option<ChildCleanup>,
}

/// Bounded child cleanup, including failures the caller may need to investigate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChildCleanup {
    /// The operating system process identifier.
    pub pid: u32,
    /// Whether cleanup needed to request termination.
    pub kill_requested: bool,
    /// Whether the child's exit status was collected before returning.
    pub reaped: bool,
    /// Whether an unfinished child was handed to a background reaper.
    pub background_reaper: bool,
    /// Why cleanup could not be completed within its bounded wait.
    pub error: Option<String>,
}

impl ProcessReport {
    /// Whether the program exited successfully; output completeness is independent.
    pub fn success(&self) -> bool {
        self.error.is_none()
            && self.status.is_some_and(|status| status.success())
            && self
                .cleanup
                .as_ref()
                .is_none_or(|cleanup| cleanup.reaped && cleanup.error.is_none())
    }
}

/// Runs a configured command and preserves bounded evidence on every return path.
///
/// Standard input is closed and both output pipes are drained independently. Descendants
/// are not killed; if they retain a pipe, capture stops retaining bytes at the deadline.
/// A caller parsing output must check `CapturedOutput::is_complete` before accepting it.
pub fn run_command(command: &mut Command, timeout: Duration, limit: usize) -> ProcessReport {
    let started = Instant::now();
    let program = command.get_program().to_string_lossy().into_owned();
    let child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    match child {
        Ok(child) => observe_child(child, program, started, timeout, limit),
        Err(source) => ProcessReport {
            status: None,
            error: Some(ProcessError::Spawn { program, source }),
            elapsed: started.elapsed(),
            stdout: CapturedOutput::default(),
            stderr: CapturedOutput::default(),
            cleanup: None,
        },
    }
}

/// Observes an already-spawned child with the same evidence and cleanup rules.
///
/// The child should have piped output streams. The budget starts when this
/// function is called; callers starting several children must account for the
/// time each child has already spent running before they begin waiting.
pub fn wait_child(child: Child, timeout: Duration, limit: usize) -> ProcessReport {
    let program = format!("child {}", child.id());
    observe_child(child, program, Instant::now(), timeout, limit)
}

/// Runs the observation loop while a guard owns the child on every exit path.
fn observe_child(
    mut child: Child,
    program: String,
    started: Instant,
    timeout: Duration,
    limit: usize,
) -> ProcessReport {
    let stdout = drain(child.stdout.take(), limit);
    let stderr = drain(child.stderr.take(), limit);
    let mut guard = ChildGuard(Some(child));
    let (status, error) = loop {
        let Some(child) = guard.0.as_mut() else {
            break (None, None);
        };
        match child.try_wait() {
            Ok(Some(status)) => break (Some(status), None),
            Ok(None) if started.elapsed() < timeout => std::thread::sleep(POLL_INTERVAL),
            Ok(None) => break (None, Some(ProcessError::Timeout { program, timeout })),
            Err(source) => break (None, Some(ProcessError::Wait { program, source })),
        }
    };
    // Never call blocking wait after a failed kill. An unresponsive or
    // inaccessible child is explicitly reported and reaped in the background.
    let (reaped_status, cleanup) = match guard.0.take() {
        Some(child) => {
            let (status, cleanup) = finish_child(child);
            (status, Some(cleanup))
        }
        None => (None, None),
    };
    let drained_by = Instant::now() + DRAIN_GRACE;
    let stdout = stdout.take_until(drained_by);
    let stderr = stderr.take_until(drained_by);
    ProcessReport {
        status: status.or(reaped_status),
        error,
        elapsed: started.elapsed(),
        stdout,
        stderr,
        cleanup,
    }
}

/// Cleanup gets its own finite slack after the execution budget is exhausted.
const CLEANUP_GRACE: Duration = Duration::from_millis(500);

/// The nonblocking operations cleanup needs, injectable for operating-system failures.
trait ChildControl {
    fn id(&self) -> u32;
    fn poll(&mut self) -> std::io::Result<Option<ExitStatus>>;
    fn kill(&mut self) -> std::io::Result<()>;
}

impl ChildControl for Child {
    fn id(&self) -> u32 {
        Child::id(self)
    }
    fn poll(&mut self) -> std::io::Result<Option<ExitStatus>> {
        self.try_wait()
    }
    fn kill(&mut self) -> std::io::Result<()> {
        Child::kill(self)
    }
}

/// Stops and polls a child without ever making a blocking wait call.
fn stop_child(
    child: &mut impl ChildControl,
    budget: Duration,
) -> (Option<ExitStatus>, ChildCleanup) {
    let mut cleanup = ChildCleanup {
        pid: child.id(),
        kill_requested: false,
        reaped: false,
        background_reaper: false,
        error: None,
    };
    if let Ok(Some(status)) = child.poll() {
        cleanup.reaped = true;
        return (Some(status), cleanup);
    }
    cleanup.kill_requested = true;
    let kill_error = child.kill().err();
    let started = Instant::now();
    loop {
        match child.poll() {
            Ok(Some(status)) => {
                cleanup.reaped = true;
                return (Some(status), cleanup);
            }
            Ok(None) if started.elapsed() < budget => {
                std::thread::sleep(POLL_INTERVAL.min(budget.saturating_sub(started.elapsed())));
            }
            outcome => {
                cleanup.error = Some(match (kill_error, outcome) {
                    (Some(kill), Err(wait)) => format!(
                        "cannot kill child {}: {kill}; cannot reap it: {wait}",
                        cleanup.pid
                    ),
                    (Some(kill), _) => format!(
                        "cannot kill child {}: {kill}; exit was not observed within {}ms",
                        cleanup.pid,
                        budget.as_millis()
                    ),
                    (None, Err(wait)) => format!("cannot reap child {}: {wait}", cleanup.pid),
                    (None, _) => format!(
                        "child {} exit was not observed within {}ms after termination",
                        cleanup.pid,
                        budget.as_millis()
                    ),
                });
                return (None, cleanup);
            }
        }
    }
}

/// Completes bounded cleanup, handing an unfinished child to a detached reaper.
pub(super) fn finish_child(mut child: Child) -> (Option<ExitStatus>, ChildCleanup) {
    let (status, mut cleanup) = stop_child(&mut child, CLEANUP_GRACE);
    if !cleanup.reaped {
        match std::thread::Builder::new()
            .name(format!("ginary-reap-{}", cleanup.pid))
            .spawn(move || {
                let _ = child.wait();
            }) {
            Ok(_) => cleanup.background_reaper = true,
            Err(error) => {
                cleanup.error = Some(format!(
                    "{}; cannot start background reaper: {error}",
                    cleanup
                        .error
                        .as_deref()
                        .unwrap_or("child cleanup incomplete")
                ))
            }
        }
    }
    (status, cleanup)
}

#[derive(Default)]
struct Buffer {
    tail: VecDeque<u8>,
    total: u64,
    complete: bool,
    error: Option<String>,
    stopped: bool,
}

struct Drain {
    buffer: Arc<Mutex<Buffer>>,
    finished: mpsc::Receiver<()>,
}

impl Drain {
    fn take_until(self, deadline: Instant) -> CapturedOutput {
        let _ = self
            .finished
            .recv_timeout(deadline.saturating_duration_since(Instant::now()));
        let mut buffer = super::unpoison(self.buffer.lock());
        buffer.stopped = true;
        CapturedOutput {
            omitted_bytes: buffer.total.saturating_sub(buffer.tail.len() as u64),
            bytes: buffer.tail.drain(..).collect(),
            complete: buffer.complete,
            error: buffer.error.take(),
        }
    }
}

fn drain<R: Read + Send + 'static>(pipe: Option<R>, limit: usize) -> Drain {
    let buffer = Arc::new(Mutex::new(Buffer::default()));
    let (sender, finished) = mpsc::channel();
    let writer = Arc::clone(&buffer);
    let spawned = std::thread::Builder::new()
        .name("ginary-output".into())
        .spawn(move || {
            if let Some(mut pipe) = pipe {
                let mut chunk = [0; DRAIN_CHUNK];
                loop {
                    match pipe.read(&mut chunk) {
                        Ok(0) => {
                            super::unpoison(writer.lock()).complete = true;
                            break;
                        }
                        Ok(read) => {
                            let mut buffer = super::unpoison(writer.lock());
                            if buffer.stopped {
                                continue;
                            }
                            buffer.total = buffer.total.saturating_add(read as u64);
                            let drop = buffer
                                .tail
                                .len()
                                .saturating_add(read)
                                .saturating_sub(limit)
                                .min(buffer.tail.len());
                            buffer.tail.drain(..drop);
                            let skip = read.saturating_sub(limit);
                            buffer.tail.extend(&chunk[skip..read]);
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(error) => {
                            super::unpoison(writer.lock()).error = Some(error.to_string());
                            break;
                        }
                    }
                }
            } else {
                super::unpoison(writer.lock()).complete = true;
            }
            let _ = sender.send(());
        });
    if let Err(error) = spawned {
        super::unpoison(buffer.lock()).error = Some(error.to_string());
    }
    Drain { buffer, finished }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_capture_retains_exact_non_utf8_tail_and_counts_discarded_bytes() {
        let bytes: Vec<u8> = (0..20_000).map(|offset| (offset % 256) as u8).collect();
        let output = drain(Some(std::io::Cursor::new(bytes.clone())), 17)
            .take_until(Instant::now() + Duration::from_secs(5));
        assert_eq!(output.bytes, bytes[bytes.len() - 17..]);
        assert_eq!(output.omitted_bytes, 19_983);
        assert!(output.complete);
        assert!(
            !output.is_complete(),
            "a bounded tail is not the entire output"
        );
    }

    #[test]
    fn a_zero_capture_limit_still_drains_and_counts_every_byte() {
        let output = drain(Some(std::io::Cursor::new(vec![0xff; 100])), 0)
            .take_until(Instant::now() + Duration::from_secs(5));
        assert!(output.bytes.is_empty());
        assert_eq!(output.omitted_bytes, 100);
        assert!(output.complete);
    }

    struct FailedRead;

    impl Read for FailedRead {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "injected read failure",
            ))
        }
    }

    #[test]
    fn a_read_failure_is_reported_without_claiming_eof() {
        let output =
            drain(Some(FailedRead), 100).take_until(Instant::now() + Duration::from_secs(5));
        assert!(!output.complete);
        assert!(!output.is_complete());
        assert_eq!(output.error.as_deref(), Some("injected read failure"));
    }

    struct FailedChild {
        poll_count: usize,
        wait_fails: bool,
        exits_after_kill: bool,
    }

    impl ChildControl for FailedChild {
        fn id(&self) -> u32 {
            42
        }
        fn poll(&mut self) -> std::io::Result<Option<ExitStatus>> {
            self.poll_count += 1;
            if self.wait_fails {
                return Err(std::io::Error::other("injected wait failure"));
            }
            if self.exits_after_kill && self.poll_count > 1 {
                #[cfg(unix)]
                use std::os::unix::process::ExitStatusExt as _;
                #[cfg(windows)]
                use std::os::windows::process::ExitStatusExt as _;
                return Ok(Some(ExitStatus::from_raw(0)));
            }
            Ok(None)
        }
        fn kill(&mut self) -> std::io::Result<()> {
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "injected kill failure",
            ))
        }
    }

    #[test]
    fn a_failed_kill_does_not_enter_an_unbounded_wait() {
        let mut child = FailedChild {
            poll_count: 0,
            wait_fails: false,
            exits_after_kill: false,
        };
        let (status, cleanup) = stop_child(&mut child, Duration::ZERO);
        assert!(status.is_none());
        assert_eq!(child.poll_count, 2, "only nonblocking polls are used");
        assert!(cleanup.kill_requested);
        assert!(!cleanup.reaped);
        assert!(
            cleanup
                .error
                .expect("cleanup failure")
                .contains("injected kill failure")
        );
    }

    #[test]
    fn wait_and_kill_errors_both_survive_cleanup() {
        let mut child = FailedChild {
            poll_count: 0,
            wait_fails: true,
            exits_after_kill: false,
        };
        let (_, cleanup) = stop_child(&mut child, Duration::ZERO);
        let error = cleanup.error.expect("cleanup failure");
        assert!(error.contains("injected wait failure"));
        assert!(error.contains("injected kill failure"));
    }

    #[test]
    fn an_exit_racing_with_a_failed_kill_is_still_reaped() {
        let mut child = FailedChild {
            poll_count: 0,
            wait_fails: false,
            exits_after_kill: true,
        };
        let (status, cleanup) = stop_child(&mut child, Duration::ZERO);
        assert!(status.is_some());
        assert!(cleanup.reaped);
        assert!(cleanup.error.is_none());
    }
}
