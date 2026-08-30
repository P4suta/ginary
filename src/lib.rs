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
//! See `docs/dev/architecture.md` for the module map of the finished tool and
//! `docs/format.md` for the payload format the launcher will read.

#![warn(missing_docs)]
#![forbid(unsafe_code)]

pub mod cache_dir;
pub mod cli;
pub mod doctor;
pub mod target;
