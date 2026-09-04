// SPDX-License-Identifier: MIT OR Apache-2.0
//! Where a packaged application extracts its runtime, and how it gets there.
//!
//! The cache is the only thing a packaged application writes, and it is
//! written by every copy of the artifact at once — a shell script that starts
//! eight of them on a cold machine is an ordinary thing to do. So the whole
//! module is built around one property: an entry is either complete or absent,
//! and the proof of completeness is a `rename(2)` that no reader can observe
//! half of.
//!
//! ```text
//! <root>/<app>/<key>/                 a complete entry; ginary.json proves it
//! <root>/<app>/.<key>.tmp-<pid>/      one process's extraction in progress
//! <root>/<app>/.<key>.corrupt-<pid>/  an entry that failed its own check
//! ```
//!
//! The key is [`crate::trailer::Trailer::cache_key`], so two artifacts with
//! the same payload share one extraction and a rebuilt artifact gets its own.
//! Nothing in the path comes from the artifact's *file name*: an artifact a
//! user renames still finds its cache.
//!
//! ## Resolution
//!
//! `GINARY_CACHE_DIR`, then `XDG_CACHE_HOME/ginary`, then `HOME/.cache/ginary`,
//! and if none of those is set — or if the one that is cannot be created —
//! `${TMPDIR:-/tmp}/ginary-<uid>`. The uid is in the name because `/tmp` is
//! shared and a directory another user owns is not a directory this process
//! may trust, and the name is not enough on its own: the fallback root is
//! created with `mkdir(0o700)` and, when it is already there, checked — a real
//! directory, owned by this uid, that no group or other may write to — because
//! `create_dir_all` would have accepted an attacker's directory and followed
//! an attacker's symlink. Falling back *because the primary was unwritable*
//! costs one warning line on standard error; falling back because nothing was
//! set is silent, since there was nothing to warn about.
//!
//! The `<app>` component is checked too, wherever it comes from: a manifest's
//! `app` and `ginary cache clean --app` are both joined onto the root, and
//! `Path::join` with an absolute component replaces the whole path.
//!
//! [`Env`] is the snapshot every rule reads, so resolution is a pure function
//! and each rule is a unit test rather than a process with a doctored
//! environment.
//!
//! ## Extraction
//!
//! [`ensure_extracted`] is the ten steps of `docs/adr/0005`, in order, each
//! one a [`Diag`] phase:
//!
//! | phase | what it records |
//! |---|---|
//! | `cache_hit` | the entry was already complete; nothing was written |
//! | `cache_sweep` | temporary and corrupt trees removed, and those kept because their process is alive |
//! | `cache_tmp` | the temporary tree this process will extract into |
//! | `extract` | `entries` and `bytes` unpacked |
//! | `chmod` | how many files under the bindir were made executable |
//! | `sync` | whether one `syncfs` did it or a per-file fallback was needed |
//! | `rename` | `reused=true` when a concurrent process won the race |

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

#[cfg(unix)]
use rustix::io::Errno;

use crate::diag::Diag;
use crate::error::{LauncherError, PREFIX};
use crate::fault;
use crate::manifest::{MANIFEST_NAME, Manifest};
use crate::trailer::Trailer;

/// The mode the per-application directory is created with.
///
/// The cache holds executables extracted from an artifact, and `/tmp` is
/// shared: another user must not be able to add a file to a directory this
/// process is about to run programs out of.
pub const APP_DIR_MODE: u32 = 0o700;

/// The mode every file under the manifest's bindir is given after extraction.
pub const BIN_MODE: u32 = 0o755;

/// The prefix of a temporary extraction tree.
pub const TMP_PREFIX: &str = "tmp-";

/// The prefix of an entry that failed its own completeness check.
pub const CORRUPT_PREFIX: &str = "corrupt-";

/// The directory component every resolved root but the override ends in.
pub const DIR_NAME: &str = "ginary";

/// The variable that overrides the cache root outright.
pub const GINARY_CACHE_DIR_VAR: &str = "GINARY_CACHE_DIR";

/// The XDG base directory the cache lives under when it is set.
pub const XDG_CACHE_HOME_VAR: &str = "XDG_CACHE_HOME";

/// The home directory `.cache/ginary` hangs off.
pub const HOME_VAR: &str = "HOME";

/// The temporary directory the fallback root lives in.
pub const TMPDIR_VAR: &str = "TMPDIR";

/// The Windows per-user application data directory, `%LOCALAPPDATA%`.
///
/// The Windows counterpart of `XDG_CACHE_HOME`: a per-user directory that is
/// not roamed, which is exactly what a cache of extracted runtimes should not
/// be. `ginary` is appended to it.
pub const LOCALAPPDATA_VAR: &str = "LOCALAPPDATA";

/// The Windows temporary directory, `%TEMP%`.
pub const TEMP_VAR: &str = "TEMP";

/// The older spelling of [`TEMP_VAR`], read when it is not set.
pub const TMP_VAR: &str = "TMP";

/// The user name the Windows fallback root carries.
///
/// The counterpart of the uid in `${TMPDIR}/ginary-<uid>`: a temporary
/// directory can be shared, so the name says whose cache this is.
pub const USERNAME_VAR: &str = "USERNAME";

/// The temporary directory the Windows fallback falls back to.
///
/// Windows sets `%TEMP%` for every interactive process, so this is reached
/// only by a service or a scrubbed environment. It is the machine-wide
/// directory Windows itself guarantees, and the `<user>` component keeps two
/// accounts on one machine apart inside it.
pub const WINDOWS_DEFAULT_TEMP: &str = r"C:\Windows\Temp";

/// The user the Windows fallback root is named after when `%USERNAME%` is not
/// set.
pub const UNKNOWN_USER: &str = "unknown";

/// A snapshot of the process environment.
///
/// The launcher reads the environment once, at the top, and every decision
/// below it is a pure function of this value. That is what makes cache
/// resolution and [`crate::launch::plan`] unit-testable without a subprocess,
/// and it is also what keeps the two halves consistent: the variables the
/// launcher scrubs and the variables it resolves the cache from are read from
/// the same snapshot at the same instant.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Env {
    vars: BTreeMap<OsString, OsString>,
}

impl Env {
    /// Snapshots the current process environment with [`std::env::vars_os`].
    pub fn from_env() -> Self {
        Self::from_pairs(std::env::vars_os())
    }

    /// Builds a snapshot from explicit pairs.
    pub fn from_pairs<I>(pairs: I) -> Self
    where
        I: IntoIterator<Item = (OsString, OsString)>,
    {
        Self {
            vars: pairs.into_iter().collect(),
        }
    }

    /// The value of one variable, or [`None`] when it is unset.
    ///
    /// An empty value is returned as an empty string rather than as [`None`];
    /// the rules that treat an exported-but-empty variable as unset say so
    /// themselves.
    pub fn get(&self, key: &str) -> Option<&OsStr> {
        self.vars.get(OsStr::new(key)).map(OsString::as_os_str)
    }

    /// Whether a variable is set at all, whatever its value.
    pub fn contains(&self, key: &str) -> bool {
        self.vars.contains_key(OsStr::new(key))
    }

    /// Every variable name, in sorted order.
    ///
    /// This is what the pattern half of the scrubbing rule reads: `ERL_OTP*`
    /// ending in `_FLAGS` is a family rather than a list, so the launcher has
    /// to look at the names that are actually set.
    pub fn keys(&self) -> impl Iterator<Item = &OsStr> {
        self.vars.keys().map(OsString::as_os_str)
    }
}

/// Which rule produced a cache root.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// `GINARY_CACHE_DIR`, used verbatim.
    GinaryCacheDir,
    /// `XDG_CACHE_HOME`, with `ginary` appended.
    XdgCacheHome,
    /// `HOME`, with `.cache/ginary` appended.
    Home,
    /// `${TMPDIR:-/tmp}/ginary-<uid>`.
    Fallback,
    /// `%LOCALAPPDATA%\ginary`, on Windows.
    LocalAppData,
    /// `%TEMP%\ginary-<user>`, on Windows.
    WindowsFallback,
}

impl Origin {
    /// The provenance `ginary cache dir` prints.
    pub const fn describe(self) -> &'static str {
        match self {
            Self::GinaryCacheDir => "GINARY_CACHE_DIR",
            Self::XdgCacheHome => "XDG_CACHE_HOME",
            Self::Home => "HOME",
            Self::Fallback => "TMPDIR fallback",
            Self::LocalAppData => "LOCALAPPDATA",
            Self::WindowsFallback => "TEMP fallback",
        }
    }
}

/// A resolved cache root and how it was reached.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CacheDirs {
    /// The cache root itself.
    pub root: PathBuf,
    /// The rule that produced [`CacheDirs::root`].
    pub origin: Origin,
    /// Whether this is the temporary-directory fallback.
    pub is_fallback: bool,
}

impl CacheDirs {
    /// `<root>/<app>`, the directory every entry for one application lives in.
    ///
    /// It is also where a crash dump goes, which is why it is named separately
    /// from the entry: a dump belongs to the application rather than to the
    /// payload that happened to be extracted.
    pub fn app_dir(&self, app: &str) -> PathBuf {
        self.root.join(app)
    }

    /// `<root>/<app>/<key>`, one complete extraction.
    pub fn key_dir(&self, app: &str, key: &str) -> PathBuf {
        self.app_dir(app).join(key)
    }

    /// [`CacheDirs::app_dir`] in the form every path an extraction *writes*
    /// under is derived from.
    ///
    /// On Windows that is the `\\?\` form, and applying it here rather than
    /// at each call site is the whole point: a path joined onto a verbatim path
    /// is verbatim too, so the temporary tree, the files under it, the flush
    /// that walks them and the rename that publishes them are all covered by
    /// one call. Prefixing only the unpacker's destination moved the length
    /// limit one step later — into the per-file `fsync`, which reopens each
    /// path it was given. On unix [`crate::winpath::long_path`] is the identity
    /// and this is [`CacheDirs::app_dir`].
    ///
    /// It is also what [`crate::cache::ensure_extracted`] answers with, so that
    /// every path ginary itself opens afterwards — the hit check, the `.lock`,
    /// the manifest probe, [`crate::launch::preflight`] — is the one the
    /// extraction wrote. The ordinary spelling is put back in exactly two
    /// places: [`crate::launch::plan`], where the entry becomes `ROOTDIR`,
    /// `BINDIR` and text in an argument vector — a `\\?\` path is a shape
    /// `erl.exe` takes apart and puts back together rather than one it merely
    /// opens — and the removal reports, where a path is named to a person
    /// rather than opened.
    pub fn extraction_dir(&self, app: &str) -> PathBuf {
        crate::winpath::long_path(&self.app_dir(app)).into_owned()
    }
}

