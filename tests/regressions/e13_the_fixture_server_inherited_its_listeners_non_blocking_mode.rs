// SPDX-License-Identifier: MIT OR Apache-2.0
//! The download suite's own HTTP server accepted its connections from a
//! listener it had put into non-blocking mode, and on Windows an accepted
//! socket inherits that mode — so every read and every write the fixture made
//! could answer `WouldBlock`, and the fixture discarded every one of those
//! answers.
//!
//! **What went wrong.** `tests/common/http.rs` calls
//! `listener.set_nonblocking(true)` so that its accept loop can poll for a stop
//! flag, and then serves whatever `accept` hands back. POSIX says the accepted
//! socket does *not* inherit the listening socket's `O_NONBLOCK`, so on unix
//! the served connection is a blocking one and every read waits for the bytes
//! it was promised. Winsock says the opposite: the socket `accept` returns has
//! the properties of the socket it was accepted from, non-blocking mode
//! included, and `std`'s Windows `accept` is a bare `accept()` that does
//! nothing to reset it. The fixture therefore served a *non-blocking* socket
//! there, and it treats an I/O error as "nothing to do":
//!
//! - `read_request` answers `None` on any read error, so a request that had not
//!   arrived by the time `accept` returned was neither recorded nor answered.
//!   The client's attempt died in the transport, the retry that followed was
//!   the one the server saw, and the fixture's own count of the requests was
//!   short by one — which is a count two tests assert on.
//! - `write_reply` writes through `let _ = stream.write_all(..)`, so a body
//!   larger than one non-blocking write could take was truncated silently.
//! - `finish_reply`'s drain calls `set_read_timeout` and then `read`, and a
//!   timeout means nothing to a non-blocking socket: the read answers
//!   `WouldBlock` at once, the loop's `Err(_) => break` takes it for the peer
//!   letting go, and the graceful close E11 added never happened on the one
//!   platform it was written for.
//!
//! ```text
//! ---- every_attempt_failing_reports_the_url_and_how_many_were_made ----
//! thread '...' (5528) panicked at tests\download.rs:262:5:
//! assertion `left == right` failed
//!   left: 2
//!  right: 3
//!
//! test result: FAILED. 21 passed; 2 failed; 0 ignored; 0 measured
//! ```
//!
//! (`Windows build and exit-code propagation`
//! <https://github.com/P4suta/ginary/actions/runs/33837795146/job/100913758843>,
//! `tests/download.rs:262` and `tests/download.rs:436`.)
//!
//! Both failing tests count the requests the fixture saw, both wanted three and
//! both got two, and both are among the handful that open three connections in
//! a row — one per attempt of the retry rule. A blocking fixture cannot lose a
//! request that a client certainly sent: `read_request` returns `None` only on
//! a read that failed or a peer that closed, and neither is something `ureq`
//! does between opening a connection and writing its request line. A
//! non-blocking one loses it whenever `accept` wins the race against the
//! request, which is a race the server wins or loses per connection and per
//! run — which is why the same target passed twenty-three of twenty-three at
//! the commit before.
//!
//! **The input.** Any connection served on a socket whose mode the fixture did
//! not choose: a request that arrives after the accept, a reply larger than one
//! write can take, or a client that has not closed yet.
//!
//! **The correct behaviour.** The fixture decides the mode of the connections
//! it serves rather than inheriting one, and every claim below holds on a
//! socket handed to it in the mode Windows hands one over. A late request is
//! waited for and answered, a reply of any size is written whole, and the drain
//! still lets go only after the peer has — bounded, as it already is, so a
//! client that never sends or never closes is a slow test rather than a hung
//! binary.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

use crate::common::http::{DRAIN_BUDGET, Reply, Request, answer_one};

/// The path every route here answers.
const PATH: &str = "/otp.tar.zst";

/// Exactly what [`Reply::status(503)`] puts on the wire, byte for byte.
const EXPECTED: &str =
    "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";

/// How long the client waits before sending a request the server is already
/// waiting for.
///
/// Long enough that a server which read once and gave up has certainly done so,
/// and far short of every budget the fixture bounds itself by.
const LATE: Duration = Duration::from_millis(150);

/// How long the serving thread waits before accepting, so that the request is
/// certainly buffered before the first read.
const SETTLE: Duration = Duration::from_millis(150);

