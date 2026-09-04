// SPDX-License-Identifier: MIT OR Apache-2.0
//! One HTTPS fetch: what it writes, what it refuses, and what it asks twice.
//!
//! Every claim here is about a *server*, so the file drives a hand-rolled one
//! on a loopback port (`tests/common/http.rs`) rather than a fixture on disk. A
//! body that hashes wrong, a 500 that becomes a 200, a 404 that must not be
//! asked again and a connection that dies mid-body are the four failures the
//! retry policy exists for, and none of them can be written down as a file.
//!
//! The part file is asserted on as hard as the destination is. A fetch that
//! left `<dest>.part-<pid>` behind after a mismatch would fill a cache with
//! rubbish nothing ever looks at again, and a fetch that wrote a partial
//! `dest` would poison it.
// The command line half of the suite: nothing here exists in a launcher-only
// build, which fetches nothing.
#![cfg(feature = "cli")]

mod common;

use std::collections::BTreeMap;

use ginary::download::{
    self, BACKOFF_BASE, DownloadError, Expect, GITHUB_API_BASE, GITHUB_BASE_VAR, MAX_ATTEMPTS,
    MAX_TEXT_BYTES, Net, OFFLINE_VAR, PROXY_VARS,
};

use crate::common::http::{Reply, TestServer};
use crate::common::payload::{sha256, sha256_hex};

/// The body every successful fetch in this file is asked for.
const BODY: &[u8] = b"a prebuilt runtime, or near enough for a test\n";

/// A temporary directory the test owns.
fn tempdir() -> tempfile::TempDir {
    tempfile::tempdir().expect("a temporary directory")
}

/// The routes for one path answered in order.
fn routes(path: &str, replies: Vec<Reply>) -> BTreeMap<String, Vec<Reply>> {
    BTreeMap::from([(path.to_owned(), replies)])
}

// ------------------------------------------------------ the happy path --

#[test]
fn a_fetch_writes_the_body_to_the_destination_and_leaves_no_part_file() {
    let server = TestServer::one("/otp.tar.zst", Reply::ok(BODY));
    let dir = tempdir();
    let dest = dir.path().join("otp.tar.zst");

    let result = download::fetch(
        &server.url("/otp.tar.zst"),
        &dest,
        &Expect::exactly(sha256(BODY), BODY.len() as u64),
        &Net::online(),
    );

    assert_eq!(result, Ok(()), "the body verified, so the fetch succeeds");
    assert_eq!(
        std::fs::read(&dest).expect("the destination"),
        BODY,
        "the destination holds the body verbatim"
    );
    assert!(
        !download::part_path(&dest).exists(),
        "the part file is renamed onto the destination, not left beside it"
    );
    assert_eq!(server.hits("/otp.tar.zst"), 1, "one body, one request");
}

#[test]
fn a_fetch_with_nothing_expected_takes_whatever_the_server_sent() {
    let server = TestServer::one("/any", Reply::ok(BODY));
    let dir = tempdir();
    let dest = dir.path().join("any");

    let result = download::fetch(
        &server.url("/any"),
        &dest,
        &Expect::anything(),
        &Net::online(),
    );

    assert_eq!(result, Ok(()));
    assert_eq!(std::fs::read(&dest).expect("the destination"), BODY);
}

// -------------------------------------------------------- verification --

#[test]
fn a_checksum_mismatch_names_both_digests_and_removes_the_part_file() {
    let server = TestServer::one("/otp.tar.zst", Reply::ok(BODY));
    let dir = tempdir();
    let dest = dir.path().join("otp.tar.zst");
    let wrong = [0x11_u8; 32];

    let url = server.url("/otp.tar.zst");
    let result = download::fetch(
        &url,
        &dest,
        &Expect {
            sha256: Some(wrong),
            size: None,
        },
        &Net::online(),
    );

    match result {
        Err(DownloadError::ChecksumMismatch {
            url: named,
            expected,
            actual,
        }) => {
            assert_eq!(named, url);
            assert_eq!(expected, hex::encode(wrong));
            assert_eq!(actual, sha256_hex(BODY));
        }
        other => panic!("a body that hashes wrong is a checksum mismatch, not {other:?}"),
    }
    assert!(
        !dest.exists(),
        "a file that failed verification is not kept"
    );
    assert!(
        !download::part_path(&dest).exists(),
        "and neither is the part file it was streamed into"
    );
}

