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
//! Milestone B2 adds the three modules that answer questions *about* a
//! finished artifact, rather than producing one:
//!
//! - [`verify`] — the deep check: every file against the index, every ELF
//!   against the target and the allowlist;
//! - [`sbom`] — the SPDX 2.3 bill of materials;
//! - [`crashdump`] — the summary of an `erl_crash.dump`.
//!
//! See `docs/dev/architecture.md` for the module map of the finished tool and
//! `docs/format.md` for the payload format the launcher will read.

#![warn(missing_docs)]
#![forbid(unsafe_code)]

pub mod appfile;
pub mod assemble;
pub mod beam;
pub mod bundle;
pub mod cache;
pub mod cache_dir;
pub mod cache_lock;
pub mod cli;
pub mod closure;
pub mod config;
pub mod crashdump;
pub mod diag;
pub mod doctor;
pub mod elf;
pub mod error;
pub mod fault;
pub mod gleam;
pub mod inspect;
pub mod launch;
pub mod launcher;
pub mod manifest;
pub mod otp;
pub mod payload;
pub mod process;
pub mod report;
pub mod sbom;
pub mod selfexe;
pub mod strip;
pub mod target;
pub mod trailer;
pub mod verify;
