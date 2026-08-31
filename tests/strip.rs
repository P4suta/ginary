// SPDX-License-Identifier: MIT OR Apache-2.0
//! Stripping a staged root: `ginary::strip`.
//!
//! The scenario is deliberately small — one shipment application, one runtime
//! application, eight `.beam` files and no native code — because what is being
//! tested here is the *contract*, not the size reduction. The size reduction is
//! `tests/stage_run.rs`, which strips a real runtime and measures it.
//!
//! Two things make the contract testable without an Erlang installation.
//! [`FakeOtp::with_erl_script`] installs a `bin/erl` that records its argument
//! vector and exits, so a test asserts on the exact `beam_lib:strip_files/1`
//! one-liner ginary passes — and on the exact list of modules it names —
//! rather than on a substring of it. And
//! [`common::fake_otp::DUMMY_BEAM`] is *already stripped*, so a runtime that
//! does nothing leaves a tree that passes verification — which means a test
//! that wants the verification to fail has to write a module holding `Dbgi`
//! into the staged tree by hand, in the open, and the failure it produces is
//! unambiguous.
//!
//! The one test that needs a real program is gated on `strip` itself, and it
//! uses the running test binary as its native code: a real, unstripped,
//! dynamically linked ELF that is on disk at a path the test can name.
// The command line half of the suite: every claim in this file is about a
// module the `cli` feature carries, so a `--no-default-features` build has
// nothing here to run. See `docs/dev/log/C2.md`.
#![cfg(feature = "cli")]

mod common;

use std::path::{Path, PathBuf};

use ginary::assemble::{self, StageOptions, StagedRoot};
use ginary::beam::{CODE_CHUNK, DEBUG_INFO_CHUNK, DOCS_CHUNK, LINE_CHUNK};
use ginary::closure::app_dependency_closure;
use ginary::otp::OtpInfo;
use ginary::strip::{
    self, BeamOutcome, ElfOutcome, STRIP_FILES_EVAL, StripError, StripOptions, StripReport,
};
use tempfile::TempDir;

use crate::common::fake_otp::{FakeOtp, FakeOtpRoot, FakeShipment, beam_bytes};
use crate::common::tools::require_tools;

/// A staged root, the runtime it came from, and the temporary directory both
/// live in.
struct Scenario {
    _dir: TempDir,
    otp: FakeOtpRoot,
    staged: StagedRoot,
}

impl Scenario {
    /// Builds both trees and stages `notify` into `<tmp>/out`.
    ///
    /// `with_erl` decides whether the runtime carries the stub `bin/erl`, which
    /// is the difference between "the beam step runs" and "the beam step is
    /// skipped because there is no runtime to run it with".
    fn new(with_erl: bool) -> Self {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let shipment = FakeShipment::new()
            .app_with("notify", "1.0.0", |app| {
                app.applications(&["gleam_stdlib"])
                    .modules(&["notify", "notify@@main"])
                    .priv_file("greeting.txt", b"hello from priv\n")
            })
            .app_with("gleam_stdlib", "0.62.0", |app| {
                app.modules(&["gleam_stdlib", "gleam@list"])
            })
            .build_in(dir.path().join("shipment"));

        std::fs::create_dir_all(dir.path().join("otp")).expect("the otp root");
        let mut builder = FakeOtp::new();
        if with_erl {
            builder = builder.with_erl_script();
        }
        let otp = builder.build_in(dir.path().join("otp"));

        let info =
            ginary::otp::inspect_root(&otp.root).expect("the fake root is a usable OTP root");
        let set = app_dependency_closure(&shipment.root, &otp.lib(), &["notify".to_owned()], &[])
            .expect("the closure resolves");
        let staged = assemble::stage(
            &set,
            &info,
            &StageOptions::default(),
            &dir.path().join("out"),
        )
        .expect("the scenario stages");

        Self {
            _dir: dir,
            otp,
            staged,
        }
    }

    /// The runtime as `ginary::otp` sees it.
    fn otp_info(&self) -> OtpInfo {
        ginary::otp::inspect_root(&self.otp.root).expect("the fake root is a usable OTP root")
    }

    /// The staged root on disk.
    fn root(&self) -> &Path {
        self.staged.root()
    }

    /// Strips the staged root with the given options.
    fn strip(&self, opts: &StripOptions) -> Result<StripReport, StripError> {
        strip::strip(self.root(), &self.otp_info(), opts)
    }

