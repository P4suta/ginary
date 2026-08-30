// SPDX-License-Identifier: MIT OR Apache-2.0
//! The launch plan: the argument vector, the environment difference and the
//! preflight check.
//!
//! `launch::plan` is a pure function, so this file is where the launcher's
//! most consequential decision is pinned down exactly: every argument, in
//! order, and every variable set or removed, in order. The running-artifact
//! tests in `tests/launcher.rs` then assert that a real process observes the
//! same thing, which is a different claim — that the plan is what actually
//! reaches `execve` — and would be untestable if the plan itself were only
//! checked from outside.

mod common;

use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

use common::artifact::{APP, ArtifactOptions, ERTS_VSN, canonical_manifest, stage};
use common::snapshot::scrub;

use ginary::cache::Env;
use ginary::error::LauncherError;
use ginary::launch::{
    self, CRASH_DUMP_NAME, ERL_FLAGS_VAR, LaunchPlan, PreflightIssue, REMOVED_VARS,
};
use ginary::manifest::Manifest;

/// The root a plan is built against. It does not have to exist: `plan` reads
/// nothing.
const ROOT: &str = "/cache/hello/0123456789abcdef";

/// Where a crash dump goes by default — the application directory, one level
/// above the entry.
const DUMPS: &str = "/cache/hello";

fn env(pairs: &[(&str, &str)]) -> Env {
    Env::from_pairs(
        pairs
            .iter()
            .map(|(key, value)| (OsString::from(*key), OsString::from(*value))),
    )
}

fn plan_with(env: &Env, args: &[&str]) -> LaunchPlan {
    let user: Vec<OsString> = args.iter().map(OsString::from).collect();
    build(&canonical_manifest(), env, &user)
}

fn build(manifest: &Manifest, env: &Env, user: &[OsString]) -> LaunchPlan {
    match launch::plan(Path::new(ROOT), manifest, user, env, Path::new(DUMPS)) {
        Ok(plan) => plan,
        Err(error) => panic!("the canonical manifest must produce a plan: {error}"),
    }
}

fn args_of(plan: &LaunchPlan) -> Vec<String> {
    plan.args
        .iter()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect()
}

fn set_of(plan: &LaunchPlan) -> Vec<(String, String)> {
    plan.set
        .iter()
        .map(|(key, value)| {
            (
                key.to_string_lossy().into_owned(),
                value.to_string_lossy().into_owned(),
            )
        })
        .collect()
}

fn removed_of(plan: &LaunchPlan) -> Vec<String> {
    plan.remove
        .iter()
        .map(|name| name.to_string_lossy().into_owned())
        .collect()
}

fn value_of(plan: &LaunchPlan, key: &str) -> Option<String> {
    plan.set
        .iter()
        .find(|(name, _)| name == OsStr::new(key))
        .map(|(_, value)| value.to_string_lossy().into_owned())
}

/// The plan as the text a snapshot pins and `ginary trace show` will print.
fn render(plan: &LaunchPlan) -> String {
    let mut out = format!("program: {}\n", plan.program.display());
    out.push_str("argv:\n");
    for argument in args_of(plan) {
        out.push_str(&format!("  {argument}\n"));
    }
    out.push_str("set:\n");
    for (key, value) in set_of(plan) {
        out.push_str(&format!("  {key}={value}\n"));
    }
    out.push_str("remove:\n");
    for name in removed_of(plan) {
        out.push_str(&format!("  {name}\n"));
    }
    out
}

// ------------------------------------------------------ the program --

#[test]
fn the_program_is_the_launch_program_under_the_bindir() {
    let plan = plan_with(&env(&[]), &[]);
    assert_eq!(
        plan.program,
        PathBuf::from(format!("{ROOT}/erts-{ERTS_VSN}/bin/erlexec"))
    );
    assert!(
        plan.program.is_absolute(),
        "the program must be an absolute path, and it is {}",
        plan.program.display()
    );
}

// -------------------------------------------------- the argument vector --

#[test]
fn the_argument_vector_is_the_documented_order() {
    let plan = plan_with(&env(&[]), &["--name", "world"]);
    insta::assert_snapshot!(
        "launch_plan_canonical",
        scrub(
            &render(&plan),
            &[(Path::new(ROOT), "<root>"), (Path::new(DUMPS), "<app-dir>")]
        )
    );
}

#[test]
fn the_fixed_flags_come_first_and_start_epmd_is_two_arguments() {
    let plan = plan_with(&env(&[]), &[]);
    assert_eq!(
        args_of(&plan)[..5],
        [
            "-boot".to_owned(),
            format!("{ROOT}/bin/no_dot_erlang"),
            "-noshell".to_owned(),
            "+B".to_owned(),
            "-start_epmd".to_owned(),
        ]
    );
    assert_eq!(args_of(&plan)[5], "false");
}

