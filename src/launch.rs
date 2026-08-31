// SPDX-License-Identifier: MIT OR Apache-2.0
//! The argument vector and the environment the runtime is started with.
//!
//! [`plan`] is a pure function. It takes the extracted root, the manifest, the
//! user's arguments and a snapshot of the environment, and returns a
//! [`LaunchPlan`] — a program, an argv, a list of variables to set and a list
//! to remove. Nothing is executed and nothing is read from disk, which is what
//! makes the launcher's most consequential decision a unit test rather than a
//! subprocess: `GINARY_TRACE` records exactly this value immediately before
//! `execve`, so a bug report can carry the launch that failed.
//!
//! ## The argument vector
//!
//! In this order, and the order is the contract:
//!
//! ```text
//! [-args_file <root>/<launch.args_file>]
//! -boot <root>/<launch.boot>
//! -noshell
//! +B
//! +fnu | +fnl | +fna                        from launch.filename_encoding
//! [-start_epmd false]                       absent when launch.distribution
//! [-config <root>/<launch.config>]
//! [-heart]                                  when launch.heart
//! -pa <root>/<launch.pa[0]>  ...  -pa <root>/<launch.pa[n]>
//! <launch.erl_flags ...>
//! <GINARY_ERL_FLAGS split on ASCII whitespace>
//! -eval <launch.eval>
//! -extra
//! <the user's arguments, byte for byte>
//! ```
//!
//! The args file comes **first**, before anything ginary passes. `erl` takes
//! the last value of a repeated flag, so a user file placed after `-noshell`
//! could switch the shell back on; placed before it, the user's flags are
//! defaults that ginary's own flags override. That is the whole reason for the
//! position, and it is why `docs/format.md` documents the file as a set of
//! defaults rather than as overrides.
//!
//! `-extra` is the last thing ginary contributes. Everything after it is the
//! application's, carried as [`OsString`] so that an argument which is not
//! valid UTF-8 arrives as the bytes the user typed. The launcher never looks
//! at those bytes: `--help` belongs to the packaged application.
//!
//! ## The environment
//!
//! Set: `ROOTDIR`, `BINDIR`, `EMU`, `PROGNAME`, and — only when the user has
//! not set them — `HOME` and `ERL_CRASH_DUMP`. The first four are what
//! `erlexec` would have derived from its own path if it were the real one; the
//! last two are defaults, not overrides, because a user who set `HOME` meant
//! it. Then the manifest's own `launch.env`, each only when the caller has not
//! set it, and `HEART_COMMAND` when the manifest asks for `heart`.
//!
//! Removed: [`REMOVED_VARS`], plus every variable whose name begins `ERL_OTP`
//! and ends `_FLAGS`. Those carry emulator flags from the developer's own
//! Erlang installation into an artifact that ships its own, and a packaged
//! application that behaves differently on one machine because of an exported
//! `ERL_AFLAGS` is a support case nobody can reproduce.
//!
//! The removal list is built *before* `launch.env` is applied, and a name in
//! both is not set: `env` may not put back what the scrub took out. The build
//! refuses such a name, and this is the launcher's half of the same rule.

use std::ffi::{OsStr, OsString};
use std::io::Write as _;
use std::os::unix::ffi::{OsStrExt as _, OsStringExt as _};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::cache::Env;
use crate::diag::Diag;
use crate::error::{HINT_EXEC_EACCES, HINT_EXEC_ENOENT, LauncherError, PREFIX};
use crate::manifest::Manifest;

/// The variables the launcher removes by name, in the order they are removed.
pub const REMOVED_VARS: [&str; 6] = [
    "ERL_LIBS",
    "ERL_FLAGS",
    "ERL_AFLAGS",
    "ERL_ZFLAGS",
    "ERL_ROOTDIR",
    "ERL_EPMD_PORT",
];

/// The prefix of the variable family the launcher removes by pattern.
pub const REMOVED_PREFIX: &str = "ERL_OTP";

/// The suffix of the variable family the launcher removes by pattern.
pub const REMOVED_SUFFIX: &str = "_FLAGS";

