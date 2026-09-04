<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# 0015 — The Windows launcher stays resident, and holds the lock itself

Status: Accepted · 2026-09-01

## Context

Every earlier decision about the launcher rests on one system call. ADR 0003 starts the runtime
by `execve`ing `erlexec` directly, so that a packaged application is one process and not two and
the exit code needs no relaying. ADR 0010 holds a cache entry with an `flock(2)` on
`<entry>/.lock` and keeps it across that `execve`, because an `flock` belongs to the open file
description rather than to a process: `erlexec` inherits the descriptor, `beam.smp` inherits it
from `erlexec`, and the kernel releases the lock when the last of them exits.

Windows has neither call. There is no `execve` — `CreateProcess` makes a new process and leaves
the old one running — and there is no `flock`: the closest thing is a *share mode* on the open
handle, which is mandatory rather than advisory and belongs to the handle rather than to
anything a child inherits by default.

So the two mechanisms the unix launcher is built on are both absent, and D2 has to say what
replaces them.

## Decision

**The launcher stays alive as the runtime's parent.** `launch_windows::run` spawns
`erts-<vsn>\bin\erl.exe` with `std::process::Command`, waits for it, and exits with its code —
`u8::MAX` for a code that does not fit in a byte, and 1 for a child that ended with none.
`launch::plan` is unchanged and shared: the argument vector, the environment difference and
`HEART_COMMAND` are the same on both platforms, and the only thing a Windows manifest spells
differently is `launch.program`, which is `erl.exe` rather than `erlexec`. There is no `erlexec`
in a Windows runtime; `erl.exe` takes its own directory as the `bin`, the directory two levels
above it as the root, and loads the emulator as a DLL into its own process, which is why the
`erl.ini` beside it — which would name the *build machine's* `Rootdir` — is deleted during
assembly.

**The shared lock is held by this process, for the child's lifetime.** There is no exec for it
to survive, so `SharedLock::acquire` opens `<entry>/.lock` for *reading* with a share mode of
`FILE_SHARE_READ` and the launcher keeps that handle until the runtime exits. Two launchers of
the same entry both succeed — each asks for read access, and each permits read access —
and `try_exclusive`, which asks for read *and write* access while sharing neither, is refused
for as long as any of them holds it. The share-mode open is therefore tried **before** the
lock file is created: a create asks for write access, which a launcher already holding the
entry does not permit, so creating first would refuse the second launcher of an entry and leave
it running unlocked. A write handle is needed on an entry's first lock and never again.

`try_exclusive` shares one thing, `FILE_SHARE_DELETE`. It is not a weakening of the lock —
sharing deletion says nothing about read or write access, so the paragraph above is unchanged —
and it is what lets the removal that follows delete `<entry>\.lock` along with the tree it is
in.

**It does not, however, let the entry be renamed while the lock is held, and this ADR said it
did.** A real Windows kernel answered that on 2026-09-03: every complete entry the first Windows
runner found was reported `unremovable`, the rename refused with the lock still open inside the
directory. `FILE_SHARE_DELETE` permits *that file* to be deleted or renamed; it says nothing
about an ancestor directory of it. So the lock and the rename happen in that order rather than
at once — the lock proves nobody is using the entry, it is released, and then the rename makes
the claim. The window between "nobody holds this" and "it is gone" is real and is the price of
being able to prune at all; on unix, where `rename(2)` asks nothing about open descriptors, the
lock is still held across the rename and no window opens.
`ginary::platform::rename_refuses_open_children` is where that difference is written down, and
`docs/dev/log/E8.md` records the run.

That is the same correspondence ADR 0010 has, reached by a
different mechanism, with two differences a reader has to know about:

- the lock is **mandatory**, not advisory, so a program that knows nothing about ginary is held
  to it too — an editor with `<entry>\.lock` open for writing would make a prune fail rather
  than proceed;
- it belongs to the handle, so it is exactly as long-lived as this process and not one
  instruction longer. A launcher that exits while the runtime is still running releases it.

