// SPDX-License-Identifier: MIT OR Apache-2.0
//! Every run of a built artifact was given `HOME` and `XDG_CACHE_HOME` and
//! nothing else, so on Windows every run in the binary shared one cache
//! directory and they tore each other's extractions apart.
//!
//! **What went wrong.** `common::built::ArtifactRun` clears the environment —
//! which is right, a test that inherited the developer's would assert about
//! the machine — and then sets the two unix variables `ginary::cache::resolve`
//! reads. `ginary::cache::resolve_windows` reads neither. With
//! `%GINARY_CACHE_DIR%`, `%LOCALAPPDATA%`, `%TEMP%`, `%TMP%` and `%USERNAME%`
//! all unset, every run of every test in `tests/e2e_hello.rs` fell all the way
//! through to the machine directory under one name:
//!
//! ```text
//! ---- the_built_artifact_propagates_a_zero_exit_code ----
//! ginary: the runtime cache at \\?\C:\Windows\Temp\ginary-unknown\hello_ffi\
//!   .2de5b61cdc2072bd.tmp-7340\lib\kernel-11.0.3\ebin\disk_log_server.beam
//!   is unusable: The system cannot find the file specified. (os error 2)
//!
//! ---- the_built_artifact_runs_the_application_with_no_erlang_on_the_machine ----
//! ginary: reading or writing the payload failed: failed to create
//!   `\\?\C:\Windows\Temp\ginary-unknown\hello_ffi\.6010aac570d5b8a6.tmp-1812\erts-17.0.5\bin`:
//!   The system cannot find the file specified. (os error 2) while canonicalizing …
//!
//! ---- the_second_run_of_one_artifact_hits_the_cache_it_wrote ----
//! a warm run must not write a second entry
//!   left: 0
//!  right: 1
//! ```
//!
//! (`Windows build and exit-code propagation`
//! <https://github.com/P4suta/ginary/actions/runs/33751715516/job/100636537290>:
//! `tests/e2e_hello.rs:101`, `:132`, `:333`, `:469` and `:817`.) A different
//! `.beam` is missing in each run and the tar reader loses the directory it is
//! unpacking into: that is several processes extracting into and sweeping one
//! shared application directory at once. `ArtifactRun::app_dir` then looked for
//! the entry under `<home>/ginary`, where nothing had been written at all.
//!
//! **The input.** Any host whose cache root is not resolved from `HOME` or
//! `XDG_CACHE_HOME`.
//!
//! **The correct behaviour.** Which variables pin a cache root is a fact about
//! the platform, so the run sets the platform's own — and the helper proves it
//! by running `ginary::cache`'s resolver over the pairs it sets and refusing a
//! root the resolver calls a fallback. `GINARY_CACHE_DIR` is deliberately not
//! the answer: a run that used the override would prove the override works
//! rather than that the ordinary rule lands inside the directory the run owns.

use std::ffi::OsString;

use crate::common::built::{isolated_cache_root, isolating_cache_env};
use ginary::cache;
use ginary::target::Os;

/// The pairs, as an `Env` the resolvers take.
fn env_for(os: Os, home: &std::path::Path) -> cache::Env {
    cache::Env::from_pairs(
        isolating_cache_env(os, home)
            .into_iter()
            .map(|(key, value)| (OsString::from(key), OsString::from(value))),
    )
}

#[test]
fn each_platform_is_given_the_variables_its_own_resolver_reads() {
    let home = std::path::Path::new("/runs/cold-home");

    let unix: Vec<&str> = isolating_cache_env(Os::Linux, home)
        .into_iter()
        .map(|(key, _)| key)
        .collect();
    assert_eq!(
        unix,
        [cache::HOME_VAR, cache::XDG_CACHE_HOME_VAR],
        "the two variables `cache::resolve` reads, in the order it reads them"
    );

    let windows: Vec<&str> = isolating_cache_env(Os::Windows, home)
        .into_iter()
        .map(|(key, _)| key)
        .collect();
    assert_eq!(
        windows,
        [cache::LOCALAPPDATA_VAR, cache::USERNAME_VAR],
        "and the ones `cache::resolve_windows` reads; `HOME` and `XDG_CACHE_HOME` mean nothing \
         there"
    );
}

#[test]
fn the_override_is_not_how_a_run_is_isolated() {
    for os in [Os::Linux, Os::Macos, Os::Windows] {
        let home = std::path::Path::new("/runs/cold-home");
        assert!(
            !isolating_cache_env(os, home)
                .iter()
                .any(|(key, _)| *key == cache::GINARY_CACHE_DIR_VAR),
            "{os:?}: a run that set the override would prove the override works, not that \
             the platform's own rule lands under the directory the run owns"
        );
    }
}

#[test]
fn the_pairs_pin_a_root_rather_than_falling_through_to_a_shared_one() {
    let home = std::path::Path::new("/runs/cold-home");

    let unix = cache::resolve(&env_for(Os::Linux, home), 1000);
    assert!(
        !unix.is_fallback,
        "the unix pairs have to be read as a choice, not a fallback: {:?}",
        unix.origin
    );
    assert!(unix.root.starts_with(home), "{}", unix.root.display());

    let windows = cache::resolve_windows(&env_for(Os::Windows, home), "tester");
    assert!(
        !windows.is_fallback,
        "and so do the Windows ones, or every run shares `%TEMP%\\ginary-<user>`: {:?}",
        windows.origin
    );
    assert!(windows.root.starts_with(home), "{}", windows.root.display());
}

#[test]
fn the_root_a_test_looks_in_is_the_root_the_platform_resolved() {
    // `ArtifactRun::app_dir` is what a test reads back, and it looked under
    // `<home>/ginary` on every platform. It has to be whatever the resolver
    // produced from the pairs the run actually set.
    let home = std::path::Path::new("/runs/cold-home");
    for os in [Os::Linux, Os::Macos, Os::Windows] {
        let expected = match os {
            Os::Windows => cache::resolve_windows(&env_for(os, home), "tester").root,
            Os::Linux | Os::Macos => cache::resolve(&env_for(os, home), 1000).root,
        };
        assert_eq!(isolated_cache_root(os, home), expected, "{os:?}");
    }
}
