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
//! **The input.** Any reply whose write fails: a transport that stops taking
//! bytes part-way through a body the head has already promised in full.
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
//!
//! ## E14 — why the last claim is made over a sink and not over a socket
//!
//! The claim above was first written as a live one: open a connection, ask for
//! a sixteen-mebibyte body, never read a byte of it, and wait thirty seconds
//! for the fixture to record the write it gave up on within `REPLY_BUDGET`. It
//! passed on Linux and failed on Windows, as the one red test in an otherwise
//! green pull request:
//!
//! ```text
//! thread '...' (3908) panicked at tests\regressions\e13_a_reply_the_fixture_could_not_write_was_sent_short_in_silence.rs:103:9:
//! a reply the fixture gave up on within 5s is recorded within 30s: a body sent short under a full
//! Content-Length that nothing says anything about is read by every retry test as a connection that
//! broke
//!
//! test result: FAILED. 358 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 34.18s
//! ```
//!
//! (`Windows build and exit-code propagation`
//! <https://github.com/P4suta/ginary/actions/runs/33854462719/job/100964443199>.)
//!
//! Nothing there is a statement about this fixture. The reply reached no write
//! that failed, so there was no failure to record, and the fault list a test
//! watches cannot tell "the fixture stayed silent about a write that failed"
//! from "no write failed". What the test really asserted was two properties of
//! the host: that sixteen mebibytes is more than a loopback connection with
//! nothing reading it will take from a *blocking* write, and that such a write
//! ends within thirty seconds. Neither holds everywhere — Windows buffers a
//! blocking send far past the sixty-four kilobytes its non-blocking one reports
//! room for, and how a send timeout ends a wait is the transport's business —
//! and the sibling test in
//! `e13_the_fixture_server_inherited_its_listeners_non_blocking_mode.rs` says as
//! much by asking the host about its buffering before claiming anything, which
//! this one never did.
//!
//! So the invariant is re-expressed rather than retimed, and it is the same
//! invariant: *a reply the fixture could not write whole is named, not sent
//! short in silence*. The transport is a [`HaltingSink`] that takes a stated
//! number of bytes and then fails with a stated error, so the write does fail,
//! on every platform, in the same place; the assertions are on the bytes that
//! reached the peer and on the fault that names the reply, both exact. Nothing
//! here waits, and nothing here measures the host. What the fixture chooses for
//! itself — that a reply's wait is bounded at all — is asserted of the socket
//! [`adopt`] hands to the serving code, which is the fixture's decision rather
//! than the platform's honouring of it.
//!
//! ## E14 — and over the whole of the decision, not half of it
//!
//! That first re-expression asserted on the fault [`send_reply`] hands back,
//! which left the other half of the rule — that the fault reaches the log
//! `TestServer::requests` refuses to answer over — held by nothing: a caller
//! that asked and dropped the answer is the pre-E13 defect exactly, one level
//! up from where it was fixed, and the whole suite stayed green with it in
//! place. So the seam is [`answer_reply`], which decides *and* records, and it
//! is the one call `serve_one` makes; the assertions below read the log back
//! rather than the value a caller was free to drop. The transport is still a
//! sink, for the reason above, and that sink now refuses `Interrupted`:
//! `write_all` retries that kind for ever, so a test that asked for it would
//! hang the binary instead of failing, which is the one thing every budget in
//! this fixture exists to prevent.

use std::io::ErrorKind;
use std::net::{TcpListener, TcpStream};

use crate::common::http::{
    AfterWrite, HaltingSink, REPLY_BUDGET, REQUEST_BUDGET, Reply, adopt, after_write, answer_reply,
    fault_log, reported, send_reply,
};

/// The path these tests script a reply for.
const PATH: &str = "/otp.tar.zst";

/// How long the body of that reply is.
///
/// Long enough that a cut ten bytes into it lands inside the body rather than
/// inside the head, which is what makes the reply short *under its own
/// `Content-Length`* rather than a head that never arrived.
const BODY_BYTES: usize = 1024;

/// How many bytes of the body the transport takes before it fails.
const PARTIAL: usize = 10;

