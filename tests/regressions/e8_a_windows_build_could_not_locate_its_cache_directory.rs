// SPDX-License-Identifier: MIT OR Apache-2.0
//! Every `ginary build` on Windows failed before it started, because the
//! build side's cache resolution knew only the three unix variables.
//!
//! **What went wrong.** The Windows runner refused builds with a cache error
//! where the test expected the honest one about a missing stub:
//!
//! ```text
//! expected BundleError::Stub for linux-x86_64-gnu, got CacheDir(Unresolved)
//! ```
//!
//! ```text
//! error: cannot bundle a runtime for windows-x86_64 from `host`
//! ```
//!
//! (`Windows build and exit-code propagation`
//! <https://github.com/P4suta/ginary/actions/runs/33712111530/job/100513644404>.)
//!
//! `cache_dir::resolve` — the *build* side's projection of the launcher's
//! precedence — read `GINARY_CACHE_DIR`, `XDG_CACHE_HOME` and `HOME` and
//! nothing else, and answered [`ginary::cache_dir::CacheDirError::Unresolved`]
//! for anything the unix resolver could not place. Windows sets none of those
//! three: it spells the per-user cache `%LOCALAPPDATA%`. So on a Windows
//! machine with a perfectly ordinary environment, `ginary build` could not
//! locate a cache directory at all, and every failure downstream of it was
//! reported as that rather than as itself.
//!
//! The launcher half already knew: [`ginary::cache::resolve_windows`] has
//! answered `%LOCALAPPDATA%\ginary` since the Windows launcher landed. Only
//! the build side's projection of it was missing, and its own comment said so
//! — "a stub cache under `%LOCALAPPDATA%` is a decision nothing has needed
//! yet". A runner needed it.
//!
//! **The correct behaviour.** The projection dispatches on a named [`Os`] the
//! same way the launcher does, so both answers are asserted here rather than
//! only on a runner. The one deliberate difference from the launcher survives
//! on both platforms: a build refuses the *fallback* root — `${TMPDIR}` on
//! unix, `%TEMP%` on Windows — because a build tool that silently wrote its
//! output into a temporary directory is a build tool nobody can find the
//! output of.

use std::ffi::OsString;
use std::path::Path;

use ginary::cache_dir::{self, CacheDirError, EnvSnapshot, Source};
use ginary::target::Os;

/// `%LOCALAPPDATA%` as the runner reported it.
const LOCAL_APP_DATA: &str = r"C:\Users\runneradmin\AppData\Local";

/// A snapshot with only `LOCALAPPDATA` set, which is the ordinary Windows
/// environment: `HOME` and `XDG_CACHE_HOME` are unix conventions no Windows
/// shell exports.
fn windows_env() -> EnvSnapshot {
    EnvSnapshot {
        local_app_data: Some(OsString::from(LOCAL_APP_DATA)),
        ..EnvSnapshot::default()
    }
}

#[test]
fn a_windows_host_resolves_its_cache_under_local_app_data() {
    let resolved = cache_dir::resolve(&windows_env(), Os::Windows)
        .expect("an ordinary Windows environment locates a cache directory");

    // The base and the component are the rule; the separator between them is
    // `std::path`'s business and differs with the machine the test runs on,
    // which is why the expectation is built the same way `tests/windows.rs`
    // builds the launcher's.
    assert_eq!(
        (resolved.path, resolved.source),
        (
            Path::new(LOCAL_APP_DATA).join(ginary::cache::DIR_NAME),
            Source::LocalAppData
        ),
        "the build side places the cache where the launcher already does",
    );
}

#[test]
fn local_app_data_is_not_read_on_a_platform_that_does_not_have_it() {
    // The rule is dispatched on, not merely appended to the list: a unix host
    // that happens to export `LOCALAPPDATA` (a Wine session, a shell script
    // that copied an environment) still follows the unix precedence, and with
    // nothing else set it still refuses.
    for os in [Os::Linux, Os::Macos] {
        assert_eq!(
            cache_dir::resolve(&windows_env(), os),
            Err(CacheDirError::Unresolved),
            "{os} has no %LOCALAPPDATA%",
        );
    }
}

#[test]
fn a_windows_host_with_nothing_set_is_still_refused_rather_than_sent_to_temp() {
    // The launcher falls back to `%TEMP%\ginary-<user>` because a packaged
    // application has to start anyway. A build must not: this is the one place
    // the projection deliberately differs from `cache::resolve_windows`, and
    // it has to keep differing on Windows too.
    assert_eq!(
        cache_dir::resolve(&EnvSnapshot::default(), Os::Windows),
        Err(CacheDirError::Unresolved),
        "a build tool that wrote into %TEMP% is one nobody can find the output of",
    );
}

#[test]
fn ginary_cache_dir_still_wins_on_every_platform() {
    for os in [Os::Linux, Os::Macos, Os::Windows] {
        let env = EnvSnapshot {
            ginary_cache_dir: Some(OsString::from("/srv/ginary-cache")),
            ..windows_env()
        };
        let resolved = cache_dir::resolve(&env, os).expect("the override always resolves");
        assert_eq!(
            (resolved.path, resolved.source),
            (
                std::path::PathBuf::from("/srv/ginary-cache"),
                Source::GinaryCacheDir
            ),
            "{os}: the escape hatch is the first rule of both resolvers",
        );
    }
}