    /// Strips with the default options, or panics naming the error.
    fn stripped(&self) -> StripReport {
        match self.strip(&StripOptions::default()) {
            Ok(report) => report,
            Err(error) => panic!("stripping should succeed: {error}"),
        }
    }

    /// Writes `bytes` at `relative` inside the staged root.
    fn write(&self, relative: &str, bytes: &[u8]) {
        let path = self.root().join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("a directory in the staged root");
        }
        std::fs::write(&path, bytes)
            .unwrap_or_else(|error| panic!("cannot write {}: {error}", path.display()));
    }

    /// Every `.beam` in the staged tree, by path relative to the root, sorted.
    fn modules(&self) -> Vec<String> {
        self.snapshot()
            .into_iter()
            .map(|(path, _)| path)
            .filter(|path| path.ends_with(".beam"))
            .collect()
    }

    /// Every file in the staged tree, as `(relative path, bytes)`, sorted.
    fn snapshot(&self) -> Vec<(String, Vec<u8>)> {
        let mut found = Vec::new();
        walk(self.root(), self.root(), &mut found);
        found.sort_by(|left, right| left.0.cmp(&right.0));
        found
    }
}

/// Collects every file under `dir` as `(path relative to `root`, bytes)`.
fn walk(root: &Path, dir: &Path, into: &mut Vec<(String, Vec<u8>)>) {
    for entry in std::fs::read_dir(dir).expect("a readable directory") {
        let path = entry.expect("a readable entry").path();
        if path.is_dir() {
            walk(root, &path, into);
        } else {
            let relative = path
                .strip_prefix(root)
                .expect("a path under the root")
                .to_string_lossy()
                .replace('\\', "/");
            into.push((relative, std::fs::read(&path).expect("a readable file")));
        }
    }
}

/// The argument vector ginary must pass to the OTP installation's own `erl`.
///
/// Written out here rather than built from the constants the module exports:
/// a test that reused the code's own strings would pass whatever the code
/// happened to send. The modules are found by this file's own walk of the
/// staged tree, for the same reason — the point of the assertion is that the
/// list the runtime is given is every `.beam` that is actually there.
fn expected_erl_argv(root: &Path, modules: &[String]) -> Vec<String> {
    let mut argv = vec![
        "-noshell".to_owned(),
        "-env".to_owned(),
        "ERL_CRASH_DUMP".to_owned(),
        "/dev/null".to_owned(),
        "-eval".to_owned(),
        "Files=init:get_plain_arguments(), case beam_lib:strip_files(Files) of {ok,_} -> \
         halt(0); Err -> io:format(standard_error,\"~p~n\",[Err]), halt(1) end."
            .to_owned(),
        "-extra".to_owned(),
    ];
    argv.extend(
        modules
            .iter()
            .map(|module| root.join(module).display().to_string()),
    );
    argv
}

#[test]
fn the_exported_eval_is_the_one_liner_the_plan_specifies() {
    // The other half of `the_beam_step_runs_the_otp_roots_own_erl`: this pins
    // the constant, that pins what is actually passed. Either alone can drift.
    //
    // `strip_files/1` and not `strip_release/1`: the second takes a directory
    // and expands `<root>/lib/*/ebin/*.beam` through `filelib:wildcard/1`, so
    // the root is a glob prefix and a staged root named `out[1]` sends the
    // runtime somewhere else entirely.
    assert_eq!(
        STRIP_FILES_EVAL,
        "Files=init:get_plain_arguments(), case beam_lib:strip_files(Files) of {ok,_} -> \
         halt(0); Err -> io:format(standard_error,\"~p~n\",[Err]), halt(1) end."
    );
}

#[test]
fn a_tree_with_no_native_code_reports_nothing_to_strip() {
    // The fake runtime's ERTS binaries are shell scripts. Nothing in the tree
    // is an ELF, and that is a reported outcome rather than a failure.
    let scenario = Scenario::new(true);

    let report = scenario.stripped();

    assert_eq!(report.elf, ElfOutcome::NothingToStrip);
}

#[test]
fn the_beam_step_runs_the_otp_roots_own_erl_with_the_beam_lib_one_liner() {
    let scenario = Scenario::new(true);
    let modules = scenario.modules();

    scenario.stripped();

    assert!(!modules.is_empty(), "the scenario stages modules");
    assert_eq!(
        scenario.otp.erl_argv(),
        expected_erl_argv(scenario.root(), &modules),
        "the runtime is run by absolute path, every module arrives after -extra \
         as a path of its own, and the crash dump goes to the bit bucket"
    );
}

