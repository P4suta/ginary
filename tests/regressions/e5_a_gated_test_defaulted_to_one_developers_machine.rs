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
//! **The scan.** The last test in this file is the general form of the bug: no
//! file `git` tracks under `tests/`, `src/`, `scripts/` or `.github/` may
//! contain this machine's `$HOME`. Three details of it are load-bearing. It
//! reads *bytes*, because the first version decoded each file as UTF-8 and
//! silently dropped everything that was not — which is the class of file most
//! likely to embed an absolute path, and three tracked `.beam` fixtures did.
//! It walks `git ls-files` rather than the directory tree, because its failure
//! says *tracked* and a local `gleam build` fills
//! `tests/fixtures/hello_ffi/build/` with absolute paths that belong to
//! nobody's repository. And a file it cannot read at all is a reported
//! failure, not a pass: skipping is a decision somebody makes on the record.
//! The one exception is [`ALLOWED`], argued there and in
//! `tests/fixtures/beam/README.md`.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use crate::common::repo::root;
use crate::common::shipment::{SHIPMENT_VAR, ShipmentChoice, choose_shipment};

/// This file quotes the very paths the scan below looks for, so the scan skips
/// it by name. Nothing else under the scanned directories may.
const SELF: &str = "e5_a_gated_test_defaulted_to_one_developers_machine.rs";

/// The directories that must never name the machine they were written on.
///
/// `docs/` is deliberately absent: a milestone log quoting a CI failure is
/// supposed to reproduce the failing path verbatim, and this very bug is
/// recorded in `docs/dev/log/E5.md` with the author's path in it.
const SCANNED: [&str; 4] = ["tests", "src", "scripts", ".github"];

/// The tracked files that are allowed to contain an absolute home path, and
/// the reason each one is.
///
/// One reason, and only one: a `.beam` file is a compiled artifact, and the
/// Erlang compiler records the absolute path of the `.erl` it compiled in the
/// file's `Dbgi` chunk. These three were copied *verbatim* out of a real
/// `gleam export erlang-shipment`, which is the whole point of them — they are
/// the fixture that shows what a real compiler emits, against the hand-built
/// byte strings in `tests/beam.rs` that pin the grammar. Rewriting the chunk
/// would make them no longer what a compiler wrote; recompiling `gleam_stdlib`
/// with a relative `-o` would change every offset and size the README records
/// and would no longer be `gleam_stdlib` 1.0.5 as shipped.
///
/// So the path stays, as an argued exception rather than as a file this scan
/// happened not to be able to read. `tests/fixtures/beam/README.md` says the
/// same thing where somebody looking at the fixtures will find it. The rule
/// the exception is carved out of is unchanged: no path on one machine is a
/// default, a fallback, or a value any code reads.
const ALLOWED: [&str; 3] = [
    "tests/fixtures/beam/gleam@bool.beam",
    "tests/fixtures/beam/gleam@list.beam",
    "tests/fixtures/beam/gleam@string.beam",
];

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

#[test]
fn no_source_or_test_file_names_the_home_directory_of_the_machine_it_was_written_on() {
    let Some(home) = home_directory() else {
        eprintln!("skipping: no usable home directory to look for");
        return;
    };
    let Some(tracked) = tracked_files() else {
        eprintln!("skipping: `git ls-files` did not answer, so `tracked` would be a guess");
        return;
    };
    let mut offenders: Vec<String> = Vec::new();
    let mut unreadable: Vec<String> = Vec::new();
    for relative in tracked {
        if relative.ends_with(SELF) || ALLOWED.contains(&relative.as_str()) {
            continue;
        }
        let Ok(bytes) = std::fs::read(root().join(&relative)) else {
            unreadable.push(relative);
            continue;
        };
        for (index, line) in bytes.split(|byte| *byte == b'\n').enumerate() {
            if line
                .windows(home.len())
                .any(|window| window == home.as_bytes())
            {
                offenders.push(format!("{relative}:{}", index + 1));
            }
        }
    }
    let stale: Vec<&str> = ALLOWED
        .into_iter()
        .filter(|allowed| !root().join(allowed).is_file())
        .collect();
    assert!(
        stale.is_empty(),
        "an entry of ALLOWED names a file that is not in the tree any more. An exception nobody \
         needs is an exception nobody argued for:\n{}",
        stale.join("\n")
    );
    assert!(
        unreadable.is_empty(),
        "a tracked file under {SCANNED:?} could not be read, so nobody knows what is in it. A \
         file this scan cannot open is a reported failure, never a silent pass:\n{}",
        unreadable.join("\n")
    );
    assert!(
        offenders.is_empty(),
        "a tracked file names `{home}`, the home directory of the machine it was written on. A \
         path that exists on one machine is not a default, a fixture or a fallback:\n{}",
        offenders.join("\n")
    );
}

/// Every file `git` tracks under [`SCANNED`], repository-relative.
///
/// The walk is `git ls-files` rather than a directory read because the failure
/// this test prints says *tracked*, and a directory read also enumerates build
/// output: `tests/fixtures/hello_ffi/build/` appears the moment a contributor
/// runs `gleam build`, and it contains absolute paths by construction. A
/// gitignored artifact naming this machine is not a bug in the repository.
///
/// `None` when `git` cannot answer at all — no `git` on `PATH`, or a source
/// tree unpacked from a tarball. That is a reported skip rather than a quiet
/// fallback to the directory read, because the two answer different questions.
fn tracked_files() -> Option<Vec<String>> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(root())
        .args(["ls-files", "-z", "--"])
        .args(SCANNED)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(
        output
            .stdout
            .split(|byte| *byte == 0)
            .filter_map(|name| std::str::from_utf8(name).ok())
            .filter(|name| !name.is_empty())
            .map(str::to_owned)
            .collect(),
    )
}

/// The home directory of the account running the suite, when it is one a file
/// could plausibly and wrongly hard-code.
///
/// `/` and `/root` are the two that a container hands out and that a hundred
/// unrelated lines contain, so they answer `None` rather than flooding the
/// failure with noise.
fn home_directory() -> Option<String> {
    let home = std::env::var("HOME")
        .ok()
        .or_else(|| std::env::var("USERPROFILE").ok())?;
    let trimmed = home.trim_end_matches('/').to_owned();
    if trimmed.is_empty() || trimmed == "/root" {
        return None;
    }
    Some(trimmed)
}