/// Checks that an application name may be joined onto the cache root.
///
/// The `<app>` component of `<cache>/<app>/<key>` comes from a manifest or
/// from `ginary cache clean --app`, and both are joined onto a directory this
/// process then creates, chmods and removes trees under. `Path::join` with an
/// absolute component replaces the whole path and a `..` component walks out
/// of the root, so a value that is not a single path component is refused
/// here rather than acted on.
///
/// The rule is [`crate::manifest`]'s, so the launcher and the manifest cannot
/// disagree about what an application name is.
///
/// # Errors
///
/// [`LauncherError::Cache`] naming `root`, because the caller's next step
/// would have been to write under it.
pub fn check_app(root: &Path, app: &str) -> Result<(), LauncherError> {
    if is_app_name(app) {
        return Ok(());
    }
    Err(LauncherError::cache(
        root,
        std::io::Error::other(format!("{}", AppNameRefusal(app))),
    ))
}

/// [`check_app`], for a caller that may have been given no application at all.
///
/// [`None`] means "every application under `root`", which joins nothing onto
/// the root and so has nothing to refuse. The check happens before the root is
/// put into the spelling a removal walks it in — see [`walked`] — so what a
/// refusal names is the root the caller asked about.
fn check_app_of(root: &Path, app: Option<&str>) -> Result<(), LauncherError> {
    match app {
        Some(app) => check_app(root, app),
        None => Ok(()),
    }
}

/// Whether `app` is a name the cache may join onto a root.
///
/// The predicate behind [`check_app`], for a caller that has a better error to
/// report than [`LauncherError::Cache`]: `ginary cache clean --app` is a usage
/// mistake made at a terminal, not a cache that is unusable.
pub fn is_app_name(app: &str) -> bool {
    crate::manifest::check_name("app", app).is_ok()
}

/// The one sentence both refusals of an application name are written with.
///
/// A type rather than a function so that the command line and the launcher
/// cannot drift into explaining the same rule two different ways.
pub struct AppNameRefusal<'a>(
    /// The value that is not an application name.
    pub &'a str,
);

impl std::fmt::Display for AppNameRefusal<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "`{}` is not an application name: it must be one path component, so that it cannot \
             name a directory outside the cache",
            self.0
        )
    }
}

/// Whether `value` is an absolute path *by the XDG base directory
/// specification*, which is not the same question as
/// [`std::path::Path::is_absolute`].
///
/// The specification says a relative `XDG_CACHE_HOME` must be ignored, and it
/// defines absolute the way POSIX does: a leading `/`. `Path::is_absolute`
/// answers for the platform the code was *compiled* for, so on Windows it
/// wants a drive letter or a UNC prefix and reports `/xdg` as relative. That
/// made [`resolve`] — the unix half of the resolver, whose whole subject is
/// three POSIX environment variables — silently skip its own `XDG_CACHE_HOME`
/// branch on a Windows host and fall through to `HOME`, which is a rule about
/// the machine the resolver was built on rather than a rule about the
/// variable it is reading. The first Windows runner reported it as
/// `cache::tests::xdg_cache_home_gets_a_ginary_component` resolving
/// `/home/u\.cache\ginary` where `/xdg/ginary` was asked for.
///
/// The answer here is the same on every host, which is the point.
pub fn xdg_base_is_absolute(value: &std::ffi::OsStr) -> bool {
    // Bytes rather than a decoded string: an environment variable is not
    // required to be UTF-8 on unix, and a value that is not would otherwise
    // answer `false` for the wrong reason. Every encoding an `OsStr` can hold
    // spells `/` as this one byte, and no multi-byte sequence begins with it.
    value.as_encoded_bytes().first() == Some(&b'/')
}

/// Resolves the cache root from an environment snapshot.
///
/// Pure: nothing is created and nothing is probed. An empty value counts as
/// unset, and a relative `XDG_CACHE_HOME` is ignored as the XDG base directory
/// specification requires, while a relative `GINARY_CACHE_DIR` is honoured
/// because it is an explicit instruction rather than a convention.
pub fn resolve(env: &Env, uid: u32) -> CacheDirs {
    if let Some(value) = non_empty(env.get(GINARY_CACHE_DIR_VAR)) {
        return CacheDirs {
            root: PathBuf::from(value),
            origin: Origin::GinaryCacheDir,
            is_fallback: false,
        };
    }

    if let Some(value) = non_empty(env.get(XDG_CACHE_HOME_VAR)) {
        // The specification's own rule, not the host's: see
        // `xdg_base_is_absolute`.
        if xdg_base_is_absolute(value) {
            return CacheDirs {
                root: Path::new(value).join(DIR_NAME),
                origin: Origin::XdgCacheHome,
                is_fallback: false,
            };
        }
    }

    if let Some(value) = non_empty(env.get(HOME_VAR)) {
        return CacheDirs {
            root: Path::new(value).join(".cache").join(DIR_NAME),
            origin: Origin::Home,
            is_fallback: false,
        };
    }

    CacheDirs {
        root: fallback_root(env, uid),
        origin: Origin::Fallback,
        is_fallback: true,
    }
}

/// `${TMPDIR:-/tmp}/ginary-<uid>`.
pub fn fallback_root(env: &Env, uid: u32) -> PathBuf {
    let base = non_empty(env.get(TMPDIR_VAR)).unwrap_or_else(|| OsStr::new("/tmp"));
    Path::new(base).join(format!("{DIR_NAME}-{uid}"))
}

/// Resolves the cache root on Windows from an environment snapshot.
///
/// `GINARY_CACHE_DIR`, then `%LOCALAPPDATA%\ginary`, then
/// `%TEMP%\ginary-<user>`. The same shape as [`resolve`] and the same rules
/// about emptiness — an exported-but-empty variable counts as unset — with the
/// two roots Windows spells differently in the middle. `%LOCALAPPDATA%` is not
/// required to be absolute the way `XDG_CACHE_HOME` is: there is no
/// specification saying it may be ignored, and a relative one is a broken
/// environment rather than a convention.
///
/// Pure: nothing is created and nothing is probed. `user` is
/// [`current_user`]'s answer in the launcher and a fixed string in a test, so
/// that every rule below is a unit test rather than a process with a doctored
/// environment.
pub fn resolve_windows(env: &Env, user: &str) -> CacheDirs {
    if let Some(value) = non_empty(env.get(GINARY_CACHE_DIR_VAR)) {
        return CacheDirs {
            root: PathBuf::from(value),
            origin: Origin::GinaryCacheDir,
            is_fallback: false,
        };
    }

    if let Some(value) = non_empty(env.get(LOCALAPPDATA_VAR)) {
        return CacheDirs {
            root: Path::new(value).join(DIR_NAME),
            origin: Origin::LocalAppData,
            is_fallback: false,
        };
    }

    CacheDirs {
        root: windows_fallback_root(env, user),
        origin: Origin::WindowsFallback,
        is_fallback: true,
    }
}

/// `%TEMP%\ginary-<user>`, the Windows fallback root.
///
/// `%TEMP%`, then `%TMP%`, then [`WINDOWS_DEFAULT_TEMP`].
pub fn windows_fallback_root(env: &Env, user: &str) -> PathBuf {
    let base = non_empty(env.get(TEMP_VAR))
        .or_else(|| non_empty(env.get(TMP_VAR)))
        .unwrap_or_else(|| OsStr::new(WINDOWS_DEFAULT_TEMP));
    Path::new(base).join(format!("{DIR_NAME}-{user}"))
}

/// The user name the Windows fallback root is named after.
///
/// `%USERNAME%`, or [`UNKNOWN_USER`] when it is unset or empty. The name is a
/// path component, so a value holding a separator is refused the same way an
/// application name is and [`UNKNOWN_USER`] is used instead: a cache in the
/// wrong directory is worse than a cache nobody can tell apart.
pub fn current_user(env: &Env) -> String {
    non_empty(env.get(USERNAME_VAR))
        .and_then(OsStr::to_str)
        .filter(|name| is_user_name(name))
        .unwrap_or(UNKNOWN_USER)
        .to_owned()
}

/// Whether `name` may be joined onto a temporary directory as one component.
///
/// The rule is the one [`check_app`] applies to an application name, stated
/// for a value Windows lets a user pick: not empty, neither of the two
/// directory names that walk upwards, and holding none of the characters that
/// would make it more than one component — the two separators and the colon a
/// drive is named with. A name that fails it is not refused, because there is
/// nothing to refuse to: [`current_user`] uses [`UNKNOWN_USER`] instead, and a
/// cache nobody can tell apart is a much smaller problem than a cache in the
/// wrong directory.
fn is_user_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\', ':'])
        && !name.contains(|character: char| character.is_control())
}

/// Resolves the cache root and creates it.
///
/// When the resolved root cannot be created because the filesystem says so —
/// `EACCES` or `EROFS`, a read-only home or a directory owned by somebody else
/// — the fallback is used instead and exactly one line is written to `warn`.
/// Any other failure is the caller's to report: a cache that cannot be created
/// for a reason ginary has no answer for is not a cache to silently work
/// around.
///
/// # Errors
///
/// [`LauncherError::Cache`] when neither the resolved root nor the fallback
/// can be created.
#[cfg(unix)]
pub fn prepare(env: &Env, uid: u32, warn: &mut dyn Write) -> Result<CacheDirs, LauncherError> {
    let resolved = resolve(env, uid);
    if resolved.is_fallback {
        // Nothing was set, so the shared directory is the first choice rather
        // than the second. It is still the shared directory.
        create_fallback_root(&resolved.root, uid)?;
        return Ok(resolved);
    }
    let error = match std::fs::create_dir_all(&resolved.root) {
        Ok(()) => return Ok(resolved),
        Err(error) => error,
    };
    if !is_refusal(&error) {
        return Err(LauncherError::cache(resolved.root, error));
    }

    let fallback = CacheDirs {
        root: fallback_root(env, uid),
        origin: Origin::Fallback,
        is_fallback: true,
    };
    create_fallback_root(&fallback.root, uid)?;

    // One line, and it names both roots: an operator who sees only the second
    // has no way to tell a deliberate `GINARY_CACHE_DIR` from a home directory
    // that has gone read-only.
    let _ = writeln!(
        warn,
        "{PREFIX}the cache directory {} could not be created ({error}), using {} instead",
        resolved.root.display(),
        fallback.root.display()
    );
    let _ = warn.flush();
    Ok(fallback)
}

