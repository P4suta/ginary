// SPDX-License-Identifier: MIT OR Apache-2.0
//! `GINARY_REQUIRE_TOOLCHAIN=1` turned a cross-built stub nobody had built
//! into a failure, and took the `test` and `coverage` jobs down with it.
//!
//! **What went wrong.** `tests/common/stubfile.rs::cross_stub` searches
//! `$GINARY_STUB_DIR` and then `target/stubs` for a ginary of this version
//! cross-compiled for a target, and escalates a miss to a panic when
//! `GINARY_REQUIRE_TOOLCHAIN=1`. The `test` and `coverage` jobs of `ci.yml`
//! set that variable and never build a stub: the cross builds live in the
//! `cross-build` and `smoke-matrix` jobs, in their own containers, on their own
//! runners. So the first pull-request run failed both jobs on the same three
//! tests:
//!
//! ```text
//! thread 'a_static_musl_artifact_runs_on_alpine_with_no_erlang_and_no_network'
//! panicked at tests/common/stubfile.rs:319:5:
//! no ginary-stub-0.1.0-linux-x86_64-musl in any of
//! ["/home/runner/work/ginary/ginary/target/stubs"] and
//! GINARY_REQUIRE_TOOLCHAIN=1 forbids skipping
//! ```
//!
//! (`Test (both flavors, stable)`
//! <https://github.com/P4suta/ginary/actions/runs/33681144884/job/100417745894>
//! and `Coverage`
//! <https://github.com/P4suta/ginary/actions/runs/33681144884/job/100417746014>;
//! under coverage the same three failed inside `cargo llvm-cov`, so the 90%
//! gate never ran at all.)
//!
//! This is the same confusion E5 took out of the shipment gate, one variable
//! later: `GINARY_REQUIRE_TOOLCHAIN` is a claim about *programs the machine
//! installs*. A cross-built stub is not a program on `PATH`. It is the output
//! of `mise run stubs:build`, which needs `cross`, a docker daemon and minutes
//! per target, and a runner with a complete Erlang toolchain has none of them.
//! See `tests/regressions/e5_a_gated_test_defaulted_to_one_developers_machine.rs`
//! for the first half of the argument.
//!
//! **The input.** Any machine with the toolchain installed,
//! `GINARY_REQUIRE_TOOLCHAIN=1`, and no `target/stubs`: the two hosted jobs
//! above, and any contributor who followed `docs/dev/testing.md` without
//! running the stub task first.
//!
//! **The correct behaviour.** A missing cross stub is a loud skip however
//! `GINARY_REQUIRE_TOOLCHAIN` is set. The job that *does* build the stubs says
//! so with a variable of its own, `GINARY_REQUIRE_STUBS=1`, and that one turns
//! the same miss into a failure — because there a missing stub means the cross
//! build silently produced nothing. Two questions, two variables, and neither
//! answers for the other. The rule lives in
//! [`crate::common::stubfile::choose_cross_stub`] so both halves can be
//! asserted without a filesystem, and the new variable is documented where the
//! old one is.

use std::path::{Path, PathBuf};

use crate::common::repo::read;
use crate::common::stubfile::{REQUIRE_STUBS_VAR, StubChoice, choose_cross_stub, stub_requirement};
use crate::common::tools::REQUIRE_VAR;

/// The stub the three `tests/e2e_cross.rs` tests could not find on the runner.
const NAME: &str = "ginary-stub-0.1.0-linux-x86_64-musl";

/// The one directory the failing jobs searched.
fn searched() -> Vec<PathBuf> {
    vec![PathBuf::from(
        "/home/runner/work/ginary/ginary/target/stubs",
    )]
}

