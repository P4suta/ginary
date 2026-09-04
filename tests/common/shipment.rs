// SPDX-License-Identifier: MIT OR Apache-2.0
//! Choosing the real `erlang-shipment` a gated test runs over.
//!
//! One test in `tests/closure.rs` runs the closure computation over a shipment
//! a real `gleam export erlang-shipment` produced, rather than over the fake
//! trees the rest of the file builds. A shipment is not a program on `PATH`,
//! so [`crate::common::tools::require_tools`] cannot answer for it: the caller
//! has to say where one is, through `GINARY_TEST_SHIPMENT`.
//!
//! The distinction this module exists for is the one the first live CI run
//! found. `GINARY_REQUIRE_TOOLCHAIN=1` says "this machine installs the
//! toolchain, so a missing program is a failure and never a skip". It does not
//! and cannot say "this machine has a Gleam project exported somewhere", and a
//! default path baked into the suite is a claim about exactly one machine. So
//! the two questions are separated here:
//!
//! - nobody named a shipment — the variable is unset, or set to nothing at
//!   all, which is what an unset `vars.` expansion leaves behind: skip,
//!   loudly, whatever the toolchain flag says;
//! - somebody named one and it is not a directory: fail, whatever the flag
//!   says, because the caller asked for a run and got a typo.
//!
//! Keeping the rule in one pure function, away from the environment, is what
//! makes it testable — see
//! `tests/regressions/e5_a_gated_test_defaulted_to_one_developers_machine.rs`.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// The variable that points a gated test at a shipment of the caller's.
pub const SHIPMENT_VAR: &str = "GINARY_TEST_SHIPMENT";

/// What a gated shipment test should do, given what the environment says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShipmentChoice {
    /// Run over this directory.
    Run(PathBuf),
    /// Do not run, and print this reason on standard error.
    Skip(String),
    /// Fail, with this message: the caller named a shipment that is not one.
    Fail(String),
}

/// Decides between running, skipping and failing.
///
/// `named` is the raw value of [`SHIPMENT_VAR`], `required` is whether
/// `GINARY_REQUIRE_TOOLCHAIN` is `1`, and `is_dir` answers whether a path is a
/// directory — passed in so the rule can be tested without a filesystem.
///
/// `required` is deliberately a parameter rather than an omission: the rule is
/// that it changes nothing here, and a rule nobody can assert is not a rule.
pub fn choose_shipment(
    named: Option<&OsStr>,
    required: bool,
    is_dir: &dyn Fn(&Path) -> bool,
) -> ShipmentChoice {
    // `required` is read and discarded on purpose. The rule below is the same
    // whichever way `GINARY_REQUIRE_TOOLCHAIN` is set, and a parameter that is
    // never mentioned would let a later edit reintroduce the escalation this
    // module exists to prevent without the signature changing.
    let _ = required;

    // An empty value is a variable nobody set. `env: GINARY_TEST_SHIPMENT:
    // ${{ vars.SOMETHING }}` with the variable unset expands to the empty
    // string, and `std::env::var_os` reports that as `Some("")` rather than as
    // `None`; treating it as a *named* shipment turns the "nobody named one"
    // case this module exists to make a skip into a failure whose message has
    // empty backticks where the path should be.
    let named = named.filter(|named| !named.as_encoded_bytes().iter().all(u8::is_ascii_whitespace));
    let Some(named) = named else {
        return ShipmentChoice::Skip(format!(
            "{SHIPMENT_VAR} is not set: there is no default shipment to fall back to"
        ));
    };
    let path = PathBuf::from(named);
    if is_dir(&path) {
        return ShipmentChoice::Run(path);
    }
    ShipmentChoice::Fail(format!(
        "{SHIPMENT_VAR} names `{}`, which is not a directory: point it at a `gleam export \
         erlang-shipment` output",
        path.display()
    ))
}