/// Extra emulator flags for one run, split on ASCII whitespace.
pub const ERL_FLAGS_VAR: &str = "GINARY_ERL_FLAGS";

/// The name of the crash dump the runtime writes.
pub const CRASH_DUMP_NAME: &str = "erl_crash.dump";

/// The variable `heart` re-runs the application with.
///
/// Set only when the manifest asks for `heart`, and only when the caller has
/// not set one: a `HEART_COMMAND` the user exported is a deliberate
/// supervision policy and the launcher does not know better than it.
pub const HEART_COMMAND_VAR: &str = "HEART_COMMAND";

/// The programs [`preflight`] checks under the manifest's bindir, besides the
/// launch program itself.
pub const REQUIRED_BINARIES: [&str; 3] = ["beam.smp", "erl_child_setup", "inet_gethost"];

/// The expression `GINARY_CMD=selftest` gives `-eval` instead of the
/// manifest's.
///
/// A selftest answers one question — can this artifact start its runtime on
/// this machine — and running the application to answer it would be answering
/// a different one. The runtime comes up, halts, and no application code runs.
pub const HALT_EVAL: &str = "erlang:halt(0)";

/// Everything needed to start the runtime, and nothing else.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchPlan {
    /// The absolute path of the program to execute.
    pub program: PathBuf,
    /// The arguments after `argv[0]`.
    pub args: Vec<OsString>,
    /// Variables to set, in the order they are set.
    pub set: Vec<(OsString, OsString)>,
    /// Variables to remove, in the order they are removed.
    pub remove: Vec<OsString>,
}

/// Why a cache entry cannot be launched.
///
/// This is not a [`LauncherError`]: a failed preflight is a *suspicion* that
/// the entry is damaged, and the launcher's answer to it is to extract again
/// rather than to give up. Only the second failure becomes
/// [`LauncherError::Cache`].
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PreflightIssue {
    /// A file the runtime needs is not in the extracted tree.
    #[error("`{path}` is missing from the extracted runtime")]
    Missing {
        /// The file that is not there.
        path: PathBuf,
    },
    /// A program the runtime needs is there and cannot be run.
    #[error("`{path}` is in the extracted runtime and is not executable")]
    NotExecutable {
        /// The program whose execute bit is clear.
        path: PathBuf,
    },
}

