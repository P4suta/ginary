\* SPDX-License-Identifier: MIT OR Apache-2.0
-------------------------------- MODULE Cache --------------------------------
(***************************************************************************)
(* The extraction, locking and pruning protocol of `src/cache.rs`.          *)
(*                                                                         *)
(* One application directory, `Keys` cache entries in it, `Procs` launchers *)
(* racing over them, and one pruner.  What the model is about is the four   *)
(* things the Rust code claims and cannot test: that a running application  *)
(* never has its runtime deleted underneath it, that nobody ever launches   *)
(* out of a half-extracted tree, that a killed extraction's leftovers do    *)
(* not accumulate, and that two processes extracting one key produce one    *)
(* entry rather than two.                                                   *)
(*                                                                         *)
(* `docs/dev/formal.md` maps every action here onto the function that       *)
(* performs it and states what the model abstracts away.                    *)
(***************************************************************************)
EXTENDS Naturals, FiniteSets

CONSTANTS
    Procs,      \* the launcher processes, a small finite set
    Keys        \* the cache keys of one application, a small finite set

(***************************************************************************)
(* The four states one entry can be in.                                    *)
(*                                                                         *)
(* Absent, Complete and Trashed are what `<app>/<key>` itself is.           *)
(* TmpPartial is not: a partial extraction lives beside the entry, as       *)
(* `.<key>.tmp-<pid>`, which is why it is a set of process ids rather than  *)
(* a value of `entry` and why a key can be TmpPartial and Complete at once  *)
(* — one process extracting while another already holds the finished tree.  *)
(***************************************************************************)
Absent     == "Absent"
TmpPartial == "TmpPartial"
Complete   == "Complete"
Trashed    == "Trashed"

EntryValues == {Absent, Complete, Trashed}
EntryStates == {Absent, TmpPartial, Complete, Trashed}

(***************************************************************************)
(* The phases of one launcher, in the order `src/launcher.rs` runs them.    *)
(***************************************************************************)
Phases == {"Idle", "Extracting", "Locking", "Running", "Exited", "Crashed"}

VARIABLES
    entry,      \* Keys -> EntryValues: what <app>/<key> is
    tmps,       \* Keys -> SUBSET Procs: whose partial tree sits beside it
    alive,      \* Procs -> BOOLEAN: whether the process still exists
    pc,         \* Procs -> Phases
    key,        \* Procs -> Keys: the entry the process is working on
    renames,    \* Keys -> Nat: winning renames since the entry was last absent
    pruner,     \* "Idle" or "Holding": whether the pruner holds the exclusive lock
    pruneKey    \* Keys: the entry it holds it on

vars == <<entry, tmps, alive, pc, key, renames, pruner, pruneKey>>

(***************************************************************************)
(* The state one key is in, as the four names above spell it.               *)
(***************************************************************************)
StateOf(k) ==
    IF entry[k] = Complete THEN Complete
    ELSE IF entry[k] = Trashed THEN Trashed
    ELSE IF tmps[k] # {} THEN TmpPartial
    ELSE Absent

TypeOK ==
    /\ entry \in [Keys -> EntryValues]
    /\ tmps \in [Keys -> SUBSET Procs]
    /\ alive \in [Procs -> BOOLEAN]
    /\ pc \in [Procs -> Phases]
    /\ key \in [Procs -> Keys]
    /\ renames \in [Keys -> 0..2]
    /\ pruner \in {"Idle", "Holding"}
    /\ pruneKey \in Keys
    /\ \A k \in Keys : StateOf(k) \in EntryStates

Init ==
    /\ entry = [k \in Keys |-> Absent]
    /\ tmps = [k \in Keys |-> {}]
    /\ alive = [p \in Procs |-> TRUE]
    /\ pc = [p \in Procs |-> "Idle"]
    /\ key = [p \in Procs |-> CHOOSE k \in Keys : TRUE]
    /\ renames = [k \in Keys |-> 0]
    /\ pruner = "Idle"
    /\ pruneKey = CHOOSE k \in Keys : TRUE

-----------------------------------------------------------------------------
(***************************************************************************)
(* Actions.                                                                 *)
(***************************************************************************)

(* `ensure_extracted` step 1: `<key>/ginary.json` is a regular file, so the *)
(* entry is complete and nothing is extracted.                              *)
Hit(p, k) ==
    /\ alive[p]
    /\ pc[p] = "Idle"
    /\ entry[k] = Complete
    /\ key' = [key EXCEPT ![p] = k]
    /\ pc' = [pc EXCEPT ![p] = "Locking"]
    /\ UNCHANGED <<entry, tmps, alive, renames, pruner, pruneKey>>

(* `ensure_extracted` steps 2 and 3: this process's own leftovers go first  *)
(* — a `.<key>.tmp-<pid>` carrying this pid is by definition not in use —   *)
(* and then the temporary tree it extracts into is created.                 *)
BeginExtract(p, k) ==
    /\ alive[p]
    /\ pc[p] = "Idle"
    /\ entry[k] = Absent
    /\ key' = [key EXCEPT ![p] = k]
    /\ pc' = [pc EXCEPT ![p] = "Extracting"]
    /\ tmps' = [j \in Keys |-> IF j = k THEN tmps[j] \cup {p} ELSE tmps[j] \ {p}]
    /\ UNCHANGED <<entry, alive, renames, pruner, pruneKey>>

(* The process is killed with the temporary tree on disk and nothing        *)
(* renamed.  This is the `after-extract:pause` fault point, made a state.   *)
CrashMidExtract(p) ==
    /\ alive[p]
    /\ pc[p] = "Extracting"
    /\ alive' = [alive EXCEPT ![p] = FALSE]
    /\ pc' = [pc EXCEPT ![p] = "Crashed"]
    /\ UNCHANGED <<entry, tmps, key, renames, pruner, pruneKey>>

(* `ensure_extracted` steps 9 and 10: the rename is the completion marker,  *)
(* and there is no other one.  `rename(2)` will not replace a directory     *)
(* that is not empty, so a process that lost the race gets EEXIST, throws   *)
(* its own tree away and reuses the winner's entry.                         *)
FinishExtract(p) ==
    /\ alive[p]
    /\ pc[p] = "Extracting"
    /\ tmps' = [tmps EXCEPT ![key[p]] = tmps[key[p]] \ {p}]
    /\ pc' = [pc EXCEPT ![p] = "Locking"]
    /\ IF entry[key[p]] = Absent
         THEN /\ entry' = [entry EXCEPT ![key[p]] = Complete]
              /\ renames' = [renames EXCEPT ![key[p]] = renames[key[p]] + 1]
         ELSE UNCHANGED <<entry, renames>>
    /\ UNCHANGED <<alive, key, pruner, pruneKey>>

(* Whether a temporary tree is one somebody is extracting into right now.   *)
(*                                                                         *)
(* This is the model's reading of `cache::sweep`'s rule, which is           *)
(* `pid != self_pid && is_alive(pid)`.  The model reuses process identities *)
(* across a restart and the operating system does not — a new run of the    *)
(* application is a new pid — so "the owner is alive" has to be read here   *)
(* as "the owner is extracting into this very tree".  Every other tree      *)
(* carries a pid that is gone, which is exactly the case `is_alive` answers *)
(* `false` for, and the sweeper's own leftovers are the `!= self_pid` half  *)
(* of the same rule.                                                        *)
InUse(q, k) == alive[q] /\ pc[q] = "Extracting" /\ key[q] = k

(* `cache::sweep`: one pass over the application directory that removes     *)
(* every `.<key>.tmp-<pid>` nobody is extracting into.  One action rather   *)
(* than one per tree, because the code is one pass over one `read_dir`: a   *)
(* sweep that removed a single leftover would model a loop nobody wrote.    *)
(* It runs on a miss, which is why the sweeper has to be a launcher that is *)
(* between runs rather than any process at all.                             *)
Sweep ==
    /\ \E p \in Procs :
         /\ alive[p]
         /\ pc[p] = "Idle"
         /\ \E k \in Keys : \E q \in tmps[k] : ~InUse(q, k)
         /\ tmps' = [k \in Keys |-> {q \in tmps[k] : InUse(q, k)}]
    /\ UNCHANGED <<entry, alive, pc, key, renames, pruner, pruneKey>>

(* `launcher::lock_entry`: the shared `flock` is taken and the entry is     *)
(* then re-checked, because a prune holds its exclusive lock only across    *)
(* the rename — a launcher that arrives after it finds a lock file on a     *)
(* tree that is no longer there.  An entry that has gone sends the launcher *)
(* round again, which is the retry the code performs once.                  *)
TakeSharedLock(p) ==
    /\ alive[p]
    /\ pc[p] = "Locking"
    /\ IF entry[key[p]] = Complete
         THEN pc' = [pc EXCEPT ![p] = "Running"]
         ELSE pc' = [pc EXCEPT ![p] = "Idle"]
    /\ UNCHANGED <<entry, tmps, alive, key, renames, pruner, pruneKey>>

(* The shared lock is released when the process that `execve`d dies: the    *)
(* descriptor survives the exec and the kernel closes it.                   *)
ReleaseOnExit(p) ==
    /\ alive[p]
    /\ pc[p] = "Running"
    /\ pc' = [pc EXCEPT ![p] = "Exited"]
    /\ UNCHANGED <<entry, tmps, alive, key, renames, pruner, pruneKey>>

(* The application is started again.  A real restart is a new pid; the      *)
(* model reuses the identity, which is why `Sweep` treats a tree carrying   *)
(* the sweeper's own id as removable exactly as `cache::sweep` does.        *)
Restart(p) ==
    /\ pc[p] \in {"Exited", "Crashed"}
    /\ alive' = [alive EXCEPT ![p] = TRUE]
    /\ pc' = [pc EXCEPT ![p] = "Idle"]
    /\ UNCHANGED <<entry, tmps, key, renames, pruner, pruneKey>>

(* `prune_app`: a complete entry, old enough, whose `.lock` can be taken    *)
(* exclusively, is renamed aside.  The age is abstracted to "any entry may  *)
(* be old enough"; the exclusive lock is the conjunct that matters, and a   *)
(* lock that cannot be taken is a "leave this alone" rather than a retry.   *)
(* The rename is what makes the removal atomic for a reader, so the entry   *)
(* becomes Trashed here and Absent only when the tree is gone.              *)
PruneCheck(k) ==
    /\ pruner = "Idle"
    /\ entry[k] = Complete
    /\ \A p \in Procs : ~(pc[p] = "Running" /\ key[p] = k)
    /\ entry' = [entry EXCEPT ![k] = Trashed]
    /\ pruner' = "Holding"
    /\ pruneKey' = k
    /\ UNCHANGED <<tmps, alive, pc, key, renames>>

(* The renamed tree is removed and the exclusive lock is dropped.           *)
PruneRemove ==
    /\ pruner = "Holding"
    /\ entry' = [entry EXCEPT ![pruneKey] = Absent]
    /\ renames' = [renames EXCEPT ![pruneKey] = 0]
    /\ pruner' = "Idle"
    /\ UNCHANGED <<tmps, alive, pc, key, pruneKey>>

Next ==
    \/ \E p \in Procs, k \in Keys : Hit(p, k) \/ BeginExtract(p, k)
    \/ \E p \in Procs :
         \/ CrashMidExtract(p)
         \/ FinishExtract(p)
         \/ TakeSharedLock(p)
         \/ ReleaseOnExit(p)
         \/ Restart(p)
    \/ Sweep
    \/ \E k \in Keys : PruneCheck(k)
    \/ PruneRemove

(***************************************************************************)
(* Fairness.  Exactly what I3 needs and nothing more: every extra conjunct  *)
(* is another branch in TLC's tableau, and the liveness check is the        *)
(* expensive half of this model.                                            *)
(*                                                                         *)
(* A launcher that has started must finish, lock, run and exit, and one     *)
(* that has exited or crashed must eventually be started again.  Together   *)
(* those put a live launcher in Idle infinitely often, which is what makes  *)
(* a sweep possible at all.  Strong rather than weak fairness, because two  *)
(* launchers cycling out of phase can leave every instant with none of them *)
(* idle, and weak fairness says nothing about an action that is repeatedly  *)
(* enabled and repeatedly disabled.                                        *)
(*                                                                         *)
(* Deliberately unfair: CrashMidExtract, because a process may crash and is *)
(* never obliged to; Hit and BeginExtract, because nothing obliges anybody  *)
(* to run the application again; and the two prune actions, because         *)
(* housekeeping that never runs leaves a cache that is larger than it needs *)
(* to be rather than one that is wrong.                                    *)
(***************************************************************************)
Fairness ==
    /\ \A p \in Procs : SF_vars(FinishExtract(p))
    /\ \A p \in Procs : SF_vars(TakeSharedLock(p))
    /\ \A p \in Procs : SF_vars(ReleaseOnExit(p))
    /\ \A p \in Procs : SF_vars(Restart(p))
    /\ SF_vars(Sweep)

Spec == Init /\ [][Next]_vars /\ Fairness

-----------------------------------------------------------------------------
(***************************************************************************)
(* The four properties.                                                     *)
(***************************************************************************)

(* I1: a complete entry is never removed while a process holds its shared   *)
(* lock.  A process in Running holds it, and its entry may not be Trashed   *)
(* or Absent.                                                               *)
I1 ==
    \A k \in Keys :
        entry[k] # Complete =>
            \A p \in Procs : ~(pc[p] = "Running" /\ key[p] = k)

(* I2: no process ever launches out of a partially extracted tree.  The     *)
(* tree a running process launches from is `<app>/<key>` — never one of the *)
(* `.<key>.tmp-<pid>` beside it — so the claim is about that directory: it  *)
(* is Complete, and it got there by a rename.  The second conjunct is what  *)
(* makes this more than I1 restated: it says the rename is the completion   *)
(* marker and there is no other one, so an entry that appeared some other   *)
(* way could not be launched from.                                          *)
I2 ==
    \A p \in Procs :
        pc[p] = "Running" =>
            /\ StateOf(key[p]) = Complete
            /\ renames[key[p]] = 1

(* I3: a temporary tree nobody is extracting into does not stay forever.    *)
(* The liveness property, and the only one that needs the fairness above:   *)
(* the sweep runs on a miss, so what discharges it is another launch — and  *)
(* the pruner is what guarantees there will be one, because an application  *)
(* whose cache entry is never removed never misses again.                   *)
I3 ==
    \A k \in Keys, q \in Procs :
        (q \in tmps[k] /\ ~InUse(q, k)) ~> (q \notin tmps[k])

(* I4: two concurrent extractors of one key never both rename.  One wins    *)
(* and one reuses, so an entry is completed at most once between the        *)
(* removals that make it absent again.                                      *)
I4 == \A k \in Keys : renames[k] <= 1

=============================================================================
