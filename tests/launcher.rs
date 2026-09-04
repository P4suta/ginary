// SPDX-License-Identifier: MIT OR Apache-2.0
//! The launcher contract, asserted on real processes and no Erlang.
//!
//! Every test here runs a hand-assembled artifact — this test run's own
//! `ginary` binary with a payload and a trailer appended — whose `erlexec` is
//! a shell script that prints the environment it was given and its own
//! arguments before exiting 7. What the launcher decides is therefore visible
//! on standard output, and what it decides is the whole subject: the argument
//! vector, the environment difference, the cache, the exit code and the five
//! numbered failures.
//!
//! The environment of every run is cleared first. A launcher test that read
//! the developer's `HOME` or their real cache would pass on one machine.

// A unix file. Every test here starts a real process out of a hand-assembled
// artifact whose `erlexec` is a `#!/bin/sh` script, and asserts what `execve`
// did with it: none of that exists on Windows, where the launcher spawns and
// waits instead. `tests/windows.rs` is the other half, and holds every rule of
// the Windows launcher a Linux machine can honestly check. See
// tests/regressions/e6_the_test_helpers_did_not_compile_on_windows.rs.
#![cfg(unix)]

mod common;

use std::path::{Path, PathBuf};
#[cfg(feature = "fault-injection")]
use std::time::{Duration, Instant};

use common::artifact::{
    APP, ArtifactOptions, DUMP_ARG, ERTS_VSN, EXIT_ARG, RUN_BUDGET, Run, SIGNAL_ARG, SLEEP_ARG,
    STUB_EXIT, STUB_SLOGAN, SyntheticArtifact, canonical_manifest, names_in, read_trace,
};
use common::cachefs::{DAY, HeldLock, is_unlocked, lock_path, plant_entry, wait_until_unlocked};
use common::tools::require_tools;

use ginary::cache::Env;
use ginary::launcher::{CMD_USAGE, CMD_USAGE_EXIT};
use ginary::manifest::LaunchSpec;

/// How long the lock proof's runtime sleeps, in seconds.
///
/// Comfortably under `cachefs::LOCK_BUDGET`, which is what the assertions
/// wait against: the two are separate numbers on purpose, so that a loaded
/// machine cannot turn the proof into a race between them.
const RUNTIME_NAP: &str = "3";

fn artifact(dir: &tempfile::TempDir) -> SyntheticArtifact {
    SyntheticArtifact::build(dir.path())
}

fn ok(run: &Run) -> &Run {
    assert_eq!(
        run.code(),
        STUB_EXIT,
        "the run did not reach the runtime\n--- stdout ---\n{}\n--- stderr ---\n{}",
        run.stdout_text(),
        run.stderr_text()
    );
    run
}

/// The lines of standard error that are ginary's own.
fn ginary_lines(run: &Run) -> Vec<String> {
    run.stderr_text()
        .lines()
        .filter(|line| line.starts_with("ginary:"))
        .map(str::to_owned)
        .collect()
}

/// Waits until `predicate` holds, or fails after five seconds.
///
/// The `fault-injection` tests are the ones that pause a run long enough to
/// have something to wait for. The lock proof waits too, on
/// `cachefs::wait_until_unlocked`, because what it waits for is a lock rather
/// than a file.
#[cfg(feature = "fault-injection")]
fn wait_for(what: &str, mut predicate: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if predicate() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out waiting for {what}");
}

/// An artifact whose manifest carries the runtime settings `change` sets.
///
/// `bins` are extra programs under the bindir — `epmd` for a distributed
/// artifact, `heart` for one under `heart` — and `files` are extra staged
/// files, which is how `releases/vm.args` and `releases/sys.config` get into
/// the tree the launcher names.
fn runtime_artifact(
    dir: &tempfile::TempDir,
    bins: &[&str],
    files: &[(&str, &[u8])],
    change: impl FnOnce(&mut LaunchSpec),
) -> SyntheticArtifact {
    let mut launch = canonical_manifest().launch;
    change(&mut launch);
    SyntheticArtifact::build_with(
        dir.path(),
        &ArtifactOptions {
            erts_bins: bins.iter().map(|name| (*name).to_owned()).collect(),
            extra_files: files
                .iter()
                .map(|(path, bytes)| {
                    (
                        (*path).to_owned(),
                        0o644,
                        (*bytes).to_vec(),
                        ginary::assemble::Category::Other,
                    )
                })
                .collect(),
            launch: Some(launch),
            ..ArtifactOptions::default()
        },
    )
}

// ------------------------------------------------------ (a) environment --

#[test]
fn the_runtime_gets_rootdir_bindir_emu_and_progname() {
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = artifact(&dir);
    let run = artifact.run().output();
    ok(&run);

    let entry = artifact.key_dir();
    let env = run.env();
    assert_eq!(env.get("ROOTDIR"), Some(&entry.display().to_string()));
    assert_eq!(
        env.get("BINDIR"),
        Some(
            &entry
                .join(format!("erts-{ERTS_VSN}/bin"))
                .display()
                .to_string()
        )
    );
    assert_eq!(env.get("EMU"), Some(&"beam".to_owned()));
    assert_eq!(env.get("PROGNAME"), Some(&APP.to_owned()));
}

#[test]
fn erl_libs_is_removed_even_when_the_caller_exports_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = artifact(&dir);
    let run = artifact
        .run()
        .env("ERL_LIBS", "/opt/other/lib")
        .env("ERL_AFLAGS", "+P 1")
        .env("ERL_FLAGS", "-name x")
        .output();
    ok(&run);

    let env = run.env();
    for name in ["ERL_LIBS", "ERL_FLAGS", "ERL_AFLAGS"] {
        assert_eq!(
            env.get(name),
            Some(&"<unset>".to_owned()),
            "{name} must not reach a runtime that ships its own libraries"
        );
    }
}

#[test]
fn home_is_preserved_when_the_caller_set_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = artifact(&dir);
    let run = artifact.run().output();
    ok(&run);
    assert_eq!(
        run.env().get("HOME"),
        Some(&artifact.home().display().to_string()),
        "a HOME the user set is the user's"
    );
}

#[test]
fn home_defaults_to_the_extracted_root_when_it_is_unset() {
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = artifact(&dir);
    let run = artifact.run().without("HOME").output();
    ok(&run);
    assert_eq!(
        run.env().get("HOME"),
        Some(&artifact.key_dir().display().to_string()),
        "a runtime with no HOME writes .erlang.cookie into the working directory"
    );
}

#[test]
fn erl_crash_dump_defaults_into_the_application_directory() {
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = artifact(&dir);
    let run = artifact.run().output();
    ok(&run);
    assert_eq!(
        run.env().get("ERL_CRASH_DUMP"),
        Some(
            &artifact
                .app_dir()
                .join("erl_crash.dump")
                .display()
                .to_string()
        ),
        "the dump outlives the cache entry, so it belongs to the application directory"
    );
}

#[test]
fn an_erl_crash_dump_the_caller_set_is_left_alone() {
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = artifact(&dir);
    let run = artifact
        .run()
        .env("ERL_CRASH_DUMP", "/tmp/mine.dump")
        .output();
    ok(&run);
    assert_eq!(
        run.env().get("ERL_CRASH_DUMP"),
        Some(&"/tmp/mine.dump".to_owned())
    );
}