#[test]
fn a_length_that_is_not_the_expected_one_names_both_lengths() {
    let server = TestServer::one("/otp.tar.zst", Reply::ok(BODY));
    let dir = tempdir();
    let dest = dir.path().join("otp.tar.zst");

    let result = download::fetch(
        &server.url("/otp.tar.zst"),
        &dest,
        &Expect {
            sha256: None,
            size: Some(1),
        },
        &Net::online(),
    );

    match result {
        Err(DownloadError::SizeMismatch {
            expected, actual, ..
        }) => {
            assert_eq!(expected, 1);
            assert_eq!(actual, BODY.len() as u64);
        }
        other => panic!("a body of the wrong length is a size mismatch, not {other:?}"),
    }
    assert!(!dest.exists());
}

// --------------------------------------------------------- the retries --

#[test]
fn a_server_error_is_asked_again_and_the_second_answer_is_the_file() {
    let server = TestServer::start(routes(
        "/otp.tar.zst",
        vec![Reply::status(500), Reply::ok(BODY)],
    ));
    let dir = tempdir();
    let dest = dir.path().join("otp.tar.zst");

    let result = download::fetch(
        &server.url("/otp.tar.zst"),
        &dest,
        &Expect::exactly(sha256(BODY), BODY.len() as u64),
        &Net::online(),
    );

    assert_eq!(result, Ok(()), "a 500 is transient and the retry succeeded");
    assert_eq!(
        server.hits("/otp.tar.zst"),
        2,
        "exactly one retry: the second answer was the file"
    );
}

#[test]
fn a_body_that_stops_short_is_retried_like_any_other_transport_failure() {
    let server = TestServer::start(routes(
        "/otp.tar.zst",
        vec![
            Reply::Truncated {
                promised: BODY.len(),
                body: BODY[..10].to_vec(),
            },
            Reply::ok(BODY),
        ],
    ));
    let dir = tempdir();
    let dest = dir.path().join("otp.tar.zst");

    let result = download::fetch(
        &server.url("/otp.tar.zst"),
        &dest,
        &Expect::exactly(sha256(BODY), BODY.len() as u64),
        &Net::online(),
    );

    assert_eq!(result, Ok(()));
    assert_eq!(std::fs::read(&dest).expect("the destination"), BODY);
    assert_eq!(server.hits("/otp.tar.zst"), 2);
}

#[test]
fn a_not_found_is_asked_exactly_once_and_names_the_status() {
    let server = TestServer::start(routes("/missing.tar.zst", vec![Reply::status(404)]));
    let dir = tempdir();
    let dest = dir.path().join("missing.tar.zst");
    let url = server.url("/missing.tar.zst");

    let result = download::fetch(&url, &dest, &Expect::anything(), &Net::online());

    assert_eq!(
        result,
        Err(DownloadError::Status {
            url: url.clone(),
            status: 404,
        }),
        "a 404 is the server saying the request was wrong"
    );
    assert_eq!(
        server.hits("/missing.tar.zst"),
        1,
        "asking a wrong question twice more is not a retry"
    );
}

#[test]
fn every_attempt_failing_reports_the_url_and_how_many_were_made() {
    let server = TestServer::start(routes(
        "/otp.tar.zst",
        vec![Reply::status(503), Reply::status(503), Reply::status(503)],
    ));
    let dir = tempdir();
    let dest = dir.path().join("otp.tar.zst");
    let url = server.url("/otp.tar.zst");

    let result = download::fetch(&url, &dest, &Expect::anything(), &Net::online());

    match result {
        Err(DownloadError::Exhausted {
            url: named,
            attempts,
            last,
        }) => {
            assert_eq!(named, url);
            assert_eq!(attempts, MAX_ATTEMPTS);
            assert!(
                last.contains("503"),
                "the last failure is quoted, not summarised: {last}"
            );
        }
        other => panic!("three 5xx answers exhaust the attempts, not {other:?}"),
    }
    assert_eq!(server.hits("/otp.tar.zst"), MAX_ATTEMPTS as usize);
    assert!(!download::part_path(&dest).exists());
}

// --------------------------------------------------------- the offline --

#[test]
fn an_offline_fetch_names_the_url_and_where_the_file_would_have_gone() {
    let server = TestServer::one("/otp.tar.zst", Reply::ok(BODY));
    let dir = tempdir();
    let dest = dir.path().join("otp.tar.zst");
    let url = server.url("/otp.tar.zst");

    let result = download::fetch(&url, &dest, &Expect::anything(), &Net::offline());

    assert_eq!(
        result,
        Err(DownloadError::Offline {
            url: url.clone(),
            dest_hint: dest.clone(),
        })
    );
    assert_eq!(
        server.hits("/otp.tar.zst"),
        0,
        "an offline build does not open the socket to find out"
    );
    assert!(!dest.exists());
    assert_eq!(
        result.expect_err("offline is an error").to_string(),
        format!(
            "offline: {url} would be fetched to {}; fetch it on a connected machine, or point \
             `--catalog` at a local copy",
            dest.display()
        ),
        "the message says what to do about it, not just that it happened"
    );
}

