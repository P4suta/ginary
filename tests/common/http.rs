// SPDX-License-Identifier: MIT OR Apache-2.0
//! A local HTTP server, written by hand, for the download tests.
//!
//! `src/download.rs` has to be held to things no fixture on disk can show: a
//! body that hashes to the wrong digest, a 500 that becomes a 200 on the
//! second ask, a 404 that must *not* be asked again, a connection that dies
//! mid-body. All four are properties of a server, so the tests need one.
//!
//! It is hand-rolled rather than a dependency, and it is the smallest server
//! that can state those four claims: HTTP/1.1, `GET` only, one connection at a
//! time, `Content-Length` always, no chunking, no ranges, no keep-alive. Each
//! path is scripted with a list of [`Reply`] values that are answered in order
//! and then repeated, and every request is recorded, so a test asserts on how
//! many times the client asked as readily as on what it got back.
//!
//! It binds `127.0.0.1:0` and reports the port it was given, so any number of
//! tests run in parallel without agreeing on anything.
//!
//! Every connection it serves is put into a mode it chose rather than the one
//! `accept` happened to hand over, and every wait that opens up is bounded; see
//! [`adopt`], [`REQUEST_BUDGET`], [`REPLY_BUDGET`] and [`DRAIN_BUDGET`].
//!
//! What it cannot serve through it says rather than discards. A fixture that
//! loses a connection under-counts the requests a test asserts on, and one that
//! sends a reply short manufactures the transport failure those tests script by
//! hand — either way the client is blamed for the instrument. [`after_accept`]
//! for the accept, [`after_request_read`] for the request and [`after_write`]
//! for the reply are where the three decisions live, each a function of the
//! error's kind alone so that each can be held to on a platform this milestone
//! can run; anything they call the fixture's own failure is recorded and read
//! back by [`TestServer::requests`], which refuses to answer while one stands.
//!
//! The reply's half of that — decide, then record — is [`answer_reply`], over
//! any writer rather than over the connection, so that it is held to over a
//! [`HaltingSink`] that fails where the test says rather than over a socket
//! whose buffering belongs to the host.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long [`TestServer::wait_for_requests`] waits before giving up.
pub const WAIT_BUDGET: Duration = Duration::from_secs(10);

/// Which halves of a connection a finished reply closes.
///
/// A reply is finished by closing the *sending* half and nothing else. Closing
/// both halves and dropping the socket is an abortive close: the receiving
/// half is gone, so anything the peer had in flight — or anything the local
/// stack had not finished handing over — becomes a reset rather than data, and
/// a reset discards whatever the peer had already buffered. Unix tolerates
/// that, because a reset there is delivered after the bytes that arrived
/// before it. Windows does not: the buffered reply is thrown away and the
/// client is told `An established connection was aborted by the software in
/// your host machine. (os error 10053)`.
///
/// See `tests/regressions/e11_the_fixture_server_tore_down_a_connection_it_had_just_answered.rs`.
pub const REPLY_SHUTDOWN: Shutdown = Shutdown::Write;

/// How long the server waits for the request on a connection it has accepted.
///
/// Bounded for the same reason [`DRAIN_BUDGET`] is: this fixture serves one
/// connection at a time, so a peer that connects and then says nothing would
/// otherwise stall every request after it. Long enough that no client of this
/// fixture reaches it.
pub const REQUEST_BUDGET: Duration = Duration::from_secs(5);

/// How long one write of a reply may take to be handed to the transport.
///
/// A reply larger than the connection can buffer is written across several
/// blocking writes, each of which waits for the peer to make room. The budget
/// bounds one such wait, so a client that stops reading is a slow test rather
/// than a hung binary. Running out of it leaves the body shorter than the
/// `Content-Length` already sent, which is why it is recorded rather than
/// discarded; see [`after_write`].
pub const REPLY_BUDGET: Duration = Duration::from_secs(5);

/// How long the server waits for the peer's own close before letting go.
///
/// A graceful close reads until the peer closes, and a client that never does
/// would hang the serving thread and, with it, every later request. The budget
/// bounds that: it is long enough that no client of this fixture reaches it,
/// and short enough that a stuck one is a slow test rather than a hung binary.
pub const DRAIN_BUDGET: Duration = Duration::from_secs(5);