#[test]
fn the_runtime_starts_in_the_callers_working_directory() {
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = artifact(&dir);
    let run = artifact.run().output();
    ok(&run);
    assert_eq!(
        run.cwd().as_deref(),
        Some(
            std::fs::canonicalize(artifact.dir())
                .expect("canonicalise")
                .display()
                .to_string()
                .as_str()
        ),
        "the launcher must not chdir: a relative path in a user argument is the user's"
    );
}

// ------------------------------------------------------------ (b) argv --

#[test]
fn the_argument_vector_is_the_one_the_plan_built() {
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = artifact(&dir);
    let run = artifact.run().arg("--name").arg("world").output();
    ok(&run);

    let user = [
        std::ffi::OsString::from("--name"),
        std::ffi::OsString::from("world"),
    ];
    let plan = ginary::launch::plan(
        &artifact.key_dir(),
        &canonical_manifest(),
        &user,
        &Env::from_pairs([(
            std::ffi::OsString::from("HOME"),
            artifact.home().into_os_string(),
        )]),
        &artifact.app_dir(),
        artifact.path(),
    )
    .expect("the canonical manifest must produce a plan");

    let expected: Vec<Vec<u8>> = plan
        .args
        .iter()
        .map(|argument| {
            use std::os::unix::ffi::OsStrExt as _;
            argument.as_bytes().to_vec()
        })
        .collect();
    assert_eq!(
        run.argv(),
        expected,
        "the running artifact must pass exactly what `launch::plan` says\nactual: {:?}",
        run.argv_text()
    );
}

#[test]
fn a_user_argument_that_is_not_valid_utf8_reaches_the_runtime_unchanged() {
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = artifact(&dir);
    let run = artifact
        .run()
        .raw_arg(&[b'-', b'-', 0xff, 0xfe, b'x'])
        .output();
    ok(&run);
    assert_eq!(
        run.argv().last().map(Vec::as_slice),
        Some([b'-', b'-', 0xff, 0xfe, b'x'].as_slice()),
        "argv is bytes; the launcher must not round-trip it through UTF-8"
    );
}

#[test]
fn the_application_owns_help() {
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = artifact(&dir);
    let run = artifact.run().arg("--help").output();
    ok(&run);
    assert!(
        run.argv_text().contains(&"--help".to_owned()),
        "`--help` belongs to the packaged application, and ginary answered it instead:\n{}",
        run.stdout_text()
    );
}

// ------------------------------------------------------ (c) exit codes --

#[test]
fn the_runtime_s_exit_code_is_the_artifact_s() {
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = artifact(&dir);
    assert_eq!(artifact.run().output().code(), STUB_EXIT);
    assert_eq!(
        artifact.run().arg(EXIT_ARG).arg("3").output().code(),
        3,
        "an application's own exit code must not be rewritten"
    );
    assert_eq!(artifact.run().arg(EXIT_ARG).arg("0").output().code(), 0);
}

#[test]
fn supervise_mirrors_the_exit_code_too() {
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = artifact(&dir);
    let run = artifact
        .run()
        .env("GINARY_SUPERVISE", "1")
        .arg(EXIT_ARG)
        .arg("3")
        .output();
    assert_eq!(
        run.code(),
        3,
        "spawn-and-wait must be indistinguishable from execve to the caller\n{}",
        run.stderr_text()
    );
    assert!(
        run.argv_text().contains(&"-noshell".to_owned()),
        "the supervised child must get the same plan"
    );
}

/// The signal the stub kills itself with, and the code the shell convention
/// turns it into.
const SIGKILL: i32 = 9;

#[test]
fn a_supervised_child_killed_by_a_signal_exits_128_plus_the_signal() {
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = artifact(&dir);
    let trace = dir.path().join("signal.jsonl");

    let run = artifact
        .run()
        .env("GINARY_SUPERVISE", "1")
        .env("GINARY_TRACE", &trace)
        .arg(SIGNAL_ARG)
        .arg(SIGKILL.to_string())
        .output();

    assert_eq!(
        run.code(),
        128 + SIGKILL,
        "a parent has an exit code and nothing else with which to report a signal\n{}",
        run.stderr_text()
    );
    let records = read_trace(&trace);
    let supervise = records
        .iter()
        .rfind(|record| record.phase == "supervise")
        .expect("a supervised run must record how it ended");
    assert_eq!(
        supervise.kv.get("signal").map(String::as_str),
        Some(SIGKILL.to_string().as_str()),
        "the trace must name the signal, and it recorded {:?}",
        supervise.kv
    );
    assert_eq!(
        supervise.kv.get("exit").map(String::as_str),
        Some((128 + SIGKILL).to_string().as_str())
    );
    assert!(
        supervise.kv.contains_key("elapsed_us"),
        "the trace must say how long the run took, and it recorded {:?}",
        supervise.kv
    );
}

#[test]
fn a_crash_dump_written_during_a_supervised_run_is_reported() {
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = artifact(&dir);
    let trace = dir.path().join("dump.jsonl");

    let run = artifact
        .run()
        .env("GINARY_SUPERVISE", "1")
        .env("GINARY_TRACE", &trace)
        .arg(DUMP_ARG)
        .output();
    ok(&run);

    let dump = artifact.app_dir().join("erl_crash.dump");
    assert!(
        dump.is_file(),
        "the stub must have written the dump ERL_CRASH_DUMP named"
    );
    let reported = ginary_lines(&run);
    assert_eq!(
        reported,
        vec![format!("ginary: {STUB_SLOGAN}")],
        "the slogan is the one line a supervised crash is worth"
    );
    let records = read_trace(&trace);
    let crash = records
        .iter()
        .rfind(|record| record.phase == "crash_dump")
        .expect("the dump must reach the trace as well as standard error");
    assert_eq!(
        crash.kv.get("slogan").map(String::as_str),
        Some(STUB_SLOGAN)
    );
}

#[test]
fn a_crash_dump_the_run_did_not_write_is_not_reported() {
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = artifact(&dir);
    // Warm the cache so that the application directory exists, then plant a
    // dump the way a *previous* crash would have left one.
    ok(&artifact.run().output());
    let dump = artifact.app_dir().join("erl_crash.dump");
    std::fs::write(&dump, "Slogan: a crash from last week\n").expect("plant the old dump");
    let planted = std::fs::metadata(&dump)
        .and_then(|meta| meta.modified())
        .expect("the planted dump has a modification time");

    let run = artifact.run().env("GINARY_SUPERVISE", "1").output();
    ok(&run);

    assert!(
        ginary_lines(&run).is_empty(),
        "a dump this run did not write is not this run's news, and it said {:?}",
        ginary_lines(&run)
    );
    assert_eq!(
        std::fs::metadata(&dump)
            .and_then(|meta| meta.modified())
            .expect("the dump is still there"),
        planted,
        "and it must be left exactly as it was"
    );
}

// ----------------------------------------------------------- (d) cache --

