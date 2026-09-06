// SPDX-License-Identifier: MIT OR Apache-2.0
//! The token was sent to any URL whose *text* began with the API base, which
//! includes hosts that are not the API at all.
//!
//! **What went wrong.** E22 decided where a credential may go with
//!
//! ```rust
//! let authorization = net.token.as_ref().filter(|_| url.starts_with(GITHUB_API_BASE));
//! ```
//!
//! and `GITHUB_API_BASE` is `https://api.github.com` with no trailing
//! delimiter. A prefix of the text is not an origin. `https://api.github.com`
//! is a prefix of
//!
//! ```text
//! https://api.github.com.evil.test/repos/x/y/releases/tags/OTP-29.0.5
//! https://api.github.com@evil.test/repos/x/y/releases/tags/OTP-29.0.5
//! ```
//!
//! and of `https://api.github.com:8443/…`. The first is a host somebody else
//! registered under a name that begins with ours; the second is a userinfo
//! field, where everything before the `@` is a *username* and `evil.test` is
//! the host — the oldest phishing URL there is. Either one was handed the
//! user's GitHub token.
//!
//! It was reachable: `ginary otp update <url>` reads a catalogue document from
//! wherever it is told to, through the same [`download::get_text`]. The rule
//! the milestone wrote down — "to the API base and to nothing else" — is
//! exactly the rule this did not implement, and the test that was meant to
//! cover it used a loopback URL that shares no prefix with the base, so it
//! never approached the boundary.
//!
//! **The input.** Any URL beginning with the nineteen characters of the API
//! base and continuing with anything that is not a path, a query or a fragment.
//!
//! **The correct behaviour.** The base has to be the whole origin: the URL is
//! the base exactly, or the base followed by `/`, `?` or `#`. A different
//! host, a userinfo prefix and a different port are all a different origin and
//! get no token. The check runs on the URL as given, before
//! `GINARY_GITHUB_BASE_URL` rewrites it, so it stays a question about which
//! service is being read rather than about which machine answers.

use std::collections::BTreeMap;

use ginary::download::{self, GITHUB_API_BASE, Net, Token};

use crate::common::http::{Reply, TestServer};

/// The path every request in this file asks for.
const PATH: &str = "/repos/x/y/releases/tags/OTP-29.0.5";

/// A token that is obviously one, so a leak is obvious in a failure too.
const SECRET: &str = "ghp_a_token_that_must_not_be_handed_to_a_stranger";

/// Reads `base + PATH` with a token, through an override that sends the request
/// at `server` rather than at whatever host the URL names, and answers with the
/// `authorization` the server was given.
///
/// The override is what makes the hostile spellings testable at all: the
/// decision is made on the URL as given, and the bytes go to loopback, so no
/// test in this file resolves a name or opens a socket to anywhere.
fn authorization_sent_to(base: &str) -> Option<String> {
    let server = TestServer::one(PATH, Reply::ok(b"{}"));
    let net = Net {
        offline: false,
        base_overrides: BTreeMap::from([(base.to_owned(), server.base())]),
        token: Token::new(SECRET),
    };
    download::get_text(&format!("{base}{PATH}"), &net).expect("the fixture answers");
    server
        .requests()
        .first()
        .expect("the server was asked")
        .headers
        .get("authorization")
        .cloned()
}

#[test]
fn a_host_whose_name_merely_begins_with_the_api_base_gets_no_token() {
    assert_eq!(
        authorization_sent_to("https://api.github.com.evil.test"),
        None,
        "`api.github.com.evil.test` is a host somebody else registered, and the only thing it \
         shares with the API is the first nineteen characters of its URL. A prefix of the text \
         is not an origin"
    );
}

#[test]
fn a_userinfo_field_ending_in_the_api_base_gets_no_token() {
    assert_eq!(
        authorization_sent_to("https://api.github.com@evil.test"),
        None,
        "everything before the `@` is a username: the host here is `evil.test`. This is the \
         oldest phishing URL there is, and it defeats a prefix check by construction"
    );
}

#[test]
fn a_different_port_on_the_api_host_gets_no_token() {
    assert_eq!(
        authorization_sent_to("https://api.github.com:8443"),
        None,
        "a port is part of an origin, and nothing answers the GitHub API on 8443. Refusing it \
         costs nothing and keeps the rule stated over the whole origin rather than over a host"
    );
}

#[test]
fn the_api_itself_still_carries_the_token() {
    assert_eq!(
        authorization_sent_to(GITHUB_API_BASE),
        Some(format!("Bearer {SECRET}")),
        "the request this whole change exists for — the one the rate limit counts — still has to \
         be authenticated. A boundary check that also refuses the API is not a fix"
    );
}
