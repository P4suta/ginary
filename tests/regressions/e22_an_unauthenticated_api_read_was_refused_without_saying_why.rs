// SPDX-License-Identifier: MIT OR Apache-2.0
//! `ginary otp repack` read the GitHub release API with no credential and no
//! reading of why a refusal was a refusal, so a rate-limited run reported four
//! words and left the user to guess.
//!
//! **What went wrong.** The one API request this project makes,
//! `catalog::release_asset` through [`download::get_text`], sent an `accept`
//! header and nothing else. GitHub limits an unauthenticated read to 60 an hour
//! *by source address*, which a CI runner, a proxy and an office all share, and
//! answers over it with 403. [`download::retryable`] is 5xx only — correctly, a
//! 4xx will not become a 2xx on the third ask — so the run ended on
//!
//! ```text
//! error: cannot fetch the upstream asset: https://api.github.com/repos/gleam-community/erlang-linux-builds/releases/tags/OTP-29.0.5 answered HTTP 403
//! ```
//!
//! which is indistinguishable from a repository that is private, a tag that is
//! not there, and a token that is wrong. It failed a CI job of the first
//! release pull request on a repository that was not doing anything unusual.
//!
//! **The input.** Any 403 or 429 from the API carrying `x-ratelimit-remaining:
//! 0` or `retry-after`, which is what a shared address gets on its sixty-first
//! read of the hour.
//!
//! **The correct behaviour.** Two halves, and the second is why the first is
//! safe. The refusal is read: a rate-limited answer becomes
//! [`DownloadError::RateLimited`], which names the limit, the delay the server
//! asked for when it named one, and the variables a token comes from. And a
//! token, when there is one, is sent — to the API base and to nothing else. The
//! asset bytes come from a release download URL that redirects to a storage
//! host, so a credential on that path would be handed to a third party for no
//! purpose; `fetch` therefore sends no authorization at all, `get_text` sends it
//! only for a URL under [`download::GITHUB_API_BASE`], and the client is
//! configured so that a redirect does not carry it onward.

use std::collections::BTreeMap;

use ginary::download::{
    self, DownloadError, Expect, GITHUB_API_BASE, GITHUB_TOKEN_VARS, Net, Token,
};

use crate::common::http::{Reply, TestServer};

/// The tag path the upstream release API is asked for.
const TAG_PATH: &str = "/repos/gleam-community/erlang-linux-builds/releases/tags/OTP-29.0.5";

/// A token that is obviously one, so a leak is obvious in a failure too.
const SECRET: &str = "ghp_a_token_that_must_not_be_printed";

/// The URL a caller passes, before any base override is applied.
fn api_url(path: &str) -> String {
    format!("{GITHUB_API_BASE}{path}")
}

/// A network pointed at `server` through the base override, carrying `token`.
fn net_through(server: &TestServer, token: Option<Token>) -> Net {
    Net {
        offline: false,
        base_overrides: BTreeMap::from([(GITHUB_API_BASE.to_owned(), server.base())]),
        token,
    }
}

/// The `authorization` header of the request at `index`, if it carried one.
fn authorization(server: &TestServer, index: usize) -> Option<String> {
    server
        .requests()
        .get(index)
        .expect("the server was asked")
        .headers
        .get("authorization")
        .cloned()
}

#[test]
fn a_rate_limited_refusal_says_so_and_says_what_to_do_about_it() {
    let server = TestServer::one(
        TAG_PATH,
        Reply::headed(
            403,
            &[
                ("x-ratelimit-limit", "60"),
                ("x-ratelimit-remaining", "0"),
                ("x-ratelimit-reset", "1757106000"),
            ],
        ),
    );
    let error = download::get_text(&api_url(TAG_PATH), &net_through(&server, None))
        .expect_err("a 403 is not a document");

    assert!(
        matches!(
            error,
            DownloadError::RateLimited {
                status: 403,
                retry_after: None,
                ..
            }
        ),
        "`x-ratelimit-remaining: 0` is GitHub saying which kind of 403 this is. Reported as a \
         bare status it reads like a repository nobody may see, and the one action that fixes it \
         — waiting, or authenticating — is not in the message: {error:?}"
    );
    let rendered = error.to_string();
    assert!(
        rendered.contains("rate limit"),
        "the message has to name the thing that happened: {rendered}"
    );
    for name in GITHUB_TOKEN_VARS {
        assert!(
            rendered.contains(name),
            "the remedy is a token, and a remedy a user cannot act on is not one: the message \
             names no `{name}`: {rendered}"
        );
    }
}