#[test]
fn a_missing_cross_stub_is_a_skip_even_when_the_toolchain_is_required() {
    let dirs = searched();
    let choice = choose_cross_stub(NAME, &dirs, true, false, &|_| false);
    assert_eq!(
        choice,
        StubChoice::Skip(format!(
            "no {NAME} in any of {dirs:?}: run `mise run stubs:build` or set GINARY_STUB_DIR"
        )),
        "GINARY_REQUIRE_TOOLCHAIN says which programs the machine installs; a cross-built stub \
         is not one of them, and no amount of Erlang on the runner produces one"
    );
}

#[test]
fn a_job_that_promises_the_cross_stubs_turns_a_missing_one_into_a_failure() {
    let dirs = searched();
    for toolchain in [false, true] {
        let choice = choose_cross_stub(NAME, &dirs, toolchain, true, &|_| false);
        assert_eq!(
            choice,
            StubChoice::Fail(format!(
                "no {NAME} in any of {dirs:?}: {REQUIRE_STUBS_VAR}=1 says this job obtained \
                 the stubs, so the step that built or downloaded them produced nothing for this \
                 target — check the step that fills target/stubs in this job"
            )),
            "a job that obtained the stubs and then cannot find one has a broken step, not a \
             machine without them. Two jobs obtain them by two different means, so the message \
             names neither step: `smoke-matrix` cross-builds, `coverage` downloads \
             (GINARY_REQUIRE_TOOLCHAIN={toolchain})"
        );
    }
}

#[test]
fn the_variable_the_wiring_reads_for_the_stubs_is_not_the_one_for_the_toolchain() {
    // The rule above is pure and was never the defect. The defect was one
    // line of wiring reading `GINARY_REQUIRE_TOOLCHAIN` where it meant
    // `GINARY_REQUIRE_STUBS`, and swapping the two names back left every
    // assertion in this file green. `stub_requirement` is that line, with the
    // environment handed in.
    let only = |wanted: &'static str| {
        move |name: &str| (name == wanted).then(|| std::ffi::OsString::from("1"))
    };
    assert_eq!(
        stub_requirement(&only(REQUIRE_VAR)),
        (true, false),
        "{REQUIRE_VAR}=1 alone must not require a cross stub: that is the shape that failed the \
         `test` and `coverage` jobs of the first pull-request run"
    );
    assert_eq!(
        stub_requirement(&only(REQUIRE_STUBS_VAR)),
        (false, true),
        "{REQUIRE_STUBS_VAR}=1 alone must require a cross stub, whatever the toolchain variable \
         says"
    );
    assert_eq!(
        stub_requirement(&|_| None),
        (false, false),
        "a machine that sets neither skips, loudly"
    );
    assert_eq!(
        stub_requirement(&|_| Some(std::ffi::OsString::from("0"))),
        (false, false),
        "`=0` is a contributor saying no, not a variable that happens to be present"
    );
}

#[test]
fn the_first_directory_holding_the_stub_is_the_one_the_test_runs_against() {
    let dirs = vec![
        PathBuf::from("/from/the/environment"),
        PathBuf::from("/from/the/repository"),
    ];
    let choice = choose_cross_stub(NAME, &dirs, false, true, &|path: &Path| {
        path.starts_with("/from/the")
    });
    assert_eq!(
        choice,
        StubChoice::Run(PathBuf::from("/from/the/environment").join(NAME)),
        "GINARY_STUB_DIR is searched before the repository's own target/stubs, and a stub that \
         is there is used whatever either variable says"
    );
}

#[test]
fn the_variable_that_requires_the_stubs_is_documented_where_the_other_one_is() {
    for (relative, why) in [
        (
            "docs/dev/debugging.md",
            "the table of every GINARY_ variable a contributor may set",
        ),
        (
            "docs/dev/testing.md",
            "the page that states when a gated test skips and when it must not",
        ),
    ] {
        let text = read(relative);
        assert!(
            text.contains(REQUIRE_STUBS_VAR),
            "{relative} does not mention {REQUIRE_STUBS_VAR}, and that file is {why}. A skip \
             nobody can find the switch for is a silent skip"
        );
    }
}