/// How long a client of these tests waits for bytes before giving up.
const READ_BUDGET: Duration = Duration::from_secs(5);

/// Long enough that a server which let go immediately has certainly done so.
const HOLD: Duration = Duration::from_millis(250);

/// A body larger than everything a loopback connection can buffer, so that a
/// non-blocking write cannot take it in one go on any host this runs on.
///
/// Linux buffers at most `tcp_wmem` plus `tcp_rmem` between the two ends, which
/// is ten mebibytes on the machine this was written on and four times smaller
/// than this; Windows fills its default send buffer sixty-four kilobytes in.
/// A host that buffered the whole of it would let the write finish without ever
/// blocking, and the test that follows would pass without exercising anything,
/// so the host is asked first: see [`a_loopback_connection_blocks_before`].
const BULK_BYTES: usize = 16 * 1024 * 1024;

/// Whether a loopback connection stops taking bytes before `bytes` of them.
///
/// The precondition of `a_reply_too_large_for_one_write_is_written_whole`, put
/// to the host rather than assumed of it. A reply the socket buffers whole is
/// written in one call that never blocks, and a test of what the fixture does
/// with a *blocked* write then states nothing at all — the fault list cannot
/// save it, because the fault it would read is one that never happens. So a
/// connection of this test's own is filled, with nothing reading the other end,
/// and the answer is whether it ran out of room within `bytes`.
fn a_loopback_connection_blocks_before(bytes: usize) -> bool {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port");
    let address = listener.local_addr().expect("the bound address");
    let mut writer = TcpStream::connect(address).expect("a connection to it");
    let _reader = listener.accept().expect("the other end").0;
    writer
        .set_nonblocking(true)
        .expect("a write that reports rather than waits");

    let chunk = vec![b'x'; 64 * 1024];
    let mut sent = 0_usize;
    while sent < bytes {
        match writer.write(&chunk) {
            Ok(0) => return true,
            Ok(written) => sent += written,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            // `WouldBlock`, and anything else the host says: either way this
            // connection did not take `bytes` without stopping.
            Err(_) => return true,
        }
    }
    false
}

/// The one route these tests script.
fn routes(reply: Reply) -> BTreeMap<String, Vec<Reply>> {
    BTreeMap::from([(PATH.to_owned(), vec![reply])])
}

/// The request line and headers `ureq` would send, as bytes.
fn request_bytes() -> Vec<u8> {
    format!("GET {PATH} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n").into_bytes()
}

/// A listener in the mode `TestServer::start` puts its own into.
fn polling_listener() -> (TcpListener, u16) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port");
    let port = listener.local_addr().expect("the bound address").port();
    listener
        .set_nonblocking(true)
        .expect("a non-blocking listener");
    (listener, port)
}

/// Accepts one connection the way the fixture's own loop does, and hands it
/// over in the mode Winsock's `accept` would have.
fn accept_as_windows_does(listener: &TcpListener) -> TcpStream {
    let deadline = Instant::now() + READ_BUDGET;
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                stream
                    .set_nonblocking(true)
                    .expect("the mode an accepted socket inherits on Windows");
                return stream;
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(Instant::now() < deadline, "no connection arrived");
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("the client's connection: {error}"),
        }
    }
}

#[test]
fn a_request_that_arrives_after_the_accept_is_still_read_and_answered() {
    let (listener, port) = polling_listener();
    let served = std::thread::spawn(move || {
        let stream = accept_as_windows_does(&listener);
        answer_one(stream, &routes(Reply::status(503)))
    });

    let mut client = TcpStream::connect(("127.0.0.1", port)).expect("a connection to the server");
    client
        .set_read_timeout(Some(READ_BUDGET))
        .expect("a bounded read");
    // The connection is open and the request is not: exactly the order a
    // client and a polling accept loop reach on a loaded machine.
    std::thread::sleep(LATE);
    client.write_all(&request_bytes()).expect("the request");
    client.flush().expect("the request reaches the server");

    let mut reply = Vec::new();
    client
        .read_to_end(&mut reply)
        .expect("the whole reply, within this test's own budget");
    assert_eq!(
        String::from_utf8_lossy(&reply),
        EXPECTED,
        "a request the server was still waiting for is answered, not dropped: a connection the \
         fixture lets go of unanswered is one attempt of the retry rule that the fixture never \
         counted"
    );

    drop(client);
    let request: Option<Request> = served.join().expect("the serving thread");
    assert_eq!(
        request.map(|request| request.path),
        Some(PATH.to_owned()),
        "and it is recorded, because a request the fixture does not record is a request the \
         test says the client never made"
    );
}

