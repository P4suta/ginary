// SPDX-License-Identifier: MIT OR Apache-2.0
//! The bound on how large a document `get_text` reads counted the bytes that
//! arrived rather than the bytes that landed in memory, so a compressed answer
//! walked straight past it: sixty-four kilobytes on the wire became sixty-four
//! mebibytes in a `Vec`, under a bound of four.
//!
//! **What went wrong.** `text_once` read the body through
//! `into_with_config().limit(READ_LIMIT).read_to_vec()`, and `ureq`'s limit is
//! the *innermost* reader of the body chain: `ureq-3.4.0/src/body/mod.rs:735`
//! builds `MaybeLossyDecoder<CharsetDecoder<ContentDecoder<LimitReader<..>>>>`,
//! so the limit counts what came off the socket and the gzip decoder inflates
//! afterwards. Nothing in this crate asked for that decoder and nothing could
//! turn it off from the call site: the `gzip` feature is on (`Cargo.toml`), and
//! `ureq-3.4.0/src/run.rs:315-323` adds `accept-encoding: gzip` to every
//! request whose caller has not set that header — which `get_text`, setting
//! only `accept`, had not. Deflate reaches about 1032:1, so four mebibytes of
//! wire is some four gigabytes of `Vec`, and a mirror pointed at by
//! `GINARY_GITHUB_BASE_URL` or a `ginary otp update <url>` reaches the same
//! path.
//!
//! Three statements this milestone wrote were therefore false for any answer
//! that arrived compressed: `MAX_TEXT_BYTES` ("a document of exactly this many
//! bytes is read whole, and one byte more is refused"), `READ_LIMIT` (it makes
//! `MAX_TEXT_BYTES` "the largest document that is read"), and
//! `DownloadError::TooLarge` ("answered more than the {limit} bytes a document
//! is read within"). The suite could not see it: `tests/common/http.rs` had no
//! shape that sends a `Content-Encoding` at all, so every test of the bound
//! spoke to a server that never compressed.
//!
//! **The input.** Any answer carrying `Content-Encoding: gzip` whose body
//! inflates past [`download::MAX_TEXT_BYTES`], read through
//! [`download::get_text`].
//!
//! **The correct behaviour.** The bound is on the document, so it is counted
//! where the document is: on the decoded stream, which is what `read_to_end`
//! fills. A compressed answer that inflates past it is
//! [`DownloadError::TooLarge`] after exactly one request, like any other answer
//! over the bound, and the reader stops at the bound rather than inflating the
//! whole of it first. The transfer is bounded too, for the one case the
//! document bound cannot answer — an encoding that consumes bytes without
//! producing any — but by a number of its own, twice the document's: an
//! encoding may make a body *larger* than what it encodes, so a transfer
//! bounded by the document's own number would refuse a document at the bound
//! that a server chose to compress.

use flate2::Compression;
use flate2::write::GzEncoder;
use std::io::Write;

use ginary::download::{self, DownloadError, MAX_TEXT_BYTES, Net};

use crate::common::http::{Reply, TestServer};

/// The path every server here answers.
const PATH: &str = "/release";

/// A document far larger than the bound, from a body far smaller than it.
///
/// Sixteen times the bound, which gzip takes down to a few kilobytes: the
/// difference between the two counts is the whole of the defect, so the test
/// states it at a size no reading of "wire bytes" could call over the bound.
const INFLATED_BYTES: usize = 16 * (MAX_TEXT_BYTES as usize);

/// One gzip member holding `body`.
///
/// [`Compression::fast`] rather than the default: nothing here is about the
/// ratio, and these bodies are megabytes of one byte, which the fast setting
/// takes down just as far.
fn gzipped(body: &[u8]) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
    encoder.write_all(body).expect("a gzip member of the body");
    encoder.finish().expect("the finished gzip member")
}

/// One gzip member holding `body` with nothing compressed away.
///
/// [`Compression::none`] writes deflate's stored blocks, which every gzip
/// reader accepts and which make the member *larger* than what it holds: five
/// bytes for each 65535-byte block, plus the member's own header and trailer.
/// It is what a server that compresses everything sends for a body that will
/// not compress, and it is the case a transfer bounded by the document's own
/// number would refuse a document that is within that number.
fn stored(body: &[u8]) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::none());
    encoder.write_all(body).expect("a gzip member of the body");
    encoder.finish().expect("the finished gzip member")
}

