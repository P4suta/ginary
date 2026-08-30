<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# 0008 — Launcher exit codes 121 to 125, and maintenance through the environment

Status: Accepted · 2026-08-31

## Context

A packaged application is one file, and that file is two programs. When it starts, either the
Gleam application runs or ginary fails to start it, and the person watching sees one process,
one standard error and one exit code for both cases. Two problems follow.

**Whose exit code is it?** A user who sees `1` from `./my_app` has learned nothing: the
application exited 1, or ginary could not read its own payload, or the runtime would not link.
A support conversation that begins "it exits 1" cannot get anywhere, and a CI job that retries
on failure cannot tell a corrupt download from a test that legitimately failed. The launcher
needs numbers that the application will not produce by accident, and it needs them to mean
something specific enough to act on.

**Whose arguments are they?** The launcher runs before the application and sees the same `argv`.
Any flag it claims for itself is a flag the packaged application can never have. `--help` is the
obvious one — a Gleam CLI that cannot answer `--help` because ginary answered it first is
broken — but so is every future maintenance verb: `--uninstall`, `--extract`, `--cache-dir`.
There is no prefix that is safe forever, and a launcher that guessed would be a launcher that
extracted when it was asked to inspect.

There is also a third, quieter problem. The launcher promises never to panic. A promise with no
mechanism behind it is a promise a user discovers is broken through a Rust backtrace and an exit
code of 101, which looks exactly like an application that panicked.

## Decision

**Five numbered exit codes, 121 to 125, one per stage of the launcher.**

| code | meaning |
|---|---|
| 121 | the running executable could not be opened, or ginary panicked |
| 122 | the trailer is unusable, or the manifest is a format this build does not read |
| 123 | the payload is corrupt |
| 124 | the cache could not be written or read |
| 125 | the runtime would not start |

The range starts at 121 because everything below it is taken. 0 to 113 is the application's, 126
and 127 are the shell's (`command found but not executable`, `command not found`), 128 and above
are signals, and 125 is where `env(1)` and `timeout(1)` stop. 121 to 125 is the widest gap left
between the shell's conventions, and it is contiguous so that the range can be matched rather
than enumerated.

The codes are ordered by *how far the launcher got*, not by severity: 121 means it never read
itself, 125 means everything was in place and `execve` still failed. That ordering is what makes
a number actionable — it names the stage to look at — and it is why the manifest's
`format_version` is 122 rather than 123. A manifest from a newer ginary is intact; the bytes are
fine and the *format* is the problem, which is the same fault the trailer's own version byte
reports. Calling it corruption would send a user looking for a bad download.

**One line, prefixed `ginary: `, and an optional `hint: ` line.** Every failure prints exactly
one diagnostic, and it names the layer and the cause on the same line: `ginary: the runtime cache
at /home/u/.cache/ginary/hello/8f2a is unusable: Permission denied (os error 13)`. Two failures
carry a second line, because the message alone would send a competent person to the wrong place:
an `ENOENT` from a program that is on disk is about `ld-linux`, and an `EACCES` from a program
whose execute bit is set is about a `noexec` mount. The launcher has no verbosity setting on the
failure path, because the failure path is the one place it must be predictable.

**A panic hook, on the launcher path only.** It prints `ginary: internal error (this is a bug in
ginary): <message>` and exits 121, so a bug in ginary is still a ginary-shaped failure with a
ginary-shaped exit code and no backtrace at a user who cannot use one. The command line half
keeps the default hook: it is a developer tool and its panics are worth seeing in full.

**Maintenance travels in `GINARY_CMD`, never in `argv`.** Three values, matched exactly:

- `directory` — print the cache entry this artifact would use, and create nothing;
- `extract-only` — extract, print the entry, and do not launch;
- `inspect` — print the manifest, the payload geometry and the digest as one JSON object.

Anything else is a usage error on standard error and exit 2, the same number a clap usage error
leaves. `GINARY_CMD=Directory` and `GINARY_CMD=dir` are refused rather than guessed at.

**The environment the runtime is given is a *difference*, not a replacement.** `ROOTDIR`,
`BINDIR`, `EMU` and `PROGNAME` are set unconditionally — they are what `erlexec` would have
derived from its own path if the tree had been an ordinary OTP installation. `HOME` and
`ERL_CRASH_DUMP` are set only when the caller has not set them, because a variable the user
exported is the user's, and an exported-but-empty `HOME` is still a value. `ERL_LIBS`,
`ERL_FLAGS`, `ERL_AFLAGS`, `ERL_ZFLAGS`, `ERL_ROOTDIR`, `ERL_EPMD_PORT` and every `ERL_OTP*_FLAGS`
are removed, always, whether or not they are set.

## Consequences

An operator can act on a number. `123` says re-download; `124` says look at the disk or at
`GINARY_CACHE_DIR`; `125` with the glibc hint says the machine is older than the artifact.
`121` says report a bug. A CI job can retry on 123 and 124 and give up on 122.

A packaged application owns every argument it is given, `--help` and `--version` included, and
that stays true however many maintenance commands ginary grows: they cost a variable name, not a
flag. The cost is discoverability — nothing in `./my_app --help` mentions `GINARY_CMD` — which is
what `docs/dev/debugging.md` is for, and what a future `ginary inspect <artifact>` will answer
from the build side.

The application's own exit code passes through untouched, including any of 121 to 125 it chooses
to leave, because by then the launcher is gone: `execve` replaced it. The codes are ginary's
promise about ginary's failures, not a namespace it reserves.

Removing `ERL_*` unconditionally means a developer who deliberately exports `ERL_AFLAGS` to
change an artifact's emulator flags finds it ignored. That is the point — an application that
behaves differently on one machine because of an exported variable is a support case nobody can
reproduce — and `GINARY_ERL_FLAGS` is the deliberate way to do it, spelled so that it can only
have come from someone who read this.

`GINARY_SUPERVISE=1` keeps a parent process alive across the run instead of calling `execve`. It
exists because Windows has no `execve` and that code path has to be written and tested anyway,
and because a supervised run can report a signal — as `128 + signo`, the shell's convention — and
the `Slogan` line of a crash dump, which an `execve`d launcher cannot: it is no longer there.
