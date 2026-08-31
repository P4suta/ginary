<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# The cache protocol, model-checked

`formal/Cache.tla` is a TLA+ model of the extraction, locking and pruning protocol that
`src/cache.rs`, `src/cache_lock.rs` and `src/launcher.rs` implement between them. `mise run
formal` fetches a pinned `tla2tools.jar` into `.cache/tla/` and runs TLC over it.

The model exists because the four things that protocol claims are the four things a test suite
cannot show. A test can kill a launcher mid-extraction and watch the next one sweep — B1 does
exactly that, through `GINARY_FAULT=after-extract:pause` — but it shows one interleaving out of
a space the scheduler picks from, and the interleaving that matters is the one nobody thought
of. TLC enumerates all of them for a small instance.

## What is modelled

One application directory. `Keys` cache entries in it, `Procs` launcher processes racing over
them, and one pruner. The configuration checks two launchers and two keys; that is enough to
produce every race the protocol has, and I3 is a liveness property, which is the expensive half
of the run.

### States of one entry

| model | on disk |
|---|---|
| `Absent` | `<app>/<key>` does not exist |
| `TmpPartial` | some `.<key>.tmp-<pid>` sits beside it, holding a half-extracted tree |
| `Complete` | `<app>/<key>/ginary.json` is a regular file |
| `Trashed` | a prune has renamed it to `.<key>.trash-<pid>` and has not yet removed it |

`TmpPartial` is not a value of the entry, because a partial extraction is not the entry: it is a
sibling directory. That is why the model carries it as a set of process ids per key, and why a
key can be `TmpPartial` and `Complete` at the same time — one process extracting while another
already holds the finished tree, which is the ordinary case for a second launch.

### Actions, and the code that performs them

| action | code |
|---|---|
| `Hit` | `cache::ensure_extracted` step 1: `<key>/ginary.json` is a regular file |
| `BeginExtract` | `ensure_extracted` steps 2 and 3: `discard_incomplete`, the sweep of this pid's own leftovers, then `create_dir` of `.<key>.tmp-<pid>` |
| `CrashMidExtract` | the `after-extract` fault point: the tree is on disk and nothing is renamed |
| `FinishExtract` | `ensure_extracted` steps 9 and 10, `rename_into_place`, including its `EEXIST` branch |
| `Sweep` | `cache::sweep`, one pass over the application directory |
| `TakeSharedLock` | `launcher::lock_entry` and `cache_lock::SharedLock::acquire`, with the re-check that follows the lock |
| `ReleaseOnExit` | the lock descriptor surviving `execve` and being closed by the kernel when the runtime dies |
| `PruneCheck` | `cache::prune_app`: an entry that is complete, old enough, and whose `.lock` `cache_lock::try_exclusive` can take, renamed aside |
| `PruneRemove` | the `remove_dir_all` of the renamed tree, and the exclusive lock going away |
| `Restart` | the application being run again — not a function, but the thing that makes a sweep happen at all |

## The four properties

**I1 — a complete entry is never removed while a process holds its shared lock.** This is the
one that matters most: a running BEAM whose `ROOTDIR` is deleted underneath it does not fail, it
misbehaves. `PruneCheck` may only fire when no process is in `Running` on that key, which is the
model's reading of `try_exclusive` failing while a shared `flock` is held.

**I2 — no process ever launches out of a partially extracted tree.** The tree a launcher runs
from is `<app>/<key>`, and the invariant says two things about it: that it is `Complete`, and
that it got there by a rename. The second conjunct is what makes I2 more than I1 restated — it
says the rename is the completion marker and there is no other one, so an entry that appeared
any other way could not be launched from.

**I3 — a temporary tree nobody is extracting into does not stay forever.** The liveness
property, and the only one that needs fairness. It is discharged by another launch: the sweep
runs inside `ensure_extracted`, so a machine on which the application is never run again keeps
its leftovers, and the model says so by requiring a live launcher to reach `Idle` infinitely
often before it will claim anything.

**I4 — two concurrent extractors of one key never both rename.** One wins and one takes the
`EEXIST` branch, so an entry is completed at most once between the removals that make it absent
again.

## What the model leaves out

A model that claimed to cover everything would be a model nobody could check. These are the
abstractions, and each one is a place where the code is trusted rather than proven.

- **`mtime` and age.** `prune_app` only considers an entry whose `ginary.json` was modified more
  than `GINARY_PRUNE_DAYS` ago. The model drops the clock entirely: `PruneCheck` may fire on any
  complete, unlocked entry. That is strictly more aggressive than the code, so an invariant that
  holds here holds with the age check in place.
- **`fsync` and durability.** `sync_tree` and the `syncfs` barrier before the rename are not
  modelled at all. TLA+ has no crash-consistency semantics to express them in, and the property
  they buy — that a machine which loses power mid-extraction does not come back with a complete
  entry whose contents are zeroes — is about a failure mode below the level of this model.
- **Filesystem atomicity.** `rename(2)` replacing a name atomically, and refusing to replace a
  non-empty directory, are assumptions rather than results. So is `flock` being exclusive.
- **Errors.** Every operation either happens or does not. `ENOSPC` halfway through an extraction
  is modelled as `CrashMidExtract`, which is the same outcome — a temporary tree and no rename —
  but an `EACCES` on the rename, or a `remove_dir_all` that half-succeeds, is not represented.
- **Identity across restarts.** The model reuses a process's identity when it restarts and the
  operating system does not: a new run is a new pid. That is why `InUse` reads `cache::sweep`'s
  `pid != self_pid && is_alive(pid)` as "somebody is extracting into this very tree" rather than
  as "the owner is alive"; see the note on the operator. A model with a separate, recyclable pid
  space would also describe pid wraparound, which this one does not.
- **A removal that a new extraction races.** `Trashed` is a state of the *entry*, so while a
  prune holds one no launcher can act on that key at all. On disk the two are separate the moment
  `PruneCheck` fires: `prune_app` renames `<app>/<key>` to `.<key>.trash-<pid>` and only then
  removes the tree, so from that instant `<app>/<key>` does not exist and `ensure_extracted`
  would take the miss and extract into it. An extraction concurrent with a `remove_dir_all` is
  therefore outside the model. What makes that safe in the code is the same rename: the tree
  being removed no longer has the name the new extraction is competing for, and the exclusive
  lock the pruner holds is on the entry it renamed. Separating the trashed tree from the entry
  name — a `trash` variable, with `entry[k]` going `Absent` at `PruneCheck` — is the way to model
  it, and it is the first thing to do if this model is extended.
- **More than one application.** The model is one `<app>` directory. Nothing in the protocol
  crosses application directories, and `cache::check_app` is what keeps it that way.

## Running it

```console
mise run formal
```

The task pins the checker by URL and SHA-256 and caches it under `.cache/tla/`; the jar is never
run before its digest matches. TLC's `-deadlock` flag *disables* deadlock checking, so it is
deliberately not passed: this model has no terminal state, and turning the check off would be
giving up an invariant rather than adding one.

`tests/formal.rs` holds the model against the repository from the Rust side — that both files
exist, that the configuration names the four invariants (one the `.cfg` does not name is one TLC
never checks), that the task pins its checker, and that this document says what the model maps
onto. None of that runs TLC; a model nobody runs is worse than no model, because it reads as
evidence.
