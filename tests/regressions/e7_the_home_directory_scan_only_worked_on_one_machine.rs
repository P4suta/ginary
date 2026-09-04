// SPDX-License-Identifier: MIT OR Apache-2.0
//! The scan for hard-coded home directories looked for *this* machine's
//! `$HOME`, so on a runner it policed prose and on every other machine it
//! proved nothing.
//!
//! **What went wrong.** E5's scan read `$HOME` (or `%USERPROFILE%`) and
//! refused any tracked file under `tests/`, `src/`, `scripts/` or `.github/`
//! that contained it. On a GitHub-hosted runner `$HOME` is `/home/runner`,
//! and E6's own regression test quotes the CI transcript that identified
//! it — which necessarily contains the runner's path. All three ubuntu jobs
//! failed on it:
//!
//! ```text
//! thread 'e5_a_gated_test_defaulted_to_one_developers_machine::
//! no_source_or_test_file_names_the_home_directory_of_the_machine_it_was_written_on'
//! panicked at tests/regressions/e5_a_gated_test_defaulted_to_one_developers_machine.rs:201:5:
//! a tracked file names `/home/runner`, the home directory of the machine it was written on.
//! A path that exists on one machine is not a default, a fixture or a fallback:
//! tests/regressions/e6_the_toolchain_flag_required_a_cross_stub_nobody_built.rs:18
//! tests/regressions/e6_the_toolchain_flag_required_a_cross_stub_nobody_built.rs:64
//! ```
//!
//! (`Coverage`
//! <https://github.com/P4suta/ginary/actions/runs/33702776627/job/100485724897>,
//! `Test (both flavors, stable)`
//! <https://github.com/P4suta/ginary/actions/runs/33702776627/job/100485421869>
//! and `Cross-Linux smoke matrix`
//! <https://github.com/P4suta/ginary/actions/runs/33702776627/job/100485421511>.)
//!
//! **The input.** Any machine whose `$HOME` happens to be named in the tree,
//! and no other. The rule was unfalsifiable everywhere else: a developer's
//! path hard-coded by somebody *else* passed on every machine but theirs,
//! which is precisely the defect E5 set out to prevent.
//!
//! **The correct behaviour.** The rule is about a person's absolute home path
//! appearing in code that has to run anywhere, and it has to mean the same
//! thing on every machine. So: `/home/<name>` and `/Users/<name>` are matched
//! whoever is running the scan; `docs/` is not scanned at all, because a
//! milestone log quoting a runner's transcript is the record of a bug and not
//! a bug; a comment line inside a scanned file is the same prose and is not
//! policed either; and the fictional accounts this suite's own unit tests
//! spell — `/home/u`, `/Users/ada` — are not people. The rule and its
//! reasoning live in [`crate::common::homepath`].
//!
//! The one offender the new rule finds on this tree is line 64 of
//! `tests/regressions/e6_the_toolchain_flag_required_a_cross_stub_nobody_built.rs`,
//! which is `/home/runner/work/ginary/ginary/target/stubs` in *code*: a
//! synthetic input to a pure function that could be any absolute directory,
//! written as one machine's. The transcript in that file's documentation
//! stays exactly as it is.

use crate::common::homepath::{
    CODE_ROOTS, HomePathSite, Syntax, home_path_sites, tracked_code_files,
};
use crate::common::repo::{read, root};

/// The three tracked files allowed to hold a person's absolute home path.
///
/// The same exception E5 argued and for the same reason: the Erlang compiler
/// records the absolute path of the `.erl` it compiled in a module's `Dbgi`
/// chunk, and these three were copied verbatim out of a real
/// `gleam export erlang-shipment` — which is the whole point of them.
/// Rewriting the chunk would make them no longer what a compiler wrote. See
/// `tests/fixtures/beam/README.md`.
const ALLOWED: [&str; 3] = [
    "tests/fixtures/beam/gleam@bool.beam",
    "tests/fixtures/beam/gleam@list.beam",
    "tests/fixtures/beam/gleam@string.beam",
];

/// An absolute home path, assembled rather than written.
///
/// A file that spelled `/home/<a person>/` in one piece would be an offender
/// itself, which is the rule working and would leave this test unable to state
/// its own input. E5's file solved the same problem by exempting itself by
/// name; assembling the bytes at run time is the same argument without the
/// exemption, and it also proves the scanner reads bytes rather than tokens.
fn home_path(root: &str, account: &str, rest: &str) -> String {
    format!("/{root}/{account}/{rest}")
}