#[test]
fn the_first_run_extracts_and_the_second_hits() {
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = artifact(&dir);
    let first_trace = dir.path().join("first.jsonl");
    let second_trace = dir.path().join("second.jsonl");

    ok(&artifact.run().env("GINARY_TRACE", &first_trace).output());
    ok(&artifact.run().env("GINARY_TRACE", &second_trace).output());

    let first: Vec<String> = read_trace(&first_trace)
        .into_iter()
        .map(|record| record.phase)
        .collect();
    let second: Vec<String> = read_trace(&second_trace)
        .into_iter()
        .map(|record| record.phase)
        .collect();

    assert!(
        first.contains(&"extract".to_owned()),
        "the first run must extract, and it recorded {first:?}"
    );
    assert!(
        second.contains(&"cache_hit".to_owned()) && !second.contains(&"extract".to_owned()),
        "the second run must hit, and it recorded {second:?}"
    );
}

#[test]
fn a_successful_run_leaves_exactly_one_directory_and_no_residue() {
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = artifact(&dir);
    ok(&artifact.run().output());
    assert_eq!(
        names_in(&artifact.app_dir()),
        vec![artifact.key()],
        "the rename is the completion marker; nothing else may survive it"
    );
}

#[test]
fn a_renamed_artifact_reuses_the_same_entry() {
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = artifact(&dir);
    ok(&artifact.run().output());
    let renamed = artifact.copy_to("moved-elsewhere");
    ok(&artifact.run().program(&renamed).output());
    assert_eq!(
        names_in(&artifact.app_dir()),
        vec![artifact.key()],
        "the key comes from the payload's digest, so a copy under another name shares the entry"
    );
}

// ------------------------------------------------- (e) the cache root --

#[test]
fn ginary_cache_dir_is_honoured() {
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = artifact(&dir);
    let elsewhere = dir.path().join("elsewhere");
    let run = artifact.run().env("GINARY_CACHE_DIR", &elsewhere).output();
    ok(&run);
    assert_eq!(names_in(&elsewhere), vec![APP.to_owned()]);
    assert!(
        !artifact.cache_root().exists(),
        "the XDG root must not be touched when the override is set"
    );
    assert!(
        run.env()
            .get("ROOTDIR")
            .is_some_and(|root| root.starts_with(&elsewhere.display().to_string()))
    );
}

#[test]
fn a_read_only_cache_root_falls_back_with_one_warning() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = artifact(&dir);
    let locked = dir.path().join("locked");
    let scratch = dir.path().join("scratch");
    std::fs::create_dir(&locked).expect("create the read-only root");
    std::fs::create_dir(&scratch).expect("create TMPDIR");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o500))
        .expect("make it read-only");

    let run = artifact
        .run()
        .env("GINARY_CACHE_DIR", locked.join("cache"))
        .env("TMPDIR", &scratch)
        .output();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700)).expect("restore");

    assert_eq!(
        run.code(),
        STUB_EXIT,
        "an unwritable cache must not stop the application\n{}",
        run.stderr_text()
    );
    let warnings = ginary_lines(&run);
    assert_eq!(
        warnings.len(),
        1,
        "the fallback is worth exactly one line, and it said {warnings:?}"
    );
    assert_eq!(
        names_in(&scratch).len(),
        1,
        "the fallback root was not used"
    );
}

// -------------------------------------------------------- (f) trailer --

#[test]
fn the_magic_is_what_decides_the_mode() {
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = artifact(&dir);

    let packaged = artifact.run().output();
    assert!(
        !packaged.stderr_text().contains("Usage:"),
        "an intact artifact must never print ginary's own usage:\n{}",
        packaged.stderr_text()
    );
    assert_eq!(packaged.code(), STUB_EXIT);

    artifact.break_magic();
    let plain = artifact.run().output();
    assert_eq!(
        plain.code(),
        2,
        "without a magic the same bytes are not a packaged application, and running one with \
         no arguments is a usage error"
    );
    // What the other half *is* depends on which flavor this suite built. A
    // full ginary is the command line tool; a stub carries the launcher and
    // nothing else, and says so. Both are the same claim — the magic decided
    // the mode — and asserting only the first would leave the stub flavor
    // proving nothing here.
    if cfg!(feature = "cli") {
        assert!(
            plain.stderr_text().contains("Usage:"),
            "expected the command line usage, and got:\n{}",
            plain.stderr_text()
        );
    } else {
        assert!(
            plain
                .stderr_text()
                .contains(&ginary::launcher::no_payload_line(
                    ginary::target::Target::host()
                )),
            "expected the launcher stub's own sentence, and got:\n{}",
            plain.stderr_text()
        );
    }
}

#[test]
fn a_corrupt_geometry_exits_122_without_reaching_the_command_line() {
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = artifact(&dir);
    artifact.break_geometry();
    let run = artifact.run().output();
    assert_eq!(run.code(), 122);
    let lines = ginary_lines(&run);
    assert_eq!(lines.len(), 1, "expected one diagnostic, got {lines:?}");
    assert!(
        lines[0].contains("truncated or something was appended"),
        "`{}` does not say what is wrong with the file",
        lines[0]
    );
    assert!(
        !run.stderr_text().contains("Usage:"),
        "a broken application must never become ginary's help"
    );
}

#[test]
fn a_truncated_artifact_exits_122() {
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = artifact(&dir);
    artifact.truncate(1);
    let run = artifact.run().output();
    assert_eq!(run.code(), 122);
    assert_eq!(ginary_lines(&run).len(), 1);
}

#[test]
fn a_flipped_payload_byte_exits_123_and_leaves_no_entry() {
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = artifact(&dir);
    artifact.break_payload();
    let run = artifact.run().output();
    assert_eq!(run.code(), 123);
    assert!(
        !artifact.key_dir().exists(),
        "a payload that failed its digest must leave nothing a later run would trust"
    );
    let lines = ginary_lines(&run);
    assert_eq!(lines.len(), 1, "expected one diagnostic, got {lines:?}");
}

// ------------------------------------------------ (g) manifest version --

#[test]
fn a_manifest_format_version_this_build_cannot_read_exits_122() {
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = SyntheticArtifact::build_with(
        dir.path(),
        &ArtifactOptions {
            format_version: Some(2),
            ..ArtifactOptions::default()
        },
    );
    let run = artifact.run().output();
    assert_eq!(
        run.code(),
        122,
        "a manifest from a newer ginary is a format failure, not a corrupt payload\n{}",
        run.stderr_text()
    );
    let lines = ginary_lines(&run);
    assert_eq!(lines.len(), 1, "expected one diagnostic, got {lines:?}");
    assert!(
        lines[0].contains("manifest format version 2"),
        "`{}` does not name the version it found",
        lines[0]
    );
}

// ----------------------------------------------------- (h) concurrency --