/// What the accept loop does about an error `accept` reported.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AfterAccept {
    /// Take the next connection: this one was the peer's to throw away.
    Next,
    /// Nothing is waiting yet.
    Wait,
    /// Stop serving, and say so.
    Stop,
}

/// The accept loop's decision, as a function of the error's kind alone.
///
/// The loop owns its listener, so stopping closes the port and every later
/// request is refused — which `src/download.rs` reads as a retryable transport
/// failure, so the client asks its three times while the fixture counts fewer.
/// A peer that resets between the connect and the accept is an ordinary thing
/// for a client to do (`ureq` does it whenever it abandons a body), and
/// `accept` reports it as `ECONNABORTED` on unix and `WSAECONNRESET` or
/// `WSAECONNABORTED` on Windows; those, and an `Interrupted` call, are one
/// connection lost rather than the end of the fixture.
///
/// See `tests/regressions/e13_the_fixture_server_stopped_on_an_error_a_peer_can_cause.rs`.
pub fn after_accept(kind: std::io::ErrorKind) -> AfterAccept {
    match kind {
        std::io::ErrorKind::WouldBlock => AfterAccept::Wait,
        std::io::ErrorKind::ConnectionAborted
        | std::io::ErrorKind::ConnectionReset
        | std::io::ErrorKind::Interrupted => AfterAccept::Next,
        _ => AfterAccept::Stop,
    }
}

/// What the server does about a write of a reply that failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AfterWrite {
    /// The peer let go of a body it had stopped wanting.
    Expected,
    /// The fixture failed to send what it promised, and says so.
    Reported,
}

/// The reply writer's decision, as a function of the error's kind alone.
///
/// A reply is written under a `Content-Length` that promises the whole of it,
/// so a write that stops early sends a body shorter than the header claims —
/// which is [`Reply::Truncated`] arriving by accident, and which every retry
/// test in `tests/download.rs` reads as a connection that broke. The one write
/// failure that is not the fixture's is the peer letting go of a body it
/// stopped reading: `BrokenPipe`, `ConnectionReset` or `ConnectionAborted`,
/// which is what every client of this fixture does when it refuses a body.
/// Everything else — [`REPLY_BUDGET`] running out above all, spelled `EAGAIN`
/// on unix and `WSAETIMEDOUT` on Windows — is recorded where a test reads it.
///
/// See `tests/regressions/e13_a_reply_the_fixture_could_not_write_was_sent_short_in_silence.rs`.
pub fn after_write(kind: std::io::ErrorKind) -> AfterWrite {
    match kind {
        std::io::ErrorKind::BrokenPipe
        | std::io::ErrorKind::ConnectionReset
        | std::io::ErrorKind::ConnectionAborted => AfterWrite::Expected,
        _ => AfterWrite::Reported,
    }
}

/// What the server does about a request it could not read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AfterRequestRead {
    /// The peer let go of a request it had started sending.
    Expected,
    /// The fixture could not read what was sent to it, and says so.
    Reported,
}

/// The request reader's decision, as a function of the error's kind alone.
///
/// A request that is not read is a request that is neither recorded nor
/// answered: the client's attempt dies in the transport, the retry that follows
/// is the one the fixture sees, and every count a test asserts on is short by
/// one. That is the failure this milestone was opened for, so the only read
/// failure that may pass in silence is the one that is not the fixture's:
/// `BrokenPipe`, `ConnectionReset` or `ConnectionAborted`, which is a peer that
/// withdrew a request it had started. Everything else — [`REQUEST_BUDGET`]
/// running out above all, spelled `EAGAIN` on unix and `WSAETIMEDOUT` on
/// Windows — is recorded where a test reads it.
///
/// A peer that closes *before* sending anything is not an error at all and
/// never reaches here: nobody asked anything on that connection.
///
/// See `tests/regressions/e13_a_request_the_fixture_could_not_read_was_dropped_in_silence.rs`.
pub fn after_request_read(kind: std::io::ErrorKind) -> AfterRequestRead {
    match kind {
        std::io::ErrorKind::BrokenPipe
        | std::io::ErrorKind::ConnectionReset
        | std::io::ErrorKind::ConnectionAborted => AfterRequestRead::Expected,
        _ => AfterRequestRead::Reported,
    }
}