#[test]
fn no_directory_is_passed_to_the_runtime_where_a_module_belongs() {
    // The regression this file carries from the A2 review: a directory reaching
    // `beam_lib` is a `filelib:wildcard/1` pattern, not a path. Every plain
    // argument has to be one of the modules ginary walked.
    let scenario = Scenario::new(true);
    let modules = scenario.modules();

    scenario.stripped();

    let argv = scenario.otp.erl_argv();
    let extra = argv
        .iter()
        .position(|argument| argument == "-extra")
        .expect("the runtime is given plain arguments");
    let expected: Vec<String> = modules
        .iter()
        .map(|module| scenario.root().join(module).display().to_string())
        .collect();
    assert_eq!(argv[extra + 1..], expected[..]);
    assert!(
        !argv.contains(&scenario.root().display().to_string()),
        "the staged root itself must never be an argument: {argv:?}"
    );
}

#[test]
fn a_module_outside_an_ebin_is_handed_to_the_runtime_like_any_other() {
    // The set ginary verifies has to be the set the runtime was asked about.
    // `beam_lib:strip_release/1` rewrote `lib/*/ebin/*.beam` and nothing else,
    // so a module a shipment keeps under `priv` was counted, verified and
    // never stripped — and the build then blamed the runtime for it.
    let scenario = Scenario::new(true);
    scenario.write(
        "lib/notify/priv/helper.beam",
        &beam_bytes(&[(CODE_CHUNK, b"code".as_slice())]),
    );
    let helper = scenario
        .root()
        .join("lib/notify/priv/helper.beam")
        .display()
        .to_string();

    let report = scenario.stripped();

    assert!(
        scenario.otp.erl_argv().contains(&helper),
        "the module under priv was never named to the runtime: {:?}",
        scenario.otp.erl_argv()
    );
    assert!(
        report
            .per_file
            .iter()
            .any(|file| file.path == "lib/notify/priv/helper.beam"),
        "a module the runtime was given is a module the report accounts for: {:?}",
        report.per_file
    );
}

#[test]
fn the_beam_step_reports_every_module_in_the_tree() {
    let scenario = Scenario::new(true);

    let report = scenario.stripped();

    match report.beams {
        BeamOutcome::Stripped {
            files,
            before,
            after,
        } => {
            // Six modules: four from the shipment and one each from the seeded
            // `kernel` and `stdlib`.
            assert_eq!(files, 6, "every staged .beam is accounted for");
            assert_eq!(before, after, "an already-stripped tree does not shrink");
        }
        other => panic!("expected the beam step to run, got {other:?}"),
    }
}

#[test]
fn an_otp_root_with_no_erl_skips_the_beam_step_with_a_reason_naming_the_path() {
    // Not an error: a runtime that came from a tarball of only the ERTS
    // binaries genuinely has no `erl` to run, and refusing to stage it at all
    // would be worse than shipping unstripped modules and saying so.
    let scenario = Scenario::new(false);

    let report = scenario.stripped();

    match report.beams {
        BeamOutcome::Skipped { reason } => assert!(
            reason.contains(&scenario.otp.erl().display().to_string()),
            "the reason names the `erl` that was looked for: {reason}"
        ),
        other => panic!("expected Skipped, got {other:?}"),
    }
}

#[test]
fn a_beam_lib_failure_quotes_the_term_the_runtime_printed() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    std::fs::create_dir_all(dir.path().join("otp")).expect("the otp root");
    // A term with an apostrophe in it: `~p` prints a quoted atom that way, and
    // a helper that interpolated this into shell source would write a stub that
    // fails to parse rather than one that fails on purpose.
    let term = "{error,beam_lib,{not_a_beam_file,'Elixir.Notify'}}";
    let otp = FakeOtp::new()
        .with_failing_erl_script(term)
        .build_in(dir.path().join("otp"));
    let shipment = FakeShipment::new()
        .app("notify", "1.0.0", &[])
        .build_in(dir.path().join("shipment"));
    let info = ginary::otp::inspect_root(&otp.root).expect("a usable OTP root");
    let set = app_dependency_closure(&shipment.root, &otp.lib(), &["notify".to_owned()], &[])
        .expect("the closure resolves");
    let staged = assemble::stage(
        &set,
        &info,
        &StageOptions::default(),
        &dir.path().join("out"),
    )
    .expect("the scenario stages");

    match strip::strip(staged.root(), &info, &StripOptions::default()) {
        Err(StripError::BeamStripFailed { stderr }) => assert!(
            stderr.contains(term),
            "the Erlang term is what says why; swallowing it leaves nothing: {stderr}"
        ),
        other => panic!("expected BeamStripFailed, got {other:?}"),
    }
}