#[test]
fn eight_concurrent_cold_starts_produce_one_entry_and_no_residue() {
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = artifact(&dir);

    let children: Vec<std::process::Child> = (0..8).map(|_| artifact.run().spawn()).collect();
    let mut codes = Vec::new();
    for child in children {
        // Bounded, like every other spawn in the suite: eight processes racing
        // for one cache entry is the place a deadlock would appear, and a
        // deadlock must be a failed test rather than a stalled job.
        let output = common::bounded::wait_bounded(child, RUN_BUDGET, "a concurrent run");
        codes.push((
            output.status.code(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ));
    }

    for (code, stderr) in &codes {
        assert_eq!(*code, Some(STUB_EXIT), "a concurrent run failed:\n{stderr}");
    }
    assert_eq!(
        names_in(&artifact.app_dir()),
        vec![artifact.key()],
        "eight racing extractions must leave one entry and no temporary tree"
    );
}

// -------------------------------------------------- (i) fault injection --

#[cfg(feature = "fault-injection")]
#[test]
fn a_process_killed_mid_extraction_is_swept_by_the_next_run() {
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = artifact(&dir);

    let mut child = artifact
        .run()
        .env("GINARY_FAULT", "after-extract:pause")
        .spawn();
    let app_dir = artifact.app_dir();
    wait_for("the temporary tree to appear", || {
        app_dir.is_dir() && names_in(&app_dir).iter().any(|name| name.contains(".tmp-"))
    });
    child.kill().expect("kill the paused extraction");
    let _ = child.wait();

    let residue: Vec<String> = names_in(&app_dir);
    assert!(
        residue.iter().any(|name| name.contains(".tmp-")),
        "the killed run must leave its temporary tree behind, and left {residue:?}"
    );

    let trace = dir.path().join("sweep.jsonl");
    let run = artifact.run().env("GINARY_TRACE", &trace).output();
    ok(&run);

    let records = read_trace(&trace);
    let sweep = records
        .iter()
        .find(|record| record.phase == "cache_sweep")
        .expect("the next run must record its sweep");
    assert_eq!(
        sweep.kv.get("removed").map(String::as_str),
        Some("1"),
        "the sweep must say what it removed, and it said {:?}",
        sweep.kv
    );
    assert_eq!(names_in(&app_dir), vec![artifact.key()]);
}

#[cfg(feature = "fault-injection")]
#[test]
fn a_lost_rename_race_reuses_the_winner_s_entry() {
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = artifact(&dir);
    ok(&artifact.run().output());

    let trace = dir.path().join("race.jsonl");
    let run = artifact
        .run()
        .env("GINARY_FAULT", "rename:eexist")
        .env("GINARY_CACHE_DIR", artifact.cache_root())
        .env("GINARY_TRACE", &trace)
        .output();
    ok(&run);

    let records = read_trace(&trace);
    let rename = records
        .iter()
        .rfind(|record| record.phase == "rename")
        .expect("the rename must be a recorded phase");
    assert_eq!(
        rename.kv.get("reused").map(String::as_str),
        Some("true"),
        "losing the race must be recorded as a reuse, and it recorded {:?}",
        rename.kv
    );
    assert_eq!(
        names_in(&artifact.app_dir()),
        vec![artifact.key()],
        "the loser must remove its own tree"
    );
}

#[cfg(feature = "fault-injection")]
#[test]
fn a_payload_corrupted_under_the_reader_exits_123() {
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = artifact(&dir);
    let run = artifact
        .run()
        .env("GINARY_FAULT", "unpack:corrupt")
        .output();
    assert_eq!(run.code(), 123);
    assert!(
        !artifact.key_dir().exists(),
        "the digest is checked after the last entry, so nothing may be left behind"
    );
}

#[cfg(feature = "fault-injection")]
#[test]
fn a_panic_on_the_launcher_path_is_one_line_and_121() {
    // The launcher promises never to panic, and `main` installs a hook so that
    // a broken promise still looks like every other launcher failure. Without
    // something to trigger it the hook is a claim; `GINARY_FAULT=launcher:panic`
    // is what makes it a test.
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = artifact(&dir);

    let run = artifact
        .run()
        .env("GINARY_FAULT", "launcher:panic")
        .output();

    assert_eq!(
        run.code(),
        121,
        "a bug in ginary is a ginary failure, not an application exit code\n{}",
        run.stderr_text()
    );
    let stderr = run.stderr_text();
    assert_eq!(
        ginary_lines(&run),
        vec![format!(
            "ginary: internal error (this is a bug in ginary): {}",
            ginary::fault::PANIC_MESSAGE
        )],
        "the panic must be one attributed line: `{stderr}`"
    );
    assert!(
        !stderr.contains("panicked at") && !stderr.contains("backtrace"),
        "a user must never be shown a Rust backtrace: `{stderr}`"
    );
    assert!(
        !artifact.key_dir().exists(),
        "the panic is before anything is extracted"
    );
}

// ---------------------------------------------- (m) a runtime that will not start --

#[test]
fn a_runtime_whose_interpreter_is_missing_exits_125_with_a_hint() {
    // `execve` answers `ENOENT` for a program that is on disk when the *loader*
    // it names is not — the shape a glibc-linked runtime takes on a machine
    // without that glibc. Preflight passes, because the file is there and is
    // executable, so this is the one failure that reaches `exec` itself.
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = artifact(&dir);
    ok(&artifact.run().output());

    let program = artifact
        .key_dir()
        .join(format!("erts-{ERTS_VSN}/bin/erlexec"));
    std::fs::write(&program, "#!/nonexistent/interpreter\nexit 0\n")
        .expect("rewrite the launch program");
    set_executable(&program);

    let run = artifact.run().output();

    assert_eq!(
        run.code(),
        125,
        "a runtime that will not start is 125\n{}",
        run.stderr_text()
    );
    let stderr = run.stderr_text();
    let lines: Vec<&str> = stderr.lines().collect();
    assert_eq!(
        lines.len(),
        2,
        "the failure and its hint, and nothing else: {lines:?}"
    );
    assert!(
        lines[0].starts_with("ginary: cannot start ") && lines[0].contains("erlexec"),
        "the first line names the program: `{}`",
        lines[0]
    );
    assert!(
        lines[1].starts_with("hint: ") && lines[1].contains("ld-linux"),
        "the second line is the advice a user can act on: `{}`",
        lines[1]
    );
}

/// Gives a file mode 0755.
fn set_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
        .unwrap_or_else(|error| panic!("cannot chmod {}: {error}", path.display()));
}

// ------------------------------------------------------- (j) GINARY_CMD --

#[test]
fn ginary_cmd_directory_prints_the_entry_and_extracts_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = artifact(&dir);
    let run = artifact.run().env("GINARY_CMD", "directory").output();
    assert_eq!(run.code(), 0, "{}", run.stderr_text());
    assert_eq!(
        run.stdout_text(),
        format!("{}\n", artifact.key_dir().display())
    );
    assert!(
        !artifact.key_dir().exists(),
        "`directory` is a question, not an instruction"
    );
}

#[test]
fn ginary_cmd_extract_only_extracts_and_prints_the_entry() {
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = artifact(&dir);
    let run = artifact.run().env("GINARY_CMD", "extract-only").output();
    assert_eq!(run.code(), 0, "{}", run.stderr_text());
    assert_eq!(
        run.stdout_text(),
        format!("{}\n", artifact.key_dir().display())
    );
    assert!(artifact.key_dir().join("ginary.json").is_file());
    assert!(
        run.argv().is_empty(),
        "`extract-only` must not start the runtime"
    );
}