/// Resolves the Windows cache root and creates it.
///
/// The shape of [`prepare`], with the two differences Windows makes. The first
/// is the fallback's name: `%TEMP%\ginary-<user>` rather than
/// `${TMPDIR}/ginary-<uid>`, because there is no uid. The second is what
/// creating it checks. The unix fallback lives in a directory every account on
/// the machine can write to, so an existing one is proved to be this user's
/// before anything is extracted into it; `%TEMP%` is per-account on Windows and
/// carries an ACL that says so, and the checks that would prove the same thing
/// about `C:\Windows\Temp` are Win32 security descriptors that
/// `docs/adr/0015-windows-launcher-stays-resident.md` records as unwritten and
/// untested. So the directory is created and no ownership claim is made about
/// it — which is why [`WINDOWS_DEFAULT_TEMP`] is a last resort reached only by
/// a process whose environment has been scrubbed.
///
/// # Errors
///
/// [`LauncherError::Cache`] when neither the resolved root nor the fallback can
/// be created.
#[cfg(windows)]
pub fn prepare_windows(env: &Env, warn: &mut dyn Write) -> Result<CacheDirs, LauncherError> {
    let user = current_user(env);
    let resolved = resolve_windows(env, &user);
    if resolved.is_fallback {
        std::fs::create_dir_all(&resolved.root)
            .map_err(|source| LauncherError::cache(&resolved.root, source))?;
        return Ok(resolved);
    }
    let error = match std::fs::create_dir_all(&resolved.root) {
        Ok(()) => return Ok(resolved),
        Err(error) => error,
    };
    if !is_refusal(&error) {
        return Err(LauncherError::cache(resolved.root, error));
    }

    let fallback = CacheDirs {
        root: windows_fallback_root(env, &user),
        origin: Origin::WindowsFallback,
        is_fallback: true,
    };
    std::fs::create_dir_all(&fallback.root)
        .map_err(|source| LauncherError::cache(&fallback.root, source))?;

    // The same line the unix side writes, and it names both roots for the same
    // reason: an operator who sees only the second cannot tell a deliberate
    // `GINARY_CACHE_DIR` from a `%LOCALAPPDATA%` an installer locked down.
    let _ = writeln!(
        warn,
        "{PREFIX}the cache directory {} could not be created ({error}), using {} instead",
        resolved.root.display(),
        fallback.root.display()
    );
    let _ = warn.flush();
    Ok(fallback)
}

/// The real user id of this process.
///
/// The fallback root carries it, so two users sharing `/tmp` get two caches.
///
/// Unix only, and deliberately so: Windows has no uid, and inventing one there
/// would put a number nothing produced into a directory name. The Windows
/// fallback root is named after `%USERNAME%` instead — see [`current_user`] —
/// and [`resolve_here`] is what a caller on either platform asks.
#[cfg(unix)]
pub fn current_uid() -> u32 {
    rustix::process::getuid().as_raw()
}

/// Resolves the cache root with the rules of the platform this build runs on.
///
/// [`resolve`] on unix, with this process's uid in the fallback name;
/// [`resolve_windows`] on Windows, with `%USERNAME%`. The two rules are public
/// and testable on either machine; this is the one-line dispatch that keeps
/// every caller from repeating the `cfg`.
#[cfg(unix)]
pub fn resolve_here(env: &Env) -> CacheDirs {
    resolve(env, current_uid())
}

/// Resolves the cache root with the rules of the platform this build runs on.
///
/// See the unix half of this function for what it dispatches to and why.
#[cfg(windows)]
pub fn resolve_here(env: &Env) -> CacheDirs {
    resolve_windows(env, &current_user(env))
}

/// Resolves the cache root for this platform and creates it.
///
/// [`prepare`] on unix; `prepare_windows` on Windows, which this build does
/// not compile. The dispatch [`resolve_here`] is, for the side of the pair that
/// writes.
///
/// # Errors
///
/// Whatever the platform's own `prepare` answers.
#[cfg(unix)]
pub fn prepare_here(env: &Env, warn: &mut dyn Write) -> Result<CacheDirs, LauncherError> {
    prepare(env, current_uid(), warn)
}

/// Resolves the cache root for this platform and creates it.
///
/// See the unix half of this function.
///
/// # Errors
///
/// Whatever [`prepare_windows`] answers.
#[cfg(windows)]
pub fn prepare_here(env: &Env, warn: &mut dyn Write) -> Result<CacheDirs, LauncherError> {
    prepare_windows(env, warn)
}

/// The mode the fallback root is created with.
///
/// The same [`APP_DIR_MODE`] the per-application directory gets, and for the
/// same reason one level higher up: this one lives in a directory every user
/// on the machine can write to.
pub const FALLBACK_ROOT_MODE: u32 = APP_DIR_MODE;

/// Creates `${TMPDIR:-/tmp}/ginary-<uid>`, or proves the one that is there is
/// this user's.
///
/// `create_dir_all` would not do. It succeeds on a directory that already
/// exists whatever its owner and its mode, and it follows a symlink — so on a
/// shared machine an attacker who wins the race to create `/tmp/ginary-<uid>`
/// owns the parent of the tree this launcher extracts programs into and then
/// executes them from, and can rename that tree aside and substitute their
/// own. So the directory is created with [`mkdir(2)`] semantics, and an
/// existing one is *checked*: a real directory, owned by `uid`, that no group
/// or other may write to.
///
/// [`mkdir(2)`]: std::fs::create_dir
///
/// # Errors
///
/// [`LauncherError::Cache`] when the directory cannot be created, and when
/// what is there is not a directory this user may trust.
#[cfg(unix)]
fn create_fallback_root(root: &Path, uid: u32) -> Result<(), LauncherError> {
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

    if let Some(parent) = root.parent() {
        std::fs::create_dir_all(parent).map_err(|source| LauncherError::cache(parent, source))?;
    }
    match std::fs::create_dir(root) {
        // This process made it, so there is nothing to check but the mode it
        // is given, and the umask does not get a vote on a directory in `/tmp`.
        Ok(()) => {
            return std::fs::set_permissions(
                root,
                std::fs::Permissions::from_mode(FALLBACK_ROOT_MODE),
            )
            .map_err(|source| LauncherError::cache(root, source));
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(LauncherError::cache(root, error)),
    }

    // `symlink_metadata`, not `metadata`: a link to a directory the attacker
    // owns would answer every question below with the target's answers.
    let metadata =
        std::fs::symlink_metadata(root).map_err(|source| LauncherError::cache(root, source))?;
    let refuse = |why: &str| {
        Err(LauncherError::cache(
            root,
            std::io::Error::other(format!(
                "the shared temporary directory is not one this user may trust: {why}"
            )),
        ))
    };
    if !metadata.is_dir() {
        return refuse("it is not a directory");
    }
    if metadata.uid() != uid {
        return refuse(&format!("it is owned by uid {}", metadata.uid()));
    }
    let mode = metadata.permissions().mode();
    if mode & 0o022 != 0 {
        return refuse(&format!(
            "another user may write to it (mode {:04o})",
            mode & 0o7777
        ));
    }
    Ok(())
}

/// What one sweep removed and what it left alone.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SweepReport {
    /// The trees that were removed, sorted.
    pub removed: Vec<PathBuf>,
    /// The trees left in place because the process that owns them is alive.
    pub kept: Vec<PathBuf>,
}

/// The spelling a removal walk opens a cache directory with.
///
/// Every removal in this module — [`sweep`], [`discard_incomplete`],
/// [`prune_app`], [`uninstall`], [`prune`] and [`clean`] — walks a tree the
/// extraction wrote under [`CacheDirs::extraction_dir`], which on Windows is
/// the `\\?\` spelling. A walk that opened the ordinary spelling would stop at
/// `MAX_PATH` on exactly the deep `%LOCALAPPDATA%` entry the prefix exists for:
/// the entry would be listed, reported `Unremovable` and left on disk forever,
/// and `GINARY_CMD=uninstall` would answer that it had uninstalled nothing.
/// So the prefix is added here, once per walk, and every path `read_dir` hands
/// back inherits it.
///
/// Adding it rather than requiring it of the caller is deliberate:
/// [`crate::winpath::long_path_str`] leaves an already-verbatim path alone, so
/// a caller holding a [`CacheDirs::extraction_dir`] and one holding a
/// [`CacheDirs::app_dir`] reach the same tree. That is what lets the rule live
/// in one place per walk rather than at every call site, which is where the
/// two spellings drifted apart. On unix this is the identity.
fn walked(dir: &Path) -> std::borrow::Cow<'_, Path> {
    crate::winpath::long_path(dir)
}

/// One path a removal walk found, in the spelling it is reported by.
///
/// The inverse of [`walked`], applied to everything that leaves these
/// functions: a [`PruneReport`], a [`CleanReport`] and the path inside a
/// [`LauncherError::Cache`] are all read by a person, and `\\?\` is a fact
/// about how the tree was opened rather than part of the path's identity. It
/// is the same conversion, and for the same reason, that [`crate::launch::plan`]
/// makes when a cache path stops being ginary's business.
///
/// Unlike [`walked`] this one is compiled on every platform, because
/// [`crate::winpath::plain_path`] acts only on a path whose text begins with
/// `\\?\` and no unix path does — which is what makes the rule a test on the
/// machine ginary is developed on.
fn reported(path: &Path) -> PathBuf {
    crate::winpath::plain_path(path).into_owned()
}

