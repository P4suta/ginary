// SPDX-License-Identifier: MIT OR Apache-2.0
//! The Windows launcher: spawn the runtime, hold everything it needs alive,
//! and wait for it.
//!
//! Windows has no `execve`, so the launcher cannot hand its process over to
//! `erl.exe` the way [`crate::launch::exec`] hands it to `erlexec`. It stays
//! resident instead, which changes three things and nothing else:
//!
//! - the shared lock on the cache entry is held by *this* process for the
//!   child's lifetime, rather than being inherited across an exec;
//! - the child is assigned to a job object with
//!   `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, so a launcher that is killed takes
//!   the runtime with it rather than leaving an orphan holding the entry;
//! - `SetConsoleCtrlHandler` installs a handler that returns `TRUE` before the
//!   spawn, so Ctrl-C reaches only the child and `+B` lets the emulator decide
//!   what to do with it.
//!
//! The argument vector, the environment difference and `HEART_COMMAND` are
//! [`crate::launch::plan`]'s, unchanged: the only thing a Windows manifest
//! spells differently is `launch.program`, which is `erl.exe`. See
//! `docs/adr/0015-windows-launcher-stays-resident.md`.
//!
//! ## The three Win32 calls, and the `unsafe` they cost
//!
//! Everything above the [`win32`] module is safe Rust. The three calls the
//! last two bullets need — `SetConsoleCtrlHandler`, `CreateJobObjectW` with
//! `SetInformationJobObject`, and `AssignProcessToJobObject` — have no safe
//! counterpart in the standard library or anywhere else, so [`win32`] is the
//! one place in this crate that carries `#[allow(unsafe_code)]`. It is a
//! module rather than three scattered blocks so that the exception is one
//! reviewable surface, and every function in it is total: each answers
//! `false` on failure and none of them can make the launcher fail to start a
//! runtime it would otherwise have started.
//!
//! **None of it has ever run.** It is compiled for
//! `x86_64-pc-windows-gnu` and nothing more; there is no Windows machine and
//! no `erl.exe` in this milestone, and `docs/dev/log/D2.md` says exactly which
//! claims that leaves untested.

use std::ffi::OsString;
use std::io::Write as _;
use std::path::Path;
use std::process::ExitCode;

use crate::cache::Env;
use crate::diag::Diag;
use crate::error::LauncherError;
use crate::launch::LaunchPlan;
use crate::manifest::Manifest;

/// Builds the plan for one launch, which is [`crate::launch::plan`].
///
/// A separate name rather than a re-export so that the Windows entry point
/// reads as one module: the plan is pure and platform-independent, and the
/// only thing that differs is what is done with it.
///
/// # Errors
///
/// Whatever [`crate::launch::plan`] answers.
pub fn plan(
    root: &Path,
    m: &Manifest,
    user_args: &[OsString],
    env: &Env,
    crash_dump_dir: &Path,
    self_exe: &Path,
) -> Result<LaunchPlan, LauncherError> {
    crate::launch::plan(root, m, user_args, env, crash_dump_dir, self_exe)
}

/// Spawns the runtime, waits for it, and mirrors its exit code.
///
/// The order matters and is the order below. The console handler is installed
/// *before* the spawn, because a Ctrl-C that arrives between the two would
/// otherwise terminate this process and take the job object — and with it the
/// runtime — down with it. The job object is created before the spawn for the
/// same reason and the child is assigned to it immediately after, which leaves
/// a window of one scheduling quantum in which a killed launcher orphans the
/// child; closing that window needs `CREATE_SUSPENDED`, which
/// [`std::process::Command`] cannot ask for, and an orphan is a much smaller
/// fault than a runtime this launcher could not start at all.
///
/// Neither facility is required. A launcher that could not install the handler
/// or could not create the job records it in the trace and starts the runtime
/// anyway: what is lost is a Ctrl-C that would have been the child's alone, or
/// a cleanup that would have followed a killed launcher, and neither is worth
/// refusing to run a packaged application over.
///
/// Returns the child's exit code, [`crate::launch::NO_EXIT_CODE`] for a child
/// that ended without one, and the numbered exit code for a runtime that never
/// started.
pub fn run(plan: LaunchPlan, diag: &Diag) -> ExitCode {
    crate::launch::record(&plan, diag);

    diag.kv(
        "console_handler",
        &[("installed", bool_str(win32::ignore_console_ctrl()))],
    );

    // Held in a local for the whole of the wait below. Dropping it closes the
    // job handle, and `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` means that closing
    // the last handle to a job terminates everything still in it.
    let job = win32::Job::create();
    diag.kv("job_object", &[("created", bool_str(job.is_some()))]);

    let mut command = std::process::Command::new(&plan.program);
    command.args(&plan.args);
    for (key, value) in &plan.set {
        command.env(key, value);
    }
    for key in &plan.remove {
        command.env_remove(key);
    }

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(source) => {
            let error = LauncherError::Exec {
                program: plan.program,
                source,
                hint: None,
            };
            let _ = writeln!(std::io::stderr(), "{}", error.report());
            return ExitCode::from(error.exit_code());
        }
    };

    if let Some(job) = job.as_ref() {
        diag.kv("job_object", &[("assigned", bool_str(job.assign(&child)))]);
    }

    let started = std::time::Instant::now();
    let status = match child.wait() {
        Ok(status) => status,
        Err(source) => {
            // The child is running and this process cannot say what it did.
            // Reporting a success would be a lie, so the launcher reports a
            // failure of its own — and returning from here drops `job`, whose
            // `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` takes the runtime down with
            // it. That is the intended outcome and not an accident of the
            // control flow: this launcher is the runtime's parent and the
            // holder of its cache entry's lock, so a runtime that outlived it
            // would be an orphan running out of a directory nothing is keeping
            // alive. A launcher that could not create the job leaves one
            // behind instead, which is the same bargain the trace records at
            // the spawn.
            let _ = writeln!(
                std::io::stderr(),
                "{}the runtime could not be waited for: {source}",
                crate::error::PREFIX
            );
            return ExitCode::from(crate::launch::NO_EXIT_CODE);
        }
    };

    let code = crate::launch::windows_exit_code(status.code());
    diag.kv(
        "spawn",
        &[
            ("exit", &code.to_string()),
            ("elapsed_us", &started.elapsed().as_micros().to_string()),
        ],
    );
    ExitCode::from(code)
}