#[test]
fn every_pa_entry_becomes_its_own_pair_under_the_root() {
    let plan = plan_with(&env(&[]), &[]);
    let args = args_of(&plan);
    let pairs: Vec<(String, String)> = args
        .windows(2)
        .filter(|window| window[0] == "-pa")
        .map(|window| (window[0].clone(), window[1].clone()))
        .collect();
    assert_eq!(
        pairs,
        vec![
            ("-pa".to_owned(), format!("{ROOT}/lib/{APP}/ebin")),
            ("-pa".to_owned(), format!("{ROOT}/lib/stdlib-8.0.3/ebin")),
        ],
        "the code path entries must keep the manifest's order and be one pair each"
    );
}

#[test]
fn the_manifest_flags_come_after_the_code_path_and_before_the_environment() {
    let plan = plan_with(&env(&[(ERL_FLAGS_VAR, "+S 2:2")]), &[]);
    let args = args_of(&plan);
    let position = |needle: &str| {
        args.iter()
            .position(|argument| argument == needle)
            .unwrap_or_else(|| panic!("`{needle}` is not in {args:?}"))
    };
    assert!(position("-pa") < position("+fnu"), "{args:?}");
    assert!(position("+fnu") < position("+S"), "{args:?}");
    assert!(position("+S") < position("-eval"), "{args:?}");
}

#[test]
fn ginary_erl_flags_splits_on_ascii_whitespace() {
    let plan = plan_with(&env(&[(ERL_FLAGS_VAR, "  +S 2:2 \t +sbwt\nnone ")]), &[]);
    let args = args_of(&plan);
    let start = args
        .iter()
        .position(|argument| argument == "+S")
        .unwrap_or_else(|| panic!("the environment flags are not in {args:?}"));
    assert_eq!(args[start..start + 4], ["+S", "2:2", "+sbwt", "none"]);
}

#[test]
fn an_empty_ginary_erl_flags_adds_nothing() {
    let bare = plan_with(&env(&[]), &[]);
    for value in ["", "   ", "\t\n"] {
        let with = plan_with(&env(&[(ERL_FLAGS_VAR, value)]), &[]);
        assert_eq!(
            args_of(&with),
            args_of(&bare),
            "`{ERL_FLAGS_VAR}={value:?}` must contribute no argument"
        );
    }
}

#[test]
fn the_eval_and_extra_are_the_last_two_things_ginary_contributes() {
    let plan = plan_with(&env(&[]), &[]);
    let args = args_of(&plan);
    assert_eq!(
        args[args.len() - 3..],
        [
            "-eval".to_owned(),
            format!("'{APP}@@main':run('{APP}')"),
            "-extra".to_owned(),
        ]
    );
}

#[test]
fn user_arguments_follow_extra_in_the_order_they_were_given() {
    let plan = plan_with(&env(&[]), &["--help", "-", "--", "-eval", "halt(1)"]);
    let args = args_of(&plan);
    let extra = args
        .iter()
        .position(|argument| argument == "-extra")
        .unwrap_or_else(|| panic!("`-extra` is not in {args:?}"));
    assert_eq!(
        args[extra + 1..],
        ["--help", "-", "--", "-eval", "halt(1)"],
        "the launcher must not interpret, reorder or drop a user argument"
    );
}

#[test]
fn a_user_argument_that_is_not_valid_utf8_survives_byte_for_byte() {
    let raw = OsString::from_vec(vec![b'-', b'-', b'n', 0xff, 0xfe, b'x']);
    let plan = build(&canonical_manifest(), &env(&[]), std::slice::from_ref(&raw));
    assert_eq!(
        plan.args.last(),
        Some(&raw),
        "a non-UTF-8 argument must arrive as the bytes the user typed"
    );
    assert_eq!(
        plan.args.last().map(|argument| argument.as_bytes()),
        Some([b'-', b'-', b'n', 0xff, 0xfe, b'x'].as_slice())
    );
}

#[test]
fn no_user_argument_at_all_still_ends_in_extra() {
    let plan = plan_with(&env(&[]), &[]);
    assert_eq!(
        plan.args.last(),
        Some(&OsString::from("-extra")),
        "`-extra` is unconditional: without it a later argument would be an emulator flag"
    );
}

// ------------------------------------------------------- the environment --

