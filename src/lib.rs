// SPDX-License-Identifier: MIT OR Apache-2.0
//! ginary packages a Gleam application and a trimmed BEAM runtime into one
//! executable.
//!
//! The crate is a single binary that runs in two modes. A plain `ginary` is the
//! command line tool that builds artifacts; a copy of the same binary with a
//! payload appended to it is the *launcher* that a packaged application runs
//! under. Only the command line half exists today.
//!
//! Milestone A0 provides the scaffolding those two halves share:
//!
//! - [`target`] — the `<os>-<arch>[-<libc>]` model used by manifests, artifact
//!   names and the stub catalogue;
//! - [`cache_dir`] — where an artifact extracts its runtime;
//! - [`doctor`] — what the local machine can and cannot do;
//! - [`cli`] — argument parsing and command dispatch.
//!
//! Milestone A1a adds the first two build-side modules and the helper they
//! share with `doctor`:
//!
//! - [`appfile`] — the subset of Erlang term syntax an `.app` file uses;
//! - [`otp`] — where the host OTP installation is and whether it is usable;
//! - [`process`] — finding a program on `PATH` and running it under a timeout,
//!   shared by [`doctor`] and [`otp`].
//!
//! Milestone A1b adds the module that turns those two into a bill of
//! materials:
//!
//! - [`closure`] — every application an artifact needs, resolved against the
//!   shipment and the OTP library.
//!
//! Milestone A1c adds the module that turns that bill of materials into a
//! directory:
//!
//! - [`assemble`] — the staging root, the exact tree the payload is made of.
//!
//! Milestone A2 adds the four modules that make the staged tree small enough
//! to ship and say what it costs:
//!
//! - [`beam`] — the chunk table of a compiled BEAM module;
//! - [`elf`] — read-only inspection of a native binary;
//! - [`strip`] — `strip(1)` on the ELF files and `beam_lib:strip_files/1` on
//!   the modules, each verified afterwards;
//! - [`report`] — the size breakdown and the `needs:` line.
//!
//! Milestone A3a adds the payload format itself, the four modules an artifact
//! is made of and read back with:
//!
//! - [`trailer`] — the 64 bytes at the end of a packaged application;
//! - [`manifest`] — `ginary.json` and `ginary.index.json`;
//! - [`payload`] — the deterministic tar and zstd stream, and its hostile
//!   reader;
//! - [`diag`] — `GINARY_DEBUG` and `GINARY_TRACE`, the launcher's only
//!   observability.
//!
//! Milestone A3b adds the launcher itself, the half of the binary a packaged
//! application actually runs:
//!
//! - [`error`] — the numbered exit codes 121 to 125 and the diagnostics that
//!   go with them;
//! - [`selfexe`] — opening the running executable by inode;
//! - [`cache`] — where the runtime extracts, and the atomic extraction;
//! - [`cache_lock`] — the `flock` a running application holds on its entry,
//!   and the exclusive one pruning needs before it may remove another;
//! - [`launch`] — the argument vector and the environment difference;
//! - [`launcher`] — the launcher-mode entry point and `GINARY_CMD`;
//! - [`fault`] — the named fault points the launcher tests arm, compiled in
//!   only under the `fault-injection` feature.
//!
//! Milestone A4 adds the four modules that turn all of the above into one
//! command, and the two commands that drive them:
//!
//! - [`config`] — `[tools.ginary]` in `gleam.toml`, and the CLI flags merged
//!   over it;
//! - [`gleam`] — finding the project and running
//!   `gleam export erlang-shipment`;
//! - [`bundle`] — the whole build, from a project to one executable;
//! - [`inspect`] — reading a packaged application from the outside.
//!
//! Milestone C1 adds the module that says what a bundled runtime really is,
//! and the multi-target plumbing around it:
//!
//! - [`erts_source`] — `host`, a directory, a tarball, the catalogue or a
//!   container image, resolved and then checked against the emulator itself.
//!
//! Milestone B2 adds the three modules that answer questions *about* a
//! finished artifact, rather than producing one:
//!
//! - [`verify`] — the deep check: every file against the index, every ELF
//!   against the target and the allowlist;
//! - [`sbom`] — the SPDX 2.3 bill of materials;
//! - [`crashdump`] — the summary of an `erl_crash.dump`.
//!
//! Milestone C2 adds the two modules a cross-target build is made of, and the
//! `cli` feature that makes a launcher-only build possible:
//!
//! - [`stubid`] — the identity marker every ginary binary carries, and the
//!   scanner that reads it back;
//! - [`stub`] — where the stub for a target comes from, and the gates it has
//!   to pass before a payload is appended to it.
//!
//! Milestone C4 adds the module that decides whether the native code in a
//! shipment can travel to the target being built for:
//!
//! - [`native`] — the objects under `priv`, the overrides and build hooks that
//!   replace them, and the two refusals a cross build owes its user.
//!
//! Milestone C3 adds the two modules a cross-target build gets its runtime
//! from, and the local pipeline that fills them:
//!
//! - [`download`] — one HTTPS fetch, hashed, retried and renamed into place;
//! - [`catalog`] — the prebuilt-OTP catalogue, the cache it fills, and
//!   `ginary otp repack`, which produces both without publishing anything.
//!
//! Milestone D2 adds the two modules Windows needs and nothing else does:
//!
//! - [`winpath`] — the `\\?\` prefix a deep cache entry is extracted under,
//!   and the identity that stands in for it on unix;
//! - `launch_windows` — the spawn-and-wait launcher, compiled only for
//!   Windows, which stays resident because there is no `execve` to hand the
//!   process over with.
//!
//! Milestone D3 adds macOS packaging, to the limit of what a Linux host can
//! prove:
//!
//! - [`macho`] — read-only inspection of Mach-O binaries: `cputype`, whether
//!   the file is fat, whether it carries a code signature, and where a named
//!   section is; not gated behind the `cli` feature, because a launched
//!   macOS artifact locates its own payload through it;
//! - [`sign_macos`] — writing the `__GINARY,__payload` section into a Mach-O
//!   stub and applying an ad-hoc code signature.
//!
//! See `docs/dev/architecture.md` for the module map of the finished tool and
//! `docs/format.md` for the payload format the launcher will read.