/// `"true"` or `"false"`, for a trace value.
const fn bool_str(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

/// The three Win32 calls the Windows launcher cannot be written without.
///
/// Every one of them is a `kernel32` entry point with no safe wrapper
/// anywhere, which is why this module — and only this module — carries
/// `#[allow(unsafe_code)]`. Each function is total: a failure is `false` or
/// [`None`] and never a panic, because this is the launcher path.
#[allow(unsafe_code)]
mod win32 {
    use std::os::windows::io::AsRawHandle as _;

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, TRUE};
    use windows_sys::Win32::System::Console::SetConsoleCtrlHandler;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject,
    };

    /// Makes this process ignore `Ctrl-C`, `Ctrl-Break` and the close event.
    ///
    /// Windows delivers a console control event to *every* process attached to
    /// the console, so a Ctrl-C reaches the runtime whatever this launcher
    /// does. What the handler changes is what happens to the launcher: without
    /// one, the default action ends this process, which closes the job handle
    /// and kills the runtime that was about to shut itself down cleanly. A
    /// handler that returns `TRUE` says the event has been dealt with, so the
    /// launcher stays alive to wait for the child and to report its exit code.
    ///
    /// Answers whether the handler was installed. It is best effort: a process
    /// with no console — a packaged application started from Explorer, or as a
    /// service — has no events to handle and `SetConsoleCtrlHandler` may say
    /// so, and that is not a reason to refuse to start.
    pub fn ignore_console_ctrl() -> bool {
        // SAFETY: `handler` is a `'static` function with the signature
        // `PHANDLER_ROUTINE` names, and the call passes no other pointer.
        unsafe { SetConsoleCtrlHandler(Some(handler), TRUE) != 0 }
    }

    /// The handler itself: every console event is reported as handled.
    ///
    /// The runtime got the same event and `+B` is what decides what to do with
    /// it — for the BEAM that is a clean shutdown on the second Ctrl-C, and the
    /// launcher must not have exited before then.
    ///
    /// # Safety
    ///
    /// Called by Windows on a thread of its own while the process is running.
    /// The body reads and writes nothing, so there is nothing for it to race
    /// with.
    unsafe extern "system" fn handler(_event: u32) -> windows_sys::core::BOOL {
        TRUE
    }

    /// An open job object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` set.
    ///
    /// Closing the last handle to such a job terminates every process still in
    /// it, so a launcher that is killed — by the task manager, by a parent
    /// script, by anything that does not run destructors — takes the runtime
    /// with it. Without it a killed launcher would leave a `beam.smp` holding
    /// the shared lock on a cache entry that nothing will ever release.
    pub struct Job(HANDLE);

    impl Job {
        /// Creates an unnamed job object and sets the kill-on-close limit.
        ///
        /// [`None`] when either step fails, which a caller treats as "no job":
        /// the runtime still starts, and what is lost is the cleanup after a
        /// launcher that is killed rather than one that exits.
        pub fn create() -> Option<Self> {
            // SAFETY: both arguments are the documented "no attributes, no
            // name" null pointers, and the answer is checked for null.
            let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
            if handle.is_null() {
                return None;
            }
            let job = Self(handle);

            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let size = u32::try_from(size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>()).ok()?;
            // SAFETY: `limits` is a live, fully initialised value of exactly
            // the type `JobObjectExtendedLimitInformation` names, and `size` is
            // its own size. `job` owns a handle `CreateJobObjectW` returned.
            let set = unsafe {
                SetInformationJobObject(
                    job.0,
                    JobObjectExtendedLimitInformation,
                    std::ptr::from_ref(&limits).cast(),
                    size,
                )
            };
            // A job whose limit could not be set is worse than none: it would
            // be closed at the end of `run` without killing anything, which is
            // a promise the trace would record as kept. `Job`'s `Drop` closes
            // the handle.
            (set != 0).then_some(job)
        }

        /// Puts `child` in this job, and answers whether it worked.
        pub fn assign(&self, child: &std::process::Child) -> bool {
            let process = child.as_raw_handle();
            // SAFETY: `self.0` is a live job handle and `process` is the child
            // handle `std` holds open for the whole of `child`'s borrow here,
            // so neither can have been closed.
            unsafe { AssignProcessToJobObject(self.0, process) != 0 }
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            // SAFETY: the handle came from `CreateJobObjectW`, is closed
            // exactly once — `Job` is neither `Copy` nor `Clone` — and is not
            // used afterwards.
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}