/// Removes the temporary and corrupt trees of dead processes from `app_dir`.
///
/// A tree is `.<key>.tmp-<pid>` or `.<key>.corrupt-<pid>`, and it is removed
/// when `is_alive` says no process with that id exists — or when the pid is
/// this process's own, because a leftover of a previous run of *this* pid is
/// by definition not in use. A tree whose process is alive is left alone and
/// reported in
/// [`SweepReport::kept`]: killing another launcher's extraction is worse than
/// leaving a directory behind.
///
/// # Errors
///
/// [`LauncherError::Cache`] when `app_dir` exists and cannot be listed. A
/// directory that is not there yet is an empty report, not an error.
pub fn sweep(app_dir: &Path, self_pid: u32, diag: &Diag) -> Result<SweepReport, LauncherError> {
    let app_dir = walked(app_dir);
    let entries = match std::fs::read_dir(&app_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(SweepReport::default());
        }
        Err(error) => return Err(LauncherError::cache(reported(&app_dir), error)),
    };

    let mut report = SweepReport::default();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(pid) = owner_pid(&name) else {
            continue;
        };
        let path = entry.path();
        if pid != self_pid && is_alive(pid) {
            report.kept.push(reported(&path));
            continue;
        }
        // Best effort: a removal that loses a race with another sweeper has
        // still reached the outcome this call was made for.
        if std::fs::remove_dir_all(&path).is_ok() || !path.exists() {
            report.removed.push(reported(&path));
        }
    }
    report.removed.sort();
    report.kept.sort();

    diag.kv(
        "cache_sweep",
        &[
            ("removed", &report.removed.len().to_string()),
            ("kept", &report.kept.len().to_string()),
        ],
    );
    Ok(report)
}

/// The process id a `.<key>.tmp-<pid>` or `.<key>.corrupt-<pid>` name carries.
///
/// [`None`] for every other name, which is how a complete entry and a
/// directory somebody else put there are left alone.
fn owner_pid(name: &OsStr) -> Option<u32> {
    let name = name.to_str()?;
    if !name.starts_with('.') {
        return None;
    }
    for prefix in [TMP_PREFIX, CORRUPT_PREFIX] {
        let marker = format!(".{prefix}");
        if let Some(position) = name.rfind(&marker) {
            let digits = &name[position + marker.len()..];
            if !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()) {
                return digits.parse().ok();
            }
        }
    }
    None
}

/// Whether a process with this id exists.
///
/// The whole of [`sweep`]'s decision, and it was a Linux filesystem lookup:
/// `Path::new("/proc").join(pid.to_string()).exists()`. That directory is
/// Linux's alone — Windows has no such namespace and macOS has not carried
/// one since 10.5 — so on two of the three platforms ginary packages for the
/// answer was `false` for every process that has ever run, and a launcher
/// sweeping the cache deleted the tree another launcher was at that moment
/// extracting into. The Windows runner is where it surfaced; see
/// `tests/regressions/e12_the_sweep_asked_proc_whether_a_process_was_alive.rs`.
///
/// Liveness is a question for the operating system's process table, so it is
/// asked of the process table: `kill(pid, 0)` on unix, which answers
/// `ESRCH` for a pid nothing holds and `EPERM` for one this user may not
/// signal, and `launch_windows::win32::process_is_alive` on Windows,
/// which opens the process object by id.
///
/// Every answer that is uncertain is `true`, in both implementations. The two
/// mistakes are not symmetric: keeping a tree whose owner has gone costs a
/// directory until the next sweep that can name it, and removing one whose
/// owner is alive destroys an extraction in progress. A number that cannot be
/// a process id at all is not uncertain and is `false`: a unix `pid_t` is a
/// *positive* `i32`, so neither `0` — which names the caller's process group
/// to `kill` — nor anything past `i32::MAX` has ever been a process, and a
/// tree naming one is a leftover like any other.
#[cfg(unix)]
fn is_alive(pid: u32) -> bool {
    let Some(pid) = i32::try_from(pid)
        .ok()
        .and_then(rustix::process::Pid::from_raw)
    else {
        return false;
    };
    !matches!(rustix::process::test_kill_process(pid), Err(Errno::SRCH))
}

/// Whether a process with this id exists.
///
/// The Windows half of the rule the unix implementation above documents.
#[cfg(windows)]
fn is_alive(pid: u32) -> bool {
    crate::launch_windows::win32::process_is_alive(pid)
}

/// Extracts the payload into the cache, or proves it is already there.
///
/// Returns the complete entry's directory, in the spelling the extraction
/// wrote it under — the `\\?\` form on Windows, `<root>/<app>/<key>` on unix.
/// Every read that follows takes that answer, so the cache-hit check, the
/// lock, the manifest probe and [`crate::launch::preflight`] all open the file
/// this function created; [`crate::launch::plan`] is where it is spelled the
/// ordinary way again, for the runtime.
///
/// The ten steps are listed in the module documentation; the two that decide
/// the module's correctness are the last two. The rename *is* the completion
/// marker, so no marker file is written, and a rename that fails with
/// `EEXIST`, `ENOTEMPTY` or `EISDIR` means another process finished first —
/// its entry is verified, this process's temporary tree is removed, and the
/// winner's directory is returned.
///
/// # Errors
///
/// [`LauncherError::Payload`] when the payload does not unpack or does not
/// hash to what `trailer` says, and [`LauncherError::Cache`] for every failure
/// that is about the filesystem rather than about the bytes. A failed
/// extraction leaves no `<key>` directory.
pub fn ensure_extracted(
    exe: &File,
    trailer: &Trailer,
    app: &str,
    dirs: &CacheDirs,
    diag: &Diag,
) -> Result<PathBuf, LauncherError> {
    check_app(&dirs.root, app)?;
    let key = trailer.cache_key();
    // One spelling, and it is the verbatim one on Windows: everything this
    // function writes hangs off `writing`, and `entry` — what it answers with,
    // what the hit check opens, what the caller locks, reads the manifest out
    // of and preflights — is joined onto the same path. See
    // [`CacheDirs::extraction_dir`]. On unix both are `<root>/<app>`.
    let writing = dirs.extraction_dir(app);
    let entry = writing.join(&key);
    let pid = std::process::id();

    // The fault that simulates a lost rename race has to reach the rename, and
    // the test that arms it necessarily runs against a cache the winner has
    // already filled. So it suppresses the hit as well: this process behaves
    // like one that started cold and was overtaken.
    let lost_race = fault::point("rename") == Some("eexist");

    // (1) The entry is complete when, and only when, `ginary.json` is a
    // regular file in it. Nothing else is a marker, because nothing else
    // arrives atomically.
    if !lost_race && entry.join(MANIFEST_NAME).is_file() {
        diag.kv("cache_hit", &[("path", &entry.display().to_string())]);
        return Ok(entry);
    }

    create_app_dir(&writing)?;
    discard_incomplete(&writing, &key, pid);

    // (2) Somebody else's leftovers, and our own from a previous run of this
    // pid. A tree whose process is alive is another launcher's, and is left.
    sweep(&writing, pid, diag)?;

    // (3) The temporary tree this process extracts into.
    let tmp = writing.join(format!(".{key}.{TMP_PREFIX}{pid}"));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir(&tmp).map_err(|source| LauncherError::cache(&tmp, source))?;
    diag.kv("cache_tmp", &[("path", &tmp.display().to_string())]);

    // Every file the payload holds is written under this directory, and a cache
    // entry is already a hundred and fifty characters deep before the
    // application is named. `tmp` is already verbatim on Windows, because it
    // was joined onto `writing`, and so is every path derived from it below:
    // the bindir the chmod walks, the files the flush reopens, and the two
    // ends of the rename.
    let manifest = match extract_into(exe, trailer, &tmp, diag) {
        Ok(manifest) => manifest,
        Err(error) => {
            // (6) A payload that did not hash, or an entry the format refuses,
            // must leave nothing a later run could mistake for an entry.
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(error);
        }
    };

    let finish = || -> Result<(), LauncherError> {
        // (7) Every program under the bindir is made executable whatever the
        // archive said, because a runtime that cannot exec its own port
        // programs is a runtime that fails much later and much less clearly.
        let bindir = tmp.join(&manifest.launch.bindir);
        let changed = chmod_tree(&bindir, BIN_MODE)?;
        diag.kv("chmod", &[("files", &changed.to_string())]);

        // (8) One barrier for the whole tree, and then the directory that will
        // hold the rename.
        let synced = sync_tree(&tmp)?;
        diag.kv("sync", &[("syncfs", if synced { "true" } else { "false" })]);
        Ok(())
    };
    if let Err(error) = finish() {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(error);
    }

    // (9) The rename is the completion marker, and (10) there is no other one.
    // Both ends are spelled the same way — Windows will not rename a verbatim
    // path onto an ordinary one — and that same spelling is what is answered,
    // so that nothing downstream opens a path this function did not write.
    match rename_into_place(&tmp, &entry, lost_race) {
        Ok(reused) => {
            diag.kv(
                "rename",
                &[("reused", if reused { "true" } else { "false" })],
            );
            Ok(entry)
        }
        Err(error) => {
            let _ = std::fs::remove_dir_all(&tmp);
            Err(error)
        }
    }
}

/// Removes `<app_dir>/<key>` when it is *not* a complete entry.
///
/// A `<key>` without a `ginary.json` is the remains of a rename that never
/// happened. It is moved aside to `.<key>.corrupt-<pid>` before it is removed,
/// so that a reader racing this process never sees a directory being emptied
/// under it.
///
/// A **complete** entry is never touched, and that is the whole point of the
/// function: this runs after step 1 has already decided not to take the hit —
/// which the lost-race fault makes it do with a complete entry in place — and
/// a version of this check that asked only whether `<key>` exists would delete
/// the winner's tree out from under every other process on the machine.
///
/// Answers whether anything was discarded. Best effort: a removal that loses a
/// race with another sweeper has still reached the outcome it was called for.
pub fn discard_incomplete(app_dir: &Path, key: &str, pid: u32) -> bool {
    let app_dir = walked(app_dir);
    let target = app_dir.join(key);
    if !target.exists() || target.join(MANIFEST_NAME).is_file() {
        return false;
    }
    let aside = app_dir.join(format!(".{key}.{CORRUPT_PREFIX}{pid}"));
    if std::fs::rename(&target, &aside).is_ok() {
        let _ = std::fs::remove_dir_all(&aside);
    }
    true
}