#[test]
fn a_module_that_still_holds_dbgi_afterwards_is_an_error_naming_the_chunk() {
    // The stub `erl` reports success and changes nothing, which is exactly the
    // failure the verification exists for: `strip_files` answering `{ok, _}`
    // over a tree it did not rewrite.
    let scenario = Scenario::new(true);
    scenario.write(
        "lib/notify/ebin/notify@debug.beam",
        &beam_bytes(&[
            (CODE_CHUNK, b"code".as_slice()),
            (DEBUG_INFO_CHUNK, b"debug".as_slice()),
        ]),
    );

    match scenario.strip(&StripOptions::default()) {
        Err(StripError::BeamStillHasChunk { path, chunk }) => {
            assert_eq!(path, "lib/notify/ebin/notify@debug.beam");
            assert_eq!(chunk, "Dbgi");
        }
        other => panic!("expected BeamStillHasChunk, got {other:?}"),
    }
}

#[test]
fn a_module_that_still_holds_docs_afterwards_is_an_error_naming_the_chunk() {
    let scenario = Scenario::new(true);
    scenario.write(
        "lib/notify/ebin/notify@docs.beam",
        &beam_bytes(&[
            (CODE_CHUNK, b"code".as_slice()),
            (DOCS_CHUNK, b"docs".as_slice()),
        ]),
    );

    match scenario.strip(&StripOptions::default()) {
        Err(StripError::BeamStillHasChunk { path, chunk }) => {
            assert_eq!(path, "lib/notify/ebin/notify@docs.beam");
            assert_eq!(chunk, "Docs");
        }
        other => panic!("expected BeamStillHasChunk, got {other:?}"),
    }
}

#[test]
fn a_module_left_without_its_code_chunk_is_an_error() {
    // The other half of the verification. A tool that removed too much is as
    // bad as one that removed nothing, and it fails later and less legibly.
    let scenario = Scenario::new(true);
    scenario.write(
        "lib/notify/ebin/notify@broken.beam",
        &beam_bytes(&[(LINE_CHUNK, b"line".as_slice())]),
    );

    match scenario.strip(&StripOptions::default()) {
        Err(StripError::BeamLostCode { path }) => {
            assert_eq!(path, "lib/notify/ebin/notify@broken.beam");
        }
        other => panic!("expected BeamLostCode, got {other:?}"),
    }
}

#[test]
fn the_line_chunk_is_kept_because_a_stack_trace_needs_it() {
    // ADR 0007's one deliberate exception. A verification that only looked for
    // what must be gone would happily accept a tree that lost this too.
    let scenario = Scenario::new(true);

    scenario.stripped();

    for (path, bytes) in scenario.snapshot() {
        if path.ends_with(".beam") {
            assert!(
                ginary::beam::has_chunk(&bytes, &LINE_CHUNK),
                "{path} lost its Line chunk"
            );
        }
    }
}

#[test]
fn nothing_is_stripped_and_no_runtime_is_run_when_both_halves_are_off() {
    let scenario = Scenario::new(true);
    let before = scenario.snapshot();

    let report = scenario
        .strip(&StripOptions {
            elf: false,
            beams: false,
        })
        .expect("stripping nothing succeeds");

    assert_eq!(report.elf, ElfOutcome::Disabled);
    assert_eq!(report.beams, BeamOutcome::Disabled);
    assert_eq!(report.per_file, vec![]);
    assert!(
        scenario.otp.erl_argv().is_empty(),
        "the runtime must not be started when the beam step is off"
    );
    assert_eq!(before, scenario.snapshot(), "the tree was touched");
}

#[test]
fn stripping_only_the_native_binaries_leaves_the_runtime_unstarted() {
    let scenario = Scenario::new(true);

    let report = scenario
        .strip(&StripOptions {
            elf: true,
            beams: false,
        })
        .expect("stripping only the ELF files succeeds");

    assert_eq!(report.beams, BeamOutcome::Disabled);
    assert!(scenario.otp.erl_argv().is_empty());
}