#[test]
fn no_tracked_code_file_names_a_persons_home_directory() {
    // The instrument first. A scan that cannot find a planted defect proves
    // nothing about a clean tree, so the calibration is part of the test and
    // not a separate one that could pass while this one was vacuous.
    let person = home_path(
        "home",
        "jbloggs",
        "projects/gleam/notify/build/erlang-shipment",
    );
    let mac = home_path("Users", "jbloggs", "Library/Caches/ginary");
    let fictional_unix = home_path("home", "u", ".cache/ginary");
    let fictional_windows = home_path("Users", "ada", "AppData/Local/ginary");
    let planted = format!(
        "//! a transcript quoting {person}\n\
         const DEFAULT_REAL_SHIPMENT: &str = \"{person}\";\n\
         #[path = \"{person}\"]\n\
         let home = \"{fictional_unix}\";\n\
         let profile = \"{fictional_windows}\";\n\
         let cache = \"{mac}\";\n\
         let documented = \"/home/<user>/.cache\";\n"
    );

    assert_eq!(
        home_path_sites(planted.as_bytes(), Syntax::Rust),
        vec![
            HomePathSite {
                line: 2,
                account: "home/jbloggs".to_owned(),
            },
            HomePathSite {
                line: 3,
                account: "home/jbloggs".to_owned(),
            },
            HomePathSite {
                line: 6,
                account: "Users/jbloggs".to_owned(),
            },
        ],
        "the scanner finds a person's home in code on both platforms' spellings, and leaves \
         alone a doc comment quoting a transcript, the fictional accounts this suite's unit \
         tests use, and a documentation placeholder. `#` opens a Rust *attribute*, not a \
         comment, so `#[path = ..]` naming one machine is code like any other line — and \
         `tests/regressions.rs` is 100 such attributes, which is why reading `#` as prose \
         everywhere would have exempted the shape this rule is most likely to meet"
    );

    // The `#` family, where it really is a comment. Two syntaxes, one scanner:
    // the same buffer has to be read differently by each, or one of the two
    // answers is wrong.
    let workflow = format!(
        "#   a workflow comment naming {mac}\n\
         GINARY_STUB_DIR: {person}\n"
    );
    assert_eq!(
        home_path_sites(workflow.as_bytes(), Syntax::Hash),
        vec![HomePathSite {
            line: 2,
            account: "home/jbloggs".to_owned(),
        }],
        "in YAML, TOML and shell a leading `#` is a comment and the line below it is not"
    );
    assert_eq!(
        home_path_sites(workflow.as_bytes(), Syntax::Rust).len(),
        2,
        "and the same bytes read as Rust are two lines of code, neither of them a comment"
    );
    assert_eq!(
        (
            Syntax::of("tests/regressions.rs"),
            Syntax::of(".github/workflows/ci.yml"),
            Syntax::of("scripts/smoke-matrix.sh"),
            Syntax::of("tests/fixtures/beam/gleam@bool.beam"),
        ),
        (Syntax::Rust, Syntax::Hash, Syntax::Hash, Syntax::Opaque),
        "and each scanned file is read in its own syntax, with anything unrecognised read as \
         code throughout, which is the answer that hides nothing"
    );

    assert!(
        !CODE_ROOTS.contains(&"docs"),
        "a milestone log is supposed to reproduce the path a runner printed; policing prose is \
         how the old rule failed"
    );

    let Some(tracked) = tracked_code_files() else {
        eprintln!("skipping: `git ls-files` did not answer, so `tracked` would be a guess");
        return;
    };
    assert!(
        tracked.len() > 100,
        "`git ls-files` answered with {} paths, which is not this repository: a scan over \
         nothing passes for the wrong reason",
        tracked.len()
    );

    let mut offenders: Vec<String> = Vec::new();
    let mut unreadable: Vec<String> = Vec::new();
    for relative in &tracked {
        if ALLOWED.contains(&relative.as_str()) {
            continue;
        }
        let Ok(bytes) = std::fs::read(root().join(relative)) else {
            unreadable.push(relative.clone());
            continue;
        };
        for site in home_path_sites(&bytes, Syntax::of(relative)) {
            offenders.push(format!("{relative}:{} names /{}", site.line, site.account));
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
        "a tracked file under {CODE_ROOTS:?} could not be read, so nobody knows what is in it. A \
         file this scan cannot open is a reported failure, never a silent pass:\n{}",
        unreadable.join("\n")
    );
    assert!(
        offenders.is_empty(),
        "a tracked file under {CODE_ROOTS:?} names a person's home directory in code. A path \
         that exists on one machine is not a default, a fixture or a fallback — and a synthetic \
         input to a pure function can be any absolute directory:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn the_scan_no_longer_asks_this_machine_what_its_home_directory_is() {
    let source = read("tests/regressions/e5_a_gated_test_defaulted_to_one_developers_machine.rs");
    assert!(
        !source.contains("USERPROFILE"),
        "the scan still reads this machine's own home directory, so it still means something \
         different on every machine and something wrong on a runner"
    );
    assert!(
        !source.contains("fn home_directory()"),
        "the machine-dependent scan is replaced by `crate::common::homepath`, not kept beside it: \
         two rules about the same thing is one rule nobody trusts"
    );
    assert!(
        source.contains("homepath"),
        "the file that argued this rule points at the module that now enforces it"
    );
}