// ---------------------------------------------------------- the policy --

#[test]
fn the_part_file_is_the_destination_plus_this_process_id() {
    let path = download::part_path(std::path::Path::new("/cache/otp/otp-29.0.5.tar.zst"));

    assert_eq!(
        path,
        std::path::PathBuf::from(format!(
            "/cache/otp/otp-29.0.5.tar.zst.part-{}",
            std::process::id()
        )),
        "two processes fetching one entry may not write one another's bytes"
    );
}

#[test]
fn the_backoff_doubles_from_the_base_at_every_attempt() {
    assert_eq!(download::backoff(1), BACKOFF_BASE);
    assert_eq!(download::backoff(2), BACKOFF_BASE * 2);
    assert_eq!(download::backoff(3), BACKOFF_BASE * 4);
}

#[test]
fn only_a_server_error_is_worth_asking_again() {
    for status in [500_u16, 502, 503, 504] {
        assert!(download::retryable(status), "{status} is transient");
    }
    for status in [200_u16, 301, 400, 401, 403, 404, 429] {
        assert!(
            !download::retryable(status),
            "{status} will not change on the second ask"
        );
    }
}

#[test]
fn a_digest_is_sixty_four_lower_case_hexadecimal_digits_and_nothing_else() {
    let digest = sha256_hex(BODY);
    assert_eq!(
        download::parse_sha256(&digest),
        Some(sha256(BODY)),
        "the spelling the catalog writes is the one that reads back"
    );

    assert_eq!(
        download::parse_sha256(&digest.to_uppercase()),
        None,
        "two spellings of one digest is a difference nothing downstream could explain"
    );
    assert_eq!(download::parse_sha256(&digest[..63]), None, "too short");
    assert_eq!(
        download::parse_sha256(&format!("{digest}0")),
        None,
        "too long"
    );
    assert_eq!(download::parse_sha256(""), None, "empty");
    assert_eq!(
        download::parse_sha256(&"g".repeat(64)),
        None,
        "not hexadecimal"
    );
}

// ------------------------------------------------------- the documents --

/// The release description every `get_text` test here is answered with.
const DOCUMENT: &str = r#"{"tag_name":"OTP-29.0.5","assets":[]}"#;

#[test]
fn a_document_is_read_back_verbatim_and_asked_for_as_json() {
    let server = TestServer::one("/release", Reply::ok(DOCUMENT.as_bytes()));

    let text = download::get_text(&server.url("/release"), &Net::online())
        .expect("a 200 with a small body");

    assert_eq!(text, DOCUMENT, "the body is the answer, byte for byte");
    assert_eq!(server.hits("/release"), 1);
    let request = server.requests().pop().expect("one request");
    assert_eq!(
        request.headers.get("accept").map(String::as_str),
        Some("application/vnd.github+json"),
        "the release API is asked in its own dialect"
    );
}

#[test]
fn a_document_behind_a_500_is_asked_for_again() {
    let server = TestServer::start(routes(
        "/release",
        vec![Reply::status(500), Reply::ok(DOCUMENT.as_bytes())],
    ));

    let text = download::get_text(&server.url("/release"), &Net::online())
        .expect("the second attempt answered");

    assert_eq!(text, DOCUMENT);
    assert_eq!(server.hits("/release"), 2, "one retry, and no more");
}

#[test]
fn a_document_that_is_not_there_is_asked_for_exactly_once() {
    let server = TestServer::one("/release", Reply::status(404));

    let error = download::get_text(&server.url("/release"), &Net::online())
        .expect_err("a tag that does not exist is not going to appear");

    match &error {
        DownloadError::Status { status, url } => {
            assert_eq!(*status, 404);
            assert!(url.ends_with("/release"), "the URL is named: {url}");
        }
        other => panic!("expected a status error, got {other:?}"),
    }
    assert_eq!(
        server.hits("/release"),
        1,
        "a 4xx is an answer, not a transport failure"
    );
}

#[test]
fn a_document_larger_than_the_bound_is_refused_rather_than_read_into_memory() {
    // One byte over is enough: the point is that there *is* a bound, not that
    // a particular size crosses it.
    let oversized = vec![b'x'; MAX_TEXT_BYTES as usize + 1];
    let server = TestServer::one("/release", Reply::ok(&oversized));
    let url = server.url("/release");

    let error = download::get_text(&url, &Net::online())
        .expect_err("a release description is not four megabytes");

    match &error {
        DownloadError::TooLarge { url: named, limit } => {
            assert_eq!(named, &url);
            assert_eq!(*limit, MAX_TEXT_BYTES);
        }
        other => panic!("expected the bound to refuse the body, got {other:?}"),
    }
    assert_eq!(
        server.hits("/release"),
        1,
        "a body over the bound is an answer, not a transport failure: it is over the bound on \
         the third ask too, so it is asked for exactly once — the same rule as the 404"
    );
}