#[test]
fn the_launcher_sets_rootdir_bindir_emu_and_progname() {
    let plan = plan_with(&env(&[]), &[]);
    assert_eq!(value_of(&plan, "ROOTDIR"), Some(ROOT.to_owned()));
    assert_eq!(
        value_of(&plan, "BINDIR"),
        Some(format!("{ROOT}/erts-{ERTS_VSN}/bin"))
    );
    assert_eq!(value_of(&plan, "EMU"), Some("beam".to_owned()));
    assert_eq!(value_of(&plan, "PROGNAME"), Some(APP.to_owned()));
}

#[test]
fn home_defaults_to_the_root_and_never_overrides_the_user() {
    assert_eq!(
        value_of(&plan_with(&env(&[]), &[]), "HOME"),
        Some(ROOT.to_owned())
    );
    assert_eq!(
        value_of(&plan_with(&env(&[("HOME", "/home/u")]), &[]), "HOME"),
        None,
        "a HOME the user set must not be in the plan at all, not even set to itself"
    );
}

#[test]
fn an_empty_home_is_a_home_the_user_set() {
    // An exported-but-empty HOME is a deliberate, if odd, thing; overriding it
    // would be the launcher deciding it knows better.
    assert_eq!(
        value_of(&plan_with(&env(&[("HOME", "")]), &[]), "HOME"),
        None
    );
}

#[test]
fn erl_crash_dump_defaults_into_the_application_directory() {
    assert_eq!(
        value_of(&plan_with(&env(&[]), &[]), "ERL_CRASH_DUMP"),
        Some(format!("{DUMPS}/{CRASH_DUMP_NAME}")),
        "the dump belongs to the application, not to the cache entry that produced it"
    );
    assert_eq!(
        value_of(
            &plan_with(&env(&[("ERL_CRASH_DUMP", "/tmp/mine.dump")]), &[]),
            "ERL_CRASH_DUMP"
        ),
        None
    );
}

#[test]
fn the_set_list_is_in_the_documented_order() {
    let plan = plan_with(&env(&[]), &[]);
    assert_eq!(
        set_of(&plan)
            .into_iter()
            .map(|(key, _)| key)
            .collect::<Vec<_>>(),
        [
            "ROOTDIR",
            "BINDIR",
            "EMU",
            "PROGNAME",
            "HOME",
            "ERL_CRASH_DUMP"
        ]
    );
}

#[test]
fn the_six_named_variables_are_always_removed() {
    // Always, whether or not they are set: the plan is what execve is given,
    // and a variable removed from an environment that did not hold it costs
    // nothing while a conditional removal is a rule with a hole in it.
    let plan = plan_with(&env(&[]), &[]);
    assert_eq!(
        removed_of(&plan),
        REMOVED_VARS
            .iter()
            .map(|name| (*name).to_owned())
            .collect::<Vec<String>>()
    );
}

#[test]
fn every_erl_otp_flags_variable_that_is_set_is_removed() {
    let plan = plan_with(
        &env(&[
            ("ERL_OTP29_FLAGS", "+P 1"),
            ("ERL_OTP26_FLAGS", "+P 2"),
            ("ERL_OTP_FLAGS", "+P 3"),
        ]),
        &[],
    );
    let removed = removed_of(&plan);
    let mut expected: Vec<String> = REMOVED_VARS.iter().map(|name| (*name).to_owned()).collect();
    expected.extend([
        "ERL_OTP26_FLAGS".to_owned(),
        "ERL_OTP29_FLAGS".to_owned(),
        "ERL_OTP_FLAGS".to_owned(),
    ]);
    assert_eq!(
        removed, expected,
        "the pattern matches come after the named six, in sorted order"
    );
}

#[test]
fn a_variable_that_only_half_matches_the_pattern_is_kept() {
    let plan = plan_with(
        &env(&[
            ("ERL_OTP29", "x"),
            ("OTP29_FLAGS", "x"),
            ("ERL_OTP29_FLAGS_EXTRA", "x"),
            ("erl_otp29_flags", "x"),
        ]),
        &[],
    );
    assert_eq!(
        removed_of(&plan),
        REMOVED_VARS
            .iter()
            .map(|name| (*name).to_owned())
            .collect::<Vec<String>>(),
        "only ERL_OTP...\u{5f}FLAGS is the family; scrubbing anything else is scrubbing a \
         variable the user owns"
    );
}

#[test]
fn nothing_is_both_set_and_removed() {
    let plan = plan_with(&env(&[("ERL_LIBS", "/x"), ("HOME", "/home/u")]), &[]);
    for (key, _) in &plan.set {
        assert!(
            !plan.remove.contains(key),
            "{} is in both lists",
            key.to_string_lossy()
        );
    }
}