#[test]
fn stripping_only_the_modules_leaves_the_native_binaries_alone() {
    let scenario = Scenario::new(true);

    let report = scenario
        .strip(&StripOptions {
            elf: false,
            beams: true,
        })
        .expect("stripping only the modules succeeds");

    assert_eq!(report.elf, ElfOutcome::Disabled);
    assert!(!scenario.otp.erl_argv().is_empty());
}

#[test]
fn stripping_twice_changes_not_one_byte() {
    // The invariant the whole project rests on: identical input produces
    // identical artifact bytes. A phase that rewrote a file on every run would
    // break it here, three phases before anyone could see it.
    let scenario = Scenario::new(true);

    scenario.stripped();
    let once = scenario.snapshot();
    scenario.stripped();
    let twice = scenario.snapshot();

    assert_eq!(once, twice);
}

#[test]
fn the_report_totals_are_the_sum_of_the_rows() {
    let scenario = Scenario::new(true);

    let report = scenario.stripped();

    assert_eq!(
        report.before_total,
        report.per_file.iter().map(|file| file.before).sum::<u64>()
    );
    assert_eq!(
        report.after_total,
        report.per_file.iter().map(|file| file.after).sum::<u64>()
    );
    assert_eq!(report.saved(), report.before_total - report.after_total);
}

#[test]
fn the_per_file_rows_are_sorted_by_path_and_relative_to_the_root() {
    let scenario = Scenario::new(true);

    let report = scenario.stripped();
    let paths: Vec<&str> = report
        .per_file
        .iter()
        .map(|file| file.path.as_str())
        .collect();

    let mut sorted = paths.clone();
    sorted.sort_unstable();
    assert_eq!(paths, sorted, "the rows are in path order");
    assert!(
        paths.iter().all(|path| !path.starts_with('/')),
        "a report that named absolute paths could not be reproduced: {paths:?}"
    );
}

#[test]
fn the_report_table_names_both_halves_and_the_total() {
    let report = StripReport {
        elf: ElfOutcome::Stripped {
            files: 4,
            before: 41_675_352,
            after: 11_742_936,
        },
        beams: BeamOutcome::Stripped {
            files: 312,
            before: 9_382_144,
            after: 3_011_072,
        },
        per_file: Vec::new(),
        before_total: 51_057_496,
        after_total: 14_754_008,
        warnings: Vec::new(),
    };

    insta::assert_snapshot!("report_table", report.to_string());
}

#[test]
fn the_report_table_says_why_when_a_half_did_not_run() {
    let report = StripReport {
        elf: ElfOutcome::NothingToStrip,
        beams: BeamOutcome::Skipped {
            reason: "no `erl` at /opt/otp/bin/erl".to_owned(),
        },
        per_file: Vec::new(),
        before_total: 0,
        after_total: 0,
        warnings: Vec::new(),
    };

    insta::assert_snapshot!("report_table_when_nothing_ran", report.to_string());
}

#[test]
fn a_disabled_report_is_the_one_a_no_strip_run_produces() {
    let report = StripReport::disabled();

    assert_eq!(report.elf, ElfOutcome::Disabled);
    assert_eq!(report.beams, BeamOutcome::Disabled);
    assert_eq!(report.saved(), 0);
}

#[test]
fn a_file_that_starts_like_an_elf_and_is_not_one_is_a_reported_skip() {
    // Four bytes of magic under `priv` — inert data, a fixture, a truncated
    // download — is not something `strip` can work on and not a reason to
    // abandon the build. `report::measure` reaches the same decision about the
    // same file; the two must not disagree.
    let scenario = Scenario::new(true);
    scenario.write("lib/notify/priv/data.bin", b"\x7fELF");

    let report = scenario.stripped();

    assert_eq!(report.elf, ElfOutcome::NothingToStrip);
    assert_eq!(report.warnings.len(), 1, "{:?}", report.warnings);
    assert!(
        report.warnings[0].contains("lib/notify/priv/data.bin"),
        "the skip names the file: {:?}",
        report.warnings
    );
    assert!(
        report
            .to_string()
            .contains("warning: lib/notify/priv/data.bin"),
        "and the table prints it:\n{report}"
    );
}

