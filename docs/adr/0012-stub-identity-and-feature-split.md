<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# 0012 — A stub says what it is, and the `cli` feature is what makes one

Status: Accepted · 2026-08-31

## Context

[ADR 0002](0002-rust-single-binary-self-copy-stub.md) settled that an artifact is a copy of the
ginary binary with a payload appended. For the host that copy is the running executable, and
nothing has to be decided: it is this version, for this machine, and it is already open. A build
for another target cannot copy itself, so it has to be handed a *stub* — the same ginary,
cross-compiled — and three questions arrive with it.

**One: how does a build know a file is a stub?** A path proves nothing. `--stub ./ginary-arm64`
could be last month's ginary, a build for the wrong architecture, an artifact somebody already
packaged, or a shell script. Every one of those produces a file that looks like an executable and
fails on the user's machine rather than at build time, and two of them — a stale version and an
artifact — produce a file that starts and then misbehaves, which is worse.

**Two: what is in a stub?** A ginary carries clap, a TOML reader and every build-side module,
and none of them is reachable once a payload is appended: a launcher never parses `argv`. Copying
them into every artifact is a few megabytes per artifact of code that cannot run.

**Three: how is any of that tested?** A stub is produced by a build configuration — a Cargo
feature — and a configuration nothing compiles is a configuration that rots. The suite has to run
in both flavors, and the sentences that differ between them have to be assertions rather than
prose.

## Decision

**Every ginary binary carries a 128-byte identity marker.** `src/stubid.rs` defines
`GINARY_STUB_ID`, rendered at compile time from `CARGO_PKG_VERSION`, `GINARY_TARGET`,
`manifest::FORMAT_VERSION` and `GINARY_FLAVOR`, and `docs/format.md` specifies its bytes. Two of
those four are not knowable inside the crate, so `build.rs` emits them: `GINARY_TARGET` maps
Cargo's own `TARGET` onto the canonical target name, because `std::env::consts` answers for the
host and a cross-compiled stub is exactly the case where the host is the wrong answer; and
`GINARY_FLAVOR` is `full` or `stub` from `CARGO_FEATURE_CLI`. `build.rs` deliberately does *not*
emit the format version: it is a constant in `manifest.rs`, and a second source for it would be a
second thing to forget.

A triple with no target name is a `panic!` in `build.rs`, which is a compile error. A stub whose
marker named the wrong target would be located, verified and packaged, and the artifact would
fail on the machine whose name it carries.

**The marker is scanned for, and exactly one occurrence is a stub.** The name is assembled at run
time from `GINARY-STUB` and `-ID\0`, so the scanner's own `.rodata` never holds it contiguously.
That is not decoration: an artifact *is* a ginary with a payload appended, so a reader that
matched itself would find two identities in every file it looked at. Zero occurrences is
`NotAStub`, more than one is `Ambiguous`, and the padding after the body's NUL must be zero, so a
marker is a record rather than a substring.

**`verify` is seven gates and the header outranks the marker.** Size, exactly one marker, this
ginary's version, this payload format, the target that was asked for, an object header that
agrees, and no trailer. The version gate is what makes stubs *version-locked*: the launcher in a
stub reads the payload this ginary writes, and "probably compatible" is not a thing a build may
decide on a user's behalf. The header gate exists because the marker is text and text copies —
`stub_copy` in the test suite rewrites one in seconds — while `e_machine` is what the linker
wrote. The trailer gate is last because it is the one question about what was *done* to the file
rather than about what the file is.

**`cli` is a default feature, and `--no-default-features` is the stub.** It gates clap and the
TOML reader, and with them `cli`, `bundle`, `gleam`, `inspect`, `verify`, `sbom`, `crashdump`,
`doctor`, `report`, `strip`, `erts_source`, `elf`, `beam`, `closure`, `appfile`, `otp`,
`cache_dir` and `stub` itself. Two modules are split rather than gated, because the launcher
needs a few items out of each: `assemble` keeps its staging *listing* types, which
`manifest::Index` and `payload::pack` are written in terms of, and gates the machinery that
builds a tree; `config` keeps the filename-encoding table, which is a rule about an emulator flag
and belongs to the launch path, and gates everything else. `process` stays whole: it is
dependency-free, nothing on the launcher path calls it, and the linker drops what `main` cannot
reach.

The order is the point. Anything the launcher needs is unconditional, and everything else is
gated — not the other way round.

**Locating a stub is a fixed order and never falls back from an instruction.** `--stub`, then
`$GINARY_STUB_DIR/ginary-stub-<version>-<target>` and `.../ginary-<version>-<target>`, then the
running executable when the target is the host, then `<cache>/stubs/<version>/<target>`. A
`--stub` that is not there is refused rather than searched past: a build that quietly packaged a
different file than the one the user named would be wrong in a way nobody could see. A target
with no stub anywhere prints every path it tried and the task that makes one.

**The suite runs in both flavors.** `mise run test:stub` is `cargo test --no-default-features`
and `mise run lint:stub` is clippy over the same, and both are in `check`. A test target whose
claims are all about the command line half carries `#![cfg(feature = "cli")]`; what is left —
the launcher contract, the payload, the cache, the trailer and the marker — runs against a binary
built without clap. `launcher::no_payload_line` exists so that the one sentence only the stub
build prints is asserted by both.

## Consequences

A cross-target artifact can be produced today, given a stub and a runtime. The stub half is
finished for Linux: `mise run stubs:build` attempts five targets into `target/stubs`, the four
Linux ones build, and `GINARY_STUB_DIR` points a build at them. `windows-x86_64` is attempted and
does not compile — the launcher path is Unix-only until Phase D — so the task reports it and
exits non-zero rather than quietly writing four files where five were asked for. The runtime half
is not finished: there is no catalogue, so a target other than the host must be given
`[tools.ginary.target.<name>] erts = "dir:..."`, and a build that is not given one refuses,
quoting the table to write. That refusal is the honest end of the chain, and the test suite
asserts exactly it: with a real cross-built musl stub and the host's glibc runtime, the stub
passes every gate and the *runtime* is refused.

The two macOS stubs cannot be built here at all — there is no macOS toolchain in a Linux
container — so `stubs:build` names five targets rather than seven and says why, and a `macos-*`
stub comes from the release build on a macOS runner. `stub::verify` refuses a Mach-O with
`NotYetSupported` rather than guessing at a check nothing on this machine could exercise.

The feature split costs a third configuration in the gate and a `#[cfg]` on about a hundred
items, most of them in `assemble.rs` and `config.rs`. It buys a stub of 1.0 MB against 3.7 MB for
the full build, and — the reason it is worth having — a compiler-checked statement of what the
launcher path actually depends on. A module that quietly grew a build-side dependency used to be
invisible; now it fails `lint:stub`.

Two claims are asserted less often than before. `tests/regressions.rs` runs 15 of its 49
modules — 29 of its 105 tests — in the stub flavor and the rest only in the full one, and the
gated integration targets run only in the full one. Nothing runs in *neither*: the default `cargo test` is unchanged, and the
milestone's own record lists what each flavor covers.
