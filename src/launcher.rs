// SPDX-License-Identifier: MIT OR Apache-2.0
//! Launcher mode: what a packaged application does instead of parsing `argv`.
//!
//! [`run`] is reached from `main` when the running executable ends in a
//! trailer, and it never returns to the command line half. The sequence is
//! fixed:
//!
//! ```text
//! Diag::from_env
//!   GINARY_CMD set?  -> directory | extract-only | inspect | selftest | uninstall, then exit
//!   read_manifest    seek to the payload, read entry 0, stop
//!   cache::prepare   resolve and create the cache root
//!   ensure_extracted the ten steps; the rename is the completion marker
//!   preflight        failed once -> remove the entry, extract again, check again
//!   cache::prune_app the stale siblings, best effort, never fatal
//!   lock_entry       flock(LOCK_SH) on <entry>/.lock, inherited across execve,
//!                    then one re-check that the entry survived being locked
//!   launch::plan     argv and the environment difference
//!   launch::exec     execve, or supervise under GINARY_SUPERVISE=1
//! ```
//!
//! `GINARY_CMD` is read before anything is extracted because three of its five
//! values are questions about an artifact rather than requests to run it, and
//! because maintenance must stay out of `argv`: a packaged application owns
//! every argument it is given, `--help` included, so ginary's own commands
//! travel in the environment where they cannot collide.
//!
//! Two steps between the preflight and the launch are this module's own.
//!
//! **Pruning is best effort and never fatal.** A stale sibling that cannot be
//! removed is a housekeeping problem, and housekeeping does not decide whether
//! an application starts. What was pruned goes to the trace, by name, and
//! nowhere else.
//!
//! **The lock is taken last, and then never released.** [`crate::cache_lock`]
//! explains why: `flock` belongs to the open file description, so a descriptor
//! without `FD_CLOEXEC` carries the lock through `execve` into the runtime, and
//! the kernel releases it when the runtime exits. Nothing has to remember to
//! unlock. A lock that cannot be taken is not a reason to refuse to run: it is
//! a pruning risk, and refusing would be worse. Taking it last also means the
//! entry is somebody else's to remove until the moment it is held, so
//! `lock_entry` re-checks the entry afterwards and extracts it once more if a
//! prune got there first.
//!
//! The preflight retry is the module's one piece of judgement. An extracted
//! tree that has lost a file is more likely to have been damaged after
//! extraction — a `tmpwatch`, a half-deleted cache — than to have been packed
//! wrong, so the entry is removed and extracted once more. A second failure is
//! [`crate::error::EXIT_CACHE`]: extracting a third time would be a loop, and
//! a loop is what a user reports as a hang.