/// Steps 4 to 6: the reader chain, the unpack and the pause point.
fn extract_into(
    exe: &File,
    trailer: &Trailer,
    tmp: &Path,
    diag: &Diag,
) -> Result<Manifest, LauncherError> {
    let mut source = exe
        .try_clone()
        .map_err(|error| LauncherError::cache(tmp, error))?;
    source
        .seek(SeekFrom::Start(trailer.payload_offset))
        .map_err(|error| LauncherError::cache(tmp, error))?;

    let manifest = crate::payload::unpack(
        Corrupting::wrap(source),
        trailer.payload_len,
        &trailer.payload_sha256,
        tmp,
    )?;
    // The manifest is interpolated onto a directory this process created, so it
    // is checked here as well as in `launch::plan`: the chmod below joins
    // `bindir` onto the temporary tree before the plan ever sees it.
    manifest.validate()?;

    let (entries, bytes) = measure(tmp).map_err(|error| LauncherError::cache(tmp, error))?;
    diag.kv(
        "extract",
        &[
            ("entries", &entries.to_string()),
            ("bytes", &bytes.to_string()),
        ],
    );

    // The tree is on disk and nothing has been renamed: this is the instant a
    // test kills the process to prove the next run sweeps what is left.
    let _ = fault::point("after-extract");
    Ok(manifest)
}

/// Renames the temporary tree onto the entry, or accepts the winner's.
///
/// `EEXIST`, `ENOTEMPTY` and `EISDIR` all mean the same thing on this path: a
/// concurrent process finished first. Its entry is verified before this one's
/// tree is thrown away, because accepting an entry without checking it would
/// turn one process's failed extraction into every process's cache hit.
fn rename_into_place(tmp: &Path, target: &Path, forced: bool) -> Result<bool, LauncherError> {
    if !forced {
        match std::fs::rename(tmp, target) {
            Ok(()) => return Ok(false),
            // Somebody else finished first, which is the rest of this function.
            Err(error) if is_occupied(&error) => {}
            Err(error) => return Err(LauncherError::cache(target, error)),
        }
    }

    if !target.join(MANIFEST_NAME).is_file() {
        return Err(LauncherError::cache(
            target,
            std::io::Error::other(
                "another process took the entry and left it without a `ginary.json`",
            ),
        ));
    }
    std::fs::remove_dir_all(tmp).map_err(|source| LauncherError::cache(tmp, source))?;
    Ok(true)
}

/// Whether a failed `rename(2)` means the destination is already an entry.
#[cfg(unix)]
fn is_occupied(error: &std::io::Error) -> bool {
    is_errno(error, &[Errno::EXIST, Errno::NOTEMPTY, Errno::ISDIR])
}

/// Whether a failed `MoveFile` means the destination is already an entry.
///
/// The same three meanings Windows spells with its own numbers. There is no
/// `Errno` to read them out of — rustix is a unix dependency — so the values
/// are named here, and named rather than written inline for the reason the unix
/// side reads them out of rustix: a bare 183 in a condition is a number nobody
/// can check.
#[cfg(windows)]
fn is_occupied(error: &std::io::Error) -> bool {
    is_win32_error(
        error,
        &[
            ERROR_FILE_EXISTS,
            ERROR_ALREADY_EXISTS,
            ERROR_DIR_NOT_EMPTY,
            ERROR_ACCESS_DENIED,
        ],
    )
}

/// Whether a failed `create_dir_all` is the filesystem refusing rather than
/// something ginary has no answer for.
#[cfg(unix)]
fn is_refusal(error: &std::io::Error) -> bool {
    is_errno(error, &[Errno::ACCESS, Errno::ROFS])
}

/// Whether a failed `create_dir_all` is Windows refusing rather than something
/// ginary has no answer for.
#[cfg(windows)]
fn is_refusal(error: &std::io::Error) -> bool {
    is_win32_error(error, &[ERROR_ACCESS_DENIED, ERROR_WRITE_PROTECT])
}

/// `ERROR_ACCESS_DENIED`: the ACL said no.
#[cfg(windows)]
const ERROR_ACCESS_DENIED: i32 = 5;

/// `ERROR_WRITE_PROTECT`: the volume is read-only.
#[cfg(windows)]
const ERROR_WRITE_PROTECT: i32 = 19;

/// `ERROR_FILE_EXISTS`: the destination name is taken.
#[cfg(windows)]
const ERROR_FILE_EXISTS: i32 = 80;

/// `ERROR_DIR_NOT_EMPTY`: the destination directory holds something.
#[cfg(windows)]
const ERROR_DIR_NOT_EMPTY: i32 = 145;

/// `ERROR_ALREADY_EXISTS`: the destination is already there.
#[cfg(windows)]
const ERROR_ALREADY_EXISTS: i32 = 183;

/// Whether `error` carries one of `wanted`, as a Win32 error code.
#[cfg(windows)]
fn is_win32_error(error: &std::io::Error, wanted: &[i32]) -> bool {
    error
        .raw_os_error()
        .is_some_and(|code| wanted.contains(&code))
}

/// Whether `error` carries one of `wanted`.
///
/// The values come from rustix rather than from a table of numbers written
/// here: POSIX fixes the *names*, and `ENOTEMPTY` is 39 on Linux and 66 on the
/// BSDs. A hardcoded 39 would make the lost-rename-race branch miss on a
/// target this file already has a `cfg` for, and turn a reuse into exit 124.
#[cfg(unix)]
fn is_errno(error: &std::io::Error, wanted: &[Errno]) -> bool {
    error
        .raw_os_error()
        .is_some_and(|code| wanted.iter().any(|errno| errno.raw_os_error() == code))
}

/// Creates `<root>/<app>` with [`APP_DIR_MODE`].
///
/// The mode is set after creation rather than through the creation mask,
/// because `create_dir_all` creates the parents too and those belong to
/// whoever configured the cache root.
#[cfg(unix)]
fn create_app_dir(app_dir: &Path) -> Result<(), LauncherError> {
    use std::os::unix::fs::PermissionsExt as _;

    std::fs::create_dir_all(app_dir).map_err(|source| LauncherError::cache(app_dir, source))?;
    std::fs::set_permissions(app_dir, std::fs::Permissions::from_mode(APP_DIR_MODE))
        .map_err(|source| LauncherError::cache(app_dir, source))
}

/// Creates `<root>/<app>`, which on Windows is all there is to do.
///
/// [`APP_DIR_MODE`] is a POSIX mode and Windows has none: access there is an
/// ACL, and a directory created under `%LOCALAPPDATA%` or `%TEMP%` inherits the
/// per-account one its parent carries. Setting nothing is therefore the
/// accurate translation of "keep other accounts out", not a weaker one — and
/// writing an ACL by hand is the Win32 security work
/// `docs/adr/0015-windows-launcher-stays-resident.md` records as out of scope.
#[cfg(windows)]
fn create_app_dir(app_dir: &Path) -> Result<(), LauncherError> {
    std::fs::create_dir_all(app_dir).map_err(|source| LauncherError::cache(app_dir, source))
}

/// Gives every regular file under `dir` the mode `mode`, and answers how many.
///
/// A `dir` that is not there is zero files rather than an error: what the
/// runtime is missing is [`crate::launch::preflight`]'s to say, and it says it
/// by naming the file.
#[cfg(unix)]
fn chmod_tree(dir: &Path, mode: u32) -> Result<usize, LauncherError> {
    use std::os::unix::fs::PermissionsExt as _;

    let mut changed = 0;
    for path in files_under(dir).map_err(|error| LauncherError::cache(dir, error))? {
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode))
            .map_err(|source| LauncherError::cache(&path, source))?;
        changed += 1;
    }
    Ok(changed)
}

/// Nothing, on a platform with no execute bit to set.
///
/// Windows decides what may be run from the file's extension and its ACL, not
/// from a mode word, so there is no bit here whose absence would stop `erl.exe`
/// from starting. The answer is therefore zero files changed rather than a
/// count of files something was done to, and the `mode` column of
/// `ginary.index.json` is informational on a Windows artifact: it records what
/// the build machine's archive said, and nothing reads it back to enforce
/// anything. `docs/dev/debugging.md` says so where a reader would look.
#[cfg(windows)]
fn chmod_tree(dir: &Path, mode: u32) -> Result<usize, LauncherError> {
    let _ = (dir, mode);
    Ok(0)
}

/// Flushes the extracted tree, and then the directory the rename will happen
/// in.
///
/// Answers whether one `syncfs` did it. The fallback is a `fsync` per file,
/// which is the same guarantee at a much higher price; it exists because
/// `syncfs` is Linux's and because a filesystem may refuse it.
#[cfg(unix)]
fn sync_tree(tmp: &Path) -> Result<bool, LauncherError> {
    let handle = File::open(tmp).map_err(|source| LauncherError::cache(tmp, source))?;
    let synced = syncfs(&handle);
    if !synced {
        for path in files_under(tmp).map_err(|error| LauncherError::cache(tmp, error))? {
            let file =
                open_for_flush(&path).map_err(|source| LauncherError::cache(&path, source))?;
            file.sync_all()
                .map_err(|source| LauncherError::cache(&path, source))?;
        }
    }
    handle
        .sync_all()
        .map_err(|source| LauncherError::cache(tmp, source))?;

    if let Some(parent) = tmp.parent() {
        let app_dir = File::open(parent).map_err(|source| LauncherError::cache(parent, source))?;
        app_dir
            .sync_all()
            .map_err(|source| LauncherError::cache(parent, source))?;
    }
    Ok(synced)
}