/// Where the serving thread leaves a failure of its own.
///
/// This fixture runs on a thread of its own, so an error it hits has nowhere to
/// be returned to: every one it discards turns into a request that was never
/// counted or a reply that went out short, and the test that reads either one
/// reports it as the client's doing. What it cannot serve through it records
/// here instead, and [`TestServer::requests`] refuses to answer while anything
/// stands in it.
///
/// Public, with [`fault_log`] and [`reported`] beside it, so that a test about
/// the fixture can hand a log to the serving code and read back what reached
/// it, rather than asserting on a value that code was free to throw away.
pub type Faults = Arc<Mutex<Vec<String>>>;

/// A fault log with nothing recorded in it.
pub fn fault_log() -> Faults {
    Arc::new(Mutex::new(Vec::new()))
}

/// Everything recorded in `faults`, in order.
///
/// A log left by a thread that panicked is read through the poison rather than
/// around it: what the fixture managed to record before it died is exactly what
/// the test needs to be told.
pub fn reported(faults: &Faults) -> Vec<String> {
    match faults.lock() {
        Ok(faults) => faults.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

/// Records one failure of the fixture itself.
fn record(faults: &Faults, fault: String) {
    if let Ok(mut faults) = faults.lock() {
        faults.push(fault);
    }
}

/// One scripted answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reply {
    /// A status and a body.
    Body {
        /// The status line's code.
        status: u16,
        /// The bytes, sent with a matching `Content-Length`.
        body: Vec<u8>,
    },
    /// A status, a body, and the `Content-Encoding` the body is in.
    ///
    /// The one header-carrying shape this fixture has, and it exists for one
    /// claim: a compressed answer is inflated by the client *after* the reader
    /// that counts it, so a bound on what arrives is no bound at all on what
    /// lands in memory. The bytes are sent exactly as given, under a
    /// `Content-Length` that counts them rather than what they inflate to,
    /// which is what a compressing server sends.
    Encoded {
        /// The status line's code.
        status: u16,
        /// The value of the `Content-Encoding` header, such as `gzip`.
        encoding: String,
        /// The bytes as they go on the wire, already encoded.
        body: Vec<u8>,
    },
    /// A `Content-Length` header that promises more than is sent, then a
    /// close: the transport failure a retry is supposed to survive.
    Truncated {
        /// The length the header promises.
        promised: usize,
        /// The bytes actually written before the close.
        body: Vec<u8>,
    },
    /// The connection is accepted and closed with nothing written at all.
    Hangup,
}

impl Reply {
    /// `200 OK` with `body`.
    pub fn ok(body: &[u8]) -> Self {
        Self::Body {
            status: 200,
            body: body.to_vec(),
        }
    }

    /// A status with an empty body.
    pub fn status(status: u16) -> Self {
        Self::Body {
            status,
            body: Vec::new(),
        }
    }
}

/// One request the server was asked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    /// The method, which is always `GET` for anything ginary sends.
    pub method: String,
    /// The path, with its query string if there was one.
    pub path: String,
    /// Every header, lower-cased name to value.
    pub headers: BTreeMap<String, String>,
}

/// A server listening on a loopback port for the life of the value.
pub struct TestServer {
    port: u16,
    requests: Arc<Mutex<Vec<Request>>>,
    faults: Faults,
    stop: Arc<AtomicBool>,
}

