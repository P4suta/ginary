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
//! The precedence is:
//!
//! 1. `GINARY_CACHE_DIR` — used verbatim, the escape hatch for read-only or
//!    `noexec` home directories;
//! 2. `XDG_CACHE_HOME` — must be an absolute path, `ginary` is appended;
//! 3. `HOME` — `.cache/ginary` is appended.
//!
//! Platform-specific bases (`~/Library/Caches` on macOS, `%LOCALAPPDATA%` on
//! Windows) are a later milestone; today every host follows the rules above.

use std::ffi::OsString;
use std::path::PathBuf;

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
}

impl EnvSnapshot {
    /// Reads the relevant variables from the current process environment.
    pub fn from_env() -> Self {
        Self {
            ginary_cache_dir: std::env::var_os("GINARY_CACHE_DIR"),
            xdg_cache_home: std::env::var_os("XDG_CACHE_HOME"),
            home: std::env::var_os("HOME"),
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
}

impl Source {
    /// Returns the name of the environment variable this source reads.
    pub const fn variable(self) -> &'static str {
        match self {
            Self::GinaryCacheDir => "GINARY_CACHE_DIR",
            Self::XdgCacheHome => "XDG_CACHE_HOME",
            Self::Home => "HOME",
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
pub fn resolve(env: &EnvSnapshot) -> Result<CacheDir, CacheDirError> {
    // One implementation of the precedence, in `cache::resolve`. This is its
    // build-side projection: the same three rules, and an error where the
    // launcher has a `${TMPDIR}` fallback, because a build tool that silently
    // wrote into `/tmp` would be a build tool nobody could find the output of.
    let mut pairs: Vec<(OsString, OsString)> = Vec::new();
    for (name, value) in [
        (crate::cache::GINARY_CACHE_DIR_VAR, &env.ginary_cache_dir),
        (crate::cache::XDG_CACHE_HOME_VAR, &env.xdg_cache_home),
        (crate::cache::HOME_VAR, &env.home),
    ] {
        if let Some(value) = value {
            pairs.push((OsString::from(name), value.clone()));
        }
    }

    // The uid only ever reaches the fallback root, and the fallback is exactly
    // the case this function refuses, so its value cannot be observed.
    let resolved = crate::cache::resolve(&crate::cache::Env::from_pairs(pairs), 0);
    let source = match resolved.origin {
        crate::cache::Origin::GinaryCacheDir => Source::GinaryCacheDir,
        crate::cache::Origin::XdgCacheHome => Source::XdgCacheHome,
        crate::cache::Origin::Home => Source::Home,
        crate::cache::Origin::Fallback => return Err(CacheDirError::Unresolved),
        // Unreachable: the snapshot above holds only the three portable
        // variables, so `resolve` has nothing to reach a Windows root with.
        // The build side's own Windows directory is not this function's — a
        // stub cache under `%LOCALAPPDATA%` is a decision nothing has needed
        // yet — and inventing a source here would be a projection of a rule
        // that does not exist.
        crate::cache::Origin::LocalAppData | crate::cache::Origin::WindowsFallback => {
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

    fn snapshot(ginary: Option<&str>, xdg: Option<&str>, home: Option<&str>) -> EnvSnapshot {
        EnvSnapshot {
            ginary_cache_dir: ginary.map(OsString::from),
            xdg_cache_home: xdg.map(OsString::from),
            home: home.map(OsString::from),
        }
    }

    #[test]
    fn ginary_cache_dir_wins_and_is_used_verbatim() {
        let resolved = resolve(&snapshot(
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
        let resolved = resolve(&snapshot(None, Some("/xdg"), Some("/home/u"))).expect("resolves");
        assert_eq!(resolved.path, PathBuf::from("/xdg/ginary"));
        assert_eq!(resolved.source, Source::XdgCacheHome);
    }

    #[test]
    fn home_gets_dot_cache_ginary() {
        let resolved = resolve(&snapshot(None, None, Some("/home/u"))).expect("resolves");
        assert_eq!(resolved.path, PathBuf::from("/home/u/.cache/ginary"));
        assert_eq!(resolved.source, Source::Home);
    }

    #[test]
    fn empty_values_count_as_unset() {
        let resolved = resolve(&snapshot(Some(""), Some(""), Some("/home/u"))).expect("resolves");
        assert_eq!(resolved.path, PathBuf::from("/home/u/.cache/ginary"));
        assert_eq!(resolved.source, Source::Home);
    }

    #[test]
    fn a_relative_xdg_cache_home_is_ignored() {
        let resolved =
            resolve(&snapshot(None, Some("relative/cache"), Some("/home/u"))).expect("resolves");
        assert_eq!(resolved.path, PathBuf::from("/home/u/.cache/ginary"));
        assert_eq!(resolved.source, Source::Home);
    }

    #[test]
    fn a_relative_ginary_cache_dir_is_still_honoured() {
        // Unlike XDG_CACHE_HOME this is an explicit ginary override, so the user
        // gets exactly what they asked for.
        let resolved = resolve(&snapshot(Some("cache"), None, None)).expect("resolves");
        assert_eq!(resolved.path, PathBuf::from("cache"));
        assert_eq!(resolved.source, Source::GinaryCacheDir);
    }

    #[test]
    fn nothing_set_is_an_error() {
        assert_eq!(
            resolve(&snapshot(None, None, None)),
            Err(CacheDirError::Unresolved)
        );
        assert_eq!(
            resolve(&snapshot(None, Some("relative"), Some(""))),
            Err(CacheDirError::Unresolved)
        );
    }

    #[test]
    fn every_source_names_its_variable() {
        assert_eq!(Source::GinaryCacheDir.variable(), "GINARY_CACHE_DIR");
        assert_eq!(Source::XdgCacheHome.variable(), "XDG_CACHE_HOME");
        assert_eq!(Source::Home.variable(), "HOME");
    }
}
