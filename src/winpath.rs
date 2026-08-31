// SPDX-License-Identifier: MIT OR Apache-2.0
//! The `\\?\` path prefix, and the identity that stands in for it everywhere
//! else.
//!
//! Windows resolves an ordinary path through a normalisation step that stops
//! at `MAX_PATH`, strips trailing dots and spaces and reserves a handful of
//! device names. A path that opens with `\\?\` skips all of it: the bytes go
//! to the object manager as they are. A cache entry is
//! `%LOCALAPPDATA%\ginary\<app>\<key>\lib\<name>-<vsn>\ebin\<module>.beam`,
//! which is a hundred and fifty characters before the application is named,
//! so an artifact extracted under a deep home directory is exactly the shape
//! that hits the limit — and it hits it in the middle of an extraction, which
//! is the worst place for a limit to be discovered.
//!
//! The rule is one sentence: **ginary opens the verbatim spelling and hands
//! `erl.exe` the ordinary one.**
//!
//! [`long_path`] is the first half, and it is applied once per *walk* of the
//! cache rather than at every path. The extraction's walk is
//! [`crate::cache::CacheDirs::extraction_dir`]: a path joined onto a verbatim
//! path is verbatim too, so prefixing the directory an extraction hangs off
//! covers the temporary tree, every file the unpacker creates, the flush that
//! reopens them, both ends of the rename — and, because
//! [`crate::cache::ensure_extracted`] *answers with* that spelling, the
//! cache-hit check, the `.lock` file, the manifest probe and
//! [`crate::launch::preflight`] as well. Prefixing only what the unpacker
//! writes moved the limit one step later rather than removing it, twice.
//!
//! The other walks are the removals — [`crate::cache::sweep`],
//! [`crate::cache::discard_incomplete`], [`crate::cache::prune_app`],
//! [`crate::cache::uninstall`], [`crate::cache::prune`] and
//! [`crate::cache::clean`] — and each applies it to the directory it was
//! given rather than trusting its caller to have done so. Prefixing an
//! already-verbatim path is a no-op, so either spelling reaches the same tree,
//! and the rule cannot drift from one call site to the next: a removal that
//! walked the ordinary spelling would list a past-`MAX_PATH` entry, lock it,
//! and then fail to rename it aside.
//!
//! [`plain_path`] is the second half, and it is applied in the two places a
//! path stops being something ginary opens. [`crate::launch::plan`] is one: a
//! cache path becomes `ROOTDIR`, `BINDIR` and text in the argument vector
//! `erl.exe` is started with, and a `\\?\` path is a shape that program takes
//! apart and reassembles rather than one it merely opens. What the launcher
//! spawns *itself* — [`crate::launch::LaunchPlan::program`] — keeps the
//! prefix, because opening it is this process's own business. The removal
//! reports are the other: a prune table and the path inside a cache error are
//! read by a person, so they name the spelling their caller asked about.
//!
//! One of the two is compiled out on unix and the other is not, and the
//! asymmetry is deliberate. [`long_path`] *adds* a prefix, and its rule fires
//! on any drive-absolute spelling, including a relative unix path that happens
//! to be called `C:/x`; so it is the identity on unix, decided at compile time.
//! [`plain_path`] *removes* one, and it fires only on a path whose first four
//! characters are `\\?\` — which no unix path has — so it is one code path on
//! every platform and is unit-tested on the machine this crate is developed on.
//!
//! [`long_path_str`] and [`plain_path_str`] are the two rules themselves,
//! spelled on text rather than on a [`Path`], because they are about Windows
//! path syntax and have to be testable here.

use std::borrow::Cow;
use std::path::Path;

/// The prefix that turns a fully-qualified path into a verbatim one.
pub const LONG_PATH_PREFIX: &str = r"\\?\";

/// The prefix a UNC path takes instead, `\\server\share` becoming
/// `\\?\UNC\server\share`.
pub const UNC_LONG_PATH_PREFIX: &str = r"\\?\UNC\";

/// The prefixed form of `path`, or `path` itself on a platform that has no
/// such thing.
///
/// On Windows this applies [`long_path_str`] to the path's text and owns the
/// result. A path that is not valid Unicode is returned unchanged: the prefix
/// is a Windows-syntax transformation and there is nothing safe to do with
/// code units that are not one.
#[cfg(windows)]
pub fn long_path(path: &Path) -> Cow<'_, Path> {
    let Some(text) = path.to_str() else {
        return Cow::Borrowed(path);
    };
    let prefixed = long_path_str(text);
    if prefixed == text {
        // The rule left this path alone — it is relative, or it already
        // carries the prefix — so nothing is copied for it either.
        return Cow::Borrowed(path);
    }
    Cow::Owned(std::path::PathBuf::from(prefixed))
}

