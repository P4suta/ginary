// SPDX-License-Identifier: MIT OR Apache-2.0
//! Running a child process under a deadline, from a test.
//!
//! Two helpers spawn a real program: `fixture::FixtureProject::export_shipment`
//! runs `gleam`, and `erl::run_staged` boots a whole BEAM. Neither may hang the
//! test binary. `Command::output` would: it waits forever, and a runtime that
//! fails to halt would cost the suite its own timeout with no diagnosis at all.
//!
//! `src/process.rs` is the product-side answer to the same hazard, and it is
//! deliberately not what this calls. `run_with_timeout` takes a program and
//! `&str` arguments and returns captured text; both callers here need an
//! environment built from nothing, a working directory, `OsString` arguments
//! and the child's exit *code*. What they borrow is the discipline rather than
//! the signature, and the two are the same discipline: stdin is the null
//! device, so `-noshell` can never block on the harness's terminal; both pipes
//! are drained by their own threads, so a chatty child cannot fill one and
//! stop; and a child that outlives its budget is killed and reported.

use std::io::Read;
use std::process::{Command, Output, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

/// How often a running child is polled for completion.
const POLL: Duration = Duration::from_millis(20);

/// How long the readers get to reach end of file after the child has exited.
///
/// Exiting closes the child's own ends of the pipes, so this is slack for the
/// reader threads to be scheduled rather than a second budget.
const DRAIN_GRACE: Duration = Duration::from_secs(10);

/// Runs `command` to completion within `budget`, capturing both output streams.
///
/// `what` names the child in every panic message, because a test that fails
/// here has usually failed for a reason outside its own assertions.
///
/// # Panics
///
/// If the program cannot be spawned or waited for, if it does not exit within
/// `budget` — it is killed first — or if its output cannot be drained within
/// [`DRAIN_GRACE`] of its exit, which means something the child left running
/// still holds the pipes.
pub fn run_bounded(command: &mut Command, budget: Duration, what: &str) -> Output {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|error| panic!("cannot run {what}: {error}"));

    let stdout = drain(child.stdout.take());
    let stderr = drain(child.stderr.take());

    let deadline = Instant::now() + budget;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(POLL),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                panic!(
                    "{what} did not exit within {}s and was killed",
                    budget.as_secs()
                );
            }
            Err(error) => panic!("cannot wait for {what}: {error}"),
        }
    };

    Output {
        status,
        stdout: collect(stdout, what, "standard output"),
        stderr: collect(stderr, what, "standard error"),
    }
}

/// Reads a pipe to the end on a thread of its own.
fn drain(pipe: Option<impl Read + Send + 'static>) -> Receiver<Vec<u8>> {
    let (sender, receiver) = mpsc::channel();
    if let Some(mut pipe) = pipe {
        std::thread::spawn(move || {
            let mut buffer = Vec::new();
            let _ = pipe.read_to_end(&mut buffer);
            let _ = sender.send(buffer);
        });
    } else {
        let _ = sender.send(Vec::new());
    }
    receiver
}

/// Takes what a reader thread collected, or says who is still holding the pipe.
fn collect(reader: Receiver<Vec<u8>>, what: &str, stream: &str) -> Vec<u8> {
    reader.recv_timeout(DRAIN_GRACE).unwrap_or_else(|error| {
        panic!("cannot read the {stream} of {what}: {error}; something it started still holds it")
    })
}