**The runtime cannot outlive the launcher.** That last point is what a job object is for.
`launch_windows::win32::Job` creates one with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` and assigns
the child to it; closing the last handle to such a job terminates everything still in it, so a
launcher that is killed — by the task manager, by a parent script, by anything that runs no
destructors — takes the runtime with it. Without it, a killed launcher would leave a `beam.smp`
running out of a cache entry whose lock nothing will ever release again.

**Ctrl-C reaches the child, and does not kill the parent.** Windows delivers a console control
event to every process attached to the console, so the runtime gets it whatever ginary does;
what matters is what happens to the launcher. Its default action would end this process, which
closes the job handle and kills the runtime that was in the middle of shutting itself down
cleanly. So `SetConsoleCtrlHandler` installs a handler that returns `TRUE` — the event has been
dealt with — before the spawn, and `+B` is left to decide what the emulator does with its copy.

Neither facility is required. A launcher that cannot install the handler or cannot create the
job records it in the trace and starts the runtime anyway: what is lost is a Ctrl-C that would
have been the child's alone, or a cleanup that would have followed a killed launcher, and
neither is worth refusing to run a packaged application over.

**`#![forbid(unsafe_code)]` becomes `#![deny(unsafe_code)]`, and one module is excepted.** The
three calls above — `SetConsoleCtrlHandler`, `CreateJobObjectW` with `SetInformationJobObject`,
and `AssignProcessToJobObject` — are `kernel32` entry points with no safe wrapper in the
standard library or anywhere else, and `forbid` cannot be lifted for a single module. So
`launch_windows::win32` carries the only `#[allow(unsafe_code)]` in the crate: one module, seven
`unsafe` blocks, each with a `SAFETY` note, every function total and every failure a `false` or
a `None`. `deny` keeps every other file exactly as strict as `forbid` was, and the exception is
one reviewable surface rather than three scattered blocks. The alternative was to ship a Windows
launcher that orphans its runtime when it is killed and dies under Ctrl-C, which is a worse
answer than a bounded exception.

**Amendment, E12: a fourth call, and the module becomes `pub(crate)`.** The exception is no
longer the resident launcher's alone. `cache::sweep` decides whether the launcher that owns a
`.<key>.tmp-<pid>` tree is still extracting into it, and it decided by looking for `/proc/<pid>`
— a directory Windows does not have, so every live launcher read as dead and the tree it was
unpacking into was deleted underneath it. `win32::process_is_alive` is `OpenProcess` with
`PROCESS_QUERY_LIMITED_INFORMATION`, the narrowest access right there is, and an immediate
`CloseHandle`; `ERROR_INVALID_PARAMETER` is the only failure read as "no such process", and
every other answer is "alive", because keeping a tree costs a directory and removing a live
one destroys an extraction in progress.

It lives in this module rather than in `cache.rs` for the reason the other three live here: a
second `#[allow(unsafe_code)]` would be a second reviewable surface, and `CLAUDE.md` requires an
ADR for one. `mod win32` therefore became `pub(crate) mod win32`, which widens what the crate can
reach, not what the exception covers — the module is not exported and no new dependency or
`windows-sys` feature was added. The counts this decision states are held to the module by
`tests/regressions/e12_three_statements_of_the_unsafe_exception_said_three_calls.rs`, because
they had already drifted twice before anything noticed.

## Consequences

The Windows launcher is **two processes where unix has one**. A `ps` on Windows shows the
artifact and `erl.exe` beneath it; the artifact's own memory is the launcher's, which is small
and idle for the whole run, and one extra process is the price of there being no `execve`.

Most of the above is **compiled and not run**. `mise run build:windows` builds both flavors for
`x86_64-pc-windows-gnu` and `stubs:build` produces the stub, and the stub *does* start under the
`cross` image's wine — it prints its payloadless-stub sentence and exits 2, which exercises
`target::Target::host`, `selfexe::open_self`'s `current_exe` route and `trailer::read_from`'s
`seek_read` loop. Nothing beyond that runs on a Linux machine.