#[test]
fn ginary_cmd_inspect_prints_the_manifest_and_the_geometry() {
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = artifact(&dir);
    let run = artifact.run().env("GINARY_CMD", "inspect").output();
    assert_eq!(run.code(), 0, "{}", run.stderr_text());

    let value: serde_json::Value =
        serde_json::from_str(&run.stdout_text()).expect("`inspect` must print one JSON object");
    assert_eq!(
        value.pointer("/manifest/app").and_then(|app| app.as_str()),
        Some(APP)
    );
    assert_eq!(
        value
            .pointer("/trailer/payload_offset")
            .and_then(serde_json::Value::as_u64),
        Some(artifact.stub_len())
    );
    assert_eq!(
        value
            .pointer("/trailer/payload_len")
            .and_then(serde_json::Value::as_u64),
        Some(artifact.packed().len)
    );
    assert_eq!(
        value
            .pointer("/trailer/sha256")
            .and_then(serde_json::Value::as_str),
        Some(hex::encode(artifact.packed().sha256).as_str())
    );
    assert!(
        !artifact.key_dir().exists(),
        "`inspect` reads entry 0 and stops"
    );
}

#[test]
fn an_unknown_ginary_cmd_is_a_usage_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = artifact(&dir);
    for value in [
        "reinstall",
        "",
        "Directory",
        "Uninstall",
        "self-test",
        "prune",
    ] {
        let run = artifact.run().env("GINARY_CMD", value).output();
        assert_eq!(
            run.code(),
            i32::from(CMD_USAGE_EXIT),
            "`GINARY_CMD={value}` must be a usage error"
        );
        assert!(
            run.stderr_text().contains(CMD_USAGE),
            "the usage must name the three commands, and it said:\n{}",
            run.stderr_text()
        );
        assert!(run.argv().is_empty(), "nothing may be launched");
        let stderr = run.stderr_text();
        for name in ["selftest", "uninstall"] {
            assert!(
                stderr.contains(name),
                "the usage a near miss prints must offer `{name}`, and it said:\n{stderr}"
            );
        }
    }
}

// ---------------------------------------------------- (k) diagnostics --

#[test]
fn ginary_debug_prints_phase_lines_to_stderr() {
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = artifact(&dir);
    let run = artifact.run().env("GINARY_DEBUG", "1").output();
    ok(&run);

    let lines: Vec<String> = run
        .stderr_text()
        .lines()
        .filter(|line| line.starts_with("ginary[debug]: "))
        .map(str::to_owned)
        .collect();
    assert!(
        lines.len() >= 4,
        "the debug output must cover the phases, and it said {lines:?}"
    );
    for phase in ["read_manifest", "extract", "exec"] {
        assert!(
            lines.iter().any(|line| line.contains(phase)),
            "no `{phase}` line in {lines:?}"
        );
    }
}

#[test]
fn nothing_is_printed_when_neither_switch_is_set() {
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = artifact(&dir);
    let run = artifact.run().output();
    ok(&run);
    assert_eq!(
        run.stderr_text(),
        "",
        "a successful launch is silent: the application owns standard error"
    );
}

#[test]
fn the_trace_records_a_launch_that_can_be_reproduced() {
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = artifact(&dir);
    let trace = dir.path().join("trace.jsonl");
    ok(&artifact
        .run()
        .env("GINARY_TRACE", &trace)
        .arg("--name")
        .arg("world")
        .output());

    let records = read_trace(&trace);
    let exec = records
        .iter()
        .rfind(|record| record.phase == "exec")
        .expect("the plan must be recorded immediately before execve");

    assert_eq!(
        exec.kv.get("program").map(String::as_str),
        Some(
            artifact
                .key_dir()
                .join(format!("erts-{ERTS_VSN}/bin/erlexec"))
                .display()
                .to_string()
                .as_str()
        )
    );

    let argv: Vec<String> = serde_json::from_str(
        exec.kv
            .get("argv")
            .expect("the exec record must carry the whole argv")
            .as_str(),
    )
    .expect("`argv` must be a JSON array of strings");
    for entry in &canonical_manifest().launch.pa {
        let expected = artifact.key_dir().join(entry).display().to_string();
        assert!(
            argv.contains(&expected),
            "the trace must hold every -pa path for the launch to be reproducible; `{expected}` \
             is not in {argv:?}"
        );
    }
    assert_eq!(argv.last().map(String::as_str), Some("world"));

    let removed: Vec<String> = serde_json::from_str(
        exec.kv
            .get("env_remove")
            .expect("the exec record must carry the environment difference")
            .as_str(),
    )
    .expect("`env_remove` must be a JSON array of strings");
    assert!(removed.contains(&"ERL_LIBS".to_owned()));
}

// ------------------------------------------------------- (l) preflight --

#[test]
fn a_damaged_entry_is_extracted_again() {
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = artifact(&dir);
    ok(&artifact.run().output());

    let beam = artifact
        .key_dir()
        .join(format!("erts-{ERTS_VSN}/bin/beam.smp"));
    std::fs::remove_file(&beam).expect("damage the entry");

    let trace = dir.path().join("repair.jsonl");
    ok(&artifact.run().env("GINARY_TRACE", &trace).output());

    assert!(beam.is_file(), "the entry must have been extracted again");
    let phases: Vec<String> = read_trace(&trace)
        .into_iter()
        .map(|record| record.phase)
        .collect();
    assert!(
        phases.contains(&"preflight_retry".to_owned()),
        "the repair must be visible in the trace, which holds {phases:?}"
    );
    assert_eq!(names_in(&artifact.app_dir()), vec![artifact.key()]);
}

#[test]
fn a_payload_that_cannot_pass_preflight_exits_124_after_one_retry() {
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = SyntheticArtifact::build_with(
        dir.path(),
        &ArtifactOptions {
            omit: vec![format!("erts-{ERTS_VSN}/bin/beam.smp")],
            ..ArtifactOptions::default()
        },
    );
    let trace = dir.path().join("hopeless.jsonl");
    let run = artifact.run().env("GINARY_TRACE", &trace).output();

    assert_eq!(
        run.code(),
        124,
        "a runtime that cannot start is a cache failure, not an exec one\n{}",
        run.stderr_text()
    );
    let lines = ginary_lines(&run);
    assert_eq!(lines.len(), 1, "expected one diagnostic, got {lines:?}");
    assert!(
        lines[0].contains("beam.smp"),
        "`{}` does not name the file that is missing",
        lines[0]
    );

    let extracts = read_trace(&trace)
        .into_iter()
        .filter(|record| record.phase == "extract")
        .count();
    assert_eq!(
        extracts, 2,
        "exactly one retry: a third extraction would be a loop a user reports as a hang"
    );
}

#[test]
fn the_launcher_never_reads_the_artifact_s_own_name() {
    // Everything above runs an artifact called `hello`. This one is called
    // something a shell would mangle, to pin that no path is derived from it.
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = artifact(&dir);
    let odd = artifact.copy_to("a name with spaces");
    let run = artifact.run().program(Path::new(&odd)).output();
    ok(&run);
    assert_eq!(run.env().get("PROGNAME"), Some(&APP.to_owned()));
    assert_eq!(names_in(&artifact.app_dir()), vec![artifact.key()]);
}

