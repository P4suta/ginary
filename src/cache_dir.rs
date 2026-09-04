// SPDX-License-Identifier: MIT OR Apache-2.0
//! Resolution of the ginary cache root.
//!
//! Bundled applications extract their runtime under this directory, so the
//! rules must be identical in the builder and in the launcher — and they are
//! the same code: [`resolve`] is a projection of [`crate::cache::resolve`],
//! which is where the precedence actually lives. This module is the build
//! side's view of it, over the three variables `ginary doctor` reports.
//!
//! The one deliberate difference is the ending. The launcher falls back to
//! `${TMPDIR:-/tmp}/ginary-<uid>` when nothing is set, because a packaged
//! application has to start anyway; the builder answers
//! [`CacheDirError::Unresolved`], because a tool that silently wrote its
//! output into `/tmp` would be a tool nobody could find the output of.
//!
//! The precedence on unix is:
//!
//! 1. `GINARY_CACHE_DIR` — used verbatim, the escape hatch for read-only or
//!    `noexec` home directories;
//! 2. `XDG_CACHE_HOME` — must be an absolute path, `ginary` is appended;
//! 3. `HOME` — `.cache/ginary` is appended.
//!
//! On Windows it is `GINARY_CACHE_DIR`, then `%LOCALAPPDATA%\ginary`, which is
//! [`crate::cache::resolve_windows`]'s precedence and therefore the directory a
//! packaged application on that machine already uses. A machine that sets none
//! of them is [`CacheDirError::Unresolved`] on either platform. Which of the
//! two rules applies is [`crate::platform::has_local_app_data`], asked of a
//! named [`crate::target::Os`] rather than of a `#[cfg]`, so both answers are
//! unit tested wherever ginary is built.
//!
//! `~/Library/Caches` on macOS is still a later milestone: macOS follows the
//! unix rules above, as it does in the launcher.

use std::ffi::OsString;
use std::path::PathBuf;

use crate::platform;
use crate::target::Os;

/// The environment variables cache resolution reads.
///
/// Constructing this explicitly keeps [`resolve`] pure and testable.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EnvSnapshot {
    /// Value of `GINARY_CACHE_DIR`.
    pub ginary_cache_dir: Option<OsString>,
    /// Value of `XDG_CACHE_HOME`.
    pub xdg_cache_home: Option<OsString>,
    /// Value of `HOME`.
    pub home: Option<OsString>,
    /// Value of `LOCALAPPDATA`, read only on Windows.
    pub local_app_data: Option<OsString>,
}

impl EnvSnapshot {
    /// Reads the relevant variables from the current process environment.
    pub fn from_env() -> Self {
        Self {
            ginary_cache_dir: std::env::var_os("GINARY_CACHE_DIR"),
            xdg_cache_home: std::env::var_os("XDG_CACHE_HOME"),
            home: std::env::var_os("HOME"),
            local_app_data: std::env::var_os(crate::cache::LOCALAPPDATA_VAR),
        }
    }
}

/// Which environment variable produced a resolved cache directory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// `GINARY_CACHE_DIR` was set.
    GinaryCacheDir,
    /// `XDG_CACHE_HOME` was set to an absolute path.
    XdgCacheHome,
    /// `HOME` was set.
    Home,
    /// `LOCALAPPDATA` was set, on Windows.
    LocalAppData,
}

impl Source {
    /// Returns the name of the environment variable this source reads.
    pub const fn variable(self) -> &'static str {
        match self {
            Self::GinaryCacheDir => "GINARY_CACHE_DIR",
            Self::XdgCacheHome => "XDG_CACHE_HOME",
            Self::Home => "HOME",
            Self::LocalAppData => crate::cache::LOCALAPPDATA_VAR,
        }
    }
}

/// A resolved cache root and the variable it came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CacheDir {
    /// The cache root. It is not created or probed by [`resolve`].
    pub path: PathBuf,
    /// The variable that produced [`CacheDir::path`].
    pub source: Source,
}

/// Failure to resolve a cache root.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CacheDirError {
    /// No usable variable was set.
    #[error(
        "cannot locate a cache directory: set GINARY_CACHE_DIR, XDG_CACHE_HOME (absolute) or HOME"
    )]
    Unresolved,
}