/// Flushes the extracted tree, one file at a time.
///
/// Windows has neither `syncfs` nor a directory handle an ordinary `open` can
/// produce — `CreateFile` on a directory needs `FILE_FLAG_BACKUP_SEMANTICS`,
/// which `std::fs::File::open` does not pass — so the two things the unix
/// version does beyond the per-file barrier cannot be done here. The answer is
/// therefore always `false`, and it means what it says on both platforms: no
/// one-call barrier was available, every file was flushed individually.
///
/// The directory entry the rename creates is not flushed, and the consequence
/// is recorded rather than hidden: a machine that loses power between the
/// rename and NTFS's own metadata flush can come back with an entry whose
/// `ginary.json` is there and whose contents are not. The launcher's answer to
/// that is the one it already has for a corrupt entry — the completeness check
/// fails and the entry is extracted again — so the cost is one repeated
/// extraction rather than a broken artifact.
#[cfg(windows)]
fn sync_tree(tmp: &Path) -> Result<bool, LauncherError> {
    for path in files_under(tmp).map_err(|error| LauncherError::cache(tmp, error))? {
        let file = open_for_flush(&path).map_err(|source| LauncherError::cache(&path, source))?;
        file.sync_all()
            .map_err(|source| LauncherError::cache(&path, source))?;
    }
    Ok(false)
}

/// Opens one just-written file for the durability barrier that follows.
///
/// Read access is what a flush conceptually needs and it is all a unix flush
/// asks for: `fsync(2)` says nothing about the descriptor's access mode, and a
/// staged tree holds files a build has already made read-only, so asking for
/// write access unconditionally would fail for the opposite reason. Windows'
/// barrier is `FlushFileBuffers`, which the kernel refuses with
/// `ERROR_ACCESS_DENIED` on a handle that was not opened for writing — every
/// cold-cache extraction on the first Windows runner stopped on the first file
/// because of it. [`crate::platform::flush_needs_write_access`] is the rule,
/// so there is one function here rather than two `#[cfg]` arms, and the claim
/// is checkable on a machine with no Windows kernel on it.
fn open_for_flush(path: &Path) -> std::io::Result<File> {
    let (read, write) = flush_open_options(crate::platform::HOST);
    File::options().read(read).write(write).open(path)
}

/// The `(read, write)` access an extraction opens a file with before it flushes
/// it to disk on `os`.
///
/// Read is always asked for; write only where
/// [`crate::platform::flush_needs_write_access`] says the durability barrier
/// needs it. A pure function of `os` so that the *wiring* — that the opener
/// consults the platform at all — is asserted on a machine with no Windows
/// kernel on it, not only the rule it consults.
fn flush_open_options(os: crate::target::Os) -> (bool, bool) {
    (true, crate::platform::flush_needs_write_access(os))
}

/// One `syncfs(2)` for the whole filesystem the tree is on.
#[cfg(target_os = "linux")]
fn syncfs(handle: &File) -> bool {
    rustix::fs::syncfs(handle).is_ok()
}

/// Elsewhere there is no such call, so the per-file fallback is the only path.
#[cfg(all(unix, not(target_os = "linux")))]
fn syncfs(handle: &File) -> bool {
    let _ = handle;
    false
}

/// Every regular file under `dir`, depth first. A missing `dir` is empty.
fn files_under(dir: &Path) -> Result<Vec<PathBuf>, std::io::Error> {
    let mut found = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(current) = pending.pop() {
        let entries = match std::fs::read_dir(&current) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            if std::fs::symlink_metadata(&path)?.is_dir() {
                pending.push(path);
            } else {
                found.push(path);
            }
        }
    }
    found.sort();
    Ok(found)
}

/// How many regular files a tree holds, and how many bytes they are.
fn measure(dir: &Path) -> Result<(usize, u64), std::io::Error> {
    let files = files_under(dir)?;
    let mut bytes = 0u64;
    for path in &files {
        bytes = bytes.saturating_add(std::fs::symlink_metadata(path)?.len());
    }
    Ok((files.len(), bytes))
}

/// The reader `unpack:corrupt` puts between the artifact and the unpacker.
///
/// Without the feature it is the source reader and nothing else, so a release
/// build has no branch here at all. With it, and with the point armed, the
/// first byte the unpacker sees is not the first byte of the payload — the
/// fault a bit flip in the page cache produces, and the one the digest exists
/// to catch.
struct Corrupting<R> {
    /// Where the bytes come from.
    inner: R,
    /// Whether the next byte read is the one to flip.
    flip_next: bool,
}

impl<R: Read> Corrupting<R> {
    /// Wraps `inner`, arming the flip only when the point is armed.
    fn wrap(inner: R) -> Self {
        Self {
            inner,
            flip_next: fault::point("unpack") == Some("corrupt"),
        }
    }
}

impl<R: Read> Read for Corrupting<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let read = self.inner.read(buf)?;
        if self.flip_next && read > 0 {
            if let Some(byte) = buf.first_mut() {
                *byte ^= 0xff;
            }
            self.flip_next = false;
        }
        Ok(read)
    }
}

/// The variable that says how old an unused cache entry may get.
///
/// A count of days. `0` disables pruning entirely, which is what a machine
/// with one artifact and a slow disk wants; anything else is an age.
pub const PRUNE_DAYS_VAR: &str = "GINARY_PRUNE_DAYS";

/// How old an unused entry may get before pruning removes it, in days.
pub const DEFAULT_PRUNE_DAYS: u64 = 14;

/// Why one prune left an entry alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeptReason {
    /// A process holds the entry's `.lock`.
    Locked,
    /// The entry is younger than the age it was pruned against.
    Fresh,
    /// The entry could not be moved aside, so it is still where it was.
    ///
    /// Nobody holds it and it is old enough to go; the file system refused —
    /// a read-only application directory, a full one, a mount that has gone
    /// away. It is reported rather than dropped, because a `kept` column that
    /// omits an entry makes the summary a count of nothing.
    Unremovable,
}

impl KeptReason {
    /// The word `ginary cache prune` prints in its `kept` column.
    pub const fn describe(self) -> &'static str {
        match self {
            Self::Locked => "locked",
            Self::Fresh => "fresh",
            Self::Unremovable => "unremovable",
        }
    }
}

/// What one prune chooses between.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PruneOptions {
    /// The age in days. `0` disables pruning; see [`PRUNE_DAYS_VAR`].
    pub days: u64,
    /// Ignore the age and consider every entry, `--all`.
    ///
    /// It never ignores the lock: `--all` is "whatever its age", not
    /// "whatever is using it".
    pub all: bool,
}

impl Default for PruneOptions {
    /// [`DEFAULT_PRUNE_DAYS`], and no `--all`.
    fn default() -> Self {
        Self {
            days: DEFAULT_PRUNE_DAYS,
            all: false,
        }
    }
}

/// What one prune removed and what it left, with the reason.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PruneReport {
    /// The entries that were removed, sorted.
    pub removed: Vec<PathBuf>,
    /// The entries that stayed, sorted, each with why.
    pub kept: Vec<(PathBuf, KeptReason)>,
}

/// The age one prune runs against, from [`PRUNE_DAYS_VAR`].
///
/// [`DEFAULT_PRUNE_DAYS`] when the variable is unset, empty, or not a number:
/// a launcher that refused to start because a variable was misspelt would be
/// trading an application for a housekeeping preference.
pub fn prune_days(env: &Env) -> u64 {
    non_empty(env.get(PRUNE_DAYS_VAR))
        .and_then(OsStr::to_str)
        .and_then(|text| text.parse::<u64>().ok())
        .unwrap_or(DEFAULT_PRUNE_DAYS)
}

/// The prefix of a tree one prune has renamed aside and is about to remove.
///
/// The rename is what makes the removal atomic from a reader's point of view,
/// exactly as `.<key>.corrupt-<pid>` is in [`discard_incomplete`]: nothing ever
/// sees `<app>/<key>` being emptied under it.
pub const TRASH_PREFIX: &str = "trash-";

/// Seconds in a day, for turning [`PruneOptions::days`] into an age.
const SECONDS_PER_DAY: u64 = 86_400;

/// Renames a locked cache entry aside, giving `lock` up in the order the
/// platform allows.
///
/// The lock proves nobody is using the entry and the rename is the claim, so
/// making the claim while still holding the proof is the order that leaves no
/// window for another process to take the entry in between. A platform that
/// refuses to rename a directory it still holds a handle inside —
/// [`crate::platform::rename_refuses_open_children`], which is Windows, and
/// which `FILE_SHARE_DELETE` on `<entry>/.lock` does not buy back — cannot
/// have both at once, and there the lock is released first: the alternative is
/// an entry that can never be pruned or uninstalled at all, which is what the
/// first Windows runner reported for every complete entry it found.
///
/// Answers whether the rename happened. `lock` is consumed either way, so a
/// caller cannot keep holding a lock on a directory that is no longer there.
fn rename_aside(path: &Path, aside: &Path, lock: crate::cache_lock::ExclusiveLock) -> bool {
    match rename_aside_order(crate::platform::HOST) {
        RenameAsideOrder::DropThenRename => {
            drop(lock);
            std::fs::rename(path, aside).is_ok()
        }
        RenameAsideOrder::RenameThenDrop => {
            let renamed = std::fs::rename(path, aside).is_ok();
            drop(lock);
            renamed
        }
    }
}

/// Whether [`rename_aside`] drops the exclusive lock before or after the
/// rename on `os`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RenameAsideOrder {
    /// Drop the lock, then rename: the platform refuses to rename a directory
    /// it still holds an open handle inside, so the proof has to be released
    /// before the claim is made. This reopens the window between "nobody holds
    /// this" and "it is gone", which is the accepted price on such a platform.
    DropThenRename,
    /// Rename while still holding the lock, then drop it: the order that leaves
    /// no window, available where a rename does not mind an open child.
    RenameThenDrop,
}

/// Which order [`rename_aside`] performs the lock-drop and the rename in on
/// `os`.
///
/// [`crate::platform::rename_refuses_open_children`] is the rule; splitting the
/// decision out from the `drop`/`rename` calls makes the *wiring* — that the
/// order is chosen from the platform and not hard-coded — a pure function a
/// Linux machine can assert both answers of.
fn rename_aside_order(os: crate::target::Os) -> RenameAsideOrder {
    if crate::platform::rename_refuses_open_children(os) {
        RenameAsideOrder::DropThenRename
    } else {
        RenameAsideOrder::RenameThenDrop
    }
}

