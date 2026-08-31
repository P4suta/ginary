<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# 0014 — Native code is reconciled in one order, and one refusal cannot be waived

Status: Accepted · 2026-09-01

## Context

A Gleam application that depends on a NIF ships the compiled object inside its `priv` directory,
and `gleam export erlang-shipment` puts it there exactly as the developer's machine built it.
Everything before this milestone could ignore that: a host build packages a host object, and the
only thing that could go wrong was already true before ginary ran.

A cross build cannot ignore it. `ginary build --target linux-aarch64-musl` over a shipment
holding an x86-64 glibc `.so` produces an artifact that extracts, starts, and dies the first time
`erlang:load_nif/2` is called on somebody else's machine. The bytes are in the payload, the
manifest says nothing about them, and the failure surfaces as a stack trace in a user's log.

Four questions arrive together.

**One: what is native code?** Not what is called `.so` — a `priv/lib/wrapper.so` that is really a
shell script is one of the shapes real projects ship, and a port program under `priv/bin` with no
extension at all is native code that has to run on the target too. And "native" is three
container formats, not one: a shipment can hold ELF, PE and Mach-O objects at once, because the
project that produced it vendors one per platform.

**Two: who is supposed to produce the object for the other machine?** ginary carries no cross
toolchain and never will: building a NIF for aarch64 musl needs that project's own compiler, its
own headers and its own build system. The tool can *detect*, *refuse* and *place*; it cannot
compile.

**Three: what happens when the developer knows better?** A shipment can hold an object that is
never loaded — a vendored `.so` for a platform this artifact is not for, a helper nobody calls —
and refusing that build outright would make ginary unusable for the project that has one.

**Four: what happens when the developer is wrong and does not know it?** A statically linked
emulator has no dynamic loader inside it. `nif_loading` is `false` for one, and a `.so` beside it
can never be opened however well its architecture agrees. This is the case where a permissive
flag would produce a broken artifact silently.

## Decision

**Objects are found by magic, under `priv`, and nowhere else.** `native::scan_shipment` walks
each shipment application's `priv` directory, reads the first bytes of every regular file, and
lists the ones that begin like an ELF, a PE or a Mach-O. `ebin` is not walked: whatever a build
system left beside the compiler's output is not loaded as native code by the artifact. A file that
begins like an object and will not parse — a truncated ELF, a `MZ` with no `PE\0\0` behind it, an
object larger than the hundred megabytes a header is read from — is listed with
`NativeKind::Unknown` and a warning naming it, never raised: the scan describes the shipment, and
deciding what to do about it belongs to the phase that knows the target.

**One order, per object, per target.**

1. `[tools.ginary.target.<name>.native]` names a replacement file for this object's path.
2. `[tools.ginary.native.<package>] build` is a command that produces one.
3. The object's own header already names this target.
4. Otherwise it is a mismatch, collected rather than raised.

The order is the answer to question two: both of the first two rules hand the work back to the
project, and they are two rules rather than one because they answer different situations — a
vendored file is a path and a build is a command, and a project that has both wants the vendored
file for the one target it vendored it for. An override answering first is what makes that
possible without deleting the hook, and it is also what keeps a compiler from running to produce
output that would have been thrown away.

**A replacement is verified, and a static object is accepted with a note.** The machine and the
container format have to be the target's. The C library is compared only when the file names one:
a Linux object with no `PT_INTERP` says which machine it is for and nothing about its libc, which
is exactly what every musl NIF built `-static` looks like. Refusing those would refuse the
ordinary case, and recording a libc for them would be a guess written into a manifest, so they are
accepted and the build prints which file was taken on that basis.

**Mismatches are one table, and `--allow-native-mismatch` waives them.** Every unaccounted object
is collected and reported once, in path order, with a `fix:` line per row naming the two
`gleam.toml` keys and the flag. The flag is the answer to question three: the objects travel as
they are and the build prints the same table as a warning. It is a flag and **not** a `gleam.toml`
key, because a project that recorded "ship it anyway" in its manifest would carry that decision
into every later build and nobody would ever see it again.

**A static runtime carrying a shared object is refused, and the flag does not lift it.** That is
the answer to question four. The remedy is `otp_variant = "dynamic"` or a gnu target, and both are
in the message. Waiving it would produce an artifact that is broken by construction rather than
merely suspect, which is the one thing a build flag must not be able to do.

The check is made *before* the four rules above, and that is not an optimisation of a check that
could as well come later: a NIF an override or a hook answered for is still a NIF, so no rule in
the loop can change the answer, and the answer is knowable from the scan and the runtime alone.
Deciding it afterwards would spend one ten-minute hook budget per configured package — the
project's own compiler — on output thrown away one line later, to arrive at the one error no flag
lifts.

