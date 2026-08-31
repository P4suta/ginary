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

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long [`TestServer::wait_for_requests`] waits before giving up.
pub const WAIT_BUDGET: Duration = Duration::from_secs(10);

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
        let stop = Arc::new(AtomicBool::new(false));
        let served = Arc::clone(&requests);
        let stopping = Arc::clone(&stop);

        std::thread::spawn(move || {
            let mut answered: BTreeMap<String, usize> = BTreeMap::new();
            while !stopping.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        serve_one(stream, &routes, &mut answered, &served);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });

        Self {
            port,
            requests,
            stop,
        }
    }

    /// A server answering one path with one reply.
    pub fn one(path: &str, reply: Reply) -> Self {
        Self::start(BTreeMap::from([(path.to_owned(), vec![reply])]))
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
    /// If the recording lock was poisoned by a panicking serving thread.
    pub fn requests(&self) -> Vec<Request> {
        self.requests.lock().expect("the request log").clone()
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
) {
    let Some(request) = read_request(&stream) else {
        return;
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

    write_reply(&mut stream, &reply);
    let _ = stream.shutdown(Shutdown::Both);
}

/// Reads the request line and the headers, and drains any body.
fn read_request(stream: &TcpStream) -> Option<Request> {
    let mut reader = BufReader::new(stream.try_clone().ok()?);
    let mut line = String::new();
    if reader.read_line(&mut line).ok()? == 0 {
        return None;
    }
    let mut parts = line.split_whitespace();
    let method = parts.next()?.to_owned();
    let path = parts.next()?.to_owned();

    let mut headers = BTreeMap::new();
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).ok()? == 0 {
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
        reader.read_exact(&mut body).ok()?;
    }

    Some(Request {
        method,
        path,
        headers,
    })
}

/// Writes one reply, including the deliberately broken shapes.
fn write_reply(stream: &mut TcpStream, reply: &Reply) {
    match reply {
        Reply::Body { status, body } => {
            let head = format!(
                "HTTP/1.1 {status} {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                reason(*status),
                body.len()
            );
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(body);
            let _ = stream.flush();
        }
        Reply::Truncated { promised, body } => {
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {promised}\r\nConnection: close\r\n\r\n"
            );
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(body);
            let _ = stream.flush();
        }
        Reply::Hangup => {}
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
