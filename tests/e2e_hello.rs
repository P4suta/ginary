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
// The command line half of the suite: every claim in this file is about a
// module the `cli` feature carries, so a `--no-default-features` build has
// nothing here to run. See `docs/dev/log/C2.md`.
#![cfg(feature = "cli")]

mod common;

use ginary::target::Target;
use serde_json::Value;

use crate::common::built::{BuiltProject, PINNED_EPOCH, names_in, sha256_of};
use crate::common::hostpath::{names_the_same_directory, printed_cwd};
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
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(&artifact)
            .expect("stat the artifact")
            .permissions()
            .mode()
            & 0o7777;
        assert_eq!(mode, 0o755, "the artifact has to be runnable by its user");
    }

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
    let printed = printed_cwd(&stdout)
        .unwrap_or_else(|| panic!("the application printed no `cwd=` line:\n{stdout}"));
    // Two spellings of one directory are one directory. On Windows the
    // runtime prints a lower-case drive letter and forward separators, and
    // `canonicalize` answers with the verbatim `\\?\` form — and `%TEMP%` on a
    // runner is an 8.3 name whose long form only the filesystem knows. See
    // `tests/regressions/e12_a_printed_working_directory_was_compared_as_text.rs`.
    assert!(
        names_the_same_directory(printed, &cwd),
        "the application must start where the user is, not where the runtime unpacked:\n\
         printed {printed}\nexpected {}\n{stdout}",
        cwd.display()
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
    assert!(
        report.get("sbom").is_none(),
        "a build that was not asked for a bill of materials names none: {report}"
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

// ------------------------------- (g) the runtime settings, end to end --

/// The `[tools.ginary]` table the runtime-settings variant builds with.
///
/// Written into the *copy* of the fixture rather than into the fixture: the
/// committed project deliberately has no table, so that every other test in
/// this file builds the plainest artifact there is.
const RUNTIME_TABLE: &str = "\n[tools.ginary]\nvm_args = \"config/vm.args\"\n\
                             sys_config = \"config/sys.config\"\n\
                             filename_encoding = \"utf8\"\n\n\
                             [tools.ginary.env]\nGINARY_E2E = \"set-by-the-artifact\"\n";

/// An args file that names nothing ginary passes itself.
const VM_ARGS: &str = "# the fixture's own emulator flags\n+SDio 4\n";

/// A `sys.config` with one application key in it.
const SYS_CONFIG: &str = "[{kernel, [{logger_level, notice}]}].\n";

/// Copies the fixture and adds the three files the runtime settings name.
///
/// # Panics
///
/// If the copy or any of the writes fails.
fn build_runtime_variant() -> BuiltProject {
    let project = BuiltProject::copy(APP);
    let config = project.root().join("config");
    std::fs::create_dir_all(&config).expect("the config directory");
    std::fs::write(config.join("vm.args"), VM_ARGS).expect("the args file");
    std::fs::write(config.join("sys.config"), SYS_CONFIG).expect("the sys.config");

    let manifest = project.root().join("gleam.toml");
    let mut text = std::fs::read_to_string(&manifest).expect("read the manifest");
    text.push_str(RUNTIME_TABLE);
    std::fs::write(&manifest, text).expect("write the manifest back");

    let output = project.build();
    assert!(
        output.status.success(),
        "`ginary build` with the runtime settings failed with {}\n--- stdout ---\n{}\n--- stderr ---\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    project
}

#[test]
fn the_runtime_settings_reach_the_exec_argv_and_the_application_still_runs() {
    let Some(_tools) = require_tools(&TOOLS) else {
        return;
    };
    let project = build_runtime_variant();

    let run = project.run("runtime").traced().args(["0", "x"]).output();

    assert_eq!(
        run.code(),
        0,
        "the application must still run with an args file and a sys.config\n--- stderr ---\n{}",
        run.stderr()
    );
    assert!(
        run.stdout().contains("hello from priv"),
        "the application is unchanged by the runtime settings:\n{}",
        run.stdout()
    );

    let trace = run.trace_text();
    let exec = trace
        .lines()
        .rfind(|line| line.contains("\"phase\":\"exec\""))
        .unwrap_or_else(|| panic!("no exec record in the trace:\n{trace}"));
    for needle in [
        "-args_file",
        "releases/vm.args",
        "-config",
        "releases/sys",
        "+fnu",
    ] {
        assert!(
            exec.contains(needle),
            "the exec record must carry `{needle}`, and it is:\n{exec}"
        );
    }
}

#[test]
fn the_env_default_reaches_the_application_and_a_caller_still_wins() {
    let Some(_tools) = require_tools(&TOOLS) else {
        return;
    };
    let project = build_runtime_variant();

    let plain = project.run("env-default").traced().args(["0"]).output();
    assert_eq!(plain.code(), 0, "--- stderr ---\n{}", plain.stderr());
    assert!(
        plain
            .trace_text()
            .contains("GINARY_E2E=set-by-the-artifact"),
        "the artifact's own default must be in the launch it recorded:\n{}",
        plain.trace_text()
    );

    let overridden = project
        .run("env-caller")
        .traced()
        .env("GINARY_E2E", "set-by-the-caller")
        .args(["0"])
        .output();
    assert_eq!(
        overridden.code(),
        0,
        "--- stderr ---\n{}",
        overridden.stderr()
    );
    assert!(
        !overridden
            .trace_text()
            .contains("GINARY_E2E=set-by-the-artifact"),
        "a variable the caller exported must not be set by the launcher at all:\n{}",
        overridden.trace_text()
    );
}

#[test]
fn an_args_file_that_names_a_flag_the_launcher_owns_fails_the_build() {
    let Some(_tools) = require_tools(&TOOLS) else {
        return;
    };
    let project = BuiltProject::copy(APP);
    let config = project.root().join("config");
    std::fs::create_dir_all(&config).expect("the config directory");
    std::fs::write(config.join("vm.args"), "+SDio 4\n-pa /opt/lib\n").expect("the args file");
    let manifest = project.root().join("gleam.toml");
    let mut text = std::fs::read_to_string(&manifest).expect("read the manifest");
    text.push_str("\n[tools.ginary]\nvm_args = \"config/vm.args\"\n");
    std::fs::write(&manifest, text).expect("write the manifest back");

    let output = project.build();

    assert!(
        !output.status.success(),
        "an args file that builds its own code path must stop the build"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("-pa") && stderr.contains("vm.args:2"),
        "the message must name the flag and the line, and it said:\n{stderr}"
    );
}

#[test]
fn a_sys_config_that_does_not_parse_fails_the_build_with_a_position() {
    let Some(_tools) = require_tools(&TOOLS) else {
        return;
    };
    let project = BuiltProject::copy(APP);
    let config = project.root().join("config");
    std::fs::create_dir_all(&config).expect("the config directory");
    std::fs::write(config.join("sys.config"), "[{kernel, #{}}].\n").expect("the sys.config");
    let manifest = project.root().join("gleam.toml");
    let mut text = std::fs::read_to_string(&manifest).expect("read the manifest");
    text.push_str("\n[tools.ginary]\nsys_config = \"config/sys.config\"\n");
    std::fs::write(&manifest, text).expect("write the manifest back");

    let output = project.build();

    assert!(
        !output.status.success(),
        "a sys.config the runtime could not consult must stop the build"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("sys.config:1:11"),
        "the message must name file, line and column, and it said:\n{stderr}"
    );
}

// ------------------------------------------------ (h) the named target --

/// Builds the fixture with extra flags, failing loudly with what it wrote.
fn build_fixture_with(args: &[&str]) -> BuiltProject {
    let project = BuiltProject::copy(APP);
    let output = project.build_with(args, &[]);
    assert!(
        output.status.success(),
        "`ginary build {}` failed with {}\n--- stdout ---\n{}\n--- stderr ---\n{}",
        args.join(" "),
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    project
}

/// `build/ginary/<app>-<host target>`, the name an explicit `--target` writes.
fn suffixed_artifact(project: &BuiltProject) -> std::path::PathBuf {
    // The artifact carries the host target's executable suffix — `.exe` on
    // Windows, nothing elsewhere — the same as the file `ginary build` writes.
    project.root().join("build/ginary").join(format!(
        "{APP}-{}{}",
        Target::host().name(),
        Target::host().exe_suffix()
    ))
}

#[test]
fn an_explicit_host_target_writes_a_suffixed_artifact_that_runs() {
    let Some(_tools) = require_tools(&TOOLS) else {
        return;
    };
    let host = Target::host().name();
    let project = build_fixture_with(&["--target", &host]);
    let artifact = suffixed_artifact(&project);

    assert!(
        artifact.is_file(),
        "a named target writes `<app>-<target>`; {} holds {:?}",
        artifact.display(),
        names_in(&project.root().join("build/ginary"))
    );
    assert!(
        !project.artifact().exists(),
        "and nothing at the plain name, which would be the same artifact twice"
    );

    let run = project
        .run_program("named-target", &artifact)
        .arg("0")
        .output();

    assert_eq!(run.code(), 0, "{}", run.stderr());
    assert!(
        run.stdout().contains("args=0"),
        "the suffixed artifact is a working application: {}",
        run.stdout()
    );
}

#[test]
fn a_suffixed_build_writes_the_manifest_beside_the_artifact() {
    let Some(_tools) = require_tools(&TOOLS) else {
        return;
    };
    let host = Target::host().name();
    let project = build_fixture_with(&["--target", &host]);
    let copy = project
        .root()
        .join("build/ginary")
        .join(format!("{APP}-{host}.json"));

    let text = std::fs::read_to_string(&copy).unwrap_or_else(|error| {
        panic!(
            "a suffixed build writes {} for a reader of the directory to read: {error}",
            copy.display()
        )
    });
    let manifest: Value = serde_json::from_str(&text).expect("the copy is the manifest as JSON");

    assert_eq!(manifest["app"], APP);
    assert_eq!(manifest["target"], host);
}

#[test]
fn the_manifest_records_what_the_bundled_runtime_is_and_where_it_came_from() {
    let Some(_tools) = require_tools(&TOOLS) else {
        return;
    };
    let project = build_fixture();
    let artifact = project.artifact();

    let output = project.ginary(&["inspect".as_ref(), "--json".as_ref(), artifact.as_os_str()]);
    assert!(
        output.status.success(),
        "inspect failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).expect("the report is JSON");
    let otp = &value["manifest"]["otp"];

    assert_eq!(
        otp["linkage"], "dynamic",
        "the host's own emulator is dynamically linked: {otp}"
    );
    // The C library this host actually has, and not `gnu` written down. A
    // platform with a single system C runtime records `null` here —
    // `Libc::None` is what `Target::host().libc` answers there — and the
    // block is right to say so, so it is the block that decides what to
    // assert. `tests/target.rs` pins `Target::host` itself.
    match ginary::target::Target::host().libc {
        ginary::target::Libc::Gnu => {
            assert_eq!(otp["libc"]["kind"], "gnu", "{otp}");
            let min = otp["libc"]["min"]
                .as_str()
                .unwrap_or_else(|| panic!("a gnu runtime records a minimum glibc: {otp}"));
            assert!(
                min.split('.').all(|part| part.parse::<u32>().is_ok()),
                "the minimum is a version and not a sentence: {min}"
            );
        }
        ginary::target::Libc::Musl => {
            assert_eq!(otp["libc"]["kind"], "musl", "{otp}");
            assert!(
                otp["libc"]["min"].is_null(),
                "musl carries no symbol versions, so there is no minimum: {otp}"
            );
        }
        ginary::target::Libc::None => assert!(
            otp["libc"].is_null(),
            "a platform with one system C runtime names no C library: {otp}"
        ),
    }
    assert_eq!(otp["nif_loading"], true, "{otp}");
    assert!(
        otp["source"]
            .as_str()
            .is_some_and(|source| source.starts_with("host:")),
        "the source names the spelling and the root it resolved to: {otp}"
    );
}

#[test]
fn the_same_target_named_twice_produces_one_artifact() {
    let Some(_tools) = require_tools(&TOOLS) else {
        return;
    };
    let host = Target::host().name();
    let project = build_fixture_with(&["--target", "host", "--target", &host]);

    let written = names_in(&project.root().join("build/ginary"));

    assert_eq!(
        written,
        vec![
            format!("{APP}-{host}{}", Target::host().exe_suffix()),
            format!("{APP}-{host}.json")
        ],
        "`host` and the host's own name are one target, so one artifact and one manifest"
    );
}

#[test]
fn a_build_for_another_target_says_which_stub_it_could_not_find() {
    let Some(_tools) = require_tools(&TOOLS) else {
        return;
    };
    let project = BuiltProject::copy(APP);
    let empty = tempfile::tempdir().expect("a temporary directory");
    let stubs = empty.path().join("stubs");
    let cache = empty.path().join("cache");
    std::fs::create_dir_all(&stubs).expect("an empty stub directory");
    std::fs::create_dir_all(&cache).expect("an empty cache");

    // The environment is what makes the assertion the same on every machine:
    // a developer with a stub already built would otherwise get a different
    // error from the same command.
    let output = project.build_with(
        &["--target", "linux-aarch64-musl"],
        &[
            ("GINARY_STUB_DIR", &stubs.display().to_string()),
            ("GINARY_CACHE_DIR", &cache.display().to_string()),
        ],
    );

    assert!(
        !output.status.success(),
        "a cross build with no stub cannot succeed:\n{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("no stub found for linux-aarch64-musl"),
        "the refusal names the target it has no stub for: {stderr}"
    );
    assert!(
        stderr.contains("stubs:build"),
        "and says how to make one: {stderr}"
    );
    assert!(
        names_in(&project.root().join("build/ginary"))
            .iter()
            .all(|name| name.starts_with(ginary::bundle::WORK_DIR_PREFIX)),
        "and nothing that looks like an artifact is left behind"
    );
}

#[test]
fn a_target_that_is_not_a_target_is_refused_and_lists_the_spellings() {
    // No toolchain: the list is resolved from the flags and the manifest
    // before anything is exported, so this is a usage failure and not a build.
    let project = BuiltProject::copy(APP);

    let output = project.build_with(&["--target", "linux-riscv64-gnu"], &[]);

    assert!(!output.status.success(), "riscv64 is not a target");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("`linux-riscv64-gnu` is not a target"),
        "the refusal names what was asked for: {stderr}"
    );
    assert!(
        stderr.contains("`linux-aarch64-musl`") && stderr.contains("`host`"),
        "and lists every spelling that would have worked: {stderr}"
    );
}
