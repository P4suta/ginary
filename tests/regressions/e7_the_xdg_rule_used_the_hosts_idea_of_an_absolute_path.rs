// SPDX-License-Identifier: MIT OR Apache-2.0
//! The unix cache resolver asked `std::path::Path` whether `XDG_CACHE_HOME`
//! was absolute, and `Path` answers for the platform the code was compiled
//! for.
//!
//! **What went wrong.** [`ginary::cache::resolve`] implements the XDG base
//! directory specification's three-step precedence, and the specification
//! requires a *relative* `XDG_CACHE_HOME` to be ignored. The check was
//! `Path::new(value).is_absolute()`. On Windows that wants a drive letter or a
//! UNC prefix, so `/xdg` is relative, the branch is skipped, and the resolver
//! falls through to `HOME` — silently, with no error and no trace. The first
//! Windows runner reported it twice, once in each resolver:
//!
//! ```text
//! ---- cache::tests::xdg_cache_home_gets_a_ginary_component stdout ----
//! thread '...' panicked at src\cache.rs:1834:9:
//! assertion `left == right` failed
//!   left: "/home/u\\.cache\\ginary"
//!  right: "/xdg/ginary"
//! ```
//!
//! (`Windows build and exit-code propagation`
//! <https://github.com/P4suta/ginary/actions/runs/33702776627/job/100485421577>;
//! `cache_dir::tests::xdg_cache_home_gets_a_ginary_component` is the same
//! failure one layer up, because `cache_dir::resolve` is a projection of this
//! one.)
//!
//! **The input.** A Windows host with `XDG_CACHE_HOME` set to a POSIX path.
//! That is not a hypothetical: `cache::resolve` is the *unix* half of the
//! resolver, compiled on every platform because its unit tests are, and
//! `resolve_windows` is what a Windows launcher actually calls — so what the
//! defect really costs is a rule that cannot be tested where it is compiled.
//! It cannot be seen from Linux at all, where `Path::is_absolute` and the
//! specification agree on every input.
//!
//! **The correct behaviour.** A function that implements a POSIX
//! specification decides what "absolute" means by that specification: a
//! leading `/`. [`ginary::cache::xdg_base_is_absolute`] is that decision,
//! pure, and it answers the same on every host — which is the whole point, and
//! is why it can be pinned here from Linux.
#![cfg(feature = "cli")]

use std::ffi::OsStr;

use ginary::cache::xdg_base_is_absolute;

use crate::common::repo::read;
use crate::common::srcscan::literal_sites;

#[test]
fn the_xdg_base_rule_is_the_specifications_leading_slash_on_every_host() {
    for absolute in ["/xdg", "/xdg/", "/", "//xdg", "/xdg/cache", "/C:/xdg"] {
        assert!(
            xdg_base_is_absolute(OsStr::new(absolute)),
            "the specification's rule is a leading `/`, whatever platform this is compiled for: \
             {absolute:?}"
        );
    }
    for relative in ["", "xdg", "xdg/cache", ".", "./xdg", "~/xdg"] {
        assert!(
            !xdg_base_is_absolute(OsStr::new(relative)),
            "a relative XDG_CACHE_HOME is ignored, and an empty value names nothing: {relative:?}"
        );
    }
    for windows in ["C:\\xdg", "C:/xdg", "\\\\?\\C:\\xdg", "\\\\host\\share"] {
        assert!(
            !xdg_base_is_absolute(OsStr::new(windows)),
            "a Windows path is not a POSIX absolute path, and this is the POSIX resolver — the \
             Windows one is `cache::resolve_windows`: {windows:?}"
        );
    }
}

#[test]
fn the_unix_resolver_no_longer_asks_the_host_what_absolute_means() {
    // The instrument: the call is found in code and ignored in the prose that
    // has to be able to name what was wrong.
    let planted = "\
// `Path::is_absolute()` answers for the platform this was compiled for\n\
        if base.is_absolute() {\n\
        if xdg_base_is_absolute(value) {\n";
    assert_eq!(
        literal_sites(planted, ".is_absolute()"),
        vec![2],
        "a comment may describe the rule; the resolver may not use it"
    );

    assert_eq!(
        literal_sites(&read("src/cache.rs"), ".is_absolute()"),
        Vec::<usize>::new(),
        "`cache::resolve` implements a POSIX specification and must decide what absolute means \
         by that specification, not by the host it was compiled for"
    );
}