/// The prefixed form of `path`, which off Windows is `path` itself.
///
/// The identity, and it borrows: the launcher joins a cache path thousands of
/// times per extraction and no unix kernel has ever heard of `\\?\`. The
/// helper exists so that the call sites are one code path rather than two.
#[cfg(not(windows))]
pub fn long_path(path: &Path) -> Cow<'_, Path> {
    Cow::Borrowed(path)
}

/// The `\\?\` form of one Windows path, as text.
///
/// The rules, in the order they are applied:
///
/// | input | output |
/// |---|---|
/// | `C:\a\b` | `\\?\C:\a\b` |
/// | `C:/a/b` | `\\?\C:\a\b` |
/// | `\\srv\share\a` | `\\?\UNC\srv\share\a` |
/// | `\\?\C:\a` | unchanged |
/// | `a\b` | unchanged |
///
/// A relative path is left alone because the prefix is only meaningful on a
/// fully-qualified one: `\\?\a\b` names a device called `a`, not a file in the
/// working directory. Forward slashes are rewritten first, because a verbatim
/// path is *not* normalised and `\\?\C:/a` would be a file whose name holds a
/// slash rather than a directory.
///
/// Available on every platform, and deliberately: the rule is Windows path
/// syntax rather than a system call, so it is a unit test on the machine this
/// crate is developed on rather than a claim nobody here can check.
pub fn long_path_str(path: &str) -> String {
    let separated = path.replace('/', SEPARATOR);

    // First, because the verbatim prefix also opens with the two separators a
    // UNC path opens with, and prefixing an already-prefixed path would name a
    // directory called `?`.
    if separated.starts_with(LONG_PATH_PREFIX) {
        return separated;
    }
    if let Some(rest) = separated.strip_prefix(UNC_PREFIX) {
        return format!("{UNC_LONG_PATH_PREFIX}{rest}");
    }
    if is_drive_absolute(&separated) {
        return format!("{LONG_PATH_PREFIX}{separated}");
    }
    // Not fully qualified, so there is no prefix to add — and the argument is
    // handed back rather than the rewritten form, because a path this rule
    // does not act on is a path it does not touch.
    path.to_owned()
}

/// The separator a verbatim path is spelled with.
const SEPARATOR: &str = r"\";

/// What a UNC path opens with, `\\server\share`.
const UNC_PREFIX: &str = r"\\";

/// Whether `path` is `<letter>:\...`, the one fully-qualified form that is not
/// a UNC path.
///
/// `C:` and `C:a` are *drive-relative*: they name a file in whatever directory
/// that drive's cursor is on, which is per-process state the prefix has no way
/// to resolve. So they are left alone with everything else that is relative.
fn is_drive_absolute(path: &str) -> bool {
    let bytes = path.as_bytes();
    matches!(bytes, [drive, b':', b'\\', ..] if drive.is_ascii_alphabetic())
}

/// The ordinary form of `path`, with any verbatim prefix removed.
///
/// The inverse of [`long_path`], and unlike it this one is compiled on every
/// platform: it acts only on a path whose text begins with `\\?\`, and no unix
/// path does. Applying it here rather than at each call site keeps the rule
/// testable on the machine ginary is developed on, which is the same reason
/// [`long_path_str`] is.
///
/// A path that is not valid Unicode is returned unchanged, for the reason
/// [`long_path`] gives: the prefix is a Windows-syntax transformation and
/// there is nothing safe to do with code units that are not one.
pub fn plain_path(path: &Path) -> Cow<'_, Path> {
    let Some(text) = path.to_str() else {
        return Cow::Borrowed(path);
    };
    let plain = plain_path_str(text);
    if plain == text {
        return Cow::Borrowed(path);
    }
    Cow::Owned(std::path::PathBuf::from(plain))
}

/// One Windows path with its `\\?\` prefix taken off, as text.
///
/// The rules, in the order they are applied:
///
/// | input | output |
/// |---|---|
/// | `\\?\C:\a\b` | `C:\a\b` |
/// | `\\?\UNC\srv\share\a` | `\\srv\share\a` |
/// | `C:\a\b` | unchanged |
/// | `/home/ada/.cache` | unchanged |
/// | `\\?\Volume{...}\a` | unchanged |
///
/// A verbatim path whose remainder is neither drive-absolute nor `UNC\` is
/// left alone: `\\?\Volume{…}` and `\\?\GLOBALROOT\…` name objects that have
/// no ordinary spelling at all, so removing the prefix would not shorten the
/// path, it would change which object it names.
pub fn plain_path_str(path: &str) -> String {
    let Some(rest) = path.strip_prefix(LONG_PATH_PREFIX) else {
        return path.to_owned();
    };
    if let Some(unc) = rest.strip_prefix(UNC_LONG_PATH_PREFIX_TAIL) {
        return format!("{UNC_PREFIX}{unc}");
    }
    if is_drive_absolute(rest) {
        return rest.to_owned();
    }
    path.to_owned()
}

/// What follows [`LONG_PATH_PREFIX`] in [`UNC_LONG_PATH_PREFIX`].
const UNC_LONG_PATH_PREFIX_TAIL: &str = r"UNC\";
