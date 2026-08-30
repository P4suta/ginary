// SPDX-License-Identifier: MIT OR Apache-2.0
//! Resolution of the ginary cache root.
//!
//! Bundled applications extract their runtime under this directory, so the
//! rules must be identical in the builder and in the launcher. Resolution is a
//! pure function over an [`EnvSnapshot`] so that every rule is unit-testable
//! without touching the real process environment.
//!
//! The precedence is:
//!
//! 1. `GINARY_CACHE_DIR` — used verbatim, the escape hatch for read-only or
//!    `noexec` home directories;
//! 2. `XDG_CACHE_HOME` — must be an absolute path, `ginary` is appended;
//! 3. `HOME` — `.cache/ginary` is appended.
//!
//! Platform-specific bases (`~/Library/Caches` on macOS, `%LOCALAPPDATA%` on
//! Windows) and the temporary-directory fallback are later milestones; today
//! every host follows the rules above.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

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
    if let Some(value) = non_empty(env.ginary_cache_dir.as_deref()) {
        return Ok(CacheDir {
            path: PathBuf::from(value),
            source: Source::GinaryCacheDir,
        });
    }

    if let Some(value) = non_empty(env.xdg_cache_home.as_deref()) {
        let base = Path::new(value);
        if base.is_absolute() {
            return Ok(CacheDir {
                path: base.join("ginary"),
                source: Source::XdgCacheHome,
            });
        }
    }

    if let Some(value) = non_empty(env.home.as_deref()) {
        return Ok(CacheDir {
            path: Path::new(value).join(".cache").join("ginary"),
            source: Source::Home,
        });
    }

    Err(CacheDirError::Unresolved)
}

/// Returns the value unless it is absent or empty.
fn non_empty(value: Option<&std::ffi::OsStr>) -> Option<&std::ffi::OsStr> {
    value.filter(|value| !value.is_empty())
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
