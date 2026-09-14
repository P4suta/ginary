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

| cluster | mutants | killed | equivalent |
|---|---|---|---|
| `appfile` (ten tests over escapes, bounds, lookaheads, nesting) | 14 | 13 | 1 |
| `payload::destined_path_for` | 5 | 5 | 0 |
| `payload::Destinations::insert` | 3 | 3 | 0 |
| `payload::set_mode` | 2 | 2 | 0 |
| `payload` bounds (`MAX_FRONT_ENTRY_BYTES`, `MACHO_HEAD_CAP`) | 2 | 2 | 0 |
| `verify::read_entry` bounds | 3 | 2 | 1 |
| `closure::AppSet::is_empty` | 2 | 2 | 0 |
| `payload::locate` section bound | 1 | 0 | 1 |
| `cache::maintenance_owns` (recorded in F1-macos-native.md) | 20 | 12 | 5 |
| `cache` classifiers (`is_errno`, `is_occupied`, `is_refusal`, `is_cache_key`, `residue_owner`) | 6 | 6 | 0 |
| `cache::files_under` | 3 | 3 | 0 |
| `cache::remove_anything` | 3 | 3 | 0 |
| `cache::prune_app` | 3 | 2 | 1 |
| `cache::clean_app` | 4 | 3 | 1 |
| `cache::sweep` and `owned_sweep_tree` | 3 | 2 | 1 |

`verify`'s eight `entry_kind` arm mutants are gone rather than killed: `verify::entry_kind` and
`payload::check_entry_type` carried the same ten-arm table, and `verify`'s header promised the two
commands "name the same shapes the same way" — a promise a reader had to check by eye, and which
two green test suites would have kept while a word drifted in one copy. There is one function now,
in `payload`, and `verify` calls it.

## Two facts about the tar reader, held rather than assumed

Of the eleven tar type flags, **seven reach ginary and four do not**: `tar::Archive` applies a GNU
long name, a GNU long link and a pax header to the *following* entry rather than yielding them,
and refuses `S` outright unless the header is GNU. So four arms of the shared table cannot be
reached through that reader, and deleting any of them changes no answer.

They are kept, because they are defence against a reader that stops consuming them — and the test
holds *that* fact instead of leaving their absence unexplained: `tests/payload.rs` asserts that
those four never come back as an `UnsupportedEntry`. If the reader ever changes, that test fails
and the arms become reachable, which is the moment they start earning their place.

## The equivalent mutants, with the argument

Mutation testing's own literature treats these as a finite category that has to be *argued*, and
an equivalent mutant whose argument is not written down costs the next reader the same hour it
cost the first. Each was checked by applying the mutation and running the target, not reasoned at.

- **`payload::locate`, `section_size < TRAILER_LEN` → `<=`.** A section of exactly `TRAILER_LEN`
  sits at the end of the file, so its bytes *are* the file's last sixty-four, and
  `Trailer::read_from` finds them before `locate` looks for a section at all. Every size that
  reaches the bound is below it, where the two readings agree. No fixture can separate them.
- **`verify::read_entry`, `head.len() < OBJECT_MAGIC_BYTES` → `<=`.** Reading a fifth byte changes
  nothing that is decided: `object_format_of` answers the same for four bytes and five, the extra
  byte comes out of the same entry so every partial sum against `bound` is one larger and the
  total is unchanged, and the length and digest are over the same bytes either way.
- **`payload::HostDestinations::insert`, `delete !`.** Removing it puts the separator before the
  first component instead of between the later ones, so `lib/ab` and `liba/b` both spell `/libab`.
  But the host prefix and the target prefix are built from the same component list by the same
  loop, so whatever garbles one garbles the other identically — and the comparison that decides is
  between them.
- **`appfile::render_float`, the `text.contains('.')` guard.** Its `None` arm cannot be reached:
  `text` is the `Debug` form of a finite `f64` with no exponent, and Rust writes a decimal point
  for every such value. That is an assumption about the standard library, so it is pinned by a
  property over normal floats and a case list of the subnormal and zero ones instead of argued in
  a comment.
- **Five in `cache::maintenance_owns`**, recorded in [F1-macos-native.md](F1-macos-native.md):
  the function checks the same bound twice and the first check makes the second unreachable.
- **`cache::prune_app` and `cache::clean_app`, `&&` in the metadata closure.** The two readings
  differ only for a regular file — nothing is both `is_dir` and `is_symlink` under
  `symlink_metadata` — and for a regular file the next term asks `maintenance_owns` for
  `<file>/ginary.json`, which no filesystem answers. Either way the entry is `Unowned`.
- **`cache::sweep`, the first of its two identical guards.** They sit either side of the lock and
  are each the other's equivalent mutant: mutate one and the other still refuses the entry, so
  only a wasted lock differs. The second is not redundant — it exists because the answer can
  change while the lock is being taken, which is the race the sweep is written around — but the
  campaign can never kill either.

One that *looked* equivalent and is not, which is why each of these is checked rather than argued.
`residue_owner`'s digit check appears to be subsumed by the `digits.parse()` under it: every
string the check refuses, the parse refuses too — except `+12`, which `u32::from_str` accepts.
Without the check, `.tmp-12` and `.tmp-+12` would be two spellings of one owner's residue. One
input in the whole space separates them.

## What this leaves

`cache` keeps the survivors this pass did not reach: `chmod_tree` and `sync_tree`'s return
replacements, the guards of `rename_into_place`, `rename_aside`, `prune`, `clean_detailed` and
`create_fallback_root`, and the fault-injection fixtures `Corrupting` and `PayloadSource`.
`launch` has one, `delete -` in the `(None, None)` arm of `run_bounded`, which needs
an `ExitStatus` that is neither exited nor signalled; `ExitStatusExt::from_raw` can build one, and
reaching the arm needs the match extracted from the middle of that function first.

The campaign is still red, and the policy question [F1-macos-native.md](F1-macos-native.md) raises
is unchanged and now has fourteen subjects rather than six: this repository has no mechanism for an
equivalent mutant — no `mutants.toml`, no `#[mutants::skip]` — and `docs/dev/v1-readiness.md` says
flatly that a surviving mutant fails its shard. Something has to give, and which thing is a
decision about this project's assurance policy rather than a change to make quietly.