// -------------------------------------------- (n) the runtime settings --

#[test]
fn a_distributed_artifact_carries_epmd_and_does_not_disable_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = runtime_artifact(&dir, &["epmd"], &[], |launch| launch.distribution = true);

    let run = artifact.run().output();
    ok(&run);

    assert!(
        artifact
            .key_dir()
            .join(format!("erts-{ERTS_VSN}/bin/epmd"))
            .is_file(),
        "a distributed artifact must bundle the daemon it stops disabling"
    );
    assert!(
        !run.argv_text()
            .iter()
            .any(|argument| argument == "-start_epmd"),
        "the runtime must be allowed to start epmd, and it was given {:?}",
        run.argv_text()
    );

    // The default artifact, for contrast: it ships no daemon and says so.
    let plain_dir = tempfile::tempdir().expect("tempdir");
    let plain = SyntheticArtifact::build(plain_dir.path());
    let plain_run = plain.run().output();
    ok(&plain_run);
    assert!(
        plain_run
            .argv_text()
            .iter()
            .any(|argument| argument == "-start_epmd"),
        "an artifact that bundles no epmd must disable it"
    );
    assert!(
        !plain
            .key_dir()
            .join(format!("erts-{ERTS_VSN}/bin/epmd"))
            .exists()
    );
}

#[test]
fn the_args_file_and_the_config_name_files_that_are_in_the_tree() {
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = runtime_artifact(
        &dir,
        &[],
        &[
            ("releases/vm.args", b"-setcookie ginary\n".as_slice()),
            ("releases/sys.config", b"[{kernel, []}].\n".as_slice()),
        ],
        |launch| {
            launch.args_file = Some("releases/vm.args".to_owned());
            launch.config = Some("releases/sys".to_owned());
        },
    );

    let run = artifact.run().output();
    ok(&run);
    let argv = run.argv_text();

    assert_eq!(
        argv.first().map(String::as_str),
        Some("-args_file"),
        "the args file leads the vector so that ginary's own flags win, and it got {argv:?}"
    );
    let args_file = artifact.key_dir().join("releases/vm.args");
    assert_eq!(argv[1], args_file.display().to_string());
    assert!(
        args_file.is_file(),
        "the file the runtime is sent to must be there"
    );

    let position = argv
        .iter()
        .position(|argument| argument == "-config")
        .unwrap_or_else(|| panic!("`-config` is not in {argv:?}"));
    assert_eq!(
        argv[position + 1],
        artifact
            .key_dir()
            .join("releases/sys")
            .display()
            .to_string(),
        "`-config` names the file without its extension"
    );
    assert!(artifact.key_dir().join("releases/sys.config").is_file());
}

#[test]
fn the_filename_encoding_flag_reaches_the_runtime() {
    for (encoding, flag) in [("utf8", "+fnu"), ("latin1", "+fnl"), ("auto", "+fna")] {
        let dir = tempfile::tempdir().expect("tempdir");
        let artifact = runtime_artifact(&dir, &[], &[], |launch| {
            launch.filename_encoding = encoding.to_owned();
        });
        let run = artifact.run().output();
        ok(&run);
        assert!(
            run.argv_text().iter().any(|argument| argument == flag),
            "`{encoding}` must reach the runtime as `{flag}`, and it got {:?}",
            run.argv_text()
        );
    }
}

#[test]
fn a_manifest_env_default_is_set_and_a_caller_s_value_wins() {
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = runtime_artifact(&dir, &[], &[], |launch| {
        launch.env = std::collections::BTreeMap::from([
            ("GINARY_ENV_ONE".to_owned(), "from-manifest".to_owned()),
            ("GINARY_ENV_TWO".to_owned(), "from-manifest".to_owned()),
        ]);
    });

    let run = artifact.run().env("GINARY_ENV_ONE", "from-caller").output();
    ok(&run);

    let env = run.env();
    assert_eq!(
        env.get("GINARY_ENV_ONE").map(String::as_str),
        Some("from-caller"),
        "a variable the caller exported must survive the launcher untouched"
    );
    assert_eq!(
        env.get("GINARY_ENV_TWO").map(String::as_str),
        Some("from-manifest"),
        "and one it did not export takes the artifact's default"
    );
}

#[test]
fn a_manifest_env_default_cannot_reintroduce_a_scrubbed_variable() {
    // The launcher applies `env` *after* the scrub, so this is the ordering
    // that matters: a manifest that named `ERL_LIBS` would otherwise put back
    // the variable the scrub had just removed. The build refuses such a name;
    // the launcher must not depend on that having happened.
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = runtime_artifact(&dir, &[], &[], |launch| {
        launch.env = std::collections::BTreeMap::from([
            ("ERL_LIBS".to_owned(), "/opt/lib".to_owned()),
            ("GINARY_ENV_ONE".to_owned(), "applied".to_owned()),
        ]);
    });

    let run = artifact.run().env("ERL_LIBS", "/from/caller").output();
    ok(&run);

    let env = run.env();
    assert_eq!(
        env.get("GINARY_ENV_ONE").map(String::as_str),
        Some("applied"),
        "the ordinary defaults in the same table are applied, so the absence below is the \
         scrub rather than `env` never running"
    );
    assert_eq!(
        env.get("ERL_LIBS").map(String::as_str),
        Some("<unset>"),
        "the scrub is unconditional and `env` may not undo it"
    );
}

#[test]
fn heart_bundles_its_program_and_names_the_artifact_in_heart_command() {
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = runtime_artifact(&dir, &["heart"], &[], |launch| launch.heart = true);

    let run = artifact.run().arg("--name").arg("world").output();
    ok(&run);

    assert!(
        artifact
            .key_dir()
            .join(format!("erts-{ERTS_VSN}/bin/heart"))
            .is_file()
    );
    assert!(
        run.argv_text().iter().any(|argument| argument == "-heart"),
        "the runtime must be told to start heart, and it got {:?}",
        run.argv_text()
    );
    assert_eq!(
        run.env().get("HEART_COMMAND").map(String::as_str),
        Some(format!("{} --name world", artifact.path().display()).as_str()),
        "heart restarts the application by re-running the artifact with its own arguments"
    );
}

#[test]
fn a_heart_command_the_caller_exported_is_left_alone() {
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = runtime_artifact(&dir, &["heart"], &[], |launch| launch.heart = true);

    let run = artifact
        .run()
        .env("HEART_COMMAND", "/usr/local/bin/supervise")
        .output();
    ok(&run);

    assert!(
        run.argv_text().iter().any(|argument| argument == "-heart"),
        "the runtime is still started under heart; only the command is the caller's, and it \
         got {:?}",
        run.argv_text()
    );
    assert_eq!(
        run.env().get("HEART_COMMAND").map(String::as_str),
        Some("/usr/local/bin/supervise")
    );
}

// ------------------------------------ (o) the lock, across execve --