// -------------------------------------------------------- the refusals --

#[test]
fn a_launch_path_that_escapes_the_root_is_refused() {
    let mut manifest = canonical_manifest();
    manifest.launch.pa[0] = "../../etc".to_owned();
    let error = launch::plan(Path::new(ROOT), &manifest, &[], &env(&[]), Path::new(DUMPS))
        .expect_err("a manifest whose code path leaves the root must be refused");
    assert_eq!(
        error.exit_code(),
        122,
        "an unusable manifest is a format failure, not a cache one"
    );
    assert!(
        error.to_string().contains("launch.pa[0]"),
        "the message must name the field, and it is `{error}`"
    );
}

#[test]
fn an_absolute_launch_program_is_refused() {
    let mut manifest = canonical_manifest();
    manifest.launch.program = "/usr/bin/erlexec".to_owned();
    let error = launch::plan(Path::new(ROOT), &manifest, &[], &env(&[]), Path::new(DUMPS))
        .expect_err("a launch program outside the extracted root must be refused");
    assert_eq!(error.exit_code(), 122);
}

// -------------------------------------------------------- the preflight --

fn runtime(dir: &Path, omit: &[&str]) -> PathBuf {
    let root = dir.join("root");
    stage(
        &root,
        &ArtifactOptions {
            omit: omit.iter().map(|path| (*path).to_owned()).collect(),
            ..ArtifactOptions::default()
        },
    );
    root
}

#[test]
fn preflight_accepts_a_complete_tree() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = runtime(dir.path(), &[]);
    assert_eq!(launch::preflight(&root, &canonical_manifest()), Ok(()));
}

#[test]
fn preflight_names_a_missing_launch_program() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = runtime(dir.path(), &[&format!("erts-{ERTS_VSN}/bin/erlexec")]);
    assert_eq!(
        launch::preflight(&root, &canonical_manifest()),
        Err(PreflightIssue::Missing {
            path: root.join(format!("erts-{ERTS_VSN}/bin/erlexec")),
        })
    );
}

#[test]
fn preflight_names_a_missing_beam_smp() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = runtime(dir.path(), &[&format!("erts-{ERTS_VSN}/bin/beam.smp")]);
    assert_eq!(
        launch::preflight(&root, &canonical_manifest()),
        Err(PreflightIssue::Missing {
            path: root.join(format!("erts-{ERTS_VSN}/bin/beam.smp")),
        })
    );
}

#[test]
fn preflight_checks_erl_child_setup_and_inet_gethost_too() {
    for name in ["erl_child_setup", "inet_gethost"] {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = runtime(dir.path(), &[&format!("erts-{ERTS_VSN}/bin/{name}")]);
        assert_eq!(
            launch::preflight(&root, &canonical_manifest()),
            Err(PreflightIssue::Missing {
                path: root.join(format!("erts-{ERTS_VSN}/bin/{name}")),
            }),
            "a runtime without {name} cannot start ports and must not pass preflight"
        );
    }
}

#[test]
fn preflight_names_a_program_without_the_execute_bit() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = tempfile::tempdir().expect("tempdir");
    let root = runtime(dir.path(), &[]);
    let program = root.join(format!("erts-{ERTS_VSN}/bin/erlexec"));
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o644))
        .expect("clear the execute bit");
    assert_eq!(
        launch::preflight(&root, &canonical_manifest()),
        Err(PreflightIssue::NotExecutable { path: program })
    );
}

#[test]
fn preflight_wants_the_boot_file_and_not_its_execute_bit() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = runtime(dir.path(), &["bin/no_dot_erlang.boot"]);
    assert_eq!(
        launch::preflight(&root, &canonical_manifest()),
        Err(PreflightIssue::Missing {
            path: root.join("bin/no_dot_erlang.boot"),
        }),
        "the manifest's `boot` is the name without the suffix; the file carries it"
    );
}

#[test]
fn a_preflight_failure_is_not_a_launcher_error() {
    // The launcher's answer to a failed preflight is to extract again, so the
    // type must not be one that carries an exit code: a suspicion is not a
    // verdict.
    let dir = tempfile::tempdir().expect("tempdir");
    let root = runtime(dir.path(), &[&format!("erts-{ERTS_VSN}/bin/beam.smp")]);
    let issue = launch::preflight(&root, &canonical_manifest())
        .expect_err("an incomplete tree must not pass preflight");
    let rendered = issue.to_string();
    assert!(
        !rendered.starts_with("ginary: "),
        "`{rendered}` is shaped like a final diagnostic and must not be"
    );
    let _: fn(&LauncherError) -> u8 = LauncherError::exit_code;
}