**`ginary doctor` reaches the same verdict without resolving a runtime.** The table's per-target
columns are `native::verdicts_for_target`, so the rules above are stated once; what `doctor`
cannot do is read the emulator, because resolving one is a cache and possibly a network fetch and
this command describes the machine rather than performing half of a build. Whether a target loads
NIFs is therefore the *configuration's* answer, reached with the two rules the build's own
selection uses: a named `otp_variant` decides, and a musl target whose `erts` is `catalog` gets
the catalogue's documented default — `catalog::DEFAULT_MUSL_VARIANT`, the static build. Guessing
from `otp_variant` alone, which this milestone did at first, made `doctor` answer `ok` for exactly
the NIF the ordinary cross-compiling manifest is refused over. A `dir:` or `tarball:` runtime that
happens to be static is the case only the build can catch, and it catches it.

**An `ET_DYN` is a program when `DF_1_PIE` says so.** An ELF's `e_type` does not distinguish a
library from a position-independent executable: every program a toolchain has linked since
binutils 2.26 is an `ET_DYN`, `/bin/ls` included. Reading them all as shared objects — which this
milestone did at first — makes the static-runtime refusal fire for any project shipping a port
program, and that refusal is the one no flag lifts and whose remedy the published catalogue cannot
satisfy for a musl target. So the discriminator is `DF_1_PIE` in `DT_FLAGS_1`, which a linker sets
on a program and not on a library. A `PT_INTERP` is *not* the discriminator, however much it looks
like one: glibc's own `libc.so.6` has an interpreter and no `DF_1_PIE`. The cost is a program
linked before that flag existed, which reads as a library and is refused with a message naming a
setting; the alternative cost was refusing every modern one.

**Reconciliation is over the closure, not over the shipment.** The scan reads every shipment
application, because a scan that decided what to describe would be deciding twice; what one target
has to answer for is then narrowed to the objects the *staged tree* holds. The shipment is
everything `gleam` exported and an artifact carries the dependency closure of one application, so
an object in an application nothing depends on never travels — and refusing a build over it would
be a refusal whose three remedies all answer for a file that was never going to be there. The
scan's own warnings are printed either way, before the narrowing.

**One hook's output directory names the target.** `<work>/native/<target>/<package>/`. The work
directory belongs to the build and a build makes as many artifacts as it was given targets; a
`make`-style hook that decides its output is up to date and writes nothing would otherwise have
the previous target's object verified and accepted in its place — and a static object, which this
document accepts for any target of its machine, would pass every check on the way out. With the
target in the path the second target's hook has an empty directory to answer for, and a hook that
writes nothing there fails the build.

**The manifest records what was shipped, and `verify` holds it to that.** Each object's row
carries the path inside the artifact, the machine, the target, whether it was replaced and by
which of the two rules — read back off the *staged tree* after the replacements were applied
rather than off the shipment. `ginary verify` cross-checks every row against the index and against
the machine of the object it streams, so a manifest that names a file the artifact does not carry,
or that records a machine the bytes do not have, is a finding.

## Consequences

- ginary refuses builds it used to write. A project that cross-compiled with a foreign NIF in its
  shipment and did not notice now gets a table naming every object and three ways out. The flag
  is the escape hatch, and it is not silent.
- Build hooks run arbitrary commands from `gleam.toml`. That is the same trust a project's own
  build system already has, and it is bounded rather than unbounded: `sh -c` in the project root,
  ten minutes, and a non-zero exit fails the build with everything the hook wrote to standard
  error. Nothing runs unless a target's build actually reaches an object of that package.
- What a build refuses depends on what that target's artifact carries, so two targets of one
  project can disagree about an object when their closures differ. That is the intended reading:
  the question this phase asks is "can the artifact I am about to write run", not "is this
  directory tidy". `ginary doctor` still lists every object under the shipment's `priv`, so
  nothing disappears from view.
- A `priv` tree deeper than 32 directories is walked no further, and the directory the walk
  stopped at is listed with a warning naming the depth. Nothing below it is read, and nothing
  about it is silent.
- Nothing here builds anything. A project with no vendored object and no hook cannot cross-build
  a NIF at all, and the message says so rather than pretending.
- The three fabricated object shapes the tests use (`tests/common/native.rs`) are hand-written
  headers. They are enough because every rule above reads a header and nothing loads one; a claim
  about a *loadable* NIF would need a cross toolchain and is not made.