#[test]
fn the_shared_lock_outlives_the_launcher_and_dies_with_the_runtime() {
    // The executable proof of the claim ADR 0010 rests on: `flock` belongs to
    // the open file description, and a descriptor without FD_CLOEXEC survives
    // `execve`. The launcher takes the lock and then execs; nothing of ginary
    // is left running, so if the lock were process-bound it would already be
    // gone by the time this test looks.
    let Some(tools) = require_tools(&["flock", "sleep"]) else {
        return;
    };
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = artifact(&dir);
    let flock = tools.path("flock");
    let sleep_dir = tools
        .path("sleep")
        .parent()
        .expect("`sleep` has a directory")
        .to_path_buf();

    // A real `PATH`, because the stub's `sleep` is a program rather than a
    // shell builtin. Nothing on the launcher path reads `PATH`.
    let mut child = artifact
        .run()
        .env("PATH", &sleep_dir)
        .arg(SLEEP_ARG)
        .arg(RUNTIME_NAP)
        .spawn();

    let lock = lock_path(&artifact.key_dir());
    assert!(
        wait_until_unlocked(flock, &lock, false),
        "while the runtime runs, {} must exist and must not be exclusively lockable: the \
         launcher execs, so a lock that did not survive execve is already gone",
        lock.display()
    );

    // `kill` reaches the stub shell and not the `sleep` it started, and that
    // is the point: the grandchild inherited the descriptor and holds the lock
    // until it exits on its own. So the wait that follows is bounded by
    // `cachefs::LOCK_BUDGET` rather than by `RUNTIME_NAP`, and the two numbers
    // are deliberately far apart.
    let _ = child.kill();
    let _ = child.wait();

    assert!(
        wait_until_unlocked(flock, &lock, true),
        "the kernel releases the lock when the last holder exits, and nothing else does"
    );
}

#[test]
fn a_finished_run_leaves_the_lock_file_and_no_lock() {
    let Some(tools) = require_tools(&["flock"]) else {
        return;
    };
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = artifact(&dir);
    ok(&artifact.run().output());

    let lock = lock_path(&artifact.key_dir());
    assert!(
        lock.is_file(),
        "the lock file stays with the entry it belongs to"
    );
    assert!(
        is_unlocked(tools.path("flock"), &lock),
        "an application that has exited holds nothing"
    );
}

// --------------------------------------------- (p) pruning on launch --

#[test]
fn an_old_sibling_is_pruned_by_the_next_run_and_the_new_entry_is_not() {
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = artifact(&dir);
    std::fs::create_dir_all(artifact.app_dir()).expect("create the application directory");
    let old = plant_entry(&artifact.app_dir(), "0000000000000000", DAY * 30);

    let trace = dir.path().join("prune.jsonl");
    ok(&artifact.run().env("GINARY_TRACE", &trace).output());

    assert!(
        !old.exists(),
        "a sibling nobody has touched for a month is what pruning is for"
    );
    assert_eq!(
        names_in(&artifact.app_dir()),
        vec![artifact.key()],
        "and nothing else is left behind"
    );
    let records = read_trace(&trace);
    let prune = records
        .iter()
        .find(|record| record.phase == "prune")
        .unwrap_or_else(|| {
            let phases: Vec<&String> = records.iter().map(|record| &record.phase).collect();
            panic!("what was pruned belongs in the trace, which holds {phases:?}")
        });
    let removed = prune
        .kv
        .get("removed_paths")
        .expect("the prune record must name what it removed");
    assert!(
        removed.contains(&old.display().to_string()),
        "a count explains nothing: the record must name the entry that vanished, and it says \
         {removed}"
    );
}

#[test]
fn an_uninstall_leaves_the_crash_dump_beside_the_entries() {
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = artifact(&dir);
    ok(&artifact.run().output());
    let dump = artifact.app_dir().join("erl_crash.dump");
    std::fs::write(&dump, b"=erl_crash_dump:0.5\nSlogan: killed\n").expect("plant a dump");

    let run = artifact.run().env("GINARY_CMD", "uninstall").output();

    assert_eq!(run.code(), 0, "{}", run.stderr_text());
    assert!(
        dump.is_file(),
        "the dump is why the application directory is worth keeping, and uninstall removes only \
         what the cache wrote:\n{}",
        run.stdout_text()
    );
    assert!(
        !artifact.key_dir().exists(),
        "the entry itself is the cache's own and goes"
    );
    assert!(
        artifact.app_dir().is_dir(),
        "and the directory holding the dump stays"
    );
}

#[test]
fn a_locked_old_sibling_survives_the_next_run() {
    let Some(tools) = require_tools(&["flock"]) else {
        return;
    };
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = artifact(&dir);
    std::fs::create_dir_all(artifact.app_dir()).expect("create the application directory");
    let busy = plant_entry(&artifact.app_dir(), "0000000000000000", DAY * 30);
    let free = plant_entry(&artifact.app_dir(), "1111111111111111", DAY * 30);
    let lock = HeldLock::take(tools.path("flock"), &busy);

    ok(&artifact.run().output());

    assert!(
        busy.join("ginary.json").is_file(),
        "another application is running out of that entry; pruning must skip it"
    );
    assert!(
        !free.exists(),
        "and its equally stale neighbour, which nobody holds, goes: the lock is what \
         separated them"
    );
    lock.release(tools.path("flock"));
}

#[test]
fn a_fresh_sibling_survives_the_next_run() {
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = artifact(&dir);
    std::fs::create_dir_all(artifact.app_dir()).expect("create the application directory");
    let fresh = plant_entry(&artifact.app_dir(), "0000000000000000", DAY);
    let old = plant_entry(&artifact.app_dir(), "1111111111111111", DAY * 30);

    ok(&artifact.run().output());

    assert!(fresh.join("ginary.json").is_file());
    assert!(
        !old.exists(),
        "the age is what decides, so a run that spared both spared nothing"
    );
}

#[test]
fn ginary_prune_days_zero_turns_pruning_off_for_a_run() {
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = artifact(&dir);
    std::fs::create_dir_all(artifact.app_dir()).expect("create the application directory");
    let old = plant_entry(&artifact.app_dir(), "0000000000000000", DAY * 400);

    ok(&artifact.run().env("GINARY_PRUNE_DAYS", "0").output());

    assert!(
        old.join("ginary.json").is_file(),
        "a user who turned pruning off must keep every entry"
    );

    // And the same entry against the default age, so that the survival above
    // is the setting rather than a prune that never happens.
    ok(&artifact.run().output());
    assert!(
        !old.exists(),
        "without the override the same sibling is four hundred days stale"
    );
}

#[test]
fn a_failing_prune_never_fails_the_launch() {
    // An application directory whose entries cannot be removed is a
    // housekeeping problem, and housekeeping does not decide whether an
    // application starts.
    use std::os::unix::fs::PermissionsExt as _;

    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = artifact(&dir);
    ok(&artifact.run().output());

    // Extract first, then plant the sibling and make it unremovable, then run
    // again. The sibling cannot be planted before the first run: that run
    // prunes as every run does, and a thirty-day-old entry would be gone
    // before this test had a chance to protect it.
    let old = plant_entry(&artifact.app_dir(), "0000000000000000", DAY * 30);
    std::fs::set_permissions(&old, std::fs::Permissions::from_mode(0o500))
        .expect("make the sibling unremovable");
    let run = artifact.run().output();
    std::fs::set_permissions(&old, std::fs::Permissions::from_mode(0o700))
        .expect("restore the mode so the tempdir can be cleaned up");

    assert_eq!(
        run.code(),
        STUB_EXIT,
        "the application must start whatever pruning could not do\n{}",
        run.stderr_text()
    );
    assert_eq!(
        ginary_lines(&run),
        Vec::<String>::new(),
        "and a best-effort prune says nothing on standard error"
    );
    assert!(
        old.join("ginary.json").is_file(),
        "the sibling is still there, which is the failure this test arranged"
    );

    // With the mode restored the same sibling goes, so the survival above is
    // the failure being tolerated rather than pruning never running.
    ok(&artifact.run().output());
    assert!(!old.exists());
}

