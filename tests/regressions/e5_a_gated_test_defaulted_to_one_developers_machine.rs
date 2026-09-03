// SPDX-License-Identifier: MIT OR Apache-2.0
//! The gated real-shipment test defaulted to a path inside one developer's
//! home directory, and `GINARY_REQUIRE_TOOLCHAIN=1` turned that into a failed
//! CI job.
//!
//! **What went wrong.** `tests/closure.rs` carried
//! `const DEFAULT_REAL_SHIPMENT: &str = "/home/<the author>/projects/gleam/notify/build/erlang-shipment"`
//! and fell back to it whenever `GINARY_TEST_SHIPMENT` was unset. On the
//! author's machine the path is a directory and the test runs; anywhere else
//! it is not, and the fallback was then escalated by the same rule
//! `require_tools` uses: `GINARY_REQUIRE_TOOLCHAIN=1` forbids skipping. CI
//! sets that variable on `test`, `smoke`, `smoke-matrix` and `coverage`, so
//! the first live run on `main` failed both the `test` job and the `coverage`
//! job with
//!
//! ```text
//! `/home/.../notify/build/erlang-shipment` is not a directory and
//! GINARY_REQUIRE_TOOLCHAIN=1 forbids skipping
//! ```
//!
//! (run <https://github.com/P4suta/ginary/actions/runs/33658759531>). The
//! coverage gate never got to run at all — the suite failed underneath it.
//!
//! **The input.** A machine with Erlang and Gleam installed,
//! `GINARY_REQUIRE_TOOLCHAIN=1`, and no `GINARY_TEST_SHIPMENT`: a hosted
//! runner, or any contributor who followed `docs/dev/testing.md`.
//!
//! **The correct behaviour.** `GINARY_REQUIRE_TOOLCHAIN` is a claim about
//! programs the machine installs, not about Gleam projects somebody exported.
//! An unnamed shipment is therefore a loud skip however that flag is set, and
//! a *named* shipment that is not a directory is a failure however it is set,
//! because the caller asked for a run and mistyped the path. No path on one
//! machine is a default. The rule lives in `tests/common/shipment.rs` so both
//! halves of it can be asserted here without a filesystem.
//!
//! **The scan.** The general form of the bug — no tracked file under `src/`,
//! `tests/`, `scripts/` or `.github/` may carry a person's absolute home
//! path — was asserted by a last test in this file, and it asked *this
//! machine* what its home directory was. That made it a rule with a different
//! meaning on every machine and the wrong meaning on a hosted runner, where
//! `$HOME` is `/home/runner` and every pasted CI transcript contains it: it
//! policed prose there, and everywhere else a developer's path hard-coded by
//! somebody else passed, which is exactly the defect above. The rule is now
//! machine-independent and lives in `crate::common::homepath`, asserted by
//! `tests/regressions/e7_the_home_directory_scan_only_worked_on_one_machine.rs`;
//! what survives unchanged is everything that was right about it — bytes
//! rather than decoded text, `git ls-files` rather than a directory walk, an
//! unreadable file reported rather than skipped, and the three `.beam`
//! fixtures as the one argued exception.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use crate::common::shipment::{SHIPMENT_VAR, ShipmentChoice, choose_shipment};

#[test]
fn an_unnamed_shipment_is_a_skip_even_when_the_toolchain_is_required() {
    let choice = choose_shipment(None, true, &|_| panic!("nothing to look at on disk"));
    assert_eq!(
        choice,
        ShipmentChoice::Skip(format!(
            "{SHIPMENT_VAR} is not set: there is no default shipment to fall back to"
        )),
        "GINARY_REQUIRE_TOOLCHAIN says which programs the machine installs; it cannot say that \
         somebody exported a Gleam project here"
    );
}

#[test]
fn a_named_shipment_that_is_a_directory_is_the_one_the_test_runs_over() {
    let named = OsStr::new("/somewhere/notify/build/erlang-shipment");
    let choice = choose_shipment(Some(named), false, &|path: &Path| {
        path == Path::new("/somewhere/notify/build/erlang-shipment")
    });
    assert_eq!(
        choice,
        ShipmentChoice::Run(PathBuf::from("/somewhere/notify/build/erlang-shipment")),
        "a caller who named a shipment gets it, whatever the toolchain flag says"
    );
}

#[test]
fn a_named_shipment_that_is_not_a_directory_fails_however_the_toolchain_flag_is_set() {
    for required in [false, true] {
        let choice = choose_shipment(Some(OsStr::new("/nowhere/shipment")), required, &|_| false);
        assert_eq!(
            choice,
            ShipmentChoice::Fail(format!(
                "{SHIPMENT_VAR} names `/nowhere/shipment`, which is not a directory: point it at \
                 a `gleam export erlang-shipment` output"
            )),
            "a named shipment that is not there is a typo, not a machine without a toolchain \
             (GINARY_REQUIRE_TOOLCHAIN={required})"
        );
    }
}

#[test]
fn an_empty_shipment_variable_is_the_same_as_an_unnamed_one() {
    // `env: GINARY_TEST_SHIPMENT: ${{ vars.SOMETHING }}` with the variable
    // unset expands to the empty string, which `std::env::var_os` reports as
    // `Some("")` and not as `None`. That is a workflow saying nothing, not a
    // caller naming a shipment, so it is the same loud skip.
    for required in [false, true] {
        for named in ["", "   ", "\t"] {
            let choice = choose_shipment(Some(OsStr::new(named)), required, &|_| {
                panic!("an empty value names nothing to look at on disk")
            });
            assert_eq!(
                choice,
                ShipmentChoice::Skip(format!(
                    "{SHIPMENT_VAR} is not set: there is no default shipment to fall back to"
                )),
                "an empty {SHIPMENT_VAR} is a variable nobody set, not a directory that is \
                 missing (GINARY_REQUIRE_TOOLCHAIN={required}, value {named:?})"
            );
        }
    }
}