/// Exactly the head that reply goes out under, byte for byte.
const HEAD: &str = "HTTP/1.1 200 OK\r\nContent-Length: 1024\r\nConnection: close\r\n\r\n";

/// Exactly the fault a reply cut short by a timed-out write leaves behind.
///
/// `timed out` is what `std::io::Error::from(ErrorKind::TimedOut)` displays,
/// which is what a Windows `WSAETIMEDOUT` arrives as.
const TIMED_OUT_FAULT: &str = "the reply to /otp.tar.zst was not written whole: timed out";

/// The reply these tests cut short.
fn scripted() -> Reply {
    Reply::ok(&vec![b'x'; BODY_BYTES])
}

/// Everything the fixture puts on the wire for `reply`, whole.
///
/// Taken from a sink that refuses nothing, so that a test can cut a reply one
/// byte short of its end without knowing how long its head is.
fn whole(reply: &Reply) -> Vec<u8> {
    let mut sink = HaltingSink::taking_everything();
    let fault = send_reply(&mut sink, PATH, reply);
    assert_eq!(
        fault, None,
        "a sink that takes everything cannot fail a write, so nothing here is the fixture's \
         failure and the bytes below are the whole of what a peer would have received"
    );
    sink.taken().to_vec()
}

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
fn a_reply_the_transport_cut_short_is_named_rather_than_sent_short_in_silence() {
    let mut sink = HaltingSink::halting_after(HEAD.len() + PARTIAL, ErrorKind::TimedOut);

    let fault = send_reply(&mut sink, PATH, &scripted());

    assert_eq!(
        sink.taken().len(),
        HEAD.len() + PARTIAL,
        "the head goes out whole — the peer has been promised {BODY_BYTES} bytes of body — and \
         only {PARTIAL} of those bytes follow it: a body shorter than the Content-Length already \
         sent, which is what `Reply::Truncated` is scripted by hand to be"
    );
    assert_eq!(
        &sink.taken()[..HEAD.len()],
        HEAD.as_bytes(),
        "and it is that head, byte for byte, so the promise the body falls short of is this one"
    );
    assert_eq!(
        fault.as_deref(),
        Some(TIMED_OUT_FAULT),
        "so the fixture says so, naming the reply it could not send: a truncated body nothing \
         says anything about is read by every retry test in tests/download.rs as a connection \
         that broke, which is a fixture failure reported as a product failure"
    );
}

#[test]
fn every_reply_shape_the_transport_cuts_short_names_the_reply_it_could_not_send() {
    for (what, reply) in [
        ("a body", scripted()),
        (
            "an encoded body",
            Reply::Encoded {
                status: 200,
                encoding: "gzip".to_owned(),
                body: vec![b'z'; BODY_BYTES],
            },
        ),
        (
            "a body already shorter than its promise",
            Reply::Truncated {
                promised: BODY_BYTES * 2,
                body: vec![b'x'; BODY_BYTES],
            },
        ),
    ] {
        let written = whole(&reply);
        let mut sink = HaltingSink::halting_after(written.len() - 1, ErrorKind::TimedOut);

        let fault = send_reply(&mut sink, PATH, &reply);

        assert_eq!(
            sink.taken().len(),
            written.len() - 1,
            "one byte of the reply carrying {what} never reached the peer"
        );
        assert_eq!(
            fault.as_deref(),
            Some(TIMED_OUT_FAULT),
            "so every shape this fixture writes says so, not just the one the first test scripts: \
             all three go out under a Content-Length that promises the whole of them"
        );
    }
}

#[test]
fn a_reply_the_peer_let_go_of_leaves_nothing_to_report() {
    for kind in [
        ErrorKind::BrokenPipe,
        ErrorKind::ConnectionReset,
        ErrorKind::ConnectionAborted,
    ] {
        let mut sink = HaltingSink::halting_after(HEAD.len() + PARTIAL, kind);

        let fault = send_reply(&mut sink, PATH, &scripted());

        assert_eq!(
            sink.taken().len(),
            HEAD.len() + PARTIAL,
            "the body still went out short — {kind:?} stops a write wherever it stopped it"
        );
        assert_eq!(
            fault, None,
            "but a client that stopped wanting the body is not the fixture's failure: `ureq` lets \
             go of every answer it abandons, and a fixture that recorded it would fail every test \
             that ran after one"
        );
    }
}