// ----------------------------------------- (q) GINARY_CMD uninstall --

#[test]
fn ginary_cmd_uninstall_removes_every_entry_of_this_application() {
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = artifact(&dir);
    ok(&artifact.run().output());
    let sibling = plant_entry(&artifact.app_dir(), "0000000000000000", DAY);

    let run = artifact.run().env("GINARY_CMD", "uninstall").output();

    assert_eq!(run.code(), 0, "{}", run.stderr_text());
    let stdout = run.stdout_text();
    for entry in [artifact.key_dir(), sibling.clone()] {
        assert!(
            stdout.contains(&format!("removed {}", entry.display())),
            "uninstall must name what it removed, and it said:\n{stdout}"
        );
    }
    assert!(
        stdout.contains("total: 2 removed, 0 kept"),
        "the summary must count both columns, and it said:\n{stdout}"
    );
    assert!(!sibling.exists());
    assert!(
        !artifact.app_dir().exists(),
        "an application directory with nothing left in it goes too"
    );
}

#[test]
fn ginary_cmd_uninstall_keeps_a_locked_entry_and_still_exits_zero() {
    let Some(tools) = require_tools(&["flock"]) else {
        return;
    };
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = artifact(&dir);
    ok(&artifact.run().output());
    let busy = plant_entry(&artifact.app_dir(), "0000000000000000", DAY);
    let lock = HeldLock::take(tools.path("flock"), &busy);

    let run = artifact.run().env("GINARY_CMD", "uninstall").output();

    assert_eq!(
        run.code(),
        0,
        "a partial uninstall is reported, not failed\n{}",
        run.stderr_text()
    );
    let stdout = run.stdout_text();
    assert!(
        stdout.contains(&format!("kept {} (locked)", busy.display())),
        "uninstall must say what it kept and why, and it said:\n{stdout}"
    );
    assert!(
        stdout.contains("total: 1 removed, 1 kept"),
        "the summary said:\n{stdout}"
    );
    assert!(busy.join("ginary.json").is_file());
    assert!(
        artifact.app_dir().is_dir(),
        "an application directory that still holds something must stay"
    );
    lock.release(tools.path("flock"));
}

#[test]
fn ginary_cmd_uninstall_removes_temporary_and_corrupt_residue() {
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = artifact(&dir);
    ok(&artifact.run().output());
    let residue: PathBuf = artifact.app_dir().join(".0000000000000000.tmp-4000000000");
    std::fs::create_dir_all(residue.join("lib")).expect("plant residue");

    let run = artifact.run().env("GINARY_CMD", "uninstall").output();

    assert_eq!(run.code(), 0, "{}", run.stderr_text());
    assert!(
        !residue.exists(),
        "uninstall means the application leaves nothing behind, residue included"
    );
    assert!(!artifact.app_dir().exists());
}

#[test]
fn ginary_cmd_uninstall_on_a_cold_machine_removes_nothing_and_exits_zero() {
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = artifact(&dir);

    let run = artifact.run().env("GINARY_CMD", "uninstall").output();

    assert_eq!(run.code(), 0, "{}", run.stderr_text());
    assert_eq!(run.stdout_text(), "total: 0 removed, 0 kept\n");
    assert!(
        run.argv().is_empty(),
        "`uninstall` must not start the runtime"
    );
}

// ------------------------------------------ (r) GINARY_CMD selftest --

#[test]
fn ginary_cmd_selftest_extracts_checks_and_runs_a_halt() {
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = artifact(&dir);

    let run = artifact.run().env("GINARY_CMD", "selftest").output();

    assert_eq!(
        run.code(),
        0,
        "a healthy artifact selftests clean\n{}",
        run.stderr_text()
    );
    assert_eq!(
        run.stdout_text(),
        "extract: PASS\npreflight: PASS\nrun: PASS\n",
        "each step reports itself, in the order the launcher does them"
    );
    assert!(
        artifact.key_dir().join("ginary.json").is_file(),
        "selftest extracts, because there is nothing to test until it has"
    );
}

#[test]
fn a_selftest_run_replaces_the_eval_and_drops_the_user_s_arguments() {
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = artifact(&dir);
    let trace = dir.path().join("selftest.jsonl");

    let run = artifact
        .run()
        .env("GINARY_CMD", "selftest")
        .env("GINARY_TRACE", &trace)
        .arg("--name")
        .arg("world")
        .output();
    assert_eq!(run.code(), 0, "{}", run.stderr_text());

    let records = read_trace(&trace);
    let exec = records
        .iter()
        .rfind(|record| record.phase == "exec")
        .expect("the selftest run must be recorded like any other launch");
    let argv: Vec<String> = serde_json::from_str(
        exec.kv
            .get("argv")
            .expect("the exec record must carry the whole argv")
            .as_str(),
    )
    .expect("`argv` must be a JSON array of strings");

    let position = argv
        .iter()
        .position(|argument| argument == "-eval")
        .unwrap_or_else(|| panic!("`-eval` is not in {argv:?}"));
    assert_eq!(
        argv[position + 1],
        "erlang:halt(0)",
        "a selftest starts the runtime and stops it, and runs no application code"
    );
    assert!(
        !argv.iter().any(|argument| argument == "-extra"),
        "there are no user arguments in a selftest, so there is nothing to introduce, \
         and it got {argv:?}"
    );
}

#[test]
fn a_selftest_of_a_runtime_that_cannot_start_fails_the_step_and_exits_one() {
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = SyntheticArtifact::build_with(
        dir.path(),
        &ArtifactOptions {
            omit: vec![format!("erts-{ERTS_VSN}/bin/beam.smp")],
            ..ArtifactOptions::default()
        },
    );

    let run = artifact.run().env("GINARY_CMD", "selftest").output();

    assert_eq!(
        run.code(),
        1,
        "a selftest that found a broken artifact must say so with its exit code\n{}",
        run.stderr_text()
    );
    let stdout = run.stdout_text();
    assert!(
        stdout.contains("extract: PASS"),
        "the steps before the failure still report, and it said:\n{stdout}"
    );
    assert!(
        stdout.contains("preflight: FAIL"),
        "the failing step must be named, and it said:\n{stdout}"
    );
    assert!(
        stdout.contains("beam.smp"),
        "and it must name the file, and it said:\n{stdout}"
    );
}

#[test]
fn the_usage_line_names_all_five_commands() {
    assert_eq!(
        CMD_USAGE,
        "usage: GINARY_CMD=directory|extract-only|inspect|selftest|uninstall"
    );
}