/// Builds the plan for one launch.
///
/// `crash_dump_dir` is where `ERL_CRASH_DUMP` points when the user has not set
/// it — the *application* directory rather than the entry, so a dump survives
/// the cache entry that produced it and a user can find it after an upgrade.
/// `self_exe` is the running artifact, which is what `HEART_COMMAND` names
/// when the manifest asks for `heart`: nothing else on the machine can restart
/// this application.
///
/// # Errors
///
/// [`LauncherError::Payload`] when the manifest holds a name or a path that
/// does not stay under the extracted root. The check is
/// [`crate::manifest::Manifest::validate`] and it is made here, at the last
/// moment before the values are interpolated, rather than trusted from
/// extraction time.
pub fn plan(
    root: &Path,
    m: &Manifest,
    user_args: &[OsString],
    env: &Env,
    crash_dump_dir: &Path,
    self_exe: &Path,
) -> Result<LaunchPlan, LauncherError> {
    m.validate()?;

    let bindir = root.join(&m.launch.bindir);
    let program = bindir.join(&m.launch.program);

    let mut args: Vec<OsString> = Vec::new();

    // First of all, before ginary's own flags: `erl` takes the last value of a
    // repeated flag, so the user's file is a set of defaults this vector then
    // overrides rather than a set of overrides.
    if let Some(args_file) = &m.launch.args_file {
        args.push(OsString::from("-args_file"));
        args.push(root.join(args_file).into_os_string());
    }

    // The fixed head, in the order `docs/format.md` fixes it. `-start_epmd`
    // and `false` are two argv items, not one: the flag takes its value as a
    // separate argument, and an `erlexec` given `-start_epmd false` as a single
    // word would start the daemon this artifact deliberately does not ship.
    args.push(OsString::from("-boot"));
    args.push(root.join(&m.launch.boot).into_os_string());
    args.push(OsString::from("-noshell"));
    args.push(OsString::from("+B"));
    // A manifest whose encoding this build has no flag for contributes none.
    // The build refuses such a value, so the only way here is an artifact from
    // a newer ginary, and passing an emulator flag ginary invented would be
    // worse than leaving the runtime on its own default.
    if let Some(flag) = crate::config::filename_encoding_flag(&m.launch.filename_encoding) {
        args.push(OsString::from(flag));
    }
    // A distributed artifact bundles `epmd` and lets the runtime start it;
    // every other artifact ships no daemon and has to say so.
    if !m.launch.distribution {
        args.push(OsString::from("-start_epmd"));
        args.push(OsString::from("false"));
    }
    if let Some(config) = &m.launch.config {
        args.push(OsString::from("-config"));
        args.push(root.join(config).into_os_string());
    }
    if m.launch.heart {
        args.push(OsString::from("-heart"));
    }
    for entry in &m.launch.pa {
        args.push(OsString::from("-pa"));
        args.push(root.join(entry).into_os_string());
    }
    for flag in &m.launch.erl_flags {
        args.push(OsString::from(flag));
    }
    args.extend(split_flags(env.get(ERL_FLAGS_VAR)));
    args.push(OsString::from("-eval"));
    args.push(OsString::from(&m.launch.eval));
    // Unconditional: without it the first user argument would be read as an
    // emulator flag, and a packaged application owns every argument it is given.
    args.push(OsString::from("-extra"));
    args.extend(user_args.iter().cloned());

    // Built before the environment is: `launch.env` is applied after the
    // scrub, so a name in both lists has to be recognisable as one that was
    // removed rather than set.
    let mut remove: Vec<OsString> = REMOVED_VARS.iter().map(OsString::from).collect();
    remove.extend(
        env.keys()
            .filter(|name| is_otp_flags_family(name))
            .map(OsStr::to_os_string),
    );

    let mut set: Vec<(OsString, OsString)> = vec![
        (OsString::from("ROOTDIR"), root.as_os_str().to_os_string()),
        (OsString::from("BINDIR"), bindir.into_os_string()),
        (OsString::from("EMU"), OsString::from("beam")),
        (OsString::from("PROGNAME"), OsString::from(&m.app)),
    ];
    // Defaults, not overrides: a user who exported `HOME` meant it, and an
    // empty value is a value.
    if !env.contains("HOME") {
        set.push((OsString::from("HOME"), root.as_os_str().to_os_string()));
    }
    if !env.contains("ERL_CRASH_DUMP") {
        set.push((
            OsString::from("ERL_CRASH_DUMP"),
            crash_dump_dir.join(CRASH_DUMP_NAME).into_os_string(),
        ));
    }
    // The names the launcher decides for itself, `HEART_COMMAND` included:
    // it is pushed below rather than above, and a manifest may not get in
    // first. `config::REJECTED_ENV_NAMES` refuses these at build time, so only
    // a hand-written or older manifest reaches this, which is exactly the
    // input the launcher may not trust.
    let mut claimed: Vec<OsString> = set.iter().map(|(name, _)| name.clone()).collect();
    if m.launch.heart {
        claimed.push(OsString::from(HEART_COMMAND_VAR));
    }
    // The artifact's own defaults, on the same terms: only when the caller has
    // said nothing, never a name the scrub above has just taken out, and never
    // one the launcher has already decided.
    for (name, value) in &m.launch.env {
        let key = OsString::from(name);
        if env.contains(name) || remove.contains(&key) || claimed.contains(&key) {
            continue;
        }
        set.push((key, OsString::from(value)));
    }
    // The command `heart` re-runs, which can only be this artifact: nothing
    // else on the machine knows how to start this application. A
    // `HEART_COMMAND` the caller exported is a supervision policy of their own.
    if m.launch.heart && !env.contains(HEART_COMMAND_VAR) {
        set.push((
            OsString::from(HEART_COMMAND_VAR),
            heart_command(self_exe, user_args),
        ));
    }

    Ok(LaunchPlan {
        program,
        args,
        set,
        remove,
    })
}