impl TestServer {
    /// Starts a server answering `routes`.
    ///
    /// Each key is a path; each value is the replies for it, in order, with the
    /// last one repeated once the list runs out. A path with no route is
    /// answered `404` with an empty body.
    ///
    /// # Panics
    ///
    /// If the loopback socket cannot be bound, which is a broken machine
    /// rather than a failing assertion.
    pub fn start(routes: BTreeMap<String, Vec<Reply>>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a loopback port");
        let port = listener.local_addr().expect("the bound address").port();
        listener
            .set_nonblocking(true)
            .expect("a non-blocking listener");

        let requests = Arc::new(Mutex::new(Vec::new()));
        let faults = fault_log();
        let stop = Arc::new(AtomicBool::new(false));
        let served = Arc::clone(&requests);
        let reported = Arc::clone(&faults);
        let stopping = Arc::clone(&stop);

        std::thread::spawn(move || {
            let mut answered: BTreeMap<String, usize> = BTreeMap::new();
            while !stopping.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        serve_one(stream, &routes, &mut answered, &served, &reported);
                    }
                    Err(error) => match after_accept(error.kind()) {
                        AfterAccept::Wait => std::thread::sleep(Duration::from_millis(5)),
                        AfterAccept::Next => {}
                        AfterAccept::Stop => {
                            record(
                                &reported,
                                format!("the fixture stopped accepting connections: {error}"),
                            );
                            break;
                        }
                    },
                }
            }
        });

        Self {
            port,
            requests,
            faults,
            stop,
        }
    }

    /// A server answering one path with one reply.
    pub fn one(path: &str, reply: Reply) -> Self {
        Self::start(BTreeMap::from([(path.to_owned(), vec![reply])]))
    }

    /// The loopback port it was given, for a test that speaks to it directly
    /// rather than through a client.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// The base URL, with no trailing slash.
    pub fn base(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// The absolute URL of `path`, which must begin with a slash.
    pub fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base())
    }

    /// Every request so far, in order.
    ///
    /// # Panics
    ///
    /// If the recording lock was poisoned by a panicking serving thread, or if
    /// the fixture has recorded a failure of its own: a count read off a server
    /// that lost a connection or sent a reply short is a count of the wrong
    /// thing, and a test that asserts on it blames the client for the fixture.
    pub fn requests(&self) -> Vec<Request> {
        let faults = self.faults();
        assert!(
            faults.is_empty(),
            "the fixture server could not serve what it was scripted, so nothing counted here \
             says what the client did: {faults:?}"
        );
        self.requests.lock().expect("the request log").clone()
    }

    /// Everything the fixture could not serve through, in order.
    ///
    /// Empty for every test that is about the client, which is why
    /// [`TestServer::requests`] asserts on it rather than making each caller
    /// remember to. A test *about* the fixture reads it here, because this is
    /// the one accessor that does not assert.
    pub fn faults(&self) -> Vec<String> {
        reported(&self.faults)
    }

    /// How many requests have been made for `path`.
    pub fn hits(&self, path: &str) -> usize {
        self.requests()
            .iter()
            .filter(|request| request.path == path)
            .count()
    }

    /// Waits until at least `count` requests have arrived.
    ///
    /// # Panics
    ///
    /// If [`WAIT_BUDGET`] passes first, which makes a stalled client a failed
    /// assertion rather than a hung test binary.
    pub fn wait_for_requests(&self, count: usize) {
        let deadline = Instant::now() + WAIT_BUDGET;
        while self.requests().len() < count {
            assert!(
                Instant::now() < deadline,
                "only {} of {count} requests arrived within {WAIT_BUDGET:?}",
                self.requests().len()
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

/// Reads one request and writes the reply scripted for its path.
fn serve_one(
    mut stream: TcpStream,
    routes: &BTreeMap<String, Vec<Reply>>,
    answered: &mut BTreeMap<String, usize>,
    log: &Arc<Mutex<Vec<Request>>>,
    faults: &Faults,
) {
    if let Err(error) = adopt(&stream) {
        record(
            faults,
            format!("a connection could not be put into the mode this fixture serves in: {error}"),
        );
        return;
    }
    let request = match read_request(&stream) {
        Ok(Some(request)) => request,
        // A peer that closed before asking anything: nothing was asked, so
        // nothing is counted and nothing is wrong.
        Ok(None) => return,
        Err(error) => {
            if after_request_read(error.kind()) == AfterRequestRead::Reported {
                record(
                    faults,
                    format!(
                        "a request on a connection this fixture accepted could not be read: {error}"
                    ),
                );
            }
            return;
        }
    };
    let path = request.path.clone();
    if let Ok(mut requests) = log.lock() {
        requests.push(request);
    }

    let index = answered.entry(path.clone()).or_insert(0);
    let reply = match routes.get(&path) {
        Some(replies) if !replies.is_empty() => {
            let chosen = replies[(*index).min(replies.len() - 1)].clone();
            *index += 1;
            chosen
        }
        _ => Reply::status(404),
    };

    // Through `answer_reply`, so that both halves of the decision — which write
    // failures are the fixture's own, and that one of those reaches the log a
    // test reads — are stated once, in code a regression can hold directly.
    answer_reply(&mut stream, &path, &reply, faults);
    finish_reply(&mut stream);
}

/// Answers one connection that the caller has already accepted.
///
/// The body of the server's own loop, exposed so a test can hand it a
/// connection it controls both ends of and watch what the connection does
/// afterwards. Returns the request that was read, or `None` when the peer
/// closed before sending one — a request the fixture could not *read* is a
/// recorded failure rather than an absence, and is panicked on below.
///
/// # Panics
///
/// If the fixture could not serve the connection it was handed, for the reason
/// [`TestServer::requests`] does: a connection lost or a reply sent short is
/// the fixture's failure, and a test told only that no request arrived reports
/// it as the peer's.
pub fn answer_one(stream: TcpStream, routes: &BTreeMap<String, Vec<Reply>>) -> Option<Request> {
    let log: Arc<Mutex<Vec<Request>>> = Arc::new(Mutex::new(Vec::new()));
    let faults = fault_log();
    let mut answered = BTreeMap::new();
    serve_one(stream, routes, &mut answered, &log, &faults);
    let faults = reported(&faults);
    assert!(
        faults.is_empty(),
        "the fixture server could not serve the connection it was handed: {faults:?}"
    );
    let requests = log.lock().ok()?;
    requests.last().cloned()
}

/// Puts an accepted connection into the mode this fixture serves in.
///
/// `accept` does not hand back a socket in a mode the caller chose. POSIX says
/// the accepted socket does *not* inherit the listening socket's `O_NONBLOCK`,
/// so on unix a connection accepted from the polling listener
/// [`TestServer::start`] uses is a blocking one. Winsock says the opposite: the
/// socket `accept` returns carries the properties of the socket it was accepted
/// from, non-blocking mode included, and `std`'s Windows implementation is a
/// bare `accept()` that does nothing to reset it while the unix one goes
/// through `accept4`.
///
/// A fixture serving a non-blocking connection loses work at all three of the
/// places that once handled an I/O error by ignoring it: [`read_request`]
/// answered `None`, so a request that had not arrived by the time `accept`
/// returned was neither recorded nor answered; [`write_reply`] truncates a body
/// larger than one write can take; and [`finish_reply`]'s drain is told
/// `WouldBlock` at once, takes that for the peer letting go, and closes
/// abortively — the very thing [`REPLY_SHUTDOWN`] exists to prevent. On Windows
/// that made the request count two tests assert on short by one, per connection
/// and per run.
///
/// So the mode is chosen here rather than inherited, and the two waits it opens
/// up are bounded by [`REQUEST_BUDGET`] and [`REPLY_BUDGET`].
///
/// See `tests/regressions/e13_the_fixture_server_inherited_its_listeners_non_blocking_mode.rs`.
///
/// # Errors
///
/// Whatever the socket said, reported rather than panicked: a panic here would
/// drop the listener with the serving thread and leave every later test on this
/// server saying that no request arrived. Silently serving the connection in
/// whatever mode it arrived in is the defect this exists to close.
pub fn adopt(stream: &TcpStream) -> std::io::Result<()> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(REQUEST_BUDGET))?;
    stream.set_write_timeout(Some(REPLY_BUDGET))
}

/// Lets go of a connection whose reply has been written.
///
/// See [`REPLY_SHUTDOWN`] for why the two halves are not the same question,
/// and [`DRAIN_BUDGET`] for the bound on waiting.
fn finish_reply(stream: &mut TcpStream) {
    // The end of the body, which is what a client reading to EOF is waiting
    // for. The receiving half stays open, so nothing the peer still has in
    // flight becomes a reset.
    let _ = stream.shutdown(REPLY_SHUTDOWN);

    // Then wait for the peer's own close before letting the socket go. A
    // close that follows the peer's cannot reset anything, because there is
    // nothing left in flight to reset. The deadline is the whole of what
    // bounds this: a client that never closes makes one slow request rather
    // than a hung binary, and every client of this fixture closes at once.
    let deadline = Instant::now() + DRAIN_BUDGET;
    let mut discard = [0_u8; 1024];
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        // `Duration::ZERO` means "no timeout at all" to the socket layer,
        // which would turn an expired budget into an unbounded wait.
        if left.is_zero() {
            break;
        }
        if stream.set_read_timeout(Some(left)).is_err() {
            break;
        }
        match stream.read(&mut discard) {
            // The peer closed: the connection is this side's to drop now.
            Ok(0) => break,
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            // A timeout or a reset: there is nothing further to wait for
            // either way, and neither is a failure of the test being served.
            Err(_) => break,
        }
    }
}