#[test]
fn a_strip_that_fails_on_a_file_names_the_file_and_quotes_the_tool() {
    // ADR 0007's contract for the one failure that is an error rather than a
    // skip. The message is the whole of what the user gets, so it is pinned
    // here rather than left to the first person who hits it.
    let error = StripError::StripFailed {
        path: PathBuf::from("/tmp/out/erts-17.0.5/bin/beam.smp"),
        stderr: "strip: /tmp/out/erts-17.0.5/bin/beam.smp: file format not recognized".to_owned(),
    };

    assert_eq!(
        error.to_string(),
        "`strip` failed on `/tmp/out/erts-17.0.5/bin/beam.smp`: \
         strip: /tmp/out/erts-17.0.5/bin/beam.smp: file format not recognized"
    );
}

#[test]
fn a_strip_that_destroys_a_file_says_which_and_how() {
    let destroyed = StripError::NotElfAfterStrip {
        path: PathBuf::from("/tmp/out/priv/lib/crypto.so"),
    };
    let changed = StripError::ElfChanged {
        path: PathBuf::from("/tmp/out/priv/lib/crypto.so"),
        before: "64-bit x86_64".to_owned(),
        after: "32-bit aarch64".to_owned(),
    };

    assert_eq!(
        destroyed.to_string(),
        "`strip` left `/tmp/out/priv/lib/crypto.so`, which was an ELF file, unreadable as one"
    );
    assert_eq!(
        changed.to_string(),
        "`strip` changed `/tmp/out/priv/lib/crypto.so` from 64-bit x86_64 to 32-bit aarch64"
    );
}

/// A host shared object that carries a program interpreter, if there is one.
///
/// glibc's C library is the everyday example of the shape the ELF half has to
/// classify by `e_type` rather than by `PT_INTERP`: it is an `ET_DYN`, it is a
/// library, and it has an interpreter because it also runs as a program.
fn host_shared_object() -> Option<std::path::PathBuf> {
    [
        "/lib/x86_64-linux-gnu/libc.so.6",
        "/lib/aarch64-linux-gnu/libc.so.6",
        "/lib64/libc.so.6",
        "/usr/lib/libc.so.6",
    ]
    .iter()
    .map(std::path::PathBuf::from)
    .find(|candidate| ginary::elf::inspect(candidate).is_ok())
}

#[test]
fn a_shared_object_in_the_staged_tree_is_stripped_and_still_loads_its_dependencies() {
    let Some(_tools) = require_tools(&["strip"]) else {
        return;
    };
    let Some(library) = host_shared_object() else {
        eprintln!("skipping: no host shared object to stage");
        return;
    };
    let scenario = Scenario::new(true);
    let staged = scenario.root().join("lib/notify/priv/lib/notify_nif.so");
    std::fs::create_dir_all(staged.parent().expect("a parent")).expect("the priv lib directory");
    std::fs::copy(&library, &staged).expect("a real shared object in the staged tree");
    let before = ginary::elf::inspect(&staged).expect("the copy is an ELF file");

    scenario.stripped();

    let after = ginary::elf::inspect(&staged).expect("it is still an ELF file");
    assert_eq!(after.kind, before.kind, "a library is still a library");
    assert_eq!(after.class, before.class);
    assert_eq!(after.machine, before.machine);
    assert_eq!(
        after.needed, before.needed,
        "`--strip-unneeded` keeps what the loader reads"
    );
}

#[test]
fn a_native_binary_in_the_staged_tree_is_stripped_and_stays_the_same_machine() {
    let Some(_tools) = require_tools(&["strip"]) else {
        return;
    };
    let scenario = Scenario::new(true);
    // The running test binary is the only real, unstripped, dynamically linked
    // ELF a test can count on. Putting it where `beam.smp` goes is what makes
    // the ELF half of stripping reachable without an Erlang installation.
    let exe = std::env::current_exe().expect("the running test binary");
    let staged_beam = scenario.root().join("erts-17.0.5/bin/beam.smp");
    std::fs::copy(&exe, &staged_beam).expect("a real ELF in the staged tree");
    let before = ginary::elf::inspect(&staged_beam).expect("the copy is an ELF file");
    let before_bytes = std::fs::metadata(&staged_beam)
        .expect("a staged file")
        .len();

    let report = scenario.stripped();

    let after = ginary::elf::inspect(&staged_beam).expect("it is still an ELF file");
    let after_bytes = std::fs::metadata(&staged_beam)
        .expect("a staged file")
        .len();
    assert_eq!(after.machine, before.machine);
    assert_eq!(after.class, before.class);
    assert!(after.stripped, "`strip --strip-all` removes the .symtab");
    assert!(
        after_bytes < before_bytes,
        "stripping a debug binary has to make it smaller: {before_bytes} -> {after_bytes}"
    );
    match report.elf {
        ElfOutcome::Stripped {
            files,
            before,
            after,
        } => {
            assert_eq!(files, 1);
            assert_eq!(before, before_bytes);
            assert_eq!(after, after_bytes);
        }
        other => panic!("expected the ELF step to run, got {other:?}"),
    }
    assert!(
        report
            .per_file
            .iter()
            .any(|file| file.path == "erts-17.0.5/bin/beam.smp"),
        "the per-file rows name what was stripped: {:?}",
        report.per_file
    );
}