/// `HEART_COMMAND`: the running artifact, then the arguments it was given.
///
/// Space-separated, because that is the only shape `heart` has: it hands the
/// value to `/bin/sh -c`. That is also why every element is quoted when it
/// needs to be — a path with a space in it, an argument with a space, a `;` or
/// a `$(` — because the shell splits and expands the value again and a restart
/// that ran half the path would be worse than no restart at all. A run with no
/// arguments is the path alone, without a trailing space, and an ordinary path
/// and ordinary arguments come through unquoted, so that the common case is a
/// command a user can read and paste.
fn heart_command(self_exe: &Path, user_args: &[OsString]) -> OsString {
    let mut command = shell_word(self_exe.as_os_str());
    for argument in user_args {
        command.push(OsStr::new(" "));
        command.push(shell_word(argument));
    }
    command
}

/// One element of a `/bin/sh` command line, quoted if the shell would touch it.
///
/// Single quotes, because inside them the shell expands nothing at all; an
/// embedded `'` ends the quoting, escapes itself and starts it again, which is
/// the one construction `sh` has for it. On bytes rather than characters: the
/// value reaches `execve` as bytes and an argument that is not valid UTF-8 must
/// survive.
fn shell_word(word: &OsStr) -> OsString {
    let bytes = word.as_bytes();
    if !bytes.is_empty() && bytes.iter().all(|byte| is_shell_safe(*byte)) {
        return word.to_os_string();
    }
    let mut quoted = Vec::with_capacity(bytes.len().saturating_add(2));
    quoted.push(b'\'');
    for byte in bytes {
        if *byte == b'\'' {
            quoted.extend_from_slice(b"'\\''");
        } else {
            quoted.push(*byte);
        }
    }
    quoted.push(b'\'');
    OsString::from_vec(quoted)
}

/// Whether a byte is one `/bin/sh` gives back unchanged, unquoted.
///
/// The conservative set: letters, digits and the punctuation that appears in
/// paths and option arguments. Everything else — whitespace, quotes, `$`, `;`,
/// `*`, a byte that is not ASCII — earns the quoting.
const fn is_shell_safe(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'_' | b'@' | b'%' | b'+' | b'=' | b':' | b',' | b'.' | b'/' | b'-'
        )
}

/// Splits `GINARY_ERL_FLAGS` on ASCII whitespace, as a shell would.
///
/// On bytes rather than on characters: the value reaches `execve` as bytes and
/// a flag argument that is not valid UTF-8 must survive, so the split cannot
/// go through [`str`].
fn split_flags(value: Option<&OsStr>) -> Vec<OsString> {
    let Some(value) = value else {
        return Vec::new();
    };
    value
        .as_bytes()
        .split(|byte| byte.is_ascii_whitespace())
        .filter(|field| !field.is_empty())
        .map(|field| OsString::from_vec(field.to_vec()))
        .collect()
}

/// Whether a variable name is one of the `ERL_OTP...\_FLAGS` family.
///
/// Both ends must match. `ERL_OTP29` and `ERL_OTP29_FLAGS_EXTRA` are variables
/// the user owns, and scrubbing one of those would be the launcher deleting
/// something it was never told about.
fn is_otp_flags_family(name: &OsStr) -> bool {
    let bytes = name.as_bytes();
    bytes.starts_with(REMOVED_PREFIX.as_bytes()) && bytes.ends_with(REMOVED_SUFFIX.as_bytes())
}