#[test]
fn an_offline_document_read_opens_no_socket_at_all() {
    let server = TestServer::one("/release", Reply::ok(DOCUMENT.as_bytes()));

    let error = download::get_text(&server.url("/release"), &Net::offline())
        .expect_err("offline reads nothing");

    match &error {
        DownloadError::Offline { url, dest_hint } => {
            assert!(url.ends_with("/release"));
            assert_eq!(
                dest_hint.display().to_string(),
                "(read into memory)",
                "a document has no destination on disk to name"
            );
        }
        other => panic!("expected the offline refusal, got {other:?}"),
    }
    assert_eq!(server.hits("/release"), 0);
}

#[test]
fn a_document_read_goes_through_the_base_override() {
    let server = TestServer::one(
        "/repos/example/releases/tags/OTP-29.0.5",
        Reply::ok(DOCUMENT.as_bytes()),
    );
    let net = Net {
        offline: false,
        base_overrides: BTreeMap::from([(GITHUB_API_BASE.to_owned(), server.base())]),
    };

    let text = download::get_text(
        &format!("{GITHUB_API_BASE}/repos/example/releases/tags/OTP-29.0.5"),
        &net,
    )
    .expect("the override sends the request at the local server");

    assert_eq!(text, DOCUMENT);
    assert_eq!(server.hits("/repos/example/releases/tags/OTP-29.0.5"), 1);
}

// ------------------------------------------------------------ the net --

#[test]
fn a_base_override_redirects_a_whole_host_and_leaves_every_other_url_alone() {
    let net = Net {
        offline: false,
        base_overrides: BTreeMap::from([(
            GITHUB_API_BASE.to_owned(),
            "http://127.0.0.1:9/api".to_owned(),
        )]),
    };

    assert_eq!(
        net.rewrite(&format!(
            "{GITHUB_API_BASE}/repos/x/y/releases/tags/OTP-29.0.5"
        )),
        "http://127.0.0.1:9/api/repos/x/y/releases/tags/OTP-29.0.5",
        "a base is a prefix, so one override redirects everything under it"
    );
    assert_eq!(
        net.rewrite("https://objects.githubusercontent.com/blob"),
        "https://objects.githubusercontent.com/blob",
        "a URL that matches no base is returned unchanged"
    );
}

#[test]
fn the_longest_matching_base_is_the_one_that_wins() {
    let net = Net {
        offline: false,
        base_overrides: BTreeMap::from([
            ("https://example.test".to_owned(), "http://short".to_owned()),
            (
                "https://example.test/deep".to_owned(),
                "http://long".to_owned(),
            ),
        ]),
    };

    assert_eq!(
        net.rewrite("https://example.test/deep/file"),
        "http://long/file"
    );
    assert_eq!(
        net.rewrite("https://example.test/other"),
        "http://short/other"
    );
}

#[test]
fn the_environment_can_turn_offline_on_and_cannot_turn_it_off() {
    let on = BTreeMap::from([(OFFLINE_VAR.to_owned(), "1".to_owned())]);
    let off = BTreeMap::from([(OFFLINE_VAR.to_owned(), "0".to_owned())]);

    assert!(
        Net::from_vars(false, &on).offline,
        "{OFFLINE_VAR}=1 takes a build off the network"
    );
    assert!(
        !Net::from_vars(false, &off).offline,
        "{OFFLINE_VAR}=0 is not a request to go offline"
    );
    assert!(
        Net::from_vars(true, &off).offline,
        "a build asked to stay offline is not put back on the network by an environment"
    );
    assert!(!Net::from_vars(false, &BTreeMap::new()).offline);

    assert_eq!(
        PROXY_VARS,
        ["HTTPS_PROXY", "NO_PROXY", "https_proxy", "no_proxy"],
        "the transport reads these four; nothing here re-implements NO_PROXY matching"
    );
}

#[test]
fn the_github_base_variable_becomes_an_override_of_the_release_api() {
    let vars = BTreeMap::from([(GITHUB_BASE_VAR.to_owned(), "http://mirror.test".to_owned())]);

    let net = Net::from_vars(false, &vars);

    assert_eq!(
        net.base_overrides.get(GITHUB_API_BASE).map(String::as_str),
        Some("http://mirror.test"),
        "{GITHUB_BASE_VAR} is the mirror switch"
    );
    assert_eq!(
        net.rewrite(&format!("{GITHUB_API_BASE}/repos")),
        "http://mirror.test/repos"
    );
}