#[test]
fn a_reply_the_transport_took_whole_goes_out_byte_for_byte_and_reports_nothing() {
    let mut sink = HaltingSink::taking_everything();

    let fault = send_reply(&mut sink, PATH, &scripted());

    assert_eq!(fault, None, "nothing failed, so nothing is recorded");
    let mut expected = HEAD.as_bytes().to_vec();
    expected.extend_from_slice(&vec![b'x'; BODY_BYTES]);
    assert_eq!(
        sink.taken(),
        expected.as_slice(),
        "and the reply is the head and the whole body, with a Content-Length that counts exactly \
         the bytes that follow it: the contrast that makes a short body a lie rather than a \
         shape this fixture also sends"
    );

    let mut hangup = HaltingSink::taking_everything();
    assert_eq!(
        send_reply(&mut hangup, PATH, &Reply::Hangup),
        None,
        "and the one reply that writes nothing at all promises nothing either, so it has nothing \
         to fall short of"
    );
    assert!(hangup.taken().is_empty(), "it puts no bytes on the wire");
}

#[test]
fn a_reply_the_fixture_could_not_send_reaches_the_log_the_test_reads() {
    let faults = fault_log();
    let mut sink = HaltingSink::halting_after(HEAD.len() + PARTIAL, ErrorKind::TimedOut);

    answer_reply(&mut sink, PATH, &scripted(), &faults);

    assert_eq!(
        reported(&faults),
        vec![TIMED_OUT_FAULT.to_owned()],
        "asking whether the write that failed was the fixture's own is half of the rule; the \
         other half is that the answer reaches the log `TestServer::requests` refuses to answer \
         over. A caller that asked and dropped the answer sends the body short in silence exactly \
         as the pre-E13 fixture did, one level up from where it was fixed, so the two halves are \
         one function and this is the assertion that holds it"
    );
}

#[test]
fn a_reply_the_peer_let_go_of_leaves_the_log_empty() {
    for kind in [
        ErrorKind::BrokenPipe,
        ErrorKind::ConnectionReset,
        ErrorKind::ConnectionAborted,
    ] {
        let faults = fault_log();
        let mut sink = HaltingSink::halting_after(HEAD.len() + PARTIAL, kind);

        answer_reply(&mut sink, PATH, &scripted(), &faults);

        assert_eq!(
            reported(&faults),
            Vec::<String>::new(),
            "and nothing else reaches it: {kind:?} is a client that stopped wanting the body, and \
             a fixture that recorded one would fail every test that ran after a client refused an \
             answer"
        );
    }
}

#[test]
#[should_panic(expected = "`write_all` retries `Interrupted` for ever")]
fn a_sink_that_answers_interrupted_is_refused_rather_than_hung_on() {
    let _ = HaltingSink::halting_after(HEAD.len() + PARTIAL, ErrorKind::Interrupted);
}

#[test]
fn the_wait_a_reply_opens_up_is_bounded_by_the_fixtures_own_budget() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port");
    let address = listener.local_addr().expect("the bound address");
    let _peer = TcpStream::connect(address).expect("a connection to it");
    let (served, _) = listener.accept().expect("the other end of it");

    adopt(&served).expect("the mode this fixture serves its connections in");

    assert_eq!(
        served
            .write_timeout()
            .expect("the write timeout it was given"),
        Some(REPLY_BUDGET),
        "a reply the peer never reads must be a slow test rather than a serving thread that \
         never comes back, and this bound is the whole of what makes it one; whether a given \
         transport ends the wait exactly there is the transport's business, but a fixture that \
         asked for no bound at all could not be reported on by anything"
    );
    assert_eq!(
        served
            .read_timeout()
            .expect("the read timeout it was given"),
        Some(REQUEST_BUDGET),
        "and the request that precedes it is bounded the same way, by the same call"
    );
}
