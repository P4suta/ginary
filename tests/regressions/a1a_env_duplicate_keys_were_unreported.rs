// SPDX-License-Identifier: MIT OR Apache-2.0
//! A duplicated `env` key was reported twice with no warning.
//!
//! **What went wrong.** `AppResource` de-duplicated the eight properties it
//! reads and recorded the shadowing in `warnings`, but the keys inside `env`
//! went through a loop with no such rule: `{env, [{a, 1}, {a, 2}]}` produced
//! `env_keys == ["a", "a"]` and an empty `warnings`, so a reader could not tell
//! that one of the two values was dead.
//!
//! **The input.** An `.app` file whose `env` list names the same key twice.
//!
//! **The correct behaviour.** The first appearance keeps its place, the repeat
//! is dropped, and the shadowing is recorded in `warnings`, exactly as a
//! duplicated top-level property is.

use ginary::appfile::{AppResource, parse_terms};

/// The resource an `.app` source describes, which must be readable.
fn resource(source: &str) -> AppResource {
    let terms = match parse_terms(source) {
        Ok(terms) => terms,
        Err(error) => panic!("the source should parse, but: {error}"),
    };
    match AppResource::try_from(terms.as_slice()) {
        Ok(resource) => resource,
        Err(error) => panic!("the terms should describe an application, but: {error}"),
    }
}

#[test]
fn a_duplicate_env_key_is_listed_once_and_warned_about() {
    let resource = resource(
        "{application, dup, [{vsn, \"1.0.0\"},\n\
         {env, [{a, 1}, {b, 2}, {a, 3}]}]}.\n",
    );

    assert_eq!(resource.env_keys, ["a", "b"]);
    assert!(
        resource
            .warnings
            .iter()
            .any(|warning| warning.contains("duplicate `env` key `a`")),
        "the shadowed value must be reported: {:?}",
        resource.warnings
    );
}

#[test]
fn distinct_env_keys_produce_no_warning() {
    let resource = resource("{application, dup, [{vsn, \"1.0.0\"}, {env, [{a, 1}, {b, 2}]}]}.\n");
    assert_eq!(resource.env_keys, ["a", "b"]);
    assert!(resource.warnings.is_empty(), "{:?}", resource.warnings);
}
