// SPDX-License-Identifier: MIT OR Apache-2.0
//! Helpers shared by the integration tests.
//!
//! Cargo builds one binary per file directly under `tests/`, and each of those
//! binaries includes this module with `mod common;`. A helper that only one
//! binary uses is therefore dead code in the others, which is why the whole
//! module allows it: the alternative is a `#[allow]` on every item, or one
//! module per test binary.

#![allow(dead_code)]

pub mod artifact;
pub mod bounded;
pub mod erl;
pub mod fake_otp;
pub mod fixture;
pub mod payload;
pub mod script;
pub mod snapshot;
pub mod tools;