/// Checks that the extracted tree holds a runnable runtime.
///
/// The manifest's launch program, [`REQUIRED_BINARIES`] under the manifest's
/// bindir, and `<root>/<launch.boot>.boot`. Programs must have the user
/// execute bit; the boot file only has to exist.
///
/// # Errors
///
/// [`PreflightIssue`] naming the first file that is missing or not executable.
/// The order is the order above, so the message a user sees names the most
/// fundamental missing piece rather than whichever one a directory listing
/// happened to reach first.
pub fn preflight(root: &Path, m: &Manifest) -> Result<(), PreflightIssue> {
    let bindir = root.join(&m.launch.bindir);
    check_program(&bindir.join(&m.launch.program))?;
    for name in REQUIRED_BINARIES {
        check_program(&bindir.join(name))?;
    }

    let boot = root.join(format!("{}.boot", m.launch.boot));
    if !boot.is_file() {
        return Err(PreflightIssue::Missing { path: boot });
    }
    Ok(())
}

/// One program: it must be there, and it must have the user execute bit.
fn check_program(path: &Path) -> Result<(), PreflightIssue> {
    use std::os::unix::fs::PermissionsExt as _;

    let Ok(metadata) = std::fs::metadata(path) else {
        return Err(PreflightIssue::Missing {
            path: path.to_path_buf(),
        });
    };
    if metadata.permissions().mode() & 0o100 == 0 {
        return Err(PreflightIssue::NotExecutable {
            path: path.to_path_buf(),
        });
    }
    Ok(())
}

/// The plan a selftest runs: the same launch, with nothing of the application.
///
/// Two changes to `plan`'s output and no others, so that what a selftest
/// exercises is the argument vector the artifact would really use: the value
/// after `-eval` becomes [`HALT_EVAL`], and everything from `-extra` onwards is
/// dropped. A plan with neither is left alone, which is what a manifest a newer
/// ginary wrote might produce.
pub fn halt_plan(plan: &LaunchPlan) -> LaunchPlan {
    let mut args = plan.args.clone();
    if let Some(position) = args.iter().position(|argument| argument == "-eval")
        && let Some(expression) = args.get_mut(position + 1)
    {
        *expression = OsString::from(HALT_EVAL);
    }
    if let Some(position) = args.iter().position(|argument| argument == "-extra") {
        args.truncate(position);
    }
    LaunchPlan {
        program: plan.program.clone(),
        args,
        set: plan.set.clone(),
        remove: plan.remove.clone(),
    }
}

/// Why one bounded run of the runtime did not end cleanly.
#[derive(Debug, thiserror::Error)]
pub enum RunIssue {
    /// The program could not be started at all.
    #[error("the runtime could not be started: {source}")]
    Start {
        /// What the operating system said.
        #[source]
        source: std::io::Error,
    },
    /// It started and left a non-zero exit code.
    #[error("the runtime exited {code} rather than 0")]
    Exit {
        /// The code it left.
        code: i32,
    },
    /// It was killed by a signal.
    #[error("the runtime was killed by signal {signal}")]
    Signal {
        /// The signal that killed it.
        signal: i32,
    },
    /// It was still running when the budget ran out, and was killed.
    #[error("the runtime did not exit within {seconds} seconds and was killed")]
    Timeout {
        /// The budget, in seconds.
        seconds: u64,
    },
    /// It could not be waited for.
    #[error("the runtime could not be waited for: {source}")]
    Wait {
        /// What the operating system said.
        #[source]
        source: std::io::Error,
    },
}

