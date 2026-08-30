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

mod common;

use std::path::Path;
use std::time::{Duration, Instant};

use common::artifact::{
    APP, ArtifactOptions, DUMP_ARG, ERTS_VSN, EXIT_ARG, RUN_BUDGET, Run, SIGNAL_ARG, STUB_EXIT,
    STUB_SLOGAN, SyntheticArtifact, canonical_manifest, names_in, read_trace,
};

use ginary::cache::Env;
use ginary::launcher::{CMD_USAGE, CMD_USAGE_EXIT};

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
fn wait_for(what: &str, mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if predicate() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out waiting for {what}");
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
        "without a magic the same bytes are the build tool, and no arguments is a usage error"
    );
    assert!(
        plain.stderr_text().contains("Usage:"),
        "expected the command line usage, and got:\n{}",
        plain.stderr_text()
    );
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
    for value in ["uninstall", "", "Directory"] {
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