The `windows` job of `.github/workflows/ci.yml` runs the suite natively, and what that job
reaches of this decision is **two of its mechanisms and not four**: the share-mode lock, through
the `cfg(windows)` regression tests that take `SharedLock` twice over one entry, and
`win32::process_is_alive`, through `cache::sweep`'s. **The spawn, the job object and the console
handler have still never run**, anywhere: the job builds both flavors, runs `cargo test` and
probes `erl.exe`, and it starts no packaged artifact, while no test in the tree constructs a
`LaunchPlan` and calls `launch_windows::run` — the one call site is `launcher::start`, reached by
a launching artifact and by nothing else. The pure rules underneath all of them — the cache root,
the `\\?\` prefix, the two share modes, the exit-code mapping, the launch program and the Windows
launch plan — are unit-tested on Linux precisely because that is all a Linux machine can honestly
check.
`tests/regressions/e15_the_adr_credited_the_windows_job_with_a_spawn_that_never_ran.rs` derives
both premises from the tree rather than trusting this paragraph.

One of the platform facts `docs/dev/log/D2.md` left to a real Windows host is now **measured**
rather than assumed — and the measurement is an inference from silence, not a number printed in a
log, which is the honest way to state it. That job runs `erl -noshell -eval "halt(3)"` against
the OTP 29.0.5 `setup-beam` installs, on the `windows-2022` image this repository pins rather
than the `windows-latest` label it deliberately does not take. The step's first execution — run
[33864729638](https://github.com/P4suta/ginary/actions/runs/33864729638), job
[100996872499](https://github.com/P4suta/ginary/actions/runs/33864729638/job/100996872499),
Windows Server 2022 — printed **nothing at all** and reported `Process completed with exit code
1` in 0.54 s. Only one of the three outcomes is silent: an `erl` that cannot be found prints
PowerShell's `is not recognized as a name of a cmdlet` block, an `erl` that leaves the wrong
number prints the thrown `expected ERRORLEVEL 3, got …`, and an `erl` that leaves exactly **3**
says nothing and is then failed by the `exit $LASTEXITCODE` GitHub appends to every `pwsh` step —
each of the three reproduced under that wrapper on a real PowerShell, in `docs/dev/log/E15.md`
§10. So the emulator left 3 behind: `halt(N)` reaches the parent as the process exit code, and
`run`'s contract — spawn `erl.exe`, wait, mirror the child's code — rests on something a Windows
host has done. (erts-17.0.5 is what OTP 29.0.5 carries; that log printed no version either.) The
step now captures the code, prints it and ends on a verdict of its own, so the next run of the
job records the number directly and this citation is to be replaced by that one.
`docs/dev/log/E15.md` records the diagnosis, and
`tests/regressions/e15_a_pwsh_step_ended_with_the_code_it_asserted.rs` holds every `pwsh` step to
ending on a status of its own. What that milestone still owes is the `otp_win64_<version>.zip`
layout and the end-to-end run of a real artifact, on the same runner.

`HEART_COMMAND` quoting is the one shared rule that is **not** shared. `heart` restarts the
emulator with `CreateProcess` rather than through a shell, so the Windows `shell_word` follows
the `CommandLineToArgvW` rule — double quotes, a backslash run before a quote doubled — and not
`/bin/sh`'s single quotes, which that parser would hand back as part of the word. Only a real
`heart` restart exercises it.

Two rules are **weaker on Windows than on unix**, and both are recorded rather than papered
over. The cache's fallback root is created but not proved to be this user's: the unix side
checks the owner and the mode because `/tmp` is shared, and the equivalent proof here is a Win32
security descriptor this milestone does not write — `%TEMP%` is per-account and carries the ACL
that says so, and `C:\Windows\Temp` is a last resort reached only by a process whose environment
has been scrubbed. And the directory entry the rename creates is not flushed, because
`std::fs::File::open` cannot open a directory on Windows; a machine that loses power in that
window comes back to an entry the completeness check rejects, which costs one repeated
extraction rather than a broken artifact.

A third is `MAX_PATH`, and it is the runtime's rather than the launcher's. Every path ginary
itself opens is the `\\?\` form — the extraction, the cache-hit check, `<entry>\.lock`, the
manifest and the preflight — and `launch::plan` puts the ordinary spelling back for `ROOTDIR`,
`BINDIR` and the argument vector, because `erl.exe` takes those apart and reassembles them
rather than merely opening them. An entry past `MAX_PATH` therefore extracts, is found and is
locked, and the runtime will not start out of it. There is nothing on ginary's side left to
fix; the remedy is `%GINARY_CACHE_DIR%`.

Mode bits are a **no-op** throughout. `chmod_tree`, the unpacker's `set_mode` and assembly's own
all do nothing on Windows, and `manifest::mode_of` records what the `tar` crate writes into the
header on this platform — 0o755 for a directory, 0o644 for everything else — so that
`ginary verify`, which compares the index against the header, has nothing to report. The `mode`
column of `ginary.index.json` is informational on a Windows artifact.

It is *not* informational on a unix artifact cross-built **on** Windows, and that is a stated
limitation rather than a silent one. Such a build records 0o644 for every file, because there is
no mode to read; the launcher repairs the execute bit only under `erts-<vsn>/bin`, so a program
under `lib/<app>-<vsn>/priv/bin` or a wrapper script would arrive on the target machine without
one. The honest fix is a mode column that does not come from the build host's filesystem, which
no milestone has asked for; until then the answer is the one the README gives — build unix
artifacts on a unix machine.

The **required files a Windows artifact carries** are `erl.exe`, `beam.smp.dll` and
`inet_gethost.exe`, plus every DLL beside them. The first two are what start a runtime at all;
the third is there because `inet_gethost` is in `otp::REQUIRED_ERTS_BINARIES` on unix — a
runtime without it resolves no host name — and leaving it behind would have shipped a runtime
that fails the first time an application opens a socket by name. `assemble::windows_required_bins`
refuses a tree missing any of the three by name, and `launch::preflight` holds the extracted
artifact to the same list: the list it walks is chosen by the manifest's target, not by the
platform the launcher was compiled for. `distribution` and `heart` ask the tree for `epmd.exe`
and `heart.exe` for the same reason — a name here is a file name in somebody else's tree.
