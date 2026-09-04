// SPDX-License-Identifier: MIT OR Apache-2.0
//! A body that is not UTF-8 text was treated as a transport failure, so the
//! reader asked for it three times and transferred three undecodable bodies
//! before reporting that every attempt had failed.
//!
//! **What went wrong.** E13 settled one half of the rule and left the other
//! half one line away. `text_once` refuses a body over
//! [`download::MAX_TEXT_BYTES`] after a single request, because the size of an
//! answer is settled by the first ask; but it read the body with
//! `BodyWithConfig::read_to_string`, whose `lossy_utf8` is `false`, so a stray
//! byte that is not part of a valid UTF-8 sequence came back as
//! `io::Error(InvalidData, "stream did not contain valid UTF-8")`, became
//! `ureq::Error::Io` and fell into the `other => Attempt::Retryable(..)` arm —
//! under a comment claiming that "every other read failure here is the
//! transport's".
//!
//! It is not the transport's. A document whose bytes are not text is not text
//! on the third ask either, exactly as a document over the bound is still over
//! it and a 404 is still a 404. The reader nevertheless asked twice more and
//! then reported [`DownloadError::Exhausted`], which says every attempt failed
//! and quotes the last one — telling the reader nothing about the one thing
//! that was wrong with the answer.
//!
//! **The input.** Any URL whose body holds a byte sequence that is not valid
//! UTF-8, read through [`download::get_text`]. Both callers reach it with a URL
//! the user chose: `ginary otp update <url>` reads a catalogue that way, and
//! the release reader does once `GINARY_GITHUB_BASE_URL` points at a mirror.
//!
//! **The correct behaviour.** The decoding is an answer about the document, so
//! the first ask settles it: [`DownloadError::NotText`], naming the URL and the
//! offset of the first byte that is not text, after exactly one request. Every
//! failure of the transport — a body that stops short, a connection that dies
//! mid-read — stays retryable, which is what
//! `a_document_whose_body_stops_short_is_still_asked_for_again` pins.

use ginary::download::{self, DownloadError, Net};

use crate::common::http::{Reply, TestServer};

/// The path every server here answers.
const PATH: &str = "/release";

/// The byte no UTF-8 sequence can begin with.
const BAD: u8 = 0xff;

/// A JSON document with one byte in it that is not text.
fn undecodable() -> Vec<u8> {
    let mut body = br#"{"tag_name":"OTP-29.0.5","assets":["#.to_vec();
    body.push(BAD);
    body.extend_from_slice(b"]}");
    body
}

/// Where the text stops being text, counted rather than written down.
fn offset() -> usize {
    undecodable()
        .iter()
        .position(|byte| *byte == BAD)
        .expect("the byte this document is built around")
}

#[test]
fn a_body_that_is_not_text_is_asked_for_exactly_once() {
    let server = TestServer::one(PATH, Reply::ok(&undecodable()));
    let url = server.url(PATH);

    let error = download::get_text(&url, &Net::online())
        .expect_err("a document that is not text cannot be read as text");

    assert_eq!(
        error,
        DownloadError::NotText {
            url: url.clone(),
            offset: offset(),
        },
        "a body that is not text is an answer about the document, not a transport failure"
    );
    assert_eq!(
        server.hits(PATH),
        1,
        "bytes that are not text are not text on the third ask either, so the retries only \
         transfer the same undecodable body twice more"
    );
}

#[test]
fn the_refusal_names_the_url_and_where_the_text_stopped() {
    let server = TestServer::one(PATH, Reply::ok(&undecodable()));
    let url = server.url(PATH);

    let error = download::get_text(&url, &Net::online()).expect_err("not text");

    assert_eq!(
        error.to_string(),
        format!(
            "{url} answered bytes that are not text: byte {} is not valid UTF-8",
            offset()
        ),
        "both halves are in the message, because a reader told only that something was not text \
         knows neither which document nor where to look in it"
    );
}

#[test]
fn a_document_that_is_text_is_still_read_whole() {
    let document = r#"{"tag_name":"OTP-29.0.5","assets":[]}"#;
    let server = TestServer::one(PATH, Reply::ok(document.as_bytes()));

    let text = download::get_text(&server.url(PATH), &Net::online())
        .expect("a document that is text is read as text");

    assert_eq!(
        text, document,
        "the refusal is about the bytes, not about JSON"
    );
    assert_eq!(server.hits(PATH), 1);
}