/// Prunes the *siblings* of one entry, best effort, never failing.
///
/// `keep` is the entry this process is about to run out of and is never
/// considered. Every other `<app_dir>/<key>` whose `ginary.json` was last
/// modified more than [`PruneOptions::days`] before `now`, and whose `.lock`
/// can be taken exclusively, is renamed to `.<key>.trash-<pid>` and then
/// removed: the rename is what makes the removal atomic from a reader's point
/// of view, exactly as it is in [`discard_incomplete`].
///
/// Nothing here can fail a launch. A directory that cannot be listed, an entry
/// that cannot be renamed and an entry another process holds are all left
/// alone and reported.
pub fn prune_app(
    app_dir: &Path,
    keep: Option<&str>,
    options: PruneOptions,
    now: std::time::SystemTime,
    diag: &Diag,
) -> PruneReport {
    let mut report = PruneReport::default();
    // `--all` is `whatever its age`, so it overrides the switch as well as the
    // number: an age of zero is a preference about staleness and `--all` is a
    // request that names none.
    if options.days == 0 && !options.all {
        return report;
    }
    let app_dir = walked(app_dir);
    let Ok(entries) = std::fs::read_dir(&app_dir) else {
        return report;
    };
    let pid = std::process::id();

    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if Some(name) == keep || name.starts_with('.') {
            continue;
        }
        let path = entry.path();
        // Pruning owns *complete* entries and nothing else. A temporary tree,
        // a corrupt one and a directory somebody else put here all lack a
        // `ginary.json`, and all of them are the sweep's business.
        let Ok(manifest) = std::fs::metadata(path.join(MANIFEST_NAME)) else {
            continue;
        };
        if !manifest.is_file() {
            continue;
        }

        if !options.all {
            let Ok(modified) = manifest.modified() else {
                continue;
            };
            let age = now.duration_since(modified).unwrap_or_default();
            if age.as_secs() < options.days.saturating_mul(SECONDS_PER_DAY) {
                report.kept.push((reported(&path), KeptReason::Fresh));
                continue;
            }
        }

        // The lock decides, and a lock that could not even be taken is a
        // "leave this alone": the cost of skipping an entry is a directory
        // that stays on disk, and the cost of removing one wrongly is a
        // running application losing its runtime.
        let Some(lock) = crate::cache_lock::try_exclusive(&path) else {
            report.kept.push((reported(&path), KeptReason::Locked));
            continue;
        };

        let aside = app_dir.join(format!(".{name}.{TRASH_PREFIX}{pid}"));
        if !rename_aside(&path, &aside, lock) {
            // Nobody holds it and it is old enough to go; the file system
            // refused. Reported rather than dropped: a `kept` column that
            // silently omits an entry makes the summary a count of nothing.
            report.kept.push((reported(&path), KeptReason::Unremovable));
            continue;
        }
        if std::fs::remove_dir_all(&aside).is_ok() {
            report.removed.push(reported(&path));
        } else {
            // The entry was renamed and could not be removed, so the tree is
            // still there under a name that says nothing about what it holds.
            // Putting it back is the only honest outcome: a cache entry the
            // launcher can still hit beats a directory nobody will ever look
            // at again.
            let _ = std::fs::rename(&aside, &path);
            report.kept.push((reported(&path), KeptReason::Unremovable));
        }
    }
    report.removed.sort();
    report.kept.sort_by(|left, right| left.0.cmp(&right.0));

    record_prune(&report, diag);
    report
}

/// Records what one prune did, by name.
///
/// The paths and not just the counts: an entry that vanished is a thing a bug
/// report has to be able to explain, and "one directory was removed" explains
/// nothing. The shape is the one [`crate::launch`] records an argument vector
/// with — [`crate::launch::json_array`], the same function — so that a trace
/// reader has one thing to parse rather than two.
fn record_prune(report: &PruneReport, diag: &Diag) {
    if !diag.is_enabled() {
        return;
    }
    let removed =
        crate::launch::json_array(report.removed.iter().map(|path| path.display().to_string()));
    let kept = crate::launch::json_array(
        report
            .kept
            .iter()
            .map(|(path, reason)| format!("{} ({})", path.display(), reason.describe())),
    );
    diag.kv(
        "prune",
        &[
            ("removed", &report.removed.len().to_string()),
            ("kept", &report.kept.len().to_string()),
            ("removed_paths", &removed),
            ("kept_paths", &kept),
        ],
    );
}

/// Removes every trace of one application from the cache, lock permitting.
///
/// This is what `GINARY_CMD=uninstall` runs. Unlike [`prune_app`] it has no
/// age: everything goes, complete entries and the temporary, corrupt and
/// trashed residue beside them, and the only thing that saves an entry is a
/// process holding it. The application directory itself is removed when
/// nothing is left in it.
///
/// Best effort throughout, and reported rather than fatal: a partial uninstall
/// is a fact the caller has to be told, not a failure.
pub fn uninstall(app_dir: &Path) -> PruneReport {
    let mut report = PruneReport::default();
    let app_dir = walked(app_dir);
    let Ok(entries) = std::fs::read_dir(&app_dir) else {
        return report;
    };
    let pid = std::process::id();

    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let path = entry.path();

        // Residue is nobody's to hold: a `.<key>.tmp-<pid>` tree belongs to a
        // process that is extracting or is gone, and neither state is one an
        // uninstall asks permission from. Everything else in this directory is
        // somebody else's — the crash dump the launcher points the runtime at
        // lives here — and an uninstall that deleted it would be removing the
        // very thing the directory is worth keeping for.
        if !path.join(MANIFEST_NAME).is_file() {
            if is_cache_residue(name) && remove_anything(&path) {
                report.removed.push(reported(&path));
            }
            continue;
        }

        let Some(lock) = crate::cache_lock::try_exclusive(&path) else {
            report.kept.push((reported(&path), KeptReason::Locked));
            continue;
        };
        let aside = app_dir.join(format!(".{name}.{TRASH_PREFIX}{pid}"));
        if !rename_aside(&path, &aside, lock) {
            // Nobody holds it: the file system refused, which is a different
            // thing to tell a user and a different thing to do about it.
            report.kept.push((reported(&path), KeptReason::Unremovable));
            continue;
        }
        if std::fs::remove_dir_all(&aside).is_ok() {
            report.removed.push(reported(&path));
        } else {
            let _ = std::fs::rename(&aside, &path);
            report.kept.push((reported(&path), KeptReason::Unremovable));
        }
    }
    report.removed.sort();
    report.kept.sort_by(|left, right| left.0.cmp(&right.0));

    // Only when it is empty: an application directory that still holds an
    // entry somebody is running out of is an application that is still
    // installed, and the crash dumps beside it are still worth reading.
    if report.kept.is_empty() {
        let _ = std::fs::remove_dir(&app_dir);
    }
    report
}

/// Whether a name in an application directory is one the cache wrote.
///
/// Two shapes and no others: a bare `<key>` directory, which is an entry
/// whether or not it has a `ginary.json` in it, and the dotted
/// `.<key>.<tmp-|corrupt-|trash-><pid>` a half-finished extraction, a rejected
/// payload or an interrupted prune leaves behind. Anything else — an
/// `erl_crash.dump`, a note somebody left, a directory another tool put here —
/// is not the cache's to remove.
fn is_cache_residue(name: &str) -> bool {
    if is_cache_key(name) {
        return true;
    }
    let Some(rest) = name.strip_prefix('.') else {
        return false;
    };
    let Some((key, tail)) = rest.split_once('.') else {
        return false;
    };
    if !is_cache_key(key) {
        return false;
    }
    [TMP_PREFIX, CORRUPT_PREFIX, TRASH_PREFIX]
        .iter()
        .any(|prefix| match tail.strip_prefix(prefix) {
            Some(digits) => !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()),
            None => false,
        })
}

/// Whether a name is a cache key: [`crate::trailer::Trailer::cache_key`].
///
/// Sixteen lower-case hexadecimal digits, the first eight bytes of the
/// payload's digest.
fn is_cache_key(name: &str) -> bool {
    name.len() == CACHE_KEY_LEN
        && name
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// The length of a cache key: eight digest bytes in hexadecimal.
const CACHE_KEY_LEN: usize = 16;

/// Removes one path whatever it is, answering whether it is gone.
fn remove_anything(path: &Path) -> bool {
    let removed = match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => std::fs::remove_dir_all(path).is_ok(),
        Ok(_) => std::fs::remove_file(path).is_ok(),
        Err(_) => false,
    };
    removed || !path.exists()
}

/// Prunes every application under `root`, or one named application.
///
/// This is what `ginary cache prune` runs. `app` is checked before it is
/// joined, for the reason [`clean`] checks it: what this function does to a
/// directory is remove it.
///
/// # Errors
///
/// [`LauncherError::Cache`] when `app` is not a single path component. A
/// `root` that is not there is an empty report rather than an error, and a
/// directory that cannot be listed is skipped rather than fatal: pruning is
/// housekeeping and housekeeping does not fail a command.
pub fn prune(
    root: &Path,
    app: Option<&str>,
    options: PruneOptions,
    now: std::time::SystemTime,
) -> Result<PruneReport, LauncherError> {
    check_app_of(root, app)?;
    let root = walked(root);
    let app_dirs: Vec<PathBuf> = match app {
        Some(app) => vec![root.join(app)],
        None => match std::fs::read_dir(&root) {
            Ok(entries) => {
                let mut found: Vec<PathBuf> = entries
                    .filter_map(Result::ok)
                    .map(|entry| entry.path())
                    .collect();
                found.sort();
                found
            }
            // A cache that was never created has nothing to prune, and saying
            // so is not the same as failing.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(LauncherError::cache(reported(&root), error)),
        },
    };

    let diag = Diag::disabled();
    let mut report = PruneReport::default();
    for app_dir in app_dirs {
        let one = prune_app(&app_dir, None, options, now, &diag);
        report.removed.extend(one.removed);
        report.kept.extend(one.kept);
    }
    report.removed.sort();
    report.kept.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(report)
}

/// What one `ginary cache clean` removed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CleanReport {
    /// The directories removed, sorted.
    pub removed: Vec<PathBuf>,
    /// The total size of what was removed, in bytes.
    pub bytes: u64,
}