/// The staged listing, read back off the disk.
fn listing_sizes(root: &Path) -> Vec<(String, u64)> {
    let text = std::fs::read_to_string(root.join(assemble::LISTING_NAME))
        .expect("the staged listing is on disk");
    let listing: assemble::StageListing = serde_json::from_str(&text).expect("the listing parses");
    listing
        .files
        .iter()
        .map(|file| (file.path.clone(), file.size))
        .collect()
}

#[test]
fn refreshing_the_staged_root_re_reads_the_sizes_that_stripping_changed() {
    // Stripping rewrites files in place, so `ginary.stage.json` stops
    // describing the tree the moment it runs. A listing whose sizes are the
    // pre-strip ones is worse than none: every later phase trusts it.
    let scenario = Scenario::new(true);
    let victim = "lib/notify/ebin/notify.beam";
    let shrunk = beam_bytes(&[(CODE_CHUNK, b"x".as_slice())]);
    scenario.write(victim, &shrunk);

    let refreshed = scenario.staged.refresh().expect("the tree can be re-read");

    let size = refreshed
        .files()
        .iter()
        .find(|file| file.path == victim)
        .map(|file| file.size)
        .expect("the file is still in the listing");
    assert_eq!(size, shrunk.len() as u64, "the returned listing is stale");
    assert!(
        listing_sizes(scenario.root())
            .iter()
            .any(|(path, size)| path == victim && *size == shrunk.len() as u64),
        "ginary.stage.json on disk is stale"
    );
}

#[test]
fn refreshing_keeps_the_account_that_cannot_be_re_derived_from_the_tree() {
    // The excluded binaries, the junk that was removed and the boot references
    // are records of what staging *decided*. Nothing in the tree can recover
    // them, so a refresh that dropped them would lose `--explain` for good.
    let scenario = Scenario::new(true);

    let refreshed = scenario.staged.refresh().expect("the tree can be re-read");

    assert_eq!(
        refreshed.excluded_erts_bins(),
        scenario.staged.excluded_erts_bins()
    );
    assert_eq!(refreshed.junk_removed(), scenario.staged.junk_removed());
    assert_eq!(refreshed.boot_refs(), scenario.staged.boot_refs());
    assert_eq!(refreshed.erts_vsn(), scenario.staged.erts_vsn());
    assert_eq!(refreshed.otp_version(), scenario.staged.otp_version());
}

#[test]
fn refreshing_recomputes_the_per_application_totals() {
    let scenario = Scenario::new(true);
    let shrunk = beam_bytes(&[(CODE_CHUNK, b"x".as_slice())]);
    scenario.write("lib/notify/ebin/notify.beam", &shrunk);

    let refreshed = scenario.staged.refresh().expect("the tree can be re-read");

    let before = scenario
        .staged
        .apps()
        .iter()
        .find(|app| app.name == "notify")
        .map(|app| app.bytes)
        .expect("notify is staged");
    let after = refreshed
        .apps()
        .iter()
        .find(|app| app.name == "notify")
        .map(|app| app.bytes)
        .expect("notify is staged");
    assert!(
        after < before,
        "the application total has to follow its files: {before} -> {after}"
    );
    assert_eq!(refreshed.total_bytes(), sum_of(refreshed.files()));
}

/// The sum of a listing's file sizes.
fn sum_of(files: &[assemble::StagedFile]) -> u64 {
    files.iter().map(|file| file.size).sum()
}
