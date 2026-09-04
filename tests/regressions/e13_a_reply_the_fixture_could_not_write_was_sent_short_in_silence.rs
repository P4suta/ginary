// SPDX-License-Identifier: MIT OR Apache-2.0
//! The download suite's own HTTP server threw away every error its writes
//! reported, so a reply it could not finish sending went out short under a
//! `Content-Length` promising the whole of it — manufacturing, from the
//! fixture, the exact transport failure the retry tests script by hand.
//!
//! **What went wrong.** `write_reply` is `let _ = stream.write_all(head);
//! let _ = stream.write_all(body);`. `std::io::Write::write_all` retries only
//! `Interrupted`, so any other error returns after a partial write and the
//! fixture discarded it. E13 then gave every accepted connection
//! `set_write_timeout(Some(REPLY_BUDGET))`: a peer that stops reading now makes
//! the write fail after five seconds — `EAGAIN`, which is `WouldBlock`, on unix
//! and `WSAETIMEDOUT`, which is `TimedOut`, on Windows — and the fixture
//! answers a body shorter than the length it promised, in silence. That is
//! `Reply::Truncated` arriving by accident, and every retry test in
//! `tests/download.rs` reads it as a broken connection: a fixture failure
//! reported as a product failure, on the one platform this milestone cannot run
//! locally.
//!
//! The bound itself is right — a stalled reply must be a slow test rather than
//! a hung binary — so the fix is not to remove it but to stop the fixture lying
//! about what it sent.
//!
//! **The input.** Any reply the peer stops taking: a client that asked and then
//! never read, which reaches [`REPLY_BUDGET`] and fails the write.
//!
//! **The correct behaviour.** A write that failed because the peer let go —
//! `BrokenPipe`, `ConnectionReset`, `ConnectionAborted` — is the ordinary end
//! of a body a client abandoned, and is not the fixture's failure: `ureq` does
//! it whenever it stops reading a body short of its end, which is any answer
//! that runs past the bound it is being read within — E13's own
//! `e13_a_compressed_document_was_bounded_by_its_wire_bytes` leaves sixty-four
//! mebibytes of body on the socket that way. Every other write failure, the
//! budget's timeout included, is recorded where a test reads it, so an
//! assertion about the reply names the fixture that could not send it rather
//! than blaming the client.

use std::io::{ErrorKind, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use crate::common::http::{AfterWrite, REPLY_BUDGET, Reply, TestServer, after_write};

/// The path the last test asks for.
const PATH: &str = "/otp.tar.zst";

/// A body larger than a loopback connection can buffer, so that a client which
/// never reads leaves the server waiting in a write it cannot finish.
const BULK_BYTES: usize = 16 * 1024 * 1024;

/// How long that test waits for the fixture to give up on the reply.
///
/// [`REPLY_BUDGET`] plus room for a loaded machine to notice.
const REPORT_BUDGET: Duration = Duration::from_secs(30);

#[test]
fn a_write_that_ran_out_of_budget_is_reported() {
    for kind in [ErrorKind::TimedOut, ErrorKind::WouldBlock] {
        assert_eq!(
            after_write(kind),
            AfterWrite::Reported,
            "{kind:?} is how a socket write timeout is spelled, and a reply the fixture could \
             not finish is a body shorter than the Content-Length it promised: the transport \
             failure the retry rule exists for, arriving from the fixture"
        );
    }
}

#[test]
fn a_peer_that_let_go_of_a_body_is_not_the_fixtures_failure() {
    for kind in [
        ErrorKind::BrokenPipe,
        ErrorKind::ConnectionReset,
        ErrorKind::ConnectionAborted,
    ] {
        assert_eq!(
            after_write(kind),
            AfterWrite::Expected,
            "{kind:?} is a client that stopped wanting the body, which is what every client of \
             this fixture does when it refuses one"
        );
    }
}

#[test]
fn a_reply_the_peer_never_reads_is_reported_rather_than_sent_short() {
    let server = TestServer::one(PATH, Reply::ok(&vec![b'x'; BULK_BYTES]));

    let mut client =
        TcpStream::connect(("127.0.0.1", server.port())).expect("a connection to the server");
    client
        .write_all(
            format!("GET {PATH} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
                .as_bytes(),
        )
        .expect("the request");
    client.flush().expect("the request reaches the server");

    // Nothing is ever read, so the reply fills both buffers and the fixture
    // waits out `REPLY_BUDGET` in a write it cannot finish.
    let deadline = Instant::now() + REPORT_BUDGET;
    while server.faults().is_empty() {
        assert!(
            Instant::now() < deadline,
            "a reply the fixture gave up on within {REPLY_BUDGET:?} is recorded within \
             {REPORT_BUDGET:?}: a body sent short under a full Content-Length that nothing says \
             anything about is read by every retry test as a connection that broke"
        );
        std::thread::sleep(Duration::from_millis(25));
    }

    let faults = server.faults();
    assert!(
        faults.iter().any(|fault| fault.contains(PATH)),
        "and it names the reply it could not send: {faults:?}"
    );
    drop(client);
}
