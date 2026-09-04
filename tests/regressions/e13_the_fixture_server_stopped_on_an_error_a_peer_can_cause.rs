// SPDX-License-Identifier: MIT OR Apache-2.0
//! The download suite's own HTTP server ended its accept loop for good on any
//! error `accept` reported that was not `WouldBlock`, including the two a
//! *client* can cause — so one reset connection closed the port and every later
//! attempt of the retry rule was counted by nobody.
//!
//! **What went wrong.** `tests/common/http.rs`'s loop was
//! `Err(_) => break`, and `listener` is owned by the serving closure: breaking
//! returns from the thread, drops the listener and closes the port. Every
//! request after that is refused by the operating system,
//! `src/download.rs` classifies a refused connection as retryable, and the
//! client reports `Exhausted { attempts: 3 }` while `TestServer::hits` is short
//! by one or more — bit for bit the shape of the failure E13 was opened for:
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
//! Reaching that arm is a documented, peer-driven condition rather than a
//! broken machine: `accept` reports `ECONNABORTED` on unix, and
//! `WSAECONNRESET`/`WSAECONNABORTED` on Windows, for a client that resets
//! between the connect and the accept — which is what `ureq` does whenever it
//! abandons a body, as it does on any answer that runs past the bound it is
//! being read within: E13's own
//! `e13_a_compressed_document_was_bounded_by_its_wire_bytes` stops four
//! mebibytes into a sixty-four mebibyte document, and
//! `a_client_that_abandons_a_reply_leaves_the_server_serving` below does the
//! same by hand. The
//! fixture is the instrument the whole download suite reads, and an instrument
//! that stops at the first thing its subject legitimately does under-reports
//! silently rather than failing.
//!
//! **The input.** Any error from `accept` that the peer caused:
//! `ConnectionAborted`, `ConnectionReset`, or an `Interrupted` call.
//!
//! **The correct behaviour.** Those three mean "take the next connection", so
//! the loop keeps serving and the count stays true. `WouldBlock` still means
//! "nothing is waiting yet". Anything else does stop the loop — a listener this
//! fixture cannot serve through is real — but it is recorded where a test
//! reads it, so the next assertion names the fixture rather than reporting a
//! client that asked fewer times than it did.

use std::collections::BTreeMap;
use std::io::{ErrorKind, Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use crate::common::http::{AfterAccept, Reply, TestServer, after_accept};

/// The path these tests ask for.
const PATH: &str = "/otp.tar.zst";

/// A body larger than a loopback connection can buffer, so that a client which
/// stops reading and lets go is certainly holding a reply the server is still
/// writing.
const BULK_BYTES: usize = 16 * 1024 * 1024;

/// How long a client of these tests waits for bytes before giving up.
const READ_BUDGET: Duration = Duration::from_secs(5);

/// The request line and headers `ureq` would send, as bytes.
fn request_bytes() -> Vec<u8> {
    format!("GET {PATH} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n").into_bytes()
}

#[test]
fn an_accept_error_the_peer_caused_takes_the_next_connection() {
    for kind in [
        ErrorKind::ConnectionAborted,
        ErrorKind::ConnectionReset,
        ErrorKind::Interrupted,
    ] {
        assert_eq!(
            after_accept(kind),
            AfterAccept::Next,
            "{kind:?} is one connection the peer threw away, not the end of the fixture: a \
             server that stops here under-counts the requests every download test asserts on"
        );
    }
}

#[test]
fn nothing_waiting_yet_is_still_a_wait() {
    assert_eq!(
        after_accept(ErrorKind::WouldBlock),
        AfterAccept::Wait,
        "the listener polls so that the loop can see the stop flag; an empty queue is the \
         ordinary case"
    );
}

#[test]
fn an_error_the_fixture_cannot_serve_through_stops_it() {
    assert_eq!(
        after_accept(ErrorKind::InvalidInput),
        AfterAccept::Stop,
        "a listener that cannot be accepted from is not something to spin on"
    );
}

#[test]
fn a_client_that_abandons_a_reply_leaves_the_server_serving() {
    let document = "the second client is answered";
    let server = TestServer::start(BTreeMap::from([
        (PATH.to_owned(), vec![Reply::ok(&vec![b'x'; BULK_BYTES])]),
        ("/second".to_owned(), vec![Reply::ok(document.as_bytes())]),
    ]));

    {
        let mut abandoning =
            TcpStream::connect(("127.0.0.1", server.port())).expect("a connection to the server");
        abandoning
            .set_read_timeout(Some(READ_BUDGET))
            .expect("a bounded read");
        abandoning.write_all(&request_bytes()).expect("the request");
        abandoning.flush().expect("the request reaches the server");
        // One buffer's worth and no more, then let go with the rest of the
        // reply still in flight: the abrupt close every client makes when it
        // stops wanting a body it has already started reading.
        let mut first = [0_u8; 1024];
        abandoning
            .read_exact(&mut first)
            .expect("the beginning of the reply");
    }
    server.wait_for_requests(1);

    // Whatever the abandoned connection did to the fixture, the next client is
    // still served, and still counted.
    let mut second =
        TcpStream::connect(("127.0.0.1", server.port())).expect("a connection to the server");
    second
        .set_read_timeout(Some(READ_BUDGET))
        .expect("a bounded read");
    second
        .write_all(b"GET /second HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .expect("the second request");
    second.flush().expect("the request reaches the server");
    let mut reply = String::new();
    second
        .read_to_string(&mut reply)
        .expect("the whole second reply");

    assert!(
        reply.ends_with(document),
        "a client that let go of a body it had stopped reading is one connection, not the end \
         of the fixture; the second client got: {reply:?}"
    );
    let deadline = Instant::now() + READ_BUDGET;
    while server.hits("/second") < 1 {
        assert!(
            Instant::now() < deadline,
            "and the second request is recorded, because a request the fixture does not record \
             is a request the test says the client never made"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}