/// Reads the request line and the headers, and drains any body.
///
/// # Errors
///
/// Whatever the read said, and an `Other` for a request line that does not
/// parse: a request the fixture cannot read is a request it will neither record
/// nor answer, and the caller has to be able to tell that from the one shape
/// that is nothing at all — [`Ok(None)`], a peer that closed before sending a
/// byte. [`after_request_read`] is where the caller decides whether the failure
/// was the peer letting go or the fixture's own.
fn read_request(stream: &TcpStream) -> std::io::Result<Option<Request>> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    let mut parts = line.split_whitespace();
    let (Some(method), Some(path)) = (parts.next(), parts.next()) else {
        return Err(std::io::Error::other(format!(
            "a request line this fixture cannot serve: {line:?}"
        )));
    };
    let (method, path) = (method.to_owned(), path.to_owned());

    let mut headers = BTreeMap::new();
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 {
            break;
        }
        let header = header.trim_end_matches(['\r', '\n']);
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            headers.insert(name.trim().to_lowercase(), value.trim().to_owned());
        }
    }

    if let Some(length) = headers.get("content-length").and_then(|v| v.parse().ok()) {
        let mut body = vec![0_u8; length];
        reader.read_exact(&mut body)?;
    }

    Ok(Some(Request {
        method,
        path,
        headers,
    }))
}