#![warn(missing_docs)]
// `deny` rather than `forbid`, and the difference is one module. Every line of
// ginary is safe Rust except `launch_windows::win32`, which makes the three
// `kernel32` calls the Windows launcher cannot be written without — the console
// control handler and the job object that keeps a killed launcher from
// orphaning a runtime. Neither has a safe counterpart anywhere, and `forbid`
// cannot be lifted for a single module. That module carries the only
// `#[allow(unsafe_code)]` in the crate; `deny` keeps every other file, and
// every other target, exactly as strict as `forbid` was. See
// `docs/adr/0015-windows-launcher-stays-resident.md`.
#![deny(unsafe_code)]

#[cfg(feature = "cli")]
pub mod appfile;
pub mod assemble;
#[cfg(feature = "cli")]
pub mod beam;
#[cfg(feature = "cli")]
pub mod bundle;
pub mod cache;
#[cfg(feature = "cli")]
pub mod cache_dir;
pub mod cache_lock;
#[cfg(feature = "cli")]
pub mod catalog;
#[cfg(feature = "cli")]
pub mod cli;
#[cfg(feature = "cli")]
pub mod closure;
pub mod config;
#[cfg(feature = "cli")]
pub mod crashdump;
pub mod diag;
#[cfg(feature = "cli")]
pub mod doctor;
#[cfg(feature = "cli")]
pub mod download;
#[cfg(feature = "cli")]
pub mod elf;
pub mod error;
#[cfg(feature = "cli")]
pub mod erts_source;
pub mod fault;
#[cfg(feature = "cli")]
pub mod gleam;
#[cfg(feature = "cli")]
pub mod inspect;
pub mod launch;
#[cfg(windows)]
pub mod launch_windows;
pub mod launcher;
pub mod macho;
pub mod manifest;
#[cfg(feature = "cli")]
pub mod native;
#[cfg(feature = "cli")]
pub mod otp;
pub mod payload;
pub mod platform;
pub mod process;
#[cfg(feature = "cli")]
pub mod report;
#[cfg(feature = "cli")]
pub mod sbom;
pub mod selfexe;
#[cfg(feature = "cli")]
pub mod sign_macos;
#[cfg(feature = "cli")]
pub mod strip;
#[cfg(feature = "cli")]
pub mod stub;
pub mod stubid;
pub mod target;
pub mod trailer;
#[cfg(feature = "cli")]
pub mod verify;
pub mod winpath;
