// SPDX-License-Identifier: MIT OR Apache-2.0
//! The download suite's own HTTP server answered every failed read of a request
//! with `None` — the same answer it gives a peer that closed before saying
//! anything — so a connection whose request it could not read was dropped with
//! nothing counted and nothing answered: exactly the under-count this milestone
//! was opened for, arriving from the one place in the fixture that was still
//! silent.
//!
//! **What went wrong.** `read_request` threads five `.ok()?` sites — the
//! `try_clone`, the request line, each header line and the body's `read_exact`
//! — into an `Option<Request>`, and `serve_one` turns a `None` into `return`.
//! A read that failed and a peer that sent nothing are then the same event, and
//! the first of them is the fixture's failure: the client made a request, the
//! fixture never recorded it, and the test reads a count one short and blames
//! the client. That is the shape of the Windows failure E13 exists for:
//!
//! ```text
//! ---- every_attempt_failing_reports_the_url_and_how_many_were_made ----
//! assertion `left == right` failed
//!   left: 2
//!  right: 3
//! ```
//!
//! (`Windows build and exit-code propagation`
//! <https://github.com/P4suta/ginary/actions/runs/33837795146/job/100913758843>.)
//!
//! E13 fixed the two neighbouring silences — `after_accept` for the accept
//! loop and `after_write` for the reply — and made the fixture's module note
//! say, of the fixture as a whole, that "what it cannot serve through it says
//! rather than discards". This path made that false. It also made it newly
//! reachable in bounded time: the same round added
//! `set_read_timeout(Some(REQUEST_BUDGET))` to every accepted connection, so a
//! peer slower than the budget now *fails* the read where it used to block, and
//! the failure went straight into the `None` that means "nobody asked".
//!
//! **The input.** Any connection whose request the fixture cannot read whole: a
//! peer that opens one and then sends less than a request line before
//! [`REQUEST_BUDGET`] runs out, which is what a stalled client looks like from
//! here.
//!
//! **The correct behaviour.** The two events are two answers.
//! [`after_request_read`] decides between them from the error's kind alone, the
//! way its two neighbours do: a peer that let go — `BrokenPipe`,
//! `ConnectionReset`, `ConnectionAborted` — withdrew its request and is not the
//! fixture's failure, and everything else, the budget's own timeout above all,
//! is recorded where a test reads it. A peer that closes before sending
//! anything is still nothing at all, because that is a connection nobody asked
//! anything on.

use std::io::{ErrorKind, Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use crate::common::http::{
    AfterRequestRead, REQUEST_BUDGET, Reply, TestServer, after_request_read,
};

/// The path the server is scripted with.
const PATH: &str = "/release";

/// How long a client of these tests waits for bytes before giving up.
const READ_BUDGET: Duration = Duration::from_secs(5);

/// How long the test waits for the fixture to give up on the request.
///
/// [`REQUEST_BUDGET`] plus room for a loaded machine to notice.
const REPORT_BUDGET: Duration = Duration::from_secs(30);

#[test]
fn a_read_that_ran_out_of_budget_is_reported() {
    for kind in [ErrorKind::TimedOut, ErrorKind::WouldBlock] {
        assert_eq!(
            after_request_read(kind),
            AfterRequestRead::Reported,
            "{kind:?} is how a socket read timeout is spelled, and a request the fixture could \
             not read is one it neither records nor answers: the client's attempt dies in the \
             transport and the count every test asserts on is short by one"
        );
    }
}

#[test]
fn a_peer_that_let_go_of_a_request_is_not_the_fixtures_failure() {
    for kind in [
        ErrorKind::BrokenPipe,
        ErrorKind::ConnectionReset,
        ErrorKind::ConnectionAborted,
    ] {
        assert_eq!(
            after_request_read(kind),
            AfterRequestRead::Expected,
            "{kind:?} is a peer that withdrew a request it had started sending, which is the \
             one read failure here that is not the fixture's"
        );
    }
}

#[test]
fn a_request_the_fixture_could_not_read_is_reported_rather_than_dropped() {
    let server = TestServer::one(PATH, Reply::ok(b"{}"));

    let mut client =
        TcpStream::connect(("127.0.0.1", server.port())).expect("a connection to the server");
    // A request line that never ends: the connection is open, the fixture is
    // waiting on a read, and `REQUEST_BUDGET` is what ends the wait.
    client.write_all(b"GET ").expect("the start of a request");
    client.flush().expect("it reaches the server");

    let deadline = Instant::now() + REPORT_BUDGET;
    while server.faults().is_empty() {
        assert!(
            Instant::now() < deadline,
            "a request the fixture gave up on within {REQUEST_BUDGET:?} is recorded within \
             {REPORT_BUDGET:?}: a read that failed and a peer that never asked are not the same \
             event, and answering both with silence is how a connection the client certainly \
             opened becomes an attempt the fixture never counted"
        );
        std::thread::sleep(Duration::from_millis(25));
    }

    let faults = server.faults();
    assert!(
        faults
            .iter()
            .any(|fault| fault.to_lowercase().contains("request")),
        "and it says what it could not do, rather than leaving the next assertion to describe a \
         client that asked fewer times than it did: {faults:?}"
    );
    drop(client);
}

#[test]
fn a_peer_that_closed_before_asking_anything_is_not_a_failure() {
    let server = TestServer::one(PATH, Reply::ok(b"{}"));

    // Opened and closed with nothing on it: a connection nobody asked anything
    // on is nothing at all, and a fixture that called it a failure would fail
    // every test that ran after one.
    drop(TcpStream::connect(("127.0.0.1", server.port())).expect("a connection to the server"));

    // Then an ordinary request, which is the proof the fixture is still serving
    // and still counting.
    let mut client =
        TcpStream::connect(("127.0.0.1", server.port())).expect("a connection to the server");
    client
        .set_read_timeout(Some(READ_BUDGET))
        .expect("a bounded read");
    client
        .write_all(
            format!("GET {PATH} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
                .as_bytes(),
        )
        .expect("the request");
    client.flush().expect("it reaches the server");
    let mut reply = String::new();
    client.read_to_string(&mut reply).expect("the whole reply");

    assert!(reply.ends_with("{}"), "the next request is answered");
    assert_eq!(
        server.hits(PATH),
        1,
        "the closed connection recorded nothing and reported nothing, and `hits` asserts on the \
         fault list, so this counts both"
    );
}