/// Starts the runtime as a child and waits at most `budget` for it to exit 0.
///
/// This is `GINARY_CMD=selftest`'s runner, and the only place in the launcher
/// that puts a clock on the runtime: everywhere else the application decides
/// how long it lives. A child that outlasts the budget is killed and reported,
/// because a maintenance command that hangs is what a user reports as a broken
/// artifact.
///
/// The plan is recorded to `diag` first, exactly as [`exec`] and [`supervise`]
/// record theirs, so a selftest that failed can be reproduced from its trace.
///
/// # Errors
///
/// [`RunIssue`], naming which of the five things went wrong.
pub fn run_bounded(
    plan: LaunchPlan,
    diag: &Diag,
    budget: std::time::Duration,
) -> Result<(), RunIssue> {
    use std::os::unix::process::ExitStatusExt as _;

    record(&plan, diag);

    let mut command = std::process::Command::new(&plan.program);
    command.args(&plan.args);
    for (key, value) in &plan.set {
        command.env(key, value);
    }
    for key in &plan.remove {
        command.env_remove(key);
    }

    // The runtime's own standard output is not part of the report: a selftest
    // prints one line per step, and a runtime that greeted the world would
    // make that report unparsable. Standard error is inherited, because a
    // runtime that cannot boot says why there and that is the whole point of
    // running it.
    command.stdin(std::process::Stdio::null());
    command.stdout(std::process::Stdio::null());

    let mut child = command
        .spawn()
        .map_err(|source| RunIssue::Start { source })?;
    let deadline = std::time::Instant::now() + budget;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(source) => {
                // The same cleanup the timeout arm does: a selftest that
                // cannot wait for the runtime must not leave it running.
                let _ = child.kill();
                let _ = child.wait();
                return Err(RunIssue::Wait { source });
            }
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(RunIssue::Timeout {
                seconds: budget.as_secs(),
            });
        }
        std::thread::sleep(POLL_INTERVAL);
    };

    match (status.code(), status.signal()) {
        (Some(0), _) => Ok(()),
        (Some(code), _) => Err(RunIssue::Exit { code }),
        (None, Some(signal)) => Err(RunIssue::Signal { signal }),
        (None, None) => Err(RunIssue::Exit { code: -1 }),
    }
}

/// How often [`run_bounded`] asks whether the child has finished.
///
/// Short enough that a selftest of a healthy artifact is not visibly slower
/// than the runtime it starts, and long enough that waiting out the budget is
/// not a busy loop.
const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(10);

/// Replaces this process with the runtime.
///
/// Records the whole plan to `diag` first — argv and the environment
/// difference — because a trace that stops before the thing it was collected
/// to explain is worth nothing. Only then does it call `execve`.
///
/// Returns only on failure, and the return type says so: there is no success
/// value because success is the end of this process. `ENOENT` from a program
/// that is on disk, and `EACCES` from one whose execute bit is set, both carry
/// a [`crate::error::LauncherError::hint`].
pub fn exec(plan: LaunchPlan, diag: &Diag) -> LauncherError {
    use std::os::unix::process::CommandExt as _;

    record(&plan, diag);

    let mut command = std::process::Command::new(&plan.program);
    command.args(&plan.args);
    for (key, value) in &plan.set {
        command.env(key, value);
    }
    for key in &plan.remove {
        command.env_remove(key);
    }

    // `exec` returns only when it failed, so everything after it is the error
    // path.
    let source = command.exec();
    let hint = hint_for(&source, &plan.program);
    LauncherError::Exec {
        program: plan.program,
        source,
        hint,
    }
}

/// The advice a failed `execve` carries, if this failure has any.
///
/// `ENOENT` from a program that is on disk is never about the program: it is
/// the loader the program names. `EACCES` from a program whose execute bit is
/// set is never about the bit: it is the mount.
fn hint_for(error: &std::io::Error, program: &Path) -> Option<&'static str> {
    use std::os::unix::fs::PermissionsExt as _;

    match error.raw_os_error() {
        Some(ENOENT) if program.is_file() => Some(HINT_EXEC_ENOENT),
        Some(EACCES)
            if std::fs::metadata(program)
                .is_ok_and(|meta| meta.permissions().mode() & 0o100 != 0) =>
        {
            Some(HINT_EXEC_EACCES)
        }
        _ => None,
    }
}

/// `ENOENT`: no such file or directory.
const ENOENT: i32 = 2;

/// `EACCES`: permission denied.
const EACCES: i32 = 13;

/// Writes the plan to the trace, as the last thing before `execve`.
///
/// The argument vector and the two environment lists are JSON arrays *encoded
/// as strings*, because a `Diag` value is a string and one object per line is
/// the trace's whole contract. That is what makes `GINARY_TRACE` a
/// reproduction rather than a summary: every `-pa` and every scrubbed variable
/// is in the record.
fn record(plan: &LaunchPlan, diag: &Diag) {
    if !diag.is_enabled() {
        return;
    }
    let argv = json_array(plan.args.iter().map(|argument| argument.to_string_lossy()));
    let set = json_array(plan.set.iter().map(|(key, value)| {
        std::borrow::Cow::Owned(format!(
            "{}={}",
            key.to_string_lossy(),
            value.to_string_lossy()
        ))
    }));
    let remove = json_array(plan.remove.iter().map(|name| name.to_string_lossy()));
    diag.kv(
        "exec",
        &[
            ("program", &plan.program.display().to_string()),
            ("argv", &argv),
            ("env_set", &set),
            ("env_remove", &remove),
        ],
    );
}

