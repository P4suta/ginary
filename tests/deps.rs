// SPDX-License-Identifier: MIT OR Apache-2.0
//! The dependency record, held against the freshness gate that reads it.
//!
//! The development machine runs a `pre-push` hook that refuses a push leaving
//! a dependency the push itself touched on a stale release. That gate is not
//! something the suite can run — it starts a container and talks to the
//! network — but the conclusion it reached about this tree is a fact about two
//! committed files, and that part is checkable offline and permanently.
//!
//! What it reported was `sha2` sitting at 0.10.9 with 0.11.0 released. 0.11 is
//! a real major of the RustCrypto stack: `digest` goes to 0.11 with it, the
//! `Digest` trait moves, and `Output<Sha256>` replaces the `GenericArray` the
//! 0.10 line returned. The bump is therefore worth a test of its own rather
//! than a line in a changelog, and it is worth a second one on the lockfile:
//! two `digest` majors resolve perfectly happily and give the crate two
//! incompatible `Digest` traits, which is the failure mode a half-finished
//! migration produces.
//!
//! Ungated: a manifest is neither half of the crate.

mod common;

use crate::common::deps::{Version, dependency_requirement, locked_versions};

/// The first release of the RustCrypto stack E4 moves onto.
const SHA2_FLOOR: Version = Version {
    major: 0,
    minor: 11,
    patch: 0,
};

/// The packages of the RustCrypto stack a `sha2` bump moves, each of which
/// must resolve to exactly one version.
const CRYPTO_STACK: [&str; 3] = ["sha2", "digest", "block-buffer"];

#[test]
fn cargo_toml_asks_for_the_sha2_0_11_line() {
    let requirement =
        dependency_requirement("sha2").expect("`Cargo.toml` states a version requirement for sha2");
    let parsed = Version::parse(&requirement)
        .unwrap_or_else(|| panic!("`sha2 = \"{requirement}\"` is not a version requirement"));
    assert!(
        parsed >= SHA2_FLOOR,
        "the freshness gate reports sha2 {requirement} as stale: 0.11 is released and this is \
         the crate's hashing library, so the requirement has to be on the 0.11 line (found \
         {parsed}, wanted at least {SHA2_FLOOR})"
    );
}

#[test]
fn the_lockfile_resolves_sha2_onto_the_same_line_cargo_toml_asks_for() {
    let locked = locked_versions("sha2");
    assert_eq!(
        locked.len(),
        1,
        "`Cargo.lock` resolves {} versions of sha2 ({}); two hashing libraries in one graph is \
         two answers to the same digest",
        locked.len(),
        locked.join(", ")
    );
    let resolved = Version::parse(&locked[0])
        .unwrap_or_else(|| panic!("`Cargo.lock` sha2 version `{}` does not parse", locked[0]));
    assert!(
        resolved >= SHA2_FLOOR,
        "`Cargo.lock` still pins sha2 {resolved}; the committed lockfile is what CI builds with, \
         so a manifest bump nobody locked changes nothing"
    );
}

#[test]
fn the_crypto_stack_resolves_to_one_version_of_each_package() {
    let mut duplicated: Vec<String> = Vec::new();
    let mut missing: Vec<&str> = Vec::new();
    for package in CRYPTO_STACK {
        let locked = locked_versions(package);
        match locked.len() {
            0 => missing.push(package),
            1 => {}
            _ => duplicated.push(format!("{package}: {}", locked.join(", "))),
        }
    }
    assert!(
        missing.is_empty(),
        "`Cargo.lock` holds no entry for {}; the sha2 bump has to leave the whole stack \
         resolvable, not drop half of it",
        missing.join(", ")
    );
    assert!(
        duplicated.is_empty(),
        "the sha2 0.11 bump must not leave two majors of the RustCrypto stack in the graph — a \
         second `digest` is a second `Digest` trait, and a call site can compile against either:\
         \n{}",
        duplicated.join("\n")
    );
}

#[test]
fn every_digest_producing_dependency_is_named_in_one_place() {
    // `hex` spells every digest this crate writes and `sha2` computes it; the
    // two travel together, and a bump of one that leaves the other behind is
    // how a format change hides. Both are plain, non-optional dependencies, so
    // the launcher-only build carries them too — there is no feature to read
    // this behind.
    for package in ["sha2", "hex"] {
        let requirement = dependency_requirement(package)
            .unwrap_or_else(|| panic!("`Cargo.toml` states no requirement for {package}"));
        let parsed = Version::parse(&requirement).unwrap_or_else(|| {
            panic!("`{package} = \"{requirement}\"` is not a version requirement")
        });
        let locked = locked_versions(package);
        assert_eq!(
            locked.len(),
            1,
            "{package} resolves to {} versions ({})",
            locked.len(),
            locked.join(", ")
        );
        let resolved = Version::parse(&locked[0])
            .unwrap_or_else(|| panic!("{package} lock version `{}` does not parse", locked[0]));
        assert_eq!(
            (resolved.major, resolved.minor),
            (parsed.major, parsed.minor),
            "{package} is required as `{requirement}` and locked at `{}`: the lockfile is on a \
             different minor line from the manifest, so `cargo build --locked` in CI and a plain \
             `cargo build` here are not building the same code",
            locked[0]
        );
    }
}
