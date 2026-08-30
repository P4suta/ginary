// SPDX-License-Identifier: MIT OR Apache-2.0
//! Launcher mode: what a packaged application does instead of parsing `argv`.
//!
//! [`run`] is reached from `main` when the running executable ends in a
//! trailer, and it never returns to the command line half. The sequence is
//! fixed:
//!
//! ```text
//! Diag::from_env
//!   GINARY_CMD set?  -> directory | extract-only | inspect, then exit
//!   read_manifest    seek to the payload, read entry 0, stop
//!   cache::prepare   resolve and create the cache root
//!   ensure_extracted the ten steps; the rename is the completion marker
//!   preflight        failed once -> remove the entry, extract again, check again
//!   launch::plan     argv and the environment difference
//!   launch::exec     execve, or supervise under GINARY_SUPERVISE=1
//! ```
//!
//! `GINARY_CMD` is read before anything is extracted because two of its three
//! values are questions about an artifact rather than requests to run it, and
//! because maintenance must stay out of `argv`: a packaged application owns
//! every argument it is given, `--help` included, so ginary's own commands
//! travel in the environment where they cannot collide.
//!
//! The preflight retry is the module's one piece of judgement. An extracted
//! tree that has lost a file is more likely to have been damaged after
//! extraction — a `tmpwatch`, a half-deleted cache — than to have been packed
//! wrong, so the entry is removed and extracted once more. A second failure is
//! [`crate::error::EXIT_CACHE`]: extracting a third time would be a loop, and
//! a loop is what a user reports as a hang.

use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::{Read as _, Seek as _, SeekFrom, Write as _};
use std::path::PathBuf;
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
pub const CMD_USAGE: &str = "usage: GINARY_CMD=directory|extract-only|inspect";

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

    match dispatch(&exe, &trailer, &env, &diag) {
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
            Ok(command) => maintenance(command, exe, trailer, env, diag),
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
    let user_args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let plan = launch::plan(&entry, &manifest, &user_args, env, &crash_dump_dir)?;

    if env.get(SUPERVISE_VAR) == Some(OsStr::new("1")) {
        return Ok(launch::supervise(plan, diag, &crash_dump_dir));
    }
    // `exec` returns only when the runtime did not start.
    Err(launch::exec(plan, diag))
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

/// The three `GINARY_CMD` values.
fn maintenance(
    command: Cmd,
    exe: &File,
    trailer: &Trailer,
    env: &Env,
    diag: &Diag,
) -> Result<ExitCode, LauncherError> {
    let manifest = read_manifest(exe, trailer, diag)?;
    let mut out = std::io::stdout();

    match command {
        Cmd::Directory => {
            // A question, not an instruction: nothing is created, so
            // `directory` on a cold machine still answers.
            let dirs = cache::resolve(env, cache::current_uid());
            let entry = dirs.key_dir(&manifest.app, &trailer.cache_key());
            let _ = writeln!(out, "{}", entry.display());
        }
        Cmd::ExtractOnly => {
            let dirs = cache::prepare(env, cache::current_uid(), &mut std::io::stderr())?;
            let entry = cache::ensure_extracted(exe, trailer, &manifest.app, &dirs, diag)?;
            let _ = writeln!(out, "{}", entry.display());
        }
        Cmd::Inspect => {
            let _ = writeln!(out, "{}", inspection(&manifest, trailer)?);
        }
    }
    let _ = out.flush();
    Ok(ExitCode::SUCCESS)
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
        _ => Err(value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_three_commands_are_recognised() {
        assert_eq!(parse_cmd(OsStr::new("directory")), Ok(Cmd::Directory));
        assert_eq!(parse_cmd(OsStr::new("extract-only")), Ok(Cmd::ExtractOnly));
        assert_eq!(parse_cmd(OsStr::new("inspect")), Ok(Cmd::Inspect));
    }

    #[test]
    fn anything_else_is_a_usage_error_carrying_the_value() {
        // Including the near misses: a command protocol that guesses is a
        // command protocol that extracts when it was asked to inspect.
        for value in ["Directory", "extract_only", "dir", "", "uninstall"] {
            assert_eq!(
                parse_cmd(OsStr::new(value)),
                Err(OsStr::new(value)),
                "`{value}` must not be recognised"
            );
        }
    }

    #[test]
    fn the_usage_line_names_all_three() {
        assert_eq!(
            CMD_USAGE,
            "usage: GINARY_CMD=directory|extract-only|inspect"
        );
        for name in ["directory", "extract-only", "inspect"] {
            assert!(CMD_USAGE.contains(name), "the usage must name `{name}`");
        }
    }
}
