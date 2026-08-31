// SPDX-License-Identifier: MIT OR Apache-2.0
//! The Phase A acceptance test: `ginary build` in a real project, end to end.
//!
//! Every other test in the suite stops somewhere short of this. `tests/
//! artifact_real.rs` assembles an artifact out of the library's pieces;
//! `tests/launcher.rs` runs a real launcher over a stub runtime;
//! `tests/stage_run.rs` boots a staged tree with no payload around it. This
//! file runs the *command* a user runs, over the fixture project, and then
//! runs what it produced on a machine that is told it has no Erlang at all.
//!
//! Scrubbing is what makes that claim true. Every run of the artifact gets
//! `env_clear()`, a `PATH` that is an empty directory, and a `HOME` and
//! `XDG_CACHE_HOME` inside the test's own tree — so an artifact that only
//! worked because the developer had Erlang installed fails here, which is the
//! whole point.
//!
//! Gated on `gleam`, `erl` and `strip`: a machine without them reports a skip,
//! and `GINARY_REQUIRE_TOOLCHAIN=1` turns the skip into a failure.

mod common;

use std::os::unix::fs::PermissionsExt as _;

use crate::common::built::{BuiltProject, PINNED_EPOCH, names_in, sha256_of};
use crate::common::tools::require_tools;

/// The fixture this file builds, and therefore the artifact's file name.
const APP: &str = "hello_ffi";

/// The programs a build of the fixture needs.
const TOOLS: [&str; 3] = ["gleam", "erl", "strip"];