/// Writes one reply, including the deliberately broken shapes.
///
/// It is generic over the writer rather than taking the connection, so that
/// [`send_reply`] can be held to its claim over a transport that states what it
/// did instead of one whose buffering the test would have to provoke.
///
/// # Errors
///
/// Whatever the write said. A reply goes out under a `Content-Length` that
/// promises the whole of it, so a write that stops early is a body shorter than
/// the header claims; [`send_reply`] asks [`after_write`] whether that was the
/// peer letting go or the fixture failing.
fn write_reply<W: Write>(sink: &mut W, reply: &Reply) -> std::io::Result<()> {
    match reply {
        Reply::Body { status, body } => {
            let head = format!(
                "HTTP/1.1 {status} {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                reason(*status),
                body.len()
            );
            sink.write_all(head.as_bytes())?;
            sink.write_all(body)?;
            sink.flush()
        }
        Reply::Encoded {
            status,
            encoding,
            body,
        } => {
            let head = format!(
                "HTTP/1.1 {status} {}\r\nContent-Encoding: {encoding}\r\nContent-Length: {}\r\n\
                 Connection: close\r\n\r\n",
                reason(*status),
                body.len()
            );
            sink.write_all(head.as_bytes())?;
            sink.write_all(body)?;
            sink.flush()
        }
        Reply::Truncated { promised, body } => {
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {promised}\r\nConnection: close\r\n\r\n"
            );
            sink.write_all(head.as_bytes())?;
            sink.write_all(body)?;
            sink.flush()
        }
        Reply::Hangup => Ok(()),
    }
}

/// Writes one reply to `sink` and answers with the fixture's own failure to
/// send it whole.
///
/// The reply half of `serve_one`, over any writer rather than over the
/// connection, so that the claim can be stated of a transport that says what it
/// did rather than provoked out of the host's socket buffers. `None` is a reply
/// that went out whole, and also a peer that let go of a body it had stopped
/// wanting: [`after_write`] decides which write failures are the fixture's, and
/// only those become a fault for the caller to record.
///
/// `path` is the reply's own, and it names the reply in the fault, because a
/// fault that says only that some write failed leaves the next assertion to
/// guess which of a server's answers went out short.
///
/// [`answer_reply`] is what the server calls, and what a test about the server
/// asserts over: the fault this hands back is only half the rule, and a caller
/// that dropped it would be the same silence one level up.
///
/// See `tests/regressions/e13_a_reply_the_fixture_could_not_write_was_sent_short_in_silence.rs`.
pub fn send_reply<W: Write>(sink: &mut W, path: &str, reply: &Reply) -> Option<String> {
    match write_reply(sink, reply) {
        Ok(()) => None,
        Err(error) => match after_write(error.kind()) {
            AfterWrite::Expected => None,
            AfterWrite::Reported => Some(format!(
                "the reply to {path} was not written whole: {error}"
            )),
        },
    }
}