use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::{Read as _, Seek as _, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::cache::{self, CacheDirs, Env};
use crate::diag::Diag;
use crate::error::LauncherError;
use crate::launch;
use crate::manifest::{MANIFEST_NAME, Manifest};
use crate::payload::PayloadError;
use crate::trailer::Trailer;

/// The variable that carries a maintenance command.
pub const CMD_VAR: &str = "GINARY_CMD";

/// The variable that selects spawn-and-wait instead of `execve`.
pub const SUPERVISE_VAR: &str = "GINARY_SUPERVISE";

/// What an unrecognised [`CMD_VAR`] prints on standard error.
pub const CMD_USAGE: &str = "usage: GINARY_CMD=directory|extract-only|inspect|selftest|uninstall";

/// How long `GINARY_CMD=selftest` gives the runtime to start and halt.
///
/// A runtime that starts at all starts in well under a second; the budget is
/// there so that one that never comes up is a reported failure rather than a
/// command that hangs, which is what a user reports as a broken artifact.
pub const SELFTEST_BUDGET: std::time::Duration = std::time::Duration::from_secs(30);

/// The exit code an unrecognised [`CMD_VAR`] leaves.
///
/// Two, not one of the launcher's own codes: this is a usage error made by
/// whoever set the variable, and it is the same number the command line half
/// leaves for a usage error.
pub const CMD_USAGE_EXIT: u8 = 2;

/// Decides which half of the binary this copy is.
///
/// [`Some`] means the running executable ends in a trailer and is a packaged
/// application; [`None`] means it is the command line tool. The open file is
/// handed back with the answer so that the payload is read from the same
/// descriptor the decision was made on — a file that is replaced between the
/// two would otherwise be read as if it were the one that was checked.
///
/// # Errors
///
/// [`crate::error::LauncherError::SelfExe`] when the running executable cannot
/// be opened, and [`crate::error::LauncherError::Trailer`] when the last 64
/// bytes begin the magic and then do not describe this file. The second is why
/// this is a function rather than a `match` in `main`: a damaged artifact must
/// never fall through to the command line and answer with ginary's help text.
pub fn mode() -> Result<Option<(File, PathBuf, Trailer)>, LauncherError> {
    let (exe, exe_path) = crate::selfexe::open_self()?;
    match Trailer::read_from(&exe)? {
        Some(trailer) => Ok(Some((exe, exe_path, trailer))),
        None => Ok(None),
    }
}

/// Runs a packaged application.
///
/// `exe` is the open running executable, positioned anywhere; `exe_path` is
/// where it was resolved from, for diagnostics only. Returns the exit code the
/// process should leave — which happens only when the runtime was never
/// started, since a successful launch replaces this process.
pub fn run(exe: File, exe_path: PathBuf, trailer: Trailer) -> ExitCode {
    // The first thing on the launcher path, so that what the panic hook does
    // with a launcher bug is a test rather than a claim. Compiled out without
    // the `fault-injection` feature.
    let _ = crate::fault::point("launcher");

    let diag = Diag::from_env(&crate::diag::EnvSnapshot::from_env());
    let env = Env::from_env();
    diag.kv(
        "start",
        &[
            ("exe", &exe_path.display().to_string()),
            ("key", &trailer.cache_key()),
        ],
    );

    match dispatch(&exe, &exe_path, &trailer, &env, &diag) {
        Ok(code) => code,
        Err(error) => {
            let _ = writeln!(std::io::stderr(), "{}", error.report());
            ExitCode::from(error.exit_code())
        }
    }
}

/// The whole launcher path, with every failure as a value.
fn dispatch(
    exe: &File,
    exe_path: &Path,
    trailer: &Trailer,
    env: &Env,
    diag: &Diag,
) -> Result<ExitCode, LauncherError> {
    // Before anything is read or extracted: two of the three commands are
    // questions about an artifact rather than requests to run it, and
    // maintenance travels in the environment because a packaged application
    // owns every argument it is given.
    if let Some(value) = env.get(CMD_VAR) {
        return match parse_cmd(value) {
            Ok(command) => maintenance(command, exe, exe_path, trailer, env, diag),
            Err(_) => {
                let _ = writeln!(std::io::stderr(), "{CMD_USAGE}");
                Ok(ExitCode::from(CMD_USAGE_EXIT))
            }
        };
    }

    let manifest = read_manifest(exe, trailer, diag)?;
    let dirs = cache::prepare(env, cache::current_uid(), &mut std::io::stderr())?;
    let entry = cache::ensure_extracted(exe, trailer, &manifest.app, &dirs, diag)?;
    let entry = repair_once(exe, trailer, &manifest, &dirs, entry, diag)?;

    // The dump belongs to the application rather than to the payload that
    // happened to produce it, so it goes one level above the entry.
    let crash_dump_dir = dirs.app_dir(&manifest.app);
    prune_siblings(&crash_dump_dir, trailer, env, diag);

    // Last, and never released: the descriptor is inherited across `execve`
    // and the kernel drops the lock when the runtime exits. A lock that could
    // not be taken is recorded and ignored — the application still runs, and
    // the only cost is that a concurrent prune could take its entry.
    let (entry, lock) = lock_entry(exe, trailer, &manifest, &dirs, entry, diag)?;

    let user_args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let plan = launch::plan(
        &entry,
        &manifest,
        &user_args,
        env,
        &crash_dump_dir,
        exe_path,
    )?;

    if env.get(SUPERVISE_VAR) == Some(OsStr::new("1")) {
        let code = launch::supervise(plan, diag, &crash_dump_dir);
        // Held for the whole of the supervised run, and dropped here rather
        // than at the top of the function, so that `GINARY_SUPERVISE=1` keeps
        // the entry for exactly as long as `execve` would have.
        drop(lock);
        return Ok(code);
    }
    // `exec` returns only when the runtime did not start, so reaching the next
    // line means the lock is being dropped by a process that never launched.
    let error = launch::exec(plan, diag);
    drop(lock);
    Err(error)
}

/// Takes the shared lock, and confirms the entry survived being locked.
///
/// The lock is the last thing the launcher does before `execve`, and until it
/// is held the entry is somebody else's to remove: a prune holds the exclusive
/// lock only across its `rename`, so a launcher that arrives after the rename
/// finds no lock file at all and would otherwise `execve` into a tree that is
/// being deleted. So the entry is re-checked *after* the lock, and an entry
/// that has gone is extracted once more and locked again.
///
/// One retry, for the reason [`repair_once`] has one: a second disappearance
/// is not a race, it is a machine removing this cache as fast as ginary can
/// write it, and a loop is what a user reports as a hang.
///
/// A lock that could not be taken at all is recorded and ignored. That is not
/// the same as an entry that vanished: the first is a housekeeping risk, the
/// second is a tree that is not there.
fn lock_entry(
    exe: &File,
    trailer: &Trailer,
    manifest: &Manifest,
    dirs: &CacheDirs,
    entry: PathBuf,
    diag: &Diag,
) -> Result<(PathBuf, Option<crate::cache_lock::SharedLock>), LauncherError> {
    // The window this function exists for, made deterministic: the fault point
    // removes the entry at exactly the moment a winning prune would have.
    if crate::fault::point("before-lock").is_some() {
        let _ = std::fs::remove_dir_all(&entry);
    }

    let lock = take_lock(&entry, diag);
    if entry.join(MANIFEST_NAME).is_file() {
        return Ok((entry, lock));
    }
    diag.kv("lock_retry", &[("entry", &entry.display().to_string())]);
    drop(lock);

    let entry = cache::ensure_extracted(exe, trailer, &manifest.app, dirs, diag)?;
    let lock = take_lock(&entry, diag);
    if entry.join(MANIFEST_NAME).is_file() {
        return Ok((entry, lock));
    }
    Err(LauncherError::cache(
        &entry,
        std::io::Error::other("the cache entry was removed while it was being locked"),
    ))
}

/// The shared lock on one entry, or [`None`] and a trace record.
fn take_lock(entry: &Path, diag: &Diag) -> Option<crate::cache_lock::SharedLock> {
    match crate::cache_lock::SharedLock::acquire(entry) {
        Ok(lock) => Some(lock),
        Err(error) => {
            diag.kv("lock", &[("error", &error.to_string())]);
            None
        }
    }
}

/// Removes the stale siblings of the entry that is about to be launched.
///
/// Best effort and never fatal, which is the whole of its contract: a
/// directory that cannot be listed, an entry another process holds and one
/// that cannot be removed are all left alone, and none of them says anything
/// on standard error. What was pruned goes to the trace, because an entry that
/// vanished is a thing a bug report has to be able to explain.
fn prune_siblings(app_dir: &Path, trailer: &Trailer, env: &Env, diag: &Diag) {
    let days = cache::prune_days(env);
    let options = cache::PruneOptions { days, all: false };
    let _ = cache::prune_app(
        app_dir,
        Some(&trailer.cache_key()),
        options,
        std::time::SystemTime::now(),
        diag,
    );
}

/// Preflight, and one repair if it fails.
///
/// An extracted tree that has lost a file is far more likely to have been
/// damaged *after* extraction — a `tmpwatch`, a half-finished `rm -rf` — than
/// to have been packed wrong, so the entry is removed and extracted once more.
/// A second failure is [`crate::error::EXIT_CACHE`] and names the file:
/// extracting a third time would be a loop, and a loop is what a user reports
/// as a hang.
fn repair_once(
    exe: &File,
    trailer: &Trailer,
    manifest: &Manifest,
    dirs: &CacheDirs,
    entry: PathBuf,
    diag: &Diag,
) -> Result<PathBuf, LauncherError> {
    let issue = match launch::preflight(&entry, manifest) {
        Ok(()) => return Ok(entry),
        Err(issue) => issue,
    };
    diag.kv("preflight_retry", &[("issue", &issue.to_string())]);

    std::fs::remove_dir_all(&entry).map_err(|source| LauncherError::cache(&entry, source))?;
    let entry = cache::ensure_extracted(exe, trailer, &manifest.app, dirs, diag)?;
    match launch::preflight(&entry, manifest) {
        Ok(()) => Ok(entry),
        Err(issue) => Err(LauncherError::cache(
            entry,
            std::io::Error::other(issue.to_string()),
        )),
    }
}

/// Reads entry 0 of the payload and stops.
///
/// A seek and a few kilobytes: the manifest is the first entry precisely so
/// that the cache path can be computed, and `inspect` answered, without
/// touching the rest of a payload that may be tens of megabytes. What comes
/// back has passed [`crate::manifest::Manifest::validate`], so every path the
/// caller builds from it stays under the roots it was joined onto.
fn read_manifest(exe: &File, trailer: &Trailer, diag: &Diag) -> Result<Manifest, LauncherError> {
    let _phase = diag.phase("read_manifest");
    let mut source = exe.try_clone().map_err(LauncherError::SelfExe)?;
    source
        .seek(SeekFrom::Start(trailer.payload_offset))
        .map_err(LauncherError::SelfExe)?;
    let manifest = crate::payload::read_manifest(source.take(trailer.payload_len))?;
    // Before the application name reaches a `join`. `app` is the `<app>`
    // component of every cache path, and every path below this point is built
    // from it: a manifest that named `../escape` would otherwise have the
    // launcher create, chmod and extract into a directory outside its own
    // cache root.
    manifest.validate()?;
    Ok(manifest)
}

/// The five `GINARY_CMD` values.
fn maintenance(
    command: Cmd,
    exe: &File,
    exe_path: &Path,
    trailer: &Trailer,
    env: &Env,
    diag: &Diag,
) -> Result<ExitCode, LauncherError> {
    let manifest = read_manifest(exe, trailer, diag)?;
    let mut out = std::io::stdout();

    let code = match command {
        Cmd::Directory => {
            // A question, not an instruction: nothing is created, so
            // `directory` on a cold machine still answers.
            let dirs = cache::resolve(env, cache::current_uid());
            let entry = dirs.key_dir(&manifest.app, &trailer.cache_key());
            let _ = writeln!(out, "{}", entry.display());
            ExitCode::SUCCESS
        }
        Cmd::ExtractOnly => {
            let dirs = cache::prepare(env, cache::current_uid(), &mut std::io::stderr())?;
            let entry = cache::ensure_extracted(exe, trailer, &manifest.app, &dirs, diag)?;
            let _ = writeln!(out, "{}", entry.display());
            ExitCode::SUCCESS
        }
        Cmd::Inspect => {
            let _ = writeln!(out, "{}", inspection(&manifest, trailer)?);
            ExitCode::SUCCESS
        }
        Cmd::Uninstall => {
            // `resolve`, not `prepare`: uninstalling on a machine that never
            // ran this artifact must not create the cache it is emptying.
            let dirs = cache::resolve(env, cache::current_uid());
            cache::check_app(&dirs.root, &manifest.app)?;
            let report = cache::uninstall(&dirs.app_dir(&manifest.app));
            let _ = write!(out, "{}", render_prune(&report));
            // Zero even when something was kept: a partial uninstall is a fact
            // the caller has been told, not a failure of the command.
            ExitCode::SUCCESS
        }
        Cmd::SelfTest => {
            let dirs = cache::prepare(env, cache::current_uid(), &mut std::io::stderr())?;
            selftest(exe, exe_path, trailer, &manifest, &dirs, diag, &mut out)
        }
    };
    let _ = out.flush();
    Ok(code)
}

/// The table `GINARY_CMD=uninstall` and `ginary cache prune` both print.
///
/// One renderer so that the two commands cannot drift into describing the same
/// outcome two different ways: what was removed, what stayed and why, and a
/// summary that counts both columns even when both are zero.
pub fn render_prune(report: &cache::PruneReport) -> String {
    let mut text = String::new();
    for path in &report.removed {
        text.push_str(&format!("removed {}\n", path.display()));
    }
    for (path, reason) in &report.kept {
        text.push_str(&format!(
            "kept {} ({})\n",
            path.display(),
            reason.describe()
        ));
    }
    text.push_str(&format!(
        "total: {} removed, {} kept\n",
        report.removed.len(),
        report.kept.len()
    ));
    text
}

/// Extracts, checks and starts the runtime, reporting each step.
///
/// Three steps, in the order the launcher does them, each `PASS` or
/// `FAIL: <what went wrong>`. The run is the manifest's own plan with two
/// changes: `-eval` is given [`launch::HALT_EVAL`] instead of the
/// application's expression, and everything from `-extra` on is dropped, so a
/// selftest starts the runtime, stops it, and runs no application code at all.
///
/// The first failure ends the report: there is nothing to preflight in a tree
/// that was never extracted, and nothing to run in one that failed preflight.
fn selftest(
    exe: &File,
    exe_path: &Path,
    trailer: &Trailer,
    manifest: &Manifest,
    dirs: &CacheDirs,
    diag: &Diag,
    out: &mut impl Write,
) -> ExitCode {
    let entry = match cache::ensure_extracted(exe, trailer, &manifest.app, dirs, diag) {
        Ok(entry) => {
            let _ = writeln!(out, "extract: PASS");
            entry
        }
        Err(error) => {
            let _ = writeln!(out, "extract: FAIL: {error}");
            return ExitCode::FAILURE;
        }
    };

    if let Err(issue) = launch::preflight(&entry, manifest) {
        let _ = writeln!(out, "preflight: FAIL: {issue}");
        return ExitCode::FAILURE;
    }
    let _ = writeln!(out, "preflight: PASS");

    let crash_dump_dir = dirs.app_dir(&manifest.app);
    let plan = match launch::plan(
        &entry,
        manifest,
        &[],
        &Env::default(),
        &crash_dump_dir,
        exe_path,
    ) {
        Ok(plan) => launch::halt_plan(&plan),
        Err(error) => {
            let _ = writeln!(out, "run: FAIL: {error}");
            return ExitCode::FAILURE;
        }
    };
    match launch::run_bounded(plan, diag, SELFTEST_BUDGET) {
        Ok(()) => {
            let _ = writeln!(out, "run: PASS");
            ExitCode::SUCCESS
        }
        Err(issue) => {
            let _ = writeln!(out, "run: FAIL: {issue}");
            ExitCode::FAILURE
        }
    }
}

/// The object `GINARY_CMD=inspect` prints: the manifest and the geometry.
fn inspection(manifest: &Manifest, trailer: &Trailer) -> Result<String, LauncherError> {
    let serialise = |name: &str, error: serde_json::Error| {
        LauncherError::Payload(PayloadError::Serialise {
            name: name.to_owned(),
            source: error,
        })
    };

    let mut object = serde_json::Map::new();
    object.insert(
        "manifest".to_owned(),
        serde_json::to_value(manifest).map_err(|error| serialise(MANIFEST_NAME, error))?,
    );
    let mut geometry = serde_json::Map::new();
    geometry.insert("payload_offset".to_owned(), trailer.payload_offset.into());
    geometry.insert("payload_len".to_owned(), trailer.payload_len.into());
    geometry.insert(
        "sha256".to_owned(),
        hex::encode(trailer.payload_sha256).into(),
    );
    geometry.insert("cache_key".to_owned(), trailer.cache_key().into());
    object.insert("trailer".to_owned(), geometry.into());

    serde_json::to_string_pretty(&serde_json::Value::Object(object))
        .map_err(|error| serialise("inspect", error))
}

/// A maintenance command from [`CMD_VAR`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cmd {
    /// Print the entry directory this artifact would use, and exit.
    Directory,
    /// Extract, print the entry directory, and exit without launching.
    ExtractOnly,
    /// Print the manifest, the trailer's geometry and the digest, and exit.
    Inspect,
    /// Extract, preflight, start the runtime and halt it, reporting each step.
    SelfTest,
    /// Remove every cache entry of this application that nobody is holding.
    Uninstall,
}