#[test]
fn a_reply_too_large_for_one_write_is_written_whole() {
    assert!(
        a_loopback_connection_blocks_before(BULK_BYTES),
        "this host buffers more than {BULK_BYTES} bytes between the two ends of a loopback \
         connection, so the reply below is written in calls that never block and this test \
         cannot state the claim it exists for: say so rather than passing without it"
    );
    let body = vec![b'x'; BULK_BYTES];
    let (listener, port) = polling_listener();
    let served = std::thread::spawn(move || {
        let stream = accept_as_windows_does(&listener);
        answer_one(stream, &routes(Reply::ok(&body)))
    });

    let mut client = TcpStream::connect(("127.0.0.1", port)).expect("a connection to the server");
    client
        .set_read_timeout(Some(READ_BUDGET))
        .expect("a bounded read");
    client.write_all(&request_bytes()).expect("the request");
    client.flush().expect("the request reaches the server");
    // Nothing is read for a moment, so the whole reply has to be buffered or
    // waited for — and on this host it has to be waited for, which is the
    // condition the claim needs and the reason it is asserted rather than
    // assumed. A fixture that gives up on the first write it cannot finish
    // sends a body shorter than the length it promised, and `answer_one`
    // refuses to answer once the fixture has recorded a failure of its own.
    std::thread::sleep(HOLD);

    let mut reply = Vec::new();
    client
        .read_to_end(&mut reply)
        .expect("the whole reply, within this test's own budget");
    let head =
        format!("HTTP/1.1 200 OK\r\nContent-Length: {BULK_BYTES}\r\nConnection: close\r\n\r\n");
    assert_eq!(
        reply.len(),
        head.len() + BULK_BYTES,
        "the body is as long as the Content-Length promised: a truncated one is the transport \
         failure the retry rule exists for, arriving from the fixture rather than from a server"
    );
    assert!(
        reply.starts_with(head.as_bytes()) && reply[head.len()..].iter().all(|byte| *byte == b'x'),
        "and it is the body that was scripted, byte for byte"
    );

    drop(client);
    let request: Option<Request> = served.join().expect("the serving thread");
    assert_eq!(request.map(|request| request.path), Some(PATH.to_owned()));
}

#[test]
fn the_drain_still_waits_for_the_peers_close_on_a_socket_it_did_not_choose_the_mode_of() {
    let (listener, port) = polling_listener();
    let served = std::thread::spawn(move || {
        // The request is on the wire before the accept, so the read cannot be
        // what fails here: this test is about what happens after the reply.
        std::thread::sleep(SETTLE);
        let stream = accept_as_windows_does(&listener);
        answer_one(stream, &routes(Reply::status(503)))
    });

    let mut client = TcpStream::connect(("127.0.0.1", port)).expect("a connection to the server");
    client
        .set_read_timeout(Some(READ_BUDGET))
        .expect("a bounded read");
    client.write_all(&request_bytes()).expect("the request");
    client.flush().expect("the request reaches the server");

    let mut head = vec![0_u8; EXPECTED.len()];
    client
        .read_exact(&mut head)
        .expect("the whole reply, and not a reset in place of it");
    assert_eq!(String::from_utf8_lossy(&head), EXPECTED);

    std::thread::sleep(HOLD);
    assert!(
        !served.is_finished(),
        "the connection is still the client's until the client closes it, whatever mode the \
         socket arrived in: a drain that reads once, is told `WouldBlock` and takes that for the \
         peer letting go is not a drain, and the abortive close it leads to is what answered a \
         Windows client `os error 10053`"
    );

    drop(client);
    let deadline = Instant::now() + DRAIN_BUDGET;
    while !served.is_finished() {
        assert!(
            Instant::now() < deadline,
            "and it does let go once the client has, within {DRAIN_BUDGET:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }

    let request: Option<Request> = served.join().expect("the serving thread");
    assert_eq!(request.map(|request| request.path), Some(PATH.to_owned()));
}