/// A JSON array of strings, for a trace value.
///
/// `pub(crate)` because the cache records what it pruned the same way, and a
/// trace with two shapes of list in it is a trace with two parsers.
pub(crate) fn json_array<S: AsRef<str>>(values: impl Iterator<Item = S>) -> String {
    let owned: Vec<String> = values.map(|value| value.as_ref().to_owned()).collect();
    serde_json::to_string(&owned).unwrap_or_else(|_| "[]".to_owned())
}

/// Starts the runtime as a child and waits for it.
///
/// This is what `GINARY_SUPERVISE=1` selects, and it is the code path Windows
/// will use anyway. The child's exit code is mirrored; a child killed by a
/// signal exits `128 + signo`, the shell convention, because an exit code is
/// all a parent process has to say it with. The elapsed time, the exit status
/// and the signal go to `diag`, and if a crash dump appeared in
/// `crash_dump_dir` while the child ran, its `Slogan` line is written to the
/// trace and to standard error.
pub fn supervise(plan: LaunchPlan, diag: &Diag, crash_dump_dir: &Path) -> ExitCode {
    use std::os::unix::process::ExitStatusExt as _;

    record(&plan, diag);

    let dump = crash_dump_dir.join(CRASH_DUMP_NAME);
    let before = std::fs::metadata(&dump)
        .and_then(|meta| meta.modified())
        .ok();
    let started = std::time::Instant::now();

    let mut command = std::process::Command::new(&plan.program);
    command.args(&plan.args);
    for (key, value) in &plan.set {
        command.env(key, value);
    }
    for key in &plan.remove {
        command.env_remove(key);
    }

    let status = match command.status() {
        Ok(status) => status,
        Err(source) => {
            let hint = hint_for(&source, &plan.program);
            let error = LauncherError::Exec {
                program: plan.program,
                source,
                hint,
            };
            let _ = writeln!(std::io::stderr(), "{}", error.report());
            return ExitCode::from(error.exit_code());
        }
    };

    let signal = status.signal();
    // The shell's convention, and the only one available: a parent has an exit
    // code and nothing else with which to report a signal.
    let code = match (status.code(), signal) {
        (Some(code), _) => u8::try_from(code).unwrap_or(u8::MAX),
        (None, Some(signal)) => u8::try_from(128 + signal).unwrap_or(u8::MAX),
        (None, None) => u8::MAX,
    };
    diag.kv(
        "supervise",
        &[
            ("exit", &code.to_string()),
            (
                "signal",
                &signal.map_or_else(|| "none".to_owned(), |signal| signal.to_string()),
            ),
            ("elapsed_us", &started.elapsed().as_micros().to_string()),
        ],
    );

    if let Some(slogan) = fresh_dump_slogan(&dump, before.as_ref()) {
        diag.kv("crash_dump", &[("slogan", &slogan)]);
        let _ = writeln!(std::io::stderr(), "{PREFIX}{slogan}");
    }
    ExitCode::from(code)
}

/// The `Slogan:` line of a crash dump that was not there before the run.
///
/// A dump the previous run left is not this run's news, so the modification
/// time is compared rather than the existence: a supervised run that reported
/// somebody else's crash would be worse than one that reported nothing.
fn fresh_dump_slogan(dump: &Path, before: Option<&std::time::SystemTime>) -> Option<String> {
    let modified = std::fs::metadata(dump)
        .and_then(|meta| meta.modified())
        .ok()?;
    if before.is_some_and(|before| modified <= *before) {
        return None;
    }

    // The slogan is in the first lines of the file, and a dump is large.
    let file = std::fs::File::open(dump).ok()?;
    std::io::BufRead::lines(std::io::BufReader::new(file))
        .take(SLOGAN_SEARCH_LINES)
        .map_while(Result::ok)
        .find(|line| line.starts_with("Slogan:"))
}