/// Recognises a maintenance command, or returns the value that is not one.
///
/// Exact matches only. A protocol that guesses is a protocol that extracts
/// when it was asked to inspect.
fn parse_cmd(value: &OsStr) -> Result<Cmd, &OsStr> {
    match value.to_str() {
        Some("directory") => Ok(Cmd::Directory),
        Some("extract-only") => Ok(Cmd::ExtractOnly),
        Some("inspect") => Ok(Cmd::Inspect),
        Some("selftest") => Ok(Cmd::SelfTest),
        Some("uninstall") => Ok(Cmd::Uninstall),
        _ => Err(value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_five_commands_are_recognised() {
        assert_eq!(parse_cmd(OsStr::new("directory")), Ok(Cmd::Directory));
        assert_eq!(parse_cmd(OsStr::new("extract-only")), Ok(Cmd::ExtractOnly));
        assert_eq!(parse_cmd(OsStr::new("inspect")), Ok(Cmd::Inspect));
        assert_eq!(parse_cmd(OsStr::new("selftest")), Ok(Cmd::SelfTest));
        assert_eq!(parse_cmd(OsStr::new("uninstall")), Ok(Cmd::Uninstall));
    }

    #[test]
    fn anything_else_is_a_usage_error_carrying_the_value() {
        // Including the near misses: a command protocol that guesses is a
        // command protocol that extracts when it was asked to inspect.
        for value in [
            "Directory",
            "extract_only",
            "dir",
            "",
            "reinstall",
            "self-test",
            "Uninstall",
            "prune",
        ] {
            assert_eq!(
                parse_cmd(OsStr::new(value)),
                Err(OsStr::new(value)),
                "`{value}` must not be recognised"
            );
        }
    }

    #[test]
    fn the_usage_line_names_all_five() {
        assert_eq!(
            CMD_USAGE,
            "usage: GINARY_CMD=directory|extract-only|inspect|selftest|uninstall"
        );
        for name in [
            "directory",
            "extract-only",
            "inspect",
            "selftest",
            "uninstall",
        ] {
            assert!(CMD_USAGE.contains(name), "the usage must name `{name}`");
        }
    }

    #[test]
    fn an_empty_prune_report_is_still_a_summary() {
        assert_eq!(
            render_prune(&cache::PruneReport::default()),
            "total: 0 removed, 0 kept\n",
            "a command that removed nothing has to say so, or a caller cannot tell it ran"
        );
    }

    #[test]
    fn a_prune_report_names_what_went_and_why_what_stayed_did() {
        let report = cache::PruneReport {
            removed: vec![PathBuf::from("/c/hello/aaa")],
            kept: vec![
                (PathBuf::from("/c/hello/bbb"), cache::KeptReason::Locked),
                (PathBuf::from("/c/hello/ccc"), cache::KeptReason::Fresh),
                (
                    PathBuf::from("/c/hello/ddd"),
                    cache::KeptReason::Unremovable,
                ),
            ],
        };
        assert_eq!(
            render_prune(&report),
            "removed /c/hello/aaa\n\
             kept /c/hello/bbb (locked)\n\
             kept /c/hello/ccc (fresh)\n\
             kept /c/hello/ddd (unremovable)\n\
             total: 1 removed, 3 kept\n",
            "every reason a prune has is a word the table prints"
        );
    }
}
