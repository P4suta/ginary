<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# 0002 — One Rust binary that is both the packager and the stub

Status: Accepted · 2026-08-30

## Context

A packager needs a *stub*: the executable that ends up in front of the payload and that starts
the runtime at launch time. Existing tools obtain one by building a separate launcher — Burrito
compiles a Zig launcher, Bakeware a C one — which means the launcher is a second toolchain, a
second build, a second artifact to distribute for every target, and a second place where a bug
can hide.

The project also has to be cross-target: v1 covers Linux glibc and musl on x86_64 and aarch64,
macOS on both architectures, and Windows on x86_64. Requiring a C or Zig cross-toolchain per
target multiplies the build matrix.

Rust was chosen as the implementation language independently: it matches the house style of the
neighbouring projects, it can produce a fully static binary against musl, and it has the
`object`, `tar` and `zstd` bindings the packager needs.

## Decision

ginary is **one Rust crate producing one binary** (edition 2024, `rust-version = "1.88"`), and
that binary is its own stub.

- `ginary build` copies a ginary executable for the requested target to the output path and
  appends `[payload][trailer]` to the copy. For a host build the source of that copy is
  `current_exe()`; for a cross build it is a ginary binary for the target, obtained through
  `--stub`, `$GINARY_STUB_DIR`, the local cache or GitHub Releases.
- At start-up the binary reads the last 64 bytes of its own file. No trailer means CLI mode; a
  valid trailer means launcher mode. A trailer that is present but damaged is a hard error, not
  a fallback to the CLI, so a corrupt application never prints ginary's help.
- A Cargo feature `cli` (on by default) gates clap, the network stack, docker and catalogue
  support. `--no-default-features` yields a roughly 1 MB launcher-only `ginary-stub` for use as
  a cross-target stub.
- A stub carries an embedded 96-byte `GINARY_STUB_ID` marker recording version, target, payload
  format and flavour, so the builder can verify that a supplied stub is genuine and matches the
  target before appending anything to it.

## Consequences

There is one toolchain, one build and one test suite. A cross-target release is `cargo build
--target ...` seven times, and each output is simultaneously a usable CLI and a usable stub.
`ginary build` executed from an artifact is refused, because a stub with a payload cannot be a
stub again.

The cost is that the launcher path and the builder path share a process. Nothing on the launcher
path may pull in clap, the network, or a panic: the mode decision happens before argument
parsing, and launcher failures map to numbered exit codes. This is enforced by review and by the
prohibitions in `CLAUDE.md`, not by the compiler.

macOS breaks the "append to the end" rule: appended bytes fail `codesign --strict` and are
killed by the kernel on arm64. There the same trailer structure goes into a `__GINARY,__payload`
Mach-O section instead, which is why every consumer downstream of the locator sees only a
`(file, offset, len)` stream.