#[test]
fn a_secondary_limit_reports_the_delay_the_server_asked_for() {
    let server = TestServer::one(TAG_PATH, Reply::headed(429, &[("retry-after", "60")]));
    let error = download::get_text(&api_url(TAG_PATH), &net_through(&server, None))
        .expect_err("a 429 is not a document");

    assert!(
        matches!(
            error,
            DownloadError::RateLimited {
                status: 429,
                retry_after: Some(60),
                ..
            }
        ),
        "a secondary limit names the delay in `retry-after`, and it is the only number that tells \
         a user whether to wait or to go and make a token: {error:?}"
    );
    assert!(
        error.to_string().contains("60 seconds"),
        "the delay the server asked for belongs in the message: {error}"
    );
}

#[test]
fn a_forbidden_answer_that_is_not_a_rate_limit_is_still_a_plain_status() {
    // Both halves of "not a rate limit", because they fail differently. The
    // first is a 403 with no rate-limit headers at all. The second is the one a
    // `headers.contains_key("x-ratelimit-remaining")` would get wrong: every
    // GitHub answer carries that header, and it is the *value* that says
    // whether the budget is gone. A 403 called a rate limit sends a user to
    // wait an hour for something waiting does not fix — a private repository,
    // a token that is wrong, a tag that is not there.
    for reply in [
        Reply::status(403),
        Reply::headed(
            403,
            &[("x-ratelimit-limit", "60"), ("x-ratelimit-remaining", "5")],
        ),
    ] {
        let server = TestServer::one(TAG_PATH, reply);
        let error = download::get_text(&api_url(TAG_PATH), &net_through(&server, None))
            .expect_err("a 403 is not a document");

        assert!(
            matches!(error, DownloadError::Status { status: 403, .. }),
            "this 403 says nothing about an exhausted budget and must be reported as the status \
             it is: {error:?}"
        );
    }
}

#[test]
fn a_retry_after_that_is_not_a_count_of_seconds_is_still_a_rate_limit() {
    // `Retry-After` is defined as a delay in seconds *or* an HTTP date, and a
    // server may send either. The delay is what a message can offer and the
    // date is not, but a header this cannot parse must not turn the answer back
    // into a bare status: the reason for the refusal is the same one.
    let server = TestServer::one(
        TAG_PATH,
        Reply::headed(429, &[("retry-after", "Wed, 05 Sep 2026 21:12:00 GMT")]),
    );
    let error = download::get_text(&api_url(TAG_PATH), &net_through(&server, None))
        .expect_err("a 429 is not a document");

    assert!(
        matches!(
            error,
            DownloadError::RateLimited {
                status: 429,
                retry_after: None,
                ..
            }
        ),
        "the header is there and says what happened; only the number is missing: {error:?}"
    );
}

#[test]
fn the_api_read_carries_the_token_and_the_asset_fetch_never_does() {
    let server = TestServer::one(TAG_PATH, Reply::ok(b"{}"));
    let net = net_through(&server, Token::new(SECRET));
    download::get_text(&api_url(TAG_PATH), &net).expect("the document comes back");

    assert_eq!(
        authorization(&server, 0).as_deref(),
        Some("Bearer ghp_a_token_that_must_not_be_printed"),
        "the API read is the request the rate limit counts, and the token is what moves it off \
         the shared per-address budget"
    );

    let assets = TestServer::one("/asset.tar.zst", Reply::ok(b"runtime"));
    let directory = tempfile::tempdir().expect("a temporary directory");
    download::fetch(
        &api_url("/asset.tar.zst"),
        &directory.path().join("asset.tar.zst"),
        &Expect::anything(),
        &net_through(&assets, Token::new(SECRET)),
    )
    .expect("the asset comes back");

    assert_eq!(
        authorization(&assets, 0),
        None,
        "the asset bytes come from a release download URL that redirects to a storage host, so a \
         credential on that path is handed to a third party for nothing. `fetch` sends none at \
         all — and this URL is under the API base, so the rule cannot be `the base it was \
         rewritten from`; it is the request that is authenticated, not the host"
    );
}