/// How far into a crash dump the `Slogan:` line is looked for.
///
/// It is written near the top, and a dump is tens of megabytes: reading the
/// whole file to find a line that is in the first few is how a diagnostic
/// becomes the slowest part of a failure.
const SLOGAN_SEARCH_LINES: usize = 64;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_removed_names_are_the_documented_six() {
        assert_eq!(
            REMOVED_VARS,
            [
                "ERL_LIBS",
                "ERL_FLAGS",
                "ERL_AFLAGS",
                "ERL_ZFLAGS",
                "ERL_ROOTDIR",
                "ERL_EPMD_PORT"
            ]
        );
    }

    #[test]
    fn the_checked_binaries_are_the_documented_three_plus_the_program() {
        assert_eq!(
            REQUIRED_BINARIES,
            ["beam.smp", "erl_child_setup", "inet_gethost"]
        );
    }

    /// A file at `path` with mode `mode`.
    fn write_program(path: &Path, mode: u32) {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::write(path, b"#!/bin/sh\nexit 0\n").expect("write the program");
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
            .expect("set the mode");
    }

    fn io(code: i32) -> std::io::Error {
        std::io::Error::from_raw_os_error(code)
    }

    #[test]
    fn enoent_from_a_program_that_is_on_disk_is_about_the_loader() {
        let dir = tempfile::tempdir().expect("tempdir");
        let program = dir.path().join("erlexec");
        write_program(&program, 0o755);

        assert_eq!(
            hint_for(&io(ENOENT), &program),
            Some(HINT_EXEC_ENOENT),
            "a program that is there and answers ENOENT is naming its interpreter"
        );
    }

    #[test]
    fn enoent_from_a_program_that_is_not_there_says_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");

        assert_eq!(
            hint_for(&io(ENOENT), &dir.path().join("absent")),
            None,
            "guessing at the loader for a program that is simply missing would \
             send a user looking for the wrong thing"
        );
    }

    #[test]
    fn eacces_on_a_program_whose_execute_bit_is_set_is_about_the_mount() {
        let dir = tempfile::tempdir().expect("tempdir");
        let program = dir.path().join("erlexec");
        write_program(&program, 0o755);

        assert_eq!(hint_for(&io(EACCES), &program), Some(HINT_EXEC_EACCES));
    }

    #[test]
    fn eacces_on_a_program_whose_execute_bit_is_clear_says_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let program = dir.path().join("erlexec");
        write_program(&program, 0o644);

        assert_eq!(
            hint_for(&io(EACCES), &program),
            None,
            "the bit is the explanation, and `noexec` would be a red herring"
        );
    }

    #[test]
    fn every_other_failure_carries_no_advice() {
        let dir = tempfile::tempdir().expect("tempdir");
        let program = dir.path().join("erlexec");
        write_program(&program, 0o755);

        // ENOMEM, ELOOP, ENOEXEC: real `execve` failures ginary has nothing
        // useful to add to.
        for code in [12, 40, 8] {
            assert_eq!(hint_for(&io(code), &program), None, "os error {code}");
        }
    }

    #[test]
    fn a_preflight_issue_names_the_file_and_the_fault() {
        assert_eq!(
            PreflightIssue::Missing {
                path: PathBuf::from("/c/k/erts-17.0.5/bin/beam.smp"),
            }
            .to_string(),
            "`/c/k/erts-17.0.5/bin/beam.smp` is missing from the extracted runtime"
        );
        assert_eq!(
            PreflightIssue::NotExecutable {
                path: PathBuf::from("/c/k/erts-17.0.5/bin/erlexec"),
            }
            .to_string(),
            "`/c/k/erts-17.0.5/bin/erlexec` is in the extracted runtime and is not executable"
        );
    }
}
