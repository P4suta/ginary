<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# F1 — the mutation campaign, cluster by cluster

[F1-macos-native.md](F1-macos-native.md) records that the nightly mutation campaign has been red
every night since 2026-09-06 — run
[34747498271](https://github.com/P4suta/ginary/actions/runs/34747498271) reconciles to
`caught 733, unviable 81, missed 101, timeout 30, not_run 15` — and closed two clusters of it.
This is the rest of that work, and the method it converged on.

## The method, and why the first one was the wrong shape

The obvious loop is: apply one mutation, run the suite, see whether it fails, repeat. That is what
this work started with, and its cost is one crate rebuild per *mutant*. It is also the wrong
instrument, because it uses the campaign to *discover* which tests are missing — and a discovery
loop over n mutants is n rebuilds no matter how fast any one of them happens to be.

Most of what survives is in **pure functions**: a path rule, a parser, a bound, a classification
table. For those the campaign is not needed to know the answer. A test that covers the *decision
surface* of an expression — for every ground on which it decides, one input that trips that ground
and no other, beside the nearest input that trips none — makes every operator in that expression
observable by construction. Write that, and the campaign is confirmation rather than discovery:
one rebuild per **expression**, not per mutant.

The difference is not a matter of how long a rebuild takes on a given day. It is the count of
rebuilds, which is fixed by how the loop is written: 18 mutants across 8 expressions is 8
rebuilds instead of 18, and the second number does not depend on the machine. Each group also
links exactly one test target — the smallest that can observe it — because the number of test
binaries linked per rebuild is the other quantity the harness controls.

`destined_path_for` is the case that shows it. It refuses a Windows destination on seven
independent grounds chained with `||` and `&&`, and five of its mutants were alive. One table of
twenty-five names — a trailing dot, a trailing space, each of the seven forbidden characters, a
control character, the four reserved device names, `COM1`, `COM9`, `LPT1`, `LPT9`, and beside them
`CONSOLE`, `COMA`, `COM0`, `COM10` and `XXX1`, which trip nothing — killed all five, and will kill
the next operator mutation in that expression too.

## What this closed

One row per function the campaign reported a survivor in, from the campaign's own list of 101.
**Killed** is a test that fails when the mutation is applied; **removed** is code that no longer
contains a term to mutate, because the duplication the mutant lived in is gone.

| function | reported | killed | removed |
|---|---|---|---|
| `cache::maintenance_owns` | 10 | 10 | 0 |
| `cache::sweep` | 9 | 9 | 0 |
| `verify::entry_kind` | 8 | 0 | 8 |
| `cache::clean_app` | 7 | 6 | 1 |
| `payload::check_entry_type` | 5 | 5 | 0 |
| `appfile::Parser::parse_sequence` | 4 | 4 | 0 |
| `appfile::Parser::parse_binary` | 3 | 3 | 0 |
| `cache::chmod_tree` | 3 | 3 | 0 |
| `cache::files_under` | 3 | 3 | 0 |
| `cache::prune_app` | 3 | 2 | 1 |
| `cache::remove_anything` | 3 | 3 | 0 |
| `cache::sync_tree` | 3 | 3 | 0 |
| `verify::read_entry` | 3 | 2 | 1 |
| `appfile::Parser::parse_term` | 2 | 2 | 0 |
| `cache::Corrupting::read` | 2 | 2 | 0 |
| `cache::PayloadSource::read` | 2 | 2 | 0 |
| `cache::is_errno` | 2 | 2 | 0 |
| `closure::AppSet::is_empty` | 2 | 2 | 0 |
| `payload::destined_path_for` | 2 | 2 | 0 |
| `payload::set_mode` | 2 | 2 | 0 |
| twenty-three functions with one each | 23 | 20 | 3 |
| **total** | **101** | **87** | **14** |

**This table is the first pass's own accounting and it overstates.** The campaign's last run found
twenty-six survivors it does not show; the correction at the end of this record says which, and why
checking one mutation at a time against one target is how they were missed.

The three singles that are removed rather than killed are `payload::locate`,
`payload::HostDestinations::insert` and `appfile::render_float`; the last section says what
happened to each.

`verify`'s eight `entry_kind` arm mutants are the largest removal and the plainest:
`verify::entry_kind` and `payload::check_entry_type` carried the same ten-arm table, and `verify`'s
header promised the two commands "name the same shapes the same way" — a promise a reader had to
check by eye, and which two green test suites would have kept while a word drifted in one copy.
There is one function now, in `payload`, and `verify` calls it.

### The first pass's own numbers, and the one that was wrong

This table replaces a per-cluster one that did not reconcile: it summed to 89 of the 101 and
reported `cache::sweep` and `owned_sweep_tree` as three mutants, two killed and one equivalent.
The campaign's list has nine in `sweep` and one in `owned_sweep_tree`, and of the three the first
pass meant — the two guards' `||` and `owned_sweep_tree`'s `&&` — only `owned_sweep_tree`'s was
killed. **Both** guards survived, which the prose beside that row had actually said ("the campaign
can never kill either") while the count said otherwise. So the first pass ended at 79 killed, 14
equivalent and 8 removed rather than at 80, 13 and 8.

That miscount is the reason this table is per function and built from the campaign's list rather
than from a reading of the work: a summary written alongside the work agrees with the work, and
the list is the only thing that agrees with the campaign.

## Two facts about the tar reader, held rather than assumed

Of the eleven tar type flags, **seven reach ginary and four do not**: `tar::Archive` applies a GNU
long name, a GNU long link and a pax header to the *following* entry rather than yielding them,
and refuses `S` outright unless the header is GNU. So four arms of the shared table cannot be
reached through that reader, and deleting any of them changes no answer.

They are kept, because they are defence against a reader that stops consuming them — and the test
holds *that* fact instead of leaving their absence unexplained: `tests/payload.rs` asserts that
those four never come back as an `UnsupportedEntry`. If the reader ever changes, that test fails
and the arms become reachable, which is the moment they start earning their place.

## The fourteen that were equivalent, and are not any more

The first pass left them alive with a written argument each, and asked for a decision between
excluding them, restructuring them away, and accepting a permanently red shard. The decision was
**restructuring**, and it turned out to be cheaper than the argument for exclusion suggested — and
to find a mistake in one of the arguments.

Eight are killed and six no longer exist. Each is below with what changed.

### Killed: the term was unreachable because another term stood in front of it

- **Five in `cache::maintenance_owns`.** The function checked `len()` against
  `MAX_FRONT_ENTRY_BYTES` before opening the marker, and then read it through
  `take(MAX_FRONT_ENTRY_BYTES + 1)` and checked the length again. The pre-check saved reading a
  file it could reject from its size, and cost the read below the ability to decide anything:
  every file over the bound was gone before `take` saw it. One bound, checked where the bytes are,
  and the `+ 1`, the `>` and the `||` beside them all became reachable. The `is_symlink()` term
  beside `!is_file()` went the same way and for the same reason — `symlink_metadata` does not
  follow a link, so `is_file()` is already false for one — with what it was documenting moved into
  the comment above the line that actually does the refusing.
- **`cache::sweep`, the first of its two guards.** This is the mistake. The first pass argued the
  two guards were each other's equivalent mutant and that "only a wasted lock differs". A wasted
  lock is not nothing: `cache_lock::try_exclusive` **creates** `<entry>/.lock`. Reading the first
  guard as `&&` lets a residue with a dead owner and an *unowned* tree reach the lock, so the
  sweep writes a file into a directory it has just decided is not its own — and when that entry is
  a link, writes it through the link, outside the application directory entirely. Two assertions
  on fixtures that already existed kill it. The lesson is the one the first pass stated and did not
  apply to itself: an equivalent mutant has to be *checked*, and "nothing observable differs" is a
  claim about side effects as much as about return values.
- **`cache::sweep`, the second of the two.** Genuinely unreachable by any cache a test can hand
  the launcher, because what it decides is a race: the answers can change while the lock is being
  taken. That is what `src/fault.rs` exists for — its own header says a promise with nothing to
  trigger it is a promise no test can check — so there is a tenth fault point now,
  `sweep-locked:pause`, which sleeps holding the residue's lock between the two askings.
  `a_residue_disowned_while_the_sweep_holds_its_lock_is_kept` disowns the tree while it sleeps.
  The guard had never been exercised at all before this; the mutant was the thing that said so.
- **`cache::rename_aside`'s guard.** The two arms under it refused alike, so the guard decided
  only between refusing here and attempting a rename that cannot succeed either. Written as the
  refusal itself — `symlink_metadata(aside).err().map(|e| e.kind()) != Some(NotFound)` — the same
  question has the same answer and the comparison in it decides the whole thing: reading `!=` as
  `==` refuses every absent name, which is every ordinary call.

### Removed: the code no longer contains a term to mutate

- **`cache::prune_app` and `cache::clean_app`, `&&` in the metadata closure.** `is_dir()` under
  `symlink_metadata` is already false for a link, so `!is_symlink()` beside it was a term no input
  could reach. All six spellings of that pair now call one `is_unfollowed_dir`, which is where the
  rule and the warning about `metadata` are written once. Twelve mutants went with the duplication;
  the two the new function introduces are both killed.
- **`verify::read_entry`, `head.len() < OBJECT_MAGIC_BYTES`.** A hand-rolled loop that counted the
  bytes itself, so the limit was spelled twice — once as the count it stopped at and once as the
  capacity beside it. `entry.by_ref().take(OBJECT_MAGIC_BYTES).read_to_end(&mut head)` is the same
  read with one bound in it, and `read_to_end` retries `Interrupted` where the loop propagated it.
- **`payload::HostDestinations::insert`, `delete !`.** The directory levels were grown by pushing a
  separator between components, which needs a term for "not before the first one". The separators
  are already in the string — `lexical_destination` joined the components with them — so a level
  *is* a prefix of the path, and `match_indices('/')` hands the levels over with nothing to put
  back. Choosing offsets over a helper function was deliberate: `names` decides nothing on a host
  that is not Windows, so a function there would have contributed three *survivors* on every Linux
  shard in place of the one equivalent mutant it removed.
- **`appfile::render_float`, the `text.contains('.')` guard.** The question "does this carry a
  fraction?" was asked once per spelling, and the copy on the exponent-free side had no input that
  could answer it `no`. Asked once, of the mantissa either spelling produces, both answers have an
  input — `1e300` needs the fraction added and `1.5` does not. The whole rendering had been resting
  on a promise about the standard library, and does not any more, so `tests/appfile.rs` pins the
  output instead of the promise: every finite float is written with a dot in its mantissa and reads
  back as itself.
- **`payload::locate`, `section_size < TRAILER_LEN`.** A comparison has a boundary and this one's
  boundary is unreachable, so the bound is the subtraction instead: `section_size.checked_sub(
  TRAILER_LEN)` refuses the same sections and *names the payload* in what is left, which is the
  number `PayloadLoc::len` wanted anyway. The trailer's own `payload_len` is the same value by the
  two checks above it; the one taken from the section's geometry is the one that cannot name bytes
  outside the section.

### Two arguments for equivalence that were wrong, out of fifteen

Fifteen mutants were argued equivalent across the two passes: the fourteen above, and
`residue_owner`'s digit check, which the first pass argued and then withdrew. Two of the fifteen
arguments were wrong, and they failed in opposite directions.

`residue_owner`'s appears to be subsumed by the `digits.parse()` under it: every string the check
refuses, the parse refuses too — except `+12`, which `u32::from_str` accepts. Without the check,
`.tmp-12` and `.tmp-+12` would be two spellings of one owner's residue, and one input in the whole
space separates them. The argument had missed a term that decides something after all.

`cache::sweep`'s first guard is the other, above: the argument had missed a *side effect* rather
than a term, because taking the lock creates a file.

Neither would have been found by rereading the argument, which is the reason each one was checked
by applying the mutation and running the target instead.

## What this leaves — corrected, 2026-09-15

**"Nothing" was wrong, and the campaign's own last run is what said so.** The sentence that stood
here read: *"Every mutant the campaign reported is killed or gone."* Run
[34859119976](https://github.com/P4suta/ginary/actions/runs/34859119976) — the first full pass
after the work above, and the last one this project will run in CI — reported **twenty-six**
outcomes that are neither `caught` nor `unviable`, across seventy-eight shards before it was
cancelled.

The claim was checked per cluster, one mutation at a time, applied by hand and run against the
smallest target that could observe it. That is exactly how it went wrong: the campaign runs the
whole suite on the platform the candidate was assigned to, and a mutation checked against one test
target is not a mutation the campaign checked. Four of the `cache::sweep` survivors below were
re-checked by hand after the run named them, and they survive `cargo test --test cache` here too.
The earlier pass had not measured what it recorded.

### Thirteen are `#[cfg]` stubs whose body *is* the mutant

A function that exists only to say "this platform does not do that" has a body that is already a
constant, and cargo-mutants replacing that constant with itself is the textbook equivalent mutant.
No test can kill one, on that platform, ever:

| where | platform | the body it already has |
|---|---|---|
| `cache::chmod_tree -> Ok(0)`, `Ok(1)` | windows | `Ok(0)` |
| `cache::sync_tree -> Ok(true)`, `Ok(false)` | windows | a stub |
| `cache::syncfs -> false` | macos | `false` |
| `cache::is_occupied -> true`, `cache::is_refusal -> false` | windows | a `win32` code list |
| `launch::signal_of -> None`, `Some(-1)`, `Some(0)`, `Some(1)` | windows | no signal exists |
| `launch::hint_for -> None`, `Some("")`, `Some("xyzzy")` | windows | a stub |
| `payload::set_mode -> Ok(())` | windows | no mode bits |

This is the same finite category the fourteen above were, reached from the other direction: not a
term a change could remove, but a platform on which the function has nothing to do. Restructuring
does not apply, and the exclusion mechanism this repository does not have is the only answer that
would.

### Twelve are real, and on Linux

Every one guards an error path or a race, and none has a fixture:

- **`cache::sweep`**, four: the `NotFound` guard on `read_dir` (→ `false`), and all three mutations
  of the `NotFound` guard on `remove_dir_all`. They are the tree vanishing between the stat and the
  listing, and between the lock and the removal — the same shape as the second guard `sweep-locked`
  was added for, which is the shape of fixture they need.
- **`cache::clean_app`**, three: the same `NotFound` guard, on the same race.
- **`cache::create_fallback_root`**, one: the `AlreadyExists` guard, which is two processes
  creating `/tmp/ginary-<uid>` at once.
- **`cache::sync_tree`**, two on Linux (`-> Ok(true)` and `delete !`): what the function reports
  when `syncfs` succeeds, which on a Linux runner it always does.

Two more were named by shards this record did not download before the run was cancelled.

### And one is a kill the old gate scored as a failure

`appfile::Parser::skip_trivia`'s `!=` → `==` is a `hang`: the parser stops advancing and the suite
never terminates, which is the only detection there can be.
`scripts/ci/mutation-verdict.py` counts that as a kill. The retired gate counted it correctly and
then failed the shard anyway, because it also required cargo-mutants' own exit status to be zero —
and cargo-mutants exits nonzero on a timeout. The diff pass does not make that mistake; the
campaign it replaced did, right up to its last run.

### What this means now that CI mutates the diff

None of the twenty-six is re-detected by a pull request that does not touch those lines, so this
list is the only place they exist. The thirteen are not work. The twelve are, and
`mise run mutants` is where they will be found again.

The general lesson is the one the removals have in common rather than the exclusions they avoided.
Thirteen of the fourteen were a *second* spelling of a decision already made somewhere else — a
bound checked before the read that checks it, a link refused by a call that does not follow links,
a separator inserted into a string that already has it, a fraction demanded of text that has one.
The fourteenth was a guard against a race that nothing could create. Mutation testing found all of
them, and what it was pointing at each time was duplication rather than a missing test.
