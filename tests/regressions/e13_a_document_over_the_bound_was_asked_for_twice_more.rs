// SPDX-License-Identifier: MIT OR Apache-2.0
//! A document larger than the bound `get_text` reads within was treated as a
//! transport failure, so the reader asked for it three times and transferred
//! three oversized bodies before refusing the first of them.
//!
//! **What went wrong.** `src/download.rs`'s retry rule is one sentence: a
//! transport failure and a 5xx are asked again, a 4xx is not, because "a body
//! that is not there will not be there on the third ask, and retrying it only
//! slows the error down". `text_once` classified every failure of
//! `read_to_string` as retryable, and the limit `MAX_TEXT_BYTES` puts on that
//! read fails it exactly like a connection dying mid-body does. A release
//! description of four mebibytes and one byte was therefore fetched, refused,
//! backed off from, fetched again, refused again, backed off from again,
//! fetched a third time and reported as [`DownloadError::Exhausted`] — an error
//! that says every attempt failed and quotes the last of them, when what
//! happened is that the answer was one this build will not read at any size of
//! ask. Twelve mebibytes crossed the wire to state a bound of four.
//!
//! ```text
//! ---- a_document_larger_than_the_bound_is_refused_rather_than_read_into_memory ----
//! thread '...' (5304) panicked at tests\download.rs:436:5:
//! assertion `left == right` failed: a body that would not fit is retried like
//! any other read failure
//!   left: 2
//!  right: 3
//! ```
//!
//! (`Windows build and exit-code propagation`
//! <https://github.com/P4suta/ginary/actions/runs/33837795146/job/100913758843>,
//! `tests/download.rs:436`.)
//!
//! That failure is the fixture's, not the reader's — see
//! `e13_the_fixture_server_inherited_its_listeners_non_blocking_mode` for the
//! connection the server lost — but it is the failure that put the question. A
//! test can only count three requests if the code under it makes three, and
//! this is one of two places in the suite where a client asks three times for
//! something it can never accept. The count was the symptom; asking twice more
//! for an answer whose size is already known is the defect underneath it.
//!
//! **The input.** Any URL whose body is longer than
//! [`download::MAX_TEXT_BYTES`], read through
//! [`download::get_text`].
//!
//! **The correct behaviour.** The bound is an answer, not a failure of the
//! transport: the first ask settles it. `get_text` reports
//! [`DownloadError::TooLarge`], naming the URL and the bound its answer
//! crossed, after exactly one request — the same shape, and the same reasoning,
//! as the 404 that is asked for exactly once. Every *other* read failure stays
//! retryable, because a body that stopped short is a connection that broke and
//! that is what the retries are for.

use std::collections::BTreeMap;

use ginary::download::{self, DownloadError, MAX_ATTEMPTS, MAX_TEXT_BYTES, Net};

use crate::common::http::{Reply, TestServer};

/// The path every server here answers.
const PATH: &str = "/release";

/// A body one byte over the bound: the point is that there is a bound, not that
/// a particular size crosses it.
fn oversized() -> Vec<u8> {
    vec![b'x'; MAX_TEXT_BYTES as usize + 1]
}

#[test]
fn a_document_over_the_bound_is_refused_after_exactly_one_request() {
    let server = TestServer::one(PATH, Reply::ok(&oversized()));
    let url = server.url(PATH);

    let error = download::get_text(&url, &Net::online())
        .expect_err("a release description is not four mebibytes");

    assert_eq!(
        error,
        DownloadError::TooLarge {
            url: url.clone(),
            limit: MAX_TEXT_BYTES,
        },
        "a body over the bound is an answer about the document, not a transport failure: \
         reporting it as {:?} says every attempt failed and quotes the last one, which tells the \
         reader nothing about the one thing that was wrong",
        DownloadError::Exhausted {
            url,
            attempts: MAX_ATTEMPTS,
            last: String::new(),
        }
    );
    assert_eq!(
        server.hits(PATH),
        1,
        "and it is asked for once: a document that is too large is too large on the third ask \
         too, so the retries only transfer it twice more"
    );
}

#[test]
fn the_refusal_names_the_url_and_the_bound_it_crossed() {
    let server = TestServer::one(PATH, Reply::ok(&oversized()));
    let url = server.url(PATH);

    let error = download::get_text(&url, &Net::online()).expect_err("over the bound");

    assert_eq!(
        error.to_string(),
        format!(
            "{url} answered more than the {MAX_TEXT_BYTES} bytes a document is read within; \
             point `--catalog` at a local copy, which is read without that bound"
        ),
        "all three are in the message: a reader told only that something was too large knows \
         neither which document nor how large this build allows, and one told both still has \
         nothing to do about it — the local file this build reads with no bound at all is the \
         route the message has to name, as `Offline` names it"
    );
}

#[test]
fn a_document_of_exactly_the_bound_is_read_whole() {
    // The bound is inclusive, and it is load-bearing in both directions now
    // that it chooses between a refusal and a document: a test that only ever
    // asks one byte over it would pass an implementation that read one byte
    // less than it promises.
    let document = vec![b'x'; MAX_TEXT_BYTES as usize];
    let server = TestServer::one(PATH, Reply::ok(&document));

    let text = download::get_text(&server.url(PATH), &Net::online())
        .expect("a document of exactly the bound is within it");

    assert_eq!(
        text.len() as u64,
        MAX_TEXT_BYTES,
        "the whole of it, not the bound less one"
    );
    assert!(text.bytes().all(|byte| byte == b'x'));
    assert_eq!(
        server.hits(PATH),
        1,
        "and it is asked for once, because it was answered"
    );
}

#[test]
fn a_document_whose_body_stops_short_is_still_asked_for_again() {
    let document = r#"{"tag_name":"OTP-29.0.5","assets":[]}"#;
    let server = TestServer::start(BTreeMap::from([(
        PATH.to_owned(),
        vec![
            Reply::Truncated {
                promised: document.len(),
                body: document.as_bytes()[..10].to_vec(),
            },
            Reply::ok(document.as_bytes()),
        ],
    )]));

    let text = download::get_text(&server.url(PATH), &Net::online())
        .expect("a connection that broke mid-body is what the retries are for");

    assert_eq!(text, document);
    assert_eq!(
        server.hits(PATH),
        2,
        "the bound is the one read failure that is not the transport's; every other one is still \
         worth asking again"
    );
}
