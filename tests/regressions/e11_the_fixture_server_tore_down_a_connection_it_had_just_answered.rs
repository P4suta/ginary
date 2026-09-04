// SPDX-License-Identifier: MIT OR Apache-2.0
//! The download suite's own HTTP server closed both halves of a connection the
//! instant it had written a reply, and on Windows the reset that follows an
//! abortive close threw the reply away.
//!
//! **What went wrong.** `tests/common/http.rs` finished every reply with
//! `stream.shutdown(Shutdown::Both)` and then dropped the socket. That is an
//! abortive close: the receiving half is gone, so the stack answers anything
//! still in flight with a reset rather than with an acknowledgement, and a
//! reset tells the peer to discard what it has already buffered. Unix tolerates
//! it — a reset there is delivered *after* the bytes that arrived before it, so
//! the client reads the reply and only then sees the connection end. Windows
//! does not: the buffered reply is dropped on the floor and the client is told
//! the connection was aborted.
//!
//! ```text
//! ---- every_attempt_failing_reports_the_url_and_how_many_were_made ----
//! thread 'every_attempt_failing_reports_the_url_and_how_many_were_made' (752)
//! panicked at tests\download.rs:255:13:
//! the last failure is quoted, not summarised: io: An established connection
//! was aborted by the software in your host machine. (os error 10053)
//!
//! test result: FAILED. 22 passed; 1 failed; 0 ignored; 0 measured
//! ```
//!
//! (`Windows build and exit-code propagation`
//! <https://github.com/P4suta/ginary/actions/runs/33751715516/job/100636537290>,
//! `tests/download.rs:255`.)
//!
//! Twenty-two of the twenty-three tests in that target passed, which is the
//! shape of the evidence: this is a race the server loses sometimes, not a
//! rule it gets wrong every time. `every_attempt_failing_…` is the test that
//! opens the most connections — three, one per attempt — and the only one that
//! asserts on the *last* of them, so it is the one with the most chances to
//! lose and the least tolerance for losing. `10053` is `WSAECONNABORTED`, the
//! local stack reporting that it aborted a connection it had accepted, which is
//! exactly what a close of a socket whose receiving half was already shut down
//! does there.
//!
//! **The input.** Any client on a host whose stack discards buffered data on a
//! reset, asking a server that answers and then closes both halves at once.
//!
//! **The correct behaviour.** The graceful close every HTTP/1.1 server that
//! sends `Connection: close` performs, and in this order: write the reply, shut
//! down the *sending* half only so the peer sees the end of the body, then read
//! until the peer closes, and only then let the socket go. A close that follows
//! the peer's own close cannot reset anything, because there is nothing left in
//! flight to reset. The wait is bounded by
//! [`crate::common::http::DRAIN_BUDGET`] so that a client which never closes is
//! a slow test rather than a hung binary.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::time::{Duration, Instant};

use crate::common::http::{DRAIN_BUDGET, REPLY_SHUTDOWN, Reply, Request, answer_one};

/// The path the scripted server answers.
const PATH: &str = "/otp.tar.zst";

/// Exactly what [`Reply::status(503)`] puts on the wire, byte for byte.
const EXPECTED: &str =
    "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";

/// Long enough that a server which lets go immediately has certainly done so,
/// and far short of [`DRAIN_BUDGET`], so the assertion is about the rule rather
/// than about a scheduler.
const HOLD: Duration = Duration::from_millis(250);

/// The one route this test scripts.
fn routes() -> BTreeMap<String, Vec<Reply>> {
    BTreeMap::from([(PATH.to_owned(), vec![Reply::status(503)])])
}

#[test]
fn a_finished_reply_closes_the_sending_half_and_not_both() {
    assert_eq!(
        REPLY_SHUTDOWN,
        Shutdown::Write,
        "closing the receiving half turns everything still in flight into a reset, and a \
         reset discards the reply the peer had already buffered"
    );
    assert_ne!(
        REPLY_SHUTDOWN,
        Shutdown::Both,
        "`Shutdown::Both` is the abortive close that answered a Windows client with \
         `os error 10053` instead of with the 503 it had already been sent"
    );
}

#[test]
fn the_server_lets_go_of_a_connection_only_after_the_client_has() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port");
    let port = listener.local_addr().expect("the bound address").port();

    let served = std::thread::spawn(move || {
        let (stream, _) = listener.accept().expect("the client's connection");
        answer_one(stream, &routes())
    });

    let mut client = TcpStream::connect(("127.0.0.1", port)).expect("a connection to the server");
    client
        .write_all(format!("GET {PATH} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").as_bytes())
        .expect("the request");
    client.flush().expect("the request reaches the server");
    client
        .set_read_timeout(Some(DRAIN_BUDGET))
        .expect("a bounded read");

    let mut head = vec![0_u8; EXPECTED.len()];
    client
        .read_exact(&mut head)
        .expect("the whole reply, and not a reset in place of it");
    assert_eq!(
        String::from_utf8_lossy(&head),
        EXPECTED,
        "the reply arrives whole before anything is closed"
    );

    std::thread::sleep(HOLD);
    assert!(
        !served.is_finished(),
        "the connection is still the client's until the client closes it: a server that has \
         already let go is one whose close can reset a reply the client has not read yet"
    );

    drop(client);

    let deadline = Instant::now() + DRAIN_BUDGET;
    while !served.is_finished() {
        assert!(
            Instant::now() < deadline,
            "and it does let go once the client has, within {DRAIN_BUDGET:?}, so one client \
             that never closes cannot stall every request after it"
        );
        std::thread::sleep(Duration::from_millis(5));
    }

    let request: Option<Request> = served.join().expect("the serving thread");
    assert_eq!(
        request.map(|request| request.path),
        Some(PATH.to_owned()),
        "and the request is recorded exactly once, as it was before"
    );
}