#[test]
fn a_document_read_from_anywhere_but_the_api_carries_no_token() {
    let server = TestServer::one("/catalog.json", Reply::ok(b"{}"));
    let net = Net {
        offline: false,
        base_overrides: BTreeMap::new(),
        token: Token::new(SECRET),
    };
    download::get_text(&server.url("/catalog.json"), &net).expect("the document comes back");

    assert_eq!(
        authorization(&server, 0),
        None,
        "`ginary otp update <url>` reads a catalogue document from wherever it is told to, \
         through the same function. A token attached by the function rather than by the \
         destination is a credential posted to any host a user names"
    );
}

#[test]
fn the_token_is_not_carried_on_to_a_redirect_target() {
    let server = TestServer::start(BTreeMap::from([
        (
            TAG_PATH.to_owned(),
            vec![Reply::headed(302, &[("location", "/moved")])],
        ),
        ("/moved".to_owned(), vec![Reply::ok(b"{}")]),
    ]));
    let net = net_through(&server, Token::new(SECRET));
    download::get_text(&api_url(TAG_PATH), &net).expect("the redirect is followed");

    server.wait_for_requests(2);
    assert!(
        authorization(&server, 0).is_some(),
        "the first request is the one that was authenticated"
    );
    assert_eq!(
        authorization(&server, 1),
        None,
        "a redirect names a destination the server chose, not one this program did. Carrying the \
         credential onward is how a token reaches a host nobody vetted, and the client is \
         configured so that it does not"
    );
}

#[test]
fn a_token_never_appears_in_a_rendering_of_the_value_that_holds_it() {
    let net = Net {
        offline: false,
        base_overrides: BTreeMap::new(),
        token: Token::new(SECRET),
    };
    let rendered = format!("{net:?}");
    assert!(
        !rendered.contains(SECRET),
        "`Net` is `Debug` and travels inside the build context, so a derived `Debug` on the token \
         would put a credential into every trace and panic message that ever prints one: \
         {rendered}"
    );
    assert!(
        rendered.contains("redacted"),
        "a field that is there and hidden has to read as hidden, or the next reader takes the \
         absence for `None`: {rendered}"
    );
}

#[test]
fn an_empty_variable_is_no_token_at_all() {
    for name in GITHUB_TOKEN_VARS {
        let vars = BTreeMap::from([(name.to_owned(), String::new())]);
        assert_eq!(
            Net::from_vars(false, &vars).token,
            None,
            "a workflow that writes `{name}: ${{{{ secrets.SOMETHING }}}}` for a secret it does \
             not have exports the name with nothing in it. An empty `Authorization` header is a \
             worse answer than no header: GitHub refuses it outright, so an absent secret would \
             turn a working anonymous read into a hard failure"
        );
    }
}

#[test]
fn the_variables_are_tried_in_the_order_they_are_written_down() {
    let all = BTreeMap::from([
        (GITHUB_TOKEN_VARS[0].to_owned(), "first".to_owned()),
        (GITHUB_TOKEN_VARS[1].to_owned(), "second".to_owned()),
        (GITHUB_TOKEN_VARS[2].to_owned(), "third".to_owned()),
    ]);
    assert_eq!(
        Net::from_vars(false, &all).token,
        Token::new("first"),
        "`{}` is the one a user sets for ginary alone, so it wins over the two every other tool \
         in the shell also reads",
        GITHUB_TOKEN_VARS[0]
    );

    let two = BTreeMap::from([
        (GITHUB_TOKEN_VARS[1].to_owned(), "second".to_owned()),
        (GITHUB_TOKEN_VARS[2].to_owned(), "third".to_owned()),
    ]);
    assert_eq!(
        Net::from_vars(false, &two).token,
        Token::new("second"),
        "`{}` before `{}` is the `gh` command line's own precedence, and a shell that has both \
         set them for that reason",
        GITHUB_TOKEN_VARS[1],
        GITHUB_TOKEN_VARS[2]
    );
}
