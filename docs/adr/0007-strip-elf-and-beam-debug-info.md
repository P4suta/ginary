<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# 0007 — Strip ELF and BEAM debug information, on by default

Status: Accepted · 2026-08-31

## Context

A staged `hello_ffi` — the zero-dependency fixture, one application over `kernel` and `stdlib` —
is 66.8 MB before anything is removed from it. 56.6 MB of that is four ERTS binaries, and 56.2 MB
is `beam.smp` alone, most of which is DWARF that no packaged application will ever read. The rest
is compiled modules, and roughly two thirds of a Gleam or OTP `.beam` is the `Dbgi` and `Docs`
chunks: `gleam@list.beam` is 49 680 bytes, of which 27 895 are `Dbgi` and 7 250 are `Docs`.

An artifact is downloaded far more often than it is debugged, and it is downloaded by people who
did not build it. Shipping 66 MB where 15 MB would do is the difference between a tool someone
tries and one they do not.

Two tools do the removing and neither can do the other's half. `strip(1)` handles ELF and knows
nothing about IFF containers. `beam_lib` handles compiled modules, lives inside OTP, and can only
be run by starting a runtime. Neither is part of the Rust toolchain, and `strip` is not on every
machine.

The risk is that both tools are trusted. `strip --strip-all` on a shared object removes the
dynamic symbol table and produces a file that is smaller and will not load. `beam_lib` answers
`{ok, _}` for a list of files it found nothing in, which is indistinguishable from a successful
strip unless someone opens the files afterwards.

## Decision

**Stripping is on by default**, in `ginary stage` and in `ginary build`, and is turned off with
`--no-strip` or narrowed with `--strip-elf-only` and `--strip-beams-only`.

**ELF.** Every file in the staged tree whose first four bytes are `\x7fELF` — detected by magic,
never by extension — goes through `strip`. A shared object gets `--strip-unneeded`, because its
dynamic symbol table is what makes it loadable, and everything else gets `--strip-all`. A shared
object is an `ET_DYN` — read from `e_type`, not guessed from an absent `PT_INTERP`, since a
library may carry an interpreter and a static program may not have one — whose name is a
library's or which has no interpreter; a position-independent executable is an `ET_DYN` too and
is stripped all the way. Each file is re-inspected afterwards and has to still be an ELF of the
same class and machine. A `strip` that is not on `PATH` is a **reported skip** with a reason, not
an error: a bigger artifact is better than a build that will not run on a machine missing a
developer tool. A file that begins with the magic and does not parse as an ELF is a reported skip
too, one `warning:` line naming it, which is what `report::measure` already did with the same
file. A `strip` that runs and fails on one file *is* an error, naming the file and quoting the
tool's standard error.

**BEAM.** `<otp.root>/bin/erl` — the installation's own, by absolute path, not whatever `erl` is
on `PATH` — runs `beam_lib:strip_files/1` under a 300-second budget over every `.beam` ginary
walked, `priv` included. The modules arrive after `-extra` rather than being interpolated into
the expression, so a name cannot become Erlang source. `strip_files/1` and not `strip_release/1`:
the latter takes a *directory* and expands `<root>/lib/*/ebin/*.beam` through
`filelib:wildcard/1`, which makes the staged root a glob prefix — a root named `build[1]` matches
nothing and one named `build*` rewrites `.beam` files under sibling directories ginary was never
asked to build — and it also leaves every module outside `lib/<app>/ebin` unstripped while
ginary's own verification counted it. Passing the walked list makes the set verified the set
rewritten. Afterwards every staged `.beam` is re-read with `src/beam.rs` and must have lost
`Dbgi` and `Docs` and kept `Code`. An OTP root with no `erl` in it is a reported skip, for the
same reason a missing `strip` is.

`beam_lib` writes every module it rewrote through `zlib:gzip/1`, unconditionally, so a stripped
`.beam` on disk is a gzip member wrapping the IFF form rather than the form itself. The Erlang
code server unwraps it on the way in, and so does `beam::form`, which is what lets the
verification above — and `ginary beam chunks`, and the report — read a stripped module as
readily as an unstripped one. `flate2` with its pure-Rust backend is the decompressor.

**What is kept.** The `Line` chunk stays. It is small, and it is what turns a crash in a packaged
application into a stack trace with module names and line numbers — the one piece of debug
information whose reader is the *user* of the artifact rather than its author.

**What is lost.** `dialyzer`, `cover` and the debugger cannot be run against the modules in a
shipped artifact, and neither can anything else that wants the abstract code: `Dbgi` is where it
lives. The modules in the developer's own `build/` directory are untouched, so this costs nothing
during development; it costs only the ability to run those tools against the artifact itself.
`--no-strip` is the escape hatch, and it is one flag.

## Consequences

- The measured result for `hello_ffi` is recorded in `docs/dev/log/A2.md`: 66 775 592 bytes
  before, 12 626 130 after, a saving of 54 149 462. The budget the test suite enforces is 25 MB
  for the whole staged tree and 15 MB for `beam.smp`; both are budgets a regression should break,
  not measurements.
- Stripping is idempotent, which is what keeps "identical input produces identical artifact
  bytes" true through this phase: a second pass finds nothing to remove and writes nothing.
- `ginary.stage.json` is rewritten after stripping. A listing still holding the pre-strip sizes
  would be trusted by the manifest, the payload index and the report alike.
- `beam_lib` writes its output gzip-compressed, which costs the payload's own
  zstd some ratio, and adds `flate2` to the dependency list so that every reader of a staged
  module can unwrap it. Whether a Rust IFF filter that rewrites the modules uncompressed is worth
  building is a question for a measurement, not for this record.
- The build now depends on two external programs it did not before. Both are optional, both
  report when they are absent, and `ginary doctor` already probes `strip`.