/// Removes cached extractions under `root`.
///
/// With `app` set, only that application's directory is emptied; without it,
/// every application under `root` is. The directories themselves go, temporary
/// and corrupt trees included, and `root` stays. `app` is checked before it is
/// joined: what this function does to a directory is remove it, so a value
/// that could name one outside `root` is refused rather than acted on.
///
/// # Errors
///
/// [`LauncherError::Cache`] when `app` is not a single path component — see
/// [`check_app`] — and when a directory cannot be listed or removed. A `root`
/// that does not exist is an empty report, not an error: cleaning a cache that
/// was never created is what the caller asked for.
pub fn clean(root: &Path, app: Option<&str>) -> Result<CleanReport, LauncherError> {
    check_app_of(root, app)?;
    let root = walked(root);
    let targets: Vec<PathBuf> = match app {
        Some(app) => vec![root.join(app)],
        None => match std::fs::read_dir(&root) {
            Ok(entries) => {
                let mut found: Vec<PathBuf> = entries
                    .filter_map(Result::ok)
                    .map(|entry| entry.path())
                    .collect();
                found.sort();
                found
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(LauncherError::cache(reported(&root), error)),
        },
    };

    let mut report = CleanReport::default();
    for target in targets {
        if !target.exists() {
            continue;
        }
        let (_, bytes) =
            measure(&target).map_err(|error| LauncherError::cache(reported(&target), error))?;
        std::fs::remove_dir_all(&target)
            .map_err(|source| LauncherError::cache(reported(&target), source))?;
        report.bytes = report.bytes.saturating_add(bytes);
        report.removed.push(reported(&target));
    }
    report.removed.sort();
    Ok(report)
}

/// Returns the value unless it is absent or empty.
///
/// An exported-but-empty variable is a common shell accident, and an empty path
/// would silently mean the current working directory.
fn non_empty(value: Option<&OsStr>) -> Option<&OsStr> {
    value.filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::target::Os;

    #[test]
    fn flush_open_options_asks_for_write_only_where_the_barrier_needs_it() {
        // The wiring, not just the rule: `open_for_flush` consults the
        // platform, so a Linux machine can assert every answer of it. Windows
        // needs a writable handle for `FlushFileBuffers`; unix flushes a
        // read-only one, and must, because a staged tree holds read-only files.
        assert_eq!(flush_open_options(Os::Linux), (true, false));
        assert_eq!(flush_open_options(Os::Macos), (true, false));
        assert_eq!(flush_open_options(Os::Windows), (true, true));
    }

    #[test]
    fn rename_aside_drops_the_lock_first_only_where_a_rename_refuses_open_children() {
        assert_eq!(
            rename_aside_order(Os::Windows),
            RenameAsideOrder::DropThenRename,
            "a Windows rename refuses a directory it holds a handle inside, so the lock is \
             released first"
        );
        assert_eq!(
            rename_aside_order(Os::Linux),
            RenameAsideOrder::RenameThenDrop
        );
        assert_eq!(
            rename_aside_order(Os::Macos),
            RenameAsideOrder::RenameThenDrop
        );
    }

    fn env(pairs: &[(&str, &str)]) -> Env {
        Env::from_pairs(
            pairs
                .iter()
                .map(|(key, value)| (OsString::from(*key), OsString::from(*value))),
        )
    }

    #[test]
    fn a_snapshot_answers_for_the_variables_it_holds() {
        let env = env(&[("HOME", "/home/u"), ("EMPTY", "")]);
        assert_eq!(env.get("HOME"), Some(OsStr::new("/home/u")));
        assert_eq!(env.get("EMPTY"), Some(OsStr::new("")));
        assert_eq!(env.get("ABSENT"), None);
        assert!(env.contains("EMPTY"));
        assert!(!env.contains("ABSENT"));
        assert_eq!(
            env.keys().collect::<Vec<_>>(),
            vec![OsStr::new("EMPTY"), OsStr::new("HOME")]
        );
    }

    #[test]
    fn ginary_cache_dir_wins_and_is_used_verbatim() {
        let dirs = resolve(
            &env(&[
                ("GINARY_CACHE_DIR", "/srv/c"),
                ("XDG_CACHE_HOME", "/xdg"),
                ("HOME", "/home/u"),
            ]),
            1000,
        );
        assert_eq!(dirs.root, PathBuf::from("/srv/c"));
        assert_eq!(dirs.origin, Origin::GinaryCacheDir);
        assert!(!dirs.is_fallback);
    }

    #[test]
    fn xdg_cache_home_gets_a_ginary_component() {
        let dirs = resolve(
            &env(&[("XDG_CACHE_HOME", "/xdg"), ("HOME", "/home/u")]),
            1000,
        );
        assert_eq!(dirs.root, PathBuf::from("/xdg/ginary"));
        assert_eq!(dirs.origin, Origin::XdgCacheHome);
    }

    #[test]
    fn a_windows_shaped_xdg_cache_home_is_ignored_by_the_unix_resolver() {
        // The other half of the rule, and the half a Windows host got wrong:
        // `Path::is_absolute` answers for the platform this was compiled for,
        // so `/xdg` was relative there and `C:\\xdg` would be absolute. Both
        // questions belong to the specification instead, and both are
        // therefore the same on every host — which is what makes this
        // assertion mean anything on a Linux machine. See
        // `tests/regressions/e7_the_xdg_rule_used_the_hosts_idea_of_an_absolute_path.rs`.
        let dirs = resolve(
            &env(&[("XDG_CACHE_HOME", "C:\\xdg"), ("HOME", "/home/u")]),
            1000,
        );
        assert_eq!(dirs.root, PathBuf::from("/home/u/.cache/ginary"));
        assert_eq!(
            dirs.origin,
            Origin::Home,
            "a Windows path is not a POSIX absolute path, and this is the POSIX resolver"
        );
    }

    #[test]
    fn home_gets_dot_cache_ginary() {
        let dirs = resolve(&env(&[("HOME", "/home/u")]), 1000);
        assert_eq!(dirs.root, PathBuf::from("/home/u/.cache/ginary"));
        assert_eq!(dirs.origin, Origin::Home);
    }

    #[test]
    fn a_relative_xdg_cache_home_is_ignored_and_a_relative_override_is_not() {
        assert_eq!(
            resolve(
                &env(&[("XDG_CACHE_HOME", "rel"), ("HOME", "/home/u")]),
                1000
            )
            .root,
            PathBuf::from("/home/u/.cache/ginary")
        );
        assert_eq!(
            resolve(&env(&[("GINARY_CACHE_DIR", "rel")]), 1000).root,
            PathBuf::from("rel")
        );
    }

    #[test]
    fn empty_values_count_as_unset() {
        let dirs = resolve(
            &env(&[
                ("GINARY_CACHE_DIR", ""),
                ("XDG_CACHE_HOME", ""),
                ("HOME", "/home/u"),
            ]),
            1000,
        );
        assert_eq!(dirs.root, PathBuf::from("/home/u/.cache/ginary"));
        assert_eq!(dirs.origin, Origin::Home);
    }

    #[test]
    fn nothing_set_falls_back_to_a_uid_named_directory_under_tmp() {
        let dirs = resolve(&env(&[]), 1000);
        assert_eq!(dirs.root, PathBuf::from("/tmp/ginary-1000"));
        assert_eq!(dirs.origin, Origin::Fallback);
        assert!(dirs.is_fallback);
    }

    #[test]
    fn the_fallback_honours_tmpdir() {
        assert_eq!(
            fallback_root(&env(&[("TMPDIR", "/scratch")]), 42),
            PathBuf::from("/scratch/ginary-42")
        );
        assert_eq!(
            fallback_root(&env(&[("TMPDIR", "")]), 42),
            PathBuf::from("/tmp/ginary-42")
        );
        assert_eq!(fallback_root(&env(&[]), 0), PathBuf::from("/tmp/ginary-0"));
    }

    #[test]
    fn every_origin_names_its_provenance() {
        assert_eq!(Origin::GinaryCacheDir.describe(), "GINARY_CACHE_DIR");
        assert_eq!(Origin::XdgCacheHome.describe(), "XDG_CACHE_HOME");
        assert_eq!(Origin::Home.describe(), "HOME");
        assert_eq!(Origin::Fallback.describe(), "TMPDIR fallback");
    }

    #[test]
    fn a_removal_reports_the_ordinary_spelling_of_what_it_walked() {
        // What `read_dir` hands back under a verbatim root: the prefix is on
        // every path, because a path joined onto a verbatim path is verbatim.
        assert_eq!(
            reported(Path::new(
                r"\\?\C:\Users\ada\AppData\Local\ginary\hello\0123456789abcdef"
            )),
            PathBuf::from(r"C:\Users\ada\AppData\Local\ginary\hello\0123456789abcdef"),
            "a prune table and an uninstall report are read by a person, and the verbatim \
             prefix is a fact about how the tree was opened rather than part of the path"
        );
        assert_eq!(
            reported(Path::new(r"\\?\UNC\server\share\ginary\hello")),
            PathBuf::from(r"\\server\share\ginary\hello")
        );
        assert_eq!(
            reported(Path::new("/home/ada/.cache/ginary/hello")),
            PathBuf::from("/home/ada/.cache/ginary/hello"),
            "and a path that never carried the prefix is untouched"
        );
    }

    #[test]
    #[cfg(not(windows))]
    fn a_removal_walks_what_it_was_given_on_unix() {
        let dir = Path::new("/home/ada/.cache/ginary/hello");
        assert_eq!(
            walked(dir),
            std::borrow::Cow::Borrowed(dir),
            "the prefix is Windows path syntax, so off Windows the walk borrows its argument"
        );
    }

    #[test]
    fn the_entry_path_is_root_app_key() {
        let dirs = CacheDirs {
            root: PathBuf::from("/c"),
            origin: Origin::Home,
            is_fallback: false,
        };
        assert_eq!(dirs.app_dir("hello"), PathBuf::from("/c/hello"));
        assert_eq!(
            dirs.key_dir("hello", "0123456789abcdef"),
            PathBuf::from("/c/hello/0123456789abcdef")
        );
    }
}
