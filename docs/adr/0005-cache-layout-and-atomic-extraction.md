<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# 0005 — Cache layout and atomic extraction

Status: Accepted · 2026-08-30

## Context

A packaged application cannot run the BEAM from inside its own file: the emulator, the port
helpers and any NIF are separate executables that must exist on a filesystem. So the first run
extracts the runtime somewhere and every later run reuses it.

Where "somewhere" is decides several user-visible properties. Extracting next to the executable
breaks a read-only install; extracting into the working directory litters it, which is exactly
the failure mode already visible in this workspace as a stray `erl_crash.dump`; extracting into
a fresh temporary directory each run costs seconds on every start.

The protocol matters as much as the location. Two copies of the application started at the same
time on a cold cache will both extract. A process killed mid-extraction leaves a partial tree.
If a partial tree is ever mistaken for a complete one, the application fails in a way that looks
like a corrupt install and survives every retry.

## Decision

**Location.** `<cache>/<app>/<sha256[:16]>/`, where `<cache>` is resolved from
`GINARY_CACHE_DIR`, then `XDG_CACHE_HOME/ginary`, then `HOME/.cache/ginary`, with
`~/Library/Caches/ginary` on macOS and `%LOCALAPPDATA%\ginary` on Windows, and a
`$TMPDIR/ginary-<uid>` fallback that warns. An exported-but-empty variable counts as unset, and
a relative `XDG_CACHE_HOME` is ignored as the XDG specification requires.

Keying on the payload hash means two builds of the same application share nothing unless they
are byte-identical, and renaming or copying an artifact does not change its key.

**Extraction protocol.**

1. `<cache>/<app>/<key>/ginary.json` existing as a regular file is a hit.
2. Otherwise, sweep the application directory for `.<key>.tmp-<pid>` and `.corrupt-*` entries
   whose `pid` is no longer alive, and delete them.
3. Create `.<key>.tmp-<getpid()>`; the application directory is mode 0700.
4. Stream `exe.seek(offset)` → `Take(len)` → hashing reader → zstd decoder → tar unpacker,
   with `preserve_permissions`, without `preserve_mtime` and without xattrs, refusing to
   overwrite.
5. Enforce the entry rules of ADR 0004; entry 0 must be `ginary.json`.
6. Consume the rest of the stream and compare the SHA-256 with the trailer. A mismatch deletes
   the temporary tree and exits **123**.
7. Force mode 0755 on everything under the runtime `bin/` directory.
8. `syncfs` once on the temporary tree, falling back to per-file `sync_all`, then `fsync` the
   application directory.
9. `rename(tmp, target)`. `EEXIST` or `ENOTEMPTY` means a concurrent process won: verify
   `target/ginary.json` and delete our temporary tree.

**The rename is the completion marker.** There is no separate flag file, because a partial
rename does not exist. `ginary.json` inside the renamed directory is the only witness of
completeness.

`preflight()` then checks that `erlexec`, `beam.smp`, `erl_child_setup` and `inet_gethost` exist
and are executable and that the boot file is present. A failure means the cache was damaged
after the fact: the directory is deleted once and re-extracted, and a second failure exits
**124**.

Later, cache entries carry a `.lock` file. The launcher takes a shared `flock` and clears
`FD_CLOEXEC` before `execve`, so the lock is inherited by the emulator and held for the life of
the application; pruning deletes only directories on which an exclusive `flock` can be taken.

## Consequences

Concurrent first runs are correct without a lock, because the only shared mutation is an atomic
`rename` and the losers clean up after themselves. A process killed at any point leaves at most
a `.tmp-<pid>` directory, which the next run removes once the pid is gone.

The working directory and `/tmp` stay clean, and `ERL_CRASH_DUMP` defaults into the cache rather
than into wherever the user happened to be standing.

The costs are real. A read-only or `noexec` cache filesystem breaks the application, so the
failure must name `GINARY_CACHE_DIR` in its message. Disk usage grows with every distinct build
of every application until pruning exists. `syncfs` on a large tree can be slow on some CI
filesystems, in which case per-file `fsync` limited to files above 64 KB is the fallback. And
because the key is the payload hash, a rebuild that changes one byte re-extracts everything.

The interaction of extraction, `rename`, `flock` and pruning is intricate enough to deserve a
TLA+ model, planned as `formal/Cache.tla`.

**Implementation.** As of A0 only the first three location rules exist, in `src/cache_dir.rs`:
`GINARY_CACHE_DIR`, then an absolute `XDG_CACHE_HOME`, then `HOME`. The macOS and Windows bases
and the `$TMPDIR/ginary-<uid>` fallback are unimplemented, and an environment that matches none
of the three rules is an error rather than a warning; they land with the launcher in A3. The
extraction protocol, `preflight()` and the `.lock` file are A3 and Phase B respectively. The
decision above is the target state, not a description of the tree.
