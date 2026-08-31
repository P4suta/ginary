<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# 0010 — A cache entry is locked for the life of the runtime, and pruning honours the lock

Status: Accepted · 2026-08-31

## Context

ADR 0005 makes a cache entry either complete or absent: a `rename(2)` no reader can observe half
of. It says nothing about removal, because until B1 nothing removed an entry that was not
residue. `ginary cache clean` emptied the whole cache on demand and was the user's problem if
they ran it under a live application.

B1 adds automatic removal. Every launch prunes the stale siblings of the entry it is about to
run out of, `ginary cache prune` does the same on demand, and `GINARY_CMD=uninstall` removes
every entry of one application. All three can now delete the tree a running BEAM is executing
out of — its `beam.smp`, its `.beam` modules, the NIFs it has already `dlopen`ed and the ones it
has not yet.

The obvious answer is for the launcher to hold something and for pruning to check it. The
obvious answer does not work: the launcher `execve`s. After the launch there is no ginary
process on the machine at all, and a marker a *process* owns — a pid file, a lock held by the
launcher — dies with the launcher, which is to say immediately. A marker the runtime would have
to release is worse: a `SIGKILL`ed BEAM leaves it behind for ever, and an entry that can never
be pruned is a disk leak with a plausible-looking explanation.

## Decision

The lock is an `flock(2)` on `<entry>/.lock`, and it is held across `execve` by the file
descriptor rather than by any process.

`flock` belongs to the **open file description**, not to the process. An open file description
survives `execve` as long as its descriptor does not carry `FD_CLOEXEC`, and it is inherited by
every child. So `cache_lock::SharedLock::acquire` opens `<entry>/.lock`, clears `FD_CLOEXEC` with
`fcntl(F_SETFD, 0)`, takes `LOCK_SH`, and the launcher then `execve`s: `erlexec` inherits the
descriptor, `beam.smp` inherits it from `erlexec`, and the kernel releases the lock when the last
holder exits. Nothing has to remember to unlock, and a `SIGKILL` releases it exactly as reliably
as a clean exit.

Pruning takes the other side. `cache_lock::try_exclusive` is `flock(LOCK_EX | LOCK_NB)` on the
same file: it succeeds only when no runtime holds the shared lock. `None` means two things —
somebody holds the entry, or the lock file could not even be opened — and pruning treats them
the same, because the cost of skipping an entry is a directory that stays on disk and the cost
of removing one wrongly is a running application losing its runtime.

Removal itself is ADR 0005's own move: rename the entry to `.<key>.trash-<pid>` and then
`remove_dir_all` it, so no reader ever sees a directory being emptied under it. A removal that
fails after the rename puts the entry back, because a cache entry the launcher can still hit
beats a directory nobody will ever look at again.

The lock is taken **last**, immediately before `execve` or `supervise`, and after the preflight
and the prune. Everything before that point can still fail, and a lock held across a failure
would be a lock nothing releases until the process exits anyway.

Neither side ever blocks. The launcher's `LOCK_SH` is `LOCK_NB` too, retried for
`cache_lock::SHARED_LOCK_BUDGET` (500 ms) and then given up on: the run is recorded as unlocked
and the application starts. A blocking `LOCK_SH` would mean that any process holding the entry
exclusively — a concurrent prune, a stray `flock -x`, an operator's shell — hangs a packaged
application for as long as it lives, with no output and no exit code, which is a worse failure
than the pruning risk the lock exists to remove. Nothing else can be waiting on that lock for
long anyway: a pruner holds it only across one `rename`.

Because the lock is taken last, the entry is **re-checked** after it: `launcher::lock_entry`
confirms `<entry>/ginary.json` is still there and, if it is not, extracts once more and locks
again. A pruner holds the exclusive lock only across the rename, so a launcher that arrives after
the rename finds no lock file at all and would otherwise `execve` into a tree `remove_dir_all` is
in the middle of taking. One retry and no more, for the reason the preflight has one: a second
disappearance is not a race, and a loop is what a user reports as a hang.

## Verification

The claim that a lock survives `execve` is not an argument in this file. It is a test.

`tests/launcher.rs::the_shared_lock_outlives_the_launcher_and_dies_with_the_runtime` starts a
`SyntheticArtifact` whose `erlexec` stub sleeps for three seconds, and then, from the test process
and with util-linux `flock(1)` rather than with ginary's own code, asserts two things:

- while the child runs, `flock -n -x <entry>/.lock` **fails** — and nothing of ginary is alive at
  that moment, so a lock that had not survived `execve` would already be gone;
- once the child is killed, the same command **succeeds** — so the kernel released it, and
  nothing else had to.

The two failures the design forbids have their own tests, both under
`tests/regressions/`: `b1_a_locked_entry_blocked_the_launch.rs` holds an exclusive lock with
`flock(1)` and asserts the application still starts, and
`b1_the_entry_could_vanish_between_the_preflight_and_the_lock.rs` arms
`GINARY_FAULT=before-lock:on`, which removes the entry at exactly the moment a winning prune
would have, and asserts the launcher extracts it again rather than executing out of it.

`tests/cache_lock.rs` asserts the exclusion itself the same way: against `flock(1)`, because a
lock proved with the code that takes it proves nothing about the kernel. `HeldLock` shells out to
`flock -x <lock> sh -c 'read line'` and releases by closing the pipe rather than by killing the
process — `flock(1)` forks, and the *grandchild* is the one holding the inherited descriptor,
which is this ADR's own mechanism observed from the outside.

## Consequences

The lock file stays in the entry after the run. That is deliberate: it lives inside the tree it
locks, so removing the entry removes it, and a stale `.lock` is a zero-byte file nobody holds.

Two runs of one artifact share the entry and both take `LOCK_SH`, so neither waits for the other.
A shared lock that behaved like an exclusive one would serialise every start of an application.

A test binary that takes a `SharedLock` and then forks for any other reason leaks the descriptor
into that child, and the lock outlives the drop until the child exits. That is the mechanism
working as designed; it costs `tests/cache_lock.rs` two polling loops where an immediate
assertion would otherwise do, and it costs the launcher nothing, because the launcher execs
immediately after acquiring.

`flock` is advisory and per-filesystem. An entry on NFS without a lock daemon is an entry pruning
may remove under a running application; that is the same exposure ADR 0005's `rename` already
has, and the cache is a local-disk structure.

The age an entry is pruned against is `GINARY_PRUNE_DAYS`, defaulting to 14; `0` disables pruning
outright. A value that is not a count of days falls back to the default rather than failing a
launch: a misspelt housekeeping preference must not stop an application from starting.