/// Resolves the cache root from an environment snapshot.
///
/// Empty values count as unset, because an exported-but-empty variable is a
/// common shell accident and an empty path would silently mean the current
/// working directory. A relative `XDG_CACHE_HOME` is ignored, as the XDG base
/// directory specification requires.
pub fn resolve(env: &EnvSnapshot, os: Os) -> Result<CacheDir, CacheDirError> {
    // One implementation of the precedence, in `cache::resolve` and
    // `cache::resolve_windows`. This is their build-side projection: the same
    // rules, dispatched on the same platform question the launcher asks, and
    // an error where the launcher has a fallback, because a build tool that
    // silently wrote its output into a temporary directory would be a build
    // tool nobody could find the output of.
    let mut pairs: Vec<(OsString, OsString)> = Vec::new();
    for (name, value) in [
        (crate::cache::GINARY_CACHE_DIR_VAR, &env.ginary_cache_dir),
        (crate::cache::XDG_CACHE_HOME_VAR, &env.xdg_cache_home),
        (crate::cache::HOME_VAR, &env.home),
        (crate::cache::LOCALAPPDATA_VAR, &env.local_app_data),
    ] {
        if let Some(value) = value {
            pairs.push((OsString::from(name), value.clone()));
        }
    }
    let snapshot = crate::cache::Env::from_pairs(pairs);

    // The uid and the user name only ever reach a fallback root, and both
    // fallbacks are exactly the case this function refuses, so neither value
    // can be observed in an `Ok`.
    let resolved = if platform::has_local_app_data(os) {
        crate::cache::resolve_windows(&snapshot, crate::cache::UNKNOWN_USER)
    } else {
        crate::cache::resolve(&snapshot, 0)
    };
    let source = match resolved.origin {
        crate::cache::Origin::GinaryCacheDir => Source::GinaryCacheDir,
        crate::cache::Origin::XdgCacheHome => Source::XdgCacheHome,
        crate::cache::Origin::Home => Source::Home,
        crate::cache::Origin::LocalAppData => Source::LocalAppData,
        crate::cache::Origin::Fallback | crate::cache::Origin::WindowsFallback => {
            return Err(CacheDirError::Unresolved);
        }
    };
    Ok(CacheDir {
        path: resolved.root,
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// [`resolve`] for a host that reads `GINARY_CACHE_DIR`/`XDG_CACHE_HOME`/
    /// `HOME` — every rule below is one of that unix precedence, so it is
    /// pinned to [`Os::Linux`] rather than to `platform::HOST`. On Linux the two
    /// are the same; on Windows `platform::HOST` selects the `%LOCALAPPDATA%`
    /// resolver instead (that dispatch is [`crate::platform::has_local_app_data`]'s
    /// own unit test, and the Windows base its own regression), so asking these
    /// unix-variable rules of it would answer `Unresolved` and prove nothing.
    fn resolve_host(env: &EnvSnapshot) -> Result<CacheDir, CacheDirError> {
        resolve(env, crate::target::Os::Linux)
    }

    fn snapshot(ginary: Option<&str>, xdg: Option<&str>, home: Option<&str>) -> EnvSnapshot {
        EnvSnapshot {
            ginary_cache_dir: ginary.map(OsString::from),
            xdg_cache_home: xdg.map(OsString::from),
            home: home.map(OsString::from),
            local_app_data: None,
        }
    }

    #[test]
    fn ginary_cache_dir_wins_and_is_used_verbatim() {
        let resolved = resolve_host(&snapshot(
            Some("/srv/ginary-cache"),
            Some("/xdg"),
            Some("/home/u"),
        ))
        .expect("resolves");
        assert_eq!(resolved.path, PathBuf::from("/srv/ginary-cache"));
        assert_eq!(resolved.source, Source::GinaryCacheDir);
    }

    #[test]
    fn xdg_cache_home_gets_a_ginary_component() {
        let resolved =
            resolve_host(&snapshot(None, Some("/xdg"), Some("/home/u"))).expect("resolves");
        assert_eq!(resolved.path, PathBuf::from("/xdg/ginary"));
        assert_eq!(resolved.source, Source::XdgCacheHome);
    }

    #[test]
    fn home_gets_dot_cache_ginary() {
        let resolved = resolve_host(&snapshot(None, None, Some("/home/u"))).expect("resolves");
        assert_eq!(resolved.path, PathBuf::from("/home/u/.cache/ginary"));
        assert_eq!(resolved.source, Source::Home);
    }

    #[test]
    fn empty_values_count_as_unset() {
        let resolved =
            resolve_host(&snapshot(Some(""), Some(""), Some("/home/u"))).expect("resolves");
        assert_eq!(resolved.path, PathBuf::from("/home/u/.cache/ginary"));
        assert_eq!(resolved.source, Source::Home);
    }

    #[test]
    fn a_relative_xdg_cache_home_is_ignored() {
        let resolved = resolve_host(&snapshot(None, Some("relative/cache"), Some("/home/u")))
            .expect("resolves");
        assert_eq!(resolved.path, PathBuf::from("/home/u/.cache/ginary"));
        assert_eq!(resolved.source, Source::Home);
    }

    #[test]
    fn a_relative_ginary_cache_dir_is_still_honoured() {
        // Unlike XDG_CACHE_HOME this is an explicit ginary override, so the user
        // gets exactly what they asked for.
        let resolved = resolve_host(&snapshot(Some("cache"), None, None)).expect("resolves");
        assert_eq!(resolved.path, PathBuf::from("cache"));
        assert_eq!(resolved.source, Source::GinaryCacheDir);
    }

    #[test]
    fn nothing_set_is_an_error() {
        assert_eq!(
            resolve_host(&snapshot(None, None, None)),
            Err(CacheDirError::Unresolved)
        );
        assert_eq!(
            resolve_host(&snapshot(None, Some("relative"), Some(""))),
            Err(CacheDirError::Unresolved)
        );
    }

    #[test]
    fn every_source_names_its_variable() {
        assert_eq!(Source::GinaryCacheDir.variable(), "GINARY_CACHE_DIR");
        assert_eq!(Source::XdgCacheHome.variable(), "XDG_CACHE_HOME");
        assert_eq!(Source::Home.variable(), "HOME");
        assert_eq!(Source::LocalAppData.variable(), "LOCALAPPDATA");
    }
}