/// The bytes `reply` puts on the wire, which is not the document's length.
fn wire_bytes(reply: &Reply) -> usize {
    match reply {
        Reply::Encoded { body, .. } => body.len(),
        other => panic!("the fixture was scripted a compressed reply, not {other:?}"),
    }
}

/// A `200` whose body is `body` gzipped, as a compressing server answers.
fn compressed(body: &[u8]) -> Reply {
    Reply::Encoded {
        status: 200,
        encoding: "gzip".to_owned(),
        body: gzipped(body),
    }
}

#[test]
fn a_compressed_document_that_inflates_past_the_bound_is_refused_rather_than_read_into_memory() {
    let reply = compressed(&vec![b'x'; INFLATED_BYTES]);
    let wire = wire_bytes(&reply);
    assert!(
        (wire as u64) < MAX_TEXT_BYTES,
        "the body this states its claim on is smaller than the bound on the wire and larger than \
         it in memory, which is the difference the bound has to be counted on the right side of: \
         {wire} bytes"
    );
    let server = TestServer::one(PATH, reply);
    let url = server.url(PATH);

    // The document itself is never printed: it is sixteen times the bound, and
    // a failure that dumps it is a failure nobody can read.
    let error = match download::get_text(&url, &Net::online()) {
        Ok(text) => panic!(
            "the bound refused nothing: {} bytes of document were read into memory from {wire} \
             bytes on the wire, under a bound of {MAX_TEXT_BYTES}",
            text.len()
        ),
        Err(error) => error,
    };

    assert_eq!(
        error,
        DownloadError::TooLarge {
            url: url.clone(),
            limit: MAX_TEXT_BYTES,
        },
        "the bound is on the document, not on the transfer: a limit the client inflates the body \
         *after* is no bound at all on what is read into memory, and every message about it says \
         it is one"
    );
    assert_eq!(
        server.hits(PATH),
        1,
        "and it is asked for once, on the rule every answer over the bound follows"
    );
}

#[test]
fn a_compressed_document_of_exactly_the_bound_is_read_whole() {
    // The bound is inclusive in the count that matters, and a fix that bounded
    // the decoded stream one byte short would pass a test that only ever asks
    // for something far over it.
    let document = vec![b'x'; MAX_TEXT_BYTES as usize];
    let server = TestServer::one(PATH, compressed(&document));

    let text = download::get_text(&server.url(PATH), &Net::online())
        .expect("a compressed document of exactly the bound is within it");

    assert_eq!(
        text.len() as u64,
        MAX_TEXT_BYTES,
        "the whole of it, not the bound less one"
    );
    assert!(text.bytes().all(|byte| byte == b'x'));
}

#[test]
fn an_ordinary_compressed_document_is_still_read() {
    let document = r#"{"tag_name":"OTP-29.0.5","assets":[]}"#;
    let server = TestServer::one(PATH, compressed(document.as_bytes()));

    let text = download::get_text(&server.url(PATH), &Net::online())
        .expect("a compressed answer is an ordinary answer");

    assert_eq!(
        text, document,
        "counting the bound on the decoded stream does not stop the stream being decoded: this \
         build asks for gzip and has to read what it asked for"
    );
    assert_eq!(server.hits(PATH), 1);
}

#[test]
fn a_document_at_the_bound_whose_encoding_made_it_larger_is_still_read_whole() {
    // The other half of counting the document rather than the transfer. The
    // transfer is bounded too — an encoding that consumes bytes without
    // producing any cannot be read for ever — but that bound may not be able to
    // refuse a document the document bound admits, and an encoding is allowed
    // to make a body bigger than what it encodes.
    let document = vec![b'x'; MAX_TEXT_BYTES as usize];
    let reply = Reply::Encoded {
        status: 200,
        encoding: "gzip".to_owned(),
        body: stored(&document),
    };
    let wire = wire_bytes(&reply);
    assert!(
        (wire as u64) > MAX_TEXT_BYTES,
        "this test states nothing unless the encoding really did make the body larger than the \
         document: {wire} bytes on the wire for {MAX_TEXT_BYTES} of document"
    );
    let server = TestServer::one(PATH, reply);

    let text = download::get_text(&server.url(PATH), &Net::online())
        .expect("a document of exactly the bound is within it, whatever its transfer cost");

    assert_eq!(
        text.len() as u64,
        MAX_TEXT_BYTES,
        "the whole of it: a transfer bounded by the document's own number refuses a document at \
         the bound that a server chose to compress, which is the bound refusing what it promises \
         to read"
    );
    assert!(text.bytes().all(|byte| byte == b'x'));
}