/// Builds the fixture, failing loudly with whatever the command wrote.
///
/// A build that fails is not a test that skips: it is the test failing, and
/// the diagnosis is whatever `ginary build` printed.
fn build_fixture() -> BuiltProject {
    let project = BuiltProject::copy(APP);
    let output = project.build();
    assert!(
        output.status.success(),
        "`ginary build` failed with {}\n--- stdout ---\n{}\n--- stderr ---\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    project
}

// ------------------------------------------------- (a) build and run --

#[test]
fn ginary_build_writes_one_executable_at_the_default_output_path() {
    let Some(_tools) = require_tools(&TOOLS) else {
        return;
    };
    let project = build_fixture();
    let artifact = project.artifact();

    assert!(
        artifact.is_file(),
        "the default output is build/ginary/<app>, and there is nothing at {}",
        artifact.display()
    );
    let mode = std::fs::metadata(&artifact)
        .expect("stat the artifact")
        .permissions()
        .mode()
        & 0o7777;
    assert_eq!(mode, 0o755, "the artifact has to be runnable by its user");

    // The number `docs/dev/log/A4.md` records. Printed rather than gated: a
    // size assertion that fails on a new ERTS release is one that gets
    // deleted, and the budget belongs in the release checks.
    let size = std::fs::metadata(&artifact).expect("stat").len();
    println!("artifact: {size} bytes");
    assert!(
        size < 64 * 1024 * 1024,
        "a single-file Gleam application must not be {size} bytes"
    );
}

#[test]
fn the_built_artifact_runs_the_application_with_no_erlang_on_the_machine() {
    let Some(_tools) = require_tools(&TOOLS) else {
        return;
    };
    let project = build_fixture();

    let run = project.run("args").args(["3", "a", "b"]).output();

    assert_eq!(
        run.code(),
        3,
        "the application's own exit code must survive execve\n--- stderr ---\n{}",
        run.stderr()
    );
    let stdout = run.stdout();
    assert!(
        stdout.contains("args=3 a b"),
        "everything after the artifact's name belongs to the application:\n{stdout}"
    );
    assert!(
        stdout.contains("hello from priv"),
        "code:priv_dir/1 must find the extracted priv:\n{stdout}"
    );
    let cwd = std::fs::canonicalize(run.cwd.as_path()).expect("canonicalise");
    assert!(
        stdout.contains(&format!("cwd={}", cwd.display())),
        "the application must start where the user is, not where the runtime unpacked:\n{stdout}"
    );
}

#[test]
fn the_built_artifact_propagates_a_zero_exit_code() {
    let Some(_tools) = require_tools(&TOOLS) else {
        return;
    };
    let project = build_fixture();

    let run = project.run("zero").arg("0").output();

    assert_eq!(run.code(), 0, "--- stderr ---\n{}", run.stderr());
}

#[test]
fn the_built_artifact_reports_a_crash_as_exit_one_and_leaves_the_cwd_clean() {
    let Some(_tools) = require_tools(&TOOLS) else {
        return;
    };
    let project = build_fixture();

    let run = project.run("crash").arg("--crash").output();

    assert_eq!(
        run.code(),
        1,
        "an uncaught Gleam error is exit 1, not one of ginary's 121 to 125"
    );
    assert!(
        run.stderr().contains("runtime error"),
        "the crash must reach Gleam's own reporter:\n{}",
        run.stderr()
    );
    assert_eq!(
        run.cwd_entries(),
        Vec::<String>::new(),
        "the runtime must not write erl_crash.dump into the user's working directory"
    );
}

// ------------------------------------------ the report the build prints --

#[test]
fn the_json_report_describes_the_file_that_is_actually_on_disk() {
    let Some(_tools) = require_tools(&TOOLS) else {
        return;
    };
    let project = BuiltProject::copy(APP);

    let output = project.build_with(&["--report", "json"], &[]);
    assert!(
        output.status.success(),
        "the build failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("--report json emits one JSON object");

    assert_eq!(
        report
            .get("format_version")
            .and_then(serde_json::Value::as_u64),
        Some(1),
        "the schema names its own version: {report}"
    );
    let number = |key: &str| {
        report
            .get(key)
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_else(|| panic!("the report has no numeric `{key}`: {report}"))
    };
    let out = report
        .get("out")
        .and_then(serde_json::Value::as_str)
        .expect("the report names the file it wrote");
    assert_eq!(
        std::path::Path::new(out),
        project.artifact(),
        "the report has to name the artifact the build actually wrote"
    );

    // The three numbers against the file, not against each other: this is the
    // assertion `tests/bundle.rs` cannot make, because its report is literals.
    let on_disk = std::fs::metadata(project.artifact())
        .expect("stat the artifact")
        .len();
    assert_eq!(
        number("total_len"),
        on_disk,
        "the report's total has to be the artifact's size"
    );
    assert_eq!(
        number("total_len"),
        number("stub_len") + number("payload_len") + ginary::trailer::TRAILER_LEN,
        "stub + payload + trailer is the whole file and nothing else"
    );
    assert_eq!(
        report
            .get("sha256")
            .and_then(serde_json::Value::as_str)
            .map(str::len),
        Some(64),
        "the payload's digest is 64 hexadecimal characters: {report}"
    );
    assert_eq!(
        report.get("app").and_then(serde_json::Value::as_str),
        Some(APP)
    );
}

#[test]
fn explain_prints_both_accounts_before_the_line_that_names_the_artifact() {
    let Some(_tools) = require_tools(&TOOLS) else {
        return;
    };
    let project = BuiltProject::copy(APP);

    let output = project.build_with(&["--explain"], &[]);
    assert!(
        output.status.success(),
        "the build failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();

    // The two accounts are tables and the header row is what names them: the
    // closure's is `name vsn source origin`, the staged tree's is
    // `category bytes files`.
    let closure = stdout
        .find("origin")
        .unwrap_or_else(|| panic!("--explain must print the closure account:\n{stdout}"));
    let staged = stdout
        .find("bytes")
        .unwrap_or_else(|| panic!("--explain must print the staged account:\n{stdout}"));
    let artifact = stdout
        .find("artifact: ")
        .unwrap_or_else(|| panic!("the report must still end with the artifact line:\n{stdout}"));
    assert!(
        closure < staged && staged < artifact,
        "the accounts come in build order and before the report:\n{stdout}"
    );
}

#[test]
fn verbose_writes_the_phases_to_stderr_without_taking_the_trace_file_away() {
    let Some(_tools) = require_tools(&TOOLS) else {
        return;
    };
    let project = BuiltProject::copy(APP);
    let trace = project.dir().join("verbose-trace.jsonl");

    let output = project.build_with(&["-v"], &[("GINARY_TRACE", &trace.display().to_string())]);
    assert!(
        output.status.success(),
        "the build failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        stderr.contains("ginary[debug]:"),
        "-v is what puts the phases on standard error:\n{stderr}"
    );
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        stdout.contains("artifact: "),
        "-v must not move the report off standard output:\n{stdout}"
    );

    // A recorder that replaces the sinks rather than adding to them takes the
    // file the user explicitly asked for away.
    let recorded = std::fs::read_to_string(&trace).unwrap_or_default();
    assert!(
        recorded.contains("\"phase\""),
        "-v and GINARY_TRACE are two requests, not one: {} holds {recorded:?}",
        trace.display()
    );
}

// ---------------------------------------- (b) the cache and determinism --

#[test]
fn the_second_run_of_one_artifact_hits_the_cache_it_wrote() {
    let Some(_tools) = require_tools(&TOOLS) else {
        return;
    };
    let project = build_fixture();

    let cold = project.run("cold").arg("0").traced().output();
    assert_eq!(cold.code(), 0, "--- stderr ---\n{}", cold.stderr());
    let cold_trace = cold.trace_text();
    assert!(
        cold_trace.contains("extract"),
        "a cold start extracts, and the trace must say so:\n{cold_trace}"
    );

    // The same HOME, so the second run resolves the same cache.
    let warm = project
        .run_program("cold", &project.artifact())
        .arg("0")
        .traced()
        .output();
    assert_eq!(warm.code(), 0, "--- stderr ---\n{}", warm.stderr());
    let warm_trace = warm.trace_text();
    assert!(
        warm_trace.contains("hit"),
        "the second run must reuse the entry the first wrote:\n{warm_trace}"
    );
    assert_eq!(
        names_in(&warm.app_dir(APP))
            .into_iter()
            .filter(|name| !name.starts_with('.'))
            .count(),
        1,
        "a warm run must not write a second entry"
    );
}

#[test]
fn two_builds_of_one_project_with_a_pinned_clock_are_byte_identical() {
    let Some(_tools) = require_tools(&TOOLS) else {
        return;
    };
    let project = BuiltProject::copy(APP);

    // Trailing separators: `--out first` would name a *file* called `first`,
    // which is the rule `tests/config.rs` pins. The two builds have to land in
    // two directories so that the second cannot overwrite the first.
    let first = project.build_with(&["--out", "first/"], &[("SOURCE_DATE_EPOCH", PINNED_EPOCH)]);
    assert!(
        first.status.success(),
        "the first build failed:\n{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let second = project.build_with(
        &["--out", "second/"],
        &[("SOURCE_DATE_EPOCH", PINNED_EPOCH)],
    );
    assert!(
        second.status.success(),
        "the second build failed:\n{}",
        String::from_utf8_lossy(&second.stderr)
    );

    let one = sha256_of(&project.root().join("first").join(APP));
    let two = sha256_of(&project.root().join("second").join(APP));
    assert_eq!(
        one, two,
        "identical input must produce identical artifact bytes"
    );
}

// ------------------------------------------------------- (c) inspect --

#[test]
fn inspect_names_the_application_the_build_packaged_and_verifies_it() {
    let Some(_tools) = require_tools(&TOOLS) else {
        return;
    };
    let project = build_fixture();
    let artifact = project.artifact();

    let output = project.ginary(&["inspect".as_ref(), artifact.as_os_str()]);
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success(),
        "inspect failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains(APP),
        "the report must name the application it packaged:\n{stdout}"
    );

    let verified = project.ginary(&[
        "inspect".as_ref(),
        "--verify".as_ref(),
        artifact.as_os_str(),
    ]);
    assert!(
        verified.status.success(),
        "a freshly built artifact must verify:\n{}\n{}",
        String::from_utf8_lossy(&verified.stdout),
        String::from_utf8_lossy(&verified.stderr)
    );
    assert!(
        String::from_utf8_lossy(&verified.stdout).contains("verify: ok"),
        "the verification must say so in as many words:\n{}",
        String::from_utf8_lossy(&verified.stdout)
    );
}

#[test]
fn a_flipped_payload_byte_fails_verify_and_still_prints_the_manifest() {
    let Some(_tools) = require_tools(&TOOLS) else {
        return;
    };
    let project = build_fixture();
    let artifact = project.artifact();
    // A byte near the end of the compressed payload, sixteen bytes before the
    // trailer: the front of the payload, where the manifest is, must still be
    // readable, so `inspect` without `--verify` still answers.
    let len = std::fs::metadata(&artifact).expect("stat").len();
    let damaged = crate::common::built::corrupt_copy(
        &artifact,
        "damaged",
        len - ginary::trailer::TRAILER_LEN - 16,
    );

    let verified = project.ginary(&["inspect".as_ref(), "--verify".as_ref(), damaged.as_os_str()]);
    assert_eq!(
        verified.status.code(),
        Some(1),
        "a payload that does not match its trailer is exit 1"
    );

    let plain = project.ginary(&["inspect".as_ref(), damaged.as_os_str()]);
    assert!(
        plain.status.success(),
        "without --verify the manifest is still readable:\n{}",
        String::from_utf8_lossy(&plain.stderr)
    );
    assert!(
        String::from_utf8_lossy(&plain.stdout).contains(APP),
        "a user has to be able to see what the damaged file was supposed to be"
    );
}

// --------------------------------------------- (d) the GINARY_CMD path --

#[test]
fn ginary_cmd_extract_only_prints_the_cache_path_of_the_built_artifact() {
    let Some(_tools) = require_tools(&TOOLS) else {
        return;
    };
    let project = build_fixture();

    let run = project
        .run("extract")
        .env("GINARY_CMD", "extract-only")
        .output();

    assert_eq!(run.code(), 0, "--- stderr ---\n{}", run.stderr());
    let printed = run.stdout().trim().to_owned();
    assert!(
        printed.starts_with(&run.app_dir(APP).display().to_string()),
        "the printed entry must be under this run's own cache: {printed}"
    );
    assert!(
        std::path::Path::new(&printed).join("ginary.json").is_file(),
        "the entry the command printed must be a complete one: {printed}"
    );
}

// -------------------------------------- (e) an artifact is not the CLI --

#[test]
fn a_packaged_application_hands_the_word_build_to_the_application() {
    let Some(_tools) = require_tools(&TOOLS) else {
        return;
    };
    let project = build_fixture();

    // The launcher never parses argv, so `<artifact> build` does not reach the
    // command line at all: `build` is the application's first argument. The
    // refusal a bundled *stub* meets is a different path, and
    // `tests/bundle.rs` covers it through `bundle::check_stub`.
    let run = project.run("subcommand").arg("build").output();

    let stdout = run.stdout();
    assert!(
        stdout.contains("args=build"),
        "every argument after the artifact's name belongs to the application:\n{stdout}"
    );
    assert_eq!(
        run.code(),
        0,
        "`build` is not an integer, so the fixture halts zero"
    );
}

// ---------------------------------------------- (f) the work directory --

#[test]
fn keep_staging_leaves_the_work_directory_and_prints_where_it_is() {
    let Some(_tools) = require_tools(&TOOLS) else {
        return;
    };
    let project = BuiltProject::copy(APP);

    let output = project.build_with(&["--keep-staging"], &[]);
    assert!(
        output.status.success(),
        "the build failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();

    let kept = project.work_dirs();
    assert_eq!(
        kept.len(),
        1,
        "--keep-staging keeps exactly the one work directory this build made: {kept:?}"
    );
    assert!(
        kept[0].join("root/bin/no_dot_erlang.boot").is_file(),
        "what is kept has to be the staging root, not an empty directory: {}",
        kept[0].display()
    );
    assert!(
        stdout.contains(&kept[0].display().to_string()),
        "a directory that is kept and not named is a directory nobody finds:\n{stdout}"
    );
}

#[test]
fn a_successful_build_removes_its_work_directory() {
    let Some(_tools) = require_tools(&TOOLS) else {
        return;
    };
    let project = build_fixture();

    assert_eq!(
        project.work_dirs(),
        Vec::<std::path::PathBuf>::new(),
        "the default must leave the project tree as it found it"
    );
}

#[cfg(feature = "fault-injection")]
#[test]
fn a_build_that_fails_while_packing_removes_its_work_directory_too() {
    let Some(_tools) = require_tools(&TOOLS) else {
        return;
    };
    let project = BuiltProject::copy(APP);

    let output = project.build_with(&[], &[("GINARY_FAULT", "pack:fail")]);

    assert!(
        !output.status.success(),
        "an armed fault must fail the build:\n{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        stderr.contains("pack"),
        "the failure must say which phase it came from: {stderr}"
    );
    assert_eq!(
        project.work_dirs(),
        Vec::<std::path::PathBuf>::new(),
        "a failed build must not leave its staging tree in the project"
    );
    assert!(
        !project.artifact().exists(),
        "a failed build must not leave a half-written artifact at {}",
        project.artifact().display()
    );
}