/// Writes one reply to `sink` and records a failure of the fixture's own.
///
/// The whole of `serve_one`'s reply: [`send_reply`] decides whether the write
/// that failed was the fixture's doing, and this records the one that was.
/// The two are one function to a caller, because a caller that asked the first
/// question and dropped the answer is the defect this milestone exists to
/// close, one level up from where it was found — a reply sent short under a
/// full `Content-Length`, in silence, which every retry test in
/// `tests/download.rs` reads as a connection that broke.
///
/// See `tests/regressions/e13_a_reply_the_fixture_could_not_write_was_sent_short_in_silence.rs`.
pub fn answer_reply<W: Write>(sink: &mut W, path: &str, reply: &Reply, faults: &Faults) {
    if let Some(fault) = send_reply(sink, path, reply) {
        record(faults, fault);
    }
}

/// The reason phrase for the handful of statuses these tests use.
fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        404 => "Not Found",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Status",
    }
}

/// A transport that takes a stated number of bytes and then fails.
///
/// The peer of a reply, for a test that has to state what the transport did
/// rather than provoke it out of the host. A socket does not offer that: how
/// much a loopback connection with nothing reading it will take before a write
/// blocks is a property of the operating system's buffering, and whether a
/// blocked write ends in the time a test is prepared to wait is a property of
/// its send timeout. Neither is a property of this fixture, and a claim about
/// the fixture written in terms of either is a claim that holds on the host it
/// was written on.
///
/// The one error it will not answer is `Interrupted`, which
/// [`HaltingSink::halting_after`] refuses: `std::io::Write::write_all` retries
/// that kind for ever, so a sink that keeps answering it never returns, and a
/// fixture that could not send a reply has to fail an assertion rather than
/// hang the test binary — the rule [`WAIT_BUDGET`], [`REPLY_BUDGET`] and
/// [`DRAIN_BUDGET`] state everywhere else.
///
/// See `tests/regressions/e13_a_reply_the_fixture_could_not_write_was_sent_short_in_silence.rs`.
pub struct HaltingSink {
    /// Everything written to it so far, in order.
    taken: Vec<u8>,
    /// How many bytes it takes in total before it starts failing.
    accepts: usize,
    /// The error every write past `accepts` reports.
    kind: std::io::ErrorKind,
}

impl HaltingSink {
    /// A sink that takes `accepts` bytes and then fails every write with
    /// `kind`.
    ///
    /// A write that spans the boundary takes what is left and reports the
    /// number, the way a socket does; the failure is the write after it.
    ///
    /// # Panics
    ///
    /// If `kind` is `Interrupted`, which is the one kind this sink cannot
    /// answer; see [`HaltingSink`]. It is refused here rather than at the write
    /// because a test that asked for it would otherwise hang instead of
    /// failing, and `after_write` calls `Interrupted` the fixture's own failure,
    /// so it is exactly the kind the next test written against this sink would
    /// reach for.
    pub fn halting_after(accepts: usize, kind: std::io::ErrorKind) -> Self {
        assert_ne!(
            kind,
            std::io::ErrorKind::Interrupted,
            "`write_all` retries `Interrupted` for ever, so a sink that answers it never returns; \
             a fixture that could not send a reply must fail an assertion rather than hang the \
             test binary"
        );
        Self {
            taken: Vec::new(),
            accepts,
            kind,
        }
    }

    /// A sink that takes everything it is given.
    pub fn taking_everything() -> Self {
        Self::halting_after(usize::MAX, std::io::ErrorKind::Other)
    }

    /// Everything it took, in order: the bytes that reached the peer.
    pub fn taken(&self) -> &[u8] {
        &self.taken
    }
}

impl Write for HaltingSink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let room = self.accepts.saturating_sub(self.taken.len());
        if room == 0 {
            return Err(std::io::Error::from(self.kind));
        }
        let take = buf.len().min(room);
        self.taken.extend_from_slice(&buf[..take]);
        Ok(take)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
