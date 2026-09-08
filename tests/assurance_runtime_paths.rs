// SPDX-License-Identifier: MIT OR Apache-2.0
//! Diagnostic and refusal paths driven through the public library surface.
#![cfg(feature = "cli")]

mod common;

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use common::native as objects;
use common::payload::SharedSink;
use common::script::{ShimStep, program};
use common::stubfile::{self, Marker};
use ginary::diag::Diag;
use ginary::launch::{self, LaunchPlan, RunIssue};
use ginary::native::{self, NativeArtifact, NativeError, NativeKind};
use ginary::stub::{self, StubError, StubOpts};
use ginary::target::Target;

fn bare_plan(program: PathBuf) -> LaunchPlan {
    LaunchPlan {
        program,
        args: Vec::new(),
        set: Vec::new(),
        remove: Vec::new(),
    }
}

fn trace() -> (Diag, SharedSink) {
    let sink = SharedSink::new();
    (
        Diag::with_sinks(None, Some(Box::new(sink.clone()))).with_sensitive(true),
        sink,
    )
}

fn events(sink: &SharedSink, phase: &str) -> Vec<serde_json::Value> {
    sink.lines()
        .iter()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .filter(|event| event["phase"] == phase)
        .collect()
}

// A real child of the supervisor, selected explicitly so the ordinary test
// inventory runs this harmlessly. The file format represents a BEAM crash dump;
// neither an Erlang installation nor platform-specific shell parsing is needed.
#[test]
fn supervision_child() {
    let Some(dir) = std::env::var_os("GINARY_ASSURANCE_SUPERVISION_CHILD") else {
        return;
    };
    let dir = PathBuf::from(dir);
    let mode = std::env::var("GINARY_ASSURANCE_DUMP_MODE").unwrap();
    let dump = dir.join(launch::CRASH_DUMP_NAME);
    match mode.as_str() {
        "fresh" => std::fs::write(
            dump,
            b"=erl_crash_dump:0.5\nSlogan: controlled runtime failure\n",
        )
        .unwrap(),
        "missing-slogan" => {
            std::fs::write(dump, b"=erl_crash_dump:0.5\nSystem version: fixture\n").unwrap()
        }
        "late-slogan" => std::fs::write(
            dump,
            format!("{}Slogan: beyond diagnostic bound\n", "header\n".repeat(64)),
        )
        .unwrap(),
        "invalid-utf8" => std::fs::write(dump, b"\xff\nSlogan: after invalid line\n").unwrap(),
        "unchanged" | "absent" => {}
        other => panic!("unknown child mode: {other}"),
    }
    let observed = serde_json::json!({
        "set": std::env::var("GINARY_ASSURANCE_PROPAGATED").ok(),
        "removed": std::env::var("GINARY_ASSURANCE_REMOVED").ok(),
    });
    std::fs::write(
        dir.join("environment.json"),
        serde_json::to_vec(&observed).unwrap(),
    )
    .unwrap();
    std::process::exit(23);
}

#[test]
fn supervision_reports_only_a_fresh_bounded_crash_slogan_and_preserves_the_exit() {
    for mode in [
        "fresh",
        "missing-slogan",
        "late-slogan",
        "invalid-utf8",
        "unchanged",
        "absent",
    ] {
        let dir = tempfile::tempdir().unwrap();
        if mode == "unchanged" {
            std::fs::write(
                dir.path().join(launch::CRASH_DUMP_NAME),
                b"Slogan: previous run\n",
            )
            .unwrap();
        }
        let (diag, sink) = trace();
        let mut plan = bare_plan(std::env::current_exe().unwrap());
        plan.args = ["--exact", "supervision_child", "--quiet"]
            .map(OsString::from)
            .to_vec();
        plan.set = vec![
            (
                "GINARY_ASSURANCE_SUPERVISION_CHILD".into(),
                dir.path().as_os_str().to_owned(),
            ),
            ("GINARY_ASSURANCE_DUMP_MODE".into(), mode.into()),
            (
                "GINARY_ASSURANCE_PROPAGATED".into(),
                "a value with spaces".into(),
            ),
            (
                "GINARY_ASSURANCE_REMOVED".into(),
                "must not reach child".into(),
            ),
        ];
        plan.remove.push("GINARY_ASSURANCE_REMOVED".into());
        assert_eq!(
            launch::supervise(plan, &diag, dir.path()),
            ExitCode::from(23),
            "{mode}"
        );
        let observed: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.path().join("environment.json")).unwrap())
                .unwrap();
        assert_eq!(observed["set"], "a value with spaces");
        assert!(observed["removed"].is_null());
        let statuses = events(&sink, "supervise");
        assert_eq!(statuses.len(), 1, "{mode}");
        assert_eq!(statuses[0]["kv"]["exit"], "23");
        let dumps = events(&sink, "crash_dump");
        if mode == "fresh" {
            assert_eq!(dumps.len(), 1);
            assert_eq!(
                dumps[0]["kv"]["slogan"],
                "Slogan: controlled runtime failure"
            );
        } else {
            assert!(
                dumps.is_empty(),
                "must not report a stale, absent or unreadable slogan: {mode}"
            );
        }
    }
}

#[test]
fn supervision_start_failure_has_a_nonzero_exit_and_an_execution_trace() {
    let dir = tempfile::tempdir().unwrap();
    let (diag, sink) = trace();
    let result = launch::supervise(
        bare_plan(dir.path().join("missing-runtime")),
        &diag,
        dir.path(),
    );
    assert_ne!(result, ExitCode::SUCCESS);
    assert_eq!(events(&sink, "exec").len(), 1);
    assert!(
        events(&sink, "supervise").is_empty(),
        "no child reached an exit status"
    );
}

#[cfg(windows)]
#[test]
fn supervision_saturates_a_real_windows_exit_status_larger_than_one_byte() {
    let dir = tempfile::tempdir().unwrap();
    let executable = program(dir.path(), "large-exit", &[ShimStep::Exit(513)]);
    assert_eq!(
        launch::supervise(bare_plan(executable), &Diag::disabled(), dir.path()),
        ExitCode::from(255)
    );
}

#[test]
fn bounded_runtime_distinguishes_missing_failed_successful_and_timed_out_children() {
    let dir = tempfile::tempdir().unwrap();
    let missing = launch::run_bounded(
        bare_plan(dir.path().join("missing-runtime")),
        &Diag::disabled(),
        Duration::from_secs(2),
    );
    assert!(matches!(missing, Err(RunIssue::Start { .. })));
    let failed = program(dir.path(), "failed", &[ShimStep::Exit(17)]);
    assert!(matches!(
        launch::run_bounded(bare_plan(failed), &Diag::disabled(), Duration::from_secs(2)),
        Err(RunIssue::Exit { code: 17 })
    ));
    let successful = program(
        dir.path(),
        "successful",
        &[ShimStep::RecordArgv, ShimStep::Exit(0)],
    );
    let mut plan = bare_plan(successful);
    plan.args = ["", "two words", "--literal", "日本語"]
        .map(OsString::from)
        .to_vec();
    launch::run_bounded(plan, &Diag::disabled(), Duration::from_secs(2)).unwrap();
    assert_eq!(
        common::script::recorded_argv(dir.path(), "successful"),
        ["", "two words", "--literal", "日本語"]
    );
    let sleeping = program(
        dir.path(),
        "sleeping",
        &[ShimStep::Sleep(2_000), ShimStep::Exit(0)],
    );
    let started = std::time::Instant::now();
    assert!(matches!(
        launch::run_bounded(
            bare_plan(sleeping),
            &Diag::disabled(),
            Duration::from_millis(75)
        ),
        Err(RunIssue::Timeout { .. })
    ));
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "the runtime budget is enforced"
    );
}

#[cfg(windows)]
#[test]
fn windows_heart_arguments_round_trip_through_the_real_process_argument_parser() {
    use std::os::windows::process::CommandExt as _;
    let dir = tempfile::tempdir().unwrap();
    let executable = program(
        dir.path(),
        "heart-argv",
        &[ShimStep::RecordArgv, ShimStep::Exit(0)],
    );
    let args = [
        "",
        "safe-._/@:=+,",
        "two words",
        "quote\"inside",
        "before\\\"quote",
        "space and trailing\\",
        "plain\\path",
        "日本語",
        "\t",
    ];
    let mut manifest = common::artifact::canonical_manifest();
    manifest.launch.heart = true;
    let plan = launch::plan(
        dir.path(),
        &manifest,
        &args.map(OsString::from),
        &ginary::cache::Env::from_pairs(Vec::<(OsString, OsString)>::new()),
        dir.path(),
        Path::new("probe"),
    )
    .unwrap();
    let command = plan
        .set
        .iter()
        .find(|(key, _)| key == launch::HEART_COMMAND_VAR)
        .unwrap()
        .1
        .to_str()
        .unwrap();
    // `probe` is a known simple argv[0]. The rest is consumed by Windows and
    // the child's runtime, not reimplemented by a test-side quoting parser.
    let raw_args = command
        .strip_prefix("probe ")
        .expect("heart command starts with the artifact");
    let status = std::process::Command::new(&executable)
        .raw_arg(raw_args)
        .status()
        .unwrap();
    assert!(status.success());
    assert_eq!(
        common::script::recorded_argv(dir.path(), "heart-argv"),
        args
    );
}

fn marker_file(dir: &Path, target: Target, name: &str, mut bytes: Vec<u8>) -> PathBuf {
    bytes.extend_from_slice(&Marker::for_target(&target).bytes());
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    path
}

#[test]
fn a_valid_stub_marker_cannot_disguise_a_foreign_elf_object() {
    let dir = tempfile::tempdir().unwrap();
    let linux: Target = "linux-x86_64-gnu".parse().unwrap();
    let windows: Target = "windows-x86_64".parse().unwrap();
    let cases = [
        (linux, 183, None, "no interpreter"),
        (
            linux,
            62,
            Some("/lib/ld-musl-x86_64.so.1"),
            "musl interpreter",
        ),
        (
            linux,
            183,
            Some("/lib/ld-linux-aarch64.so.1"),
            "glibc interpreter",
        ),
        (linux, 62, Some("/vendor/custom-loader"), "custom-loader"),
        (linux, 0xffff, None, "no interpreter"),
        (windows, 62, None, "ELF"),
    ];
    for (index, (target, machine, interp, detail)) in cases.into_iter().enumerate() {
        let path = marker_file(
            dir.path(),
            target,
            &format!("stub-{index}"),
            objects::program(machine, interp),
        );
        let error =
            stub::verify(&path, &target).expect_err("marker cannot override the object header");
        assert!(
            matches!(&error, StubError::ObjectMismatch { .. }),
            "{error}"
        );
        assert!(
            error.to_string().contains(detail),
            "diagnostic must describe the actual object: {error}"
        );
    }
}

#[test]
fn stub_refusals_distinguish_truncated_objects_from_non_object_content() {
    let dir = tempfile::tempdir().unwrap();
    for target_name in ["linux-x86_64-gnu", "macos-aarch64"] {
        let target = target_name.parse().unwrap();
        for (name, contents, expected) in [
            ("text", b"abdownload".as_slice(), "begins `ab`"),
            (
                "binary",
                b"\0\x01download".as_slice(),
                "begins `\\x00\\x01`",
            ),
        ] {
            let path = marker_file(dir.path(), target, name, contents.to_vec());
            let error = stub::verify(&path, &target).unwrap_err();
            assert!(matches!(&error, StubError::NotAnObject { .. }));
            assert!(error.to_string().contains(expected), "{error}");
        }
    }
    for (name, target, bytes) in [
        ("bad-elf", "linux-x86_64-gnu", objects::elf_magic_only()),
        ("bad-macho", "macos-aarch64", objects::macho_magic_only()),
        ("bad-pe", "windows-x86_64", objects::dos_stub()),
    ] {
        let target = target.parse().unwrap();
        let path = marker_file(dir.path(), target, name, bytes);
        assert!(
            matches!(
                stub::verify(&path, &target),
                Err(StubError::NotAnObject { .. })
            ),
            "{name}"
        );
    }
}

#[test]
fn an_explicit_stub_directory_is_refused_without_searching_other_sources() {
    let dir = tempfile::tempdir().unwrap();
    let options = StubOpts {
        explicit: Some(dir.path().to_owned()),
        env_dir: None,
        cache_dir: dir.path().join("cache"),
    };
    let error = stub::locate(&Target::host(), &options).unwrap_err();
    assert!(matches!(&error, StubError::NotAFile { .. }));
    assert!(error.to_string().contains("directory"));
    let missing = dir.path().join("absent");
    let error = stub::verify(&missing, &Target::host()).unwrap_err();
    assert!(
        matches!(&error, StubError::Io { what, .. } if what.contains(&missing.display().to_string()))
    );
    assert!(std::error::Error::source(&error).is_some());
}

#[test]
fn a_pe_with_an_unsupported_cpu_cannot_be_relabelled_as_a_windows_stub() {
    let dir = tempfile::tempdir().unwrap();
    let target: Target = "windows-x86_64".parse().unwrap();
    let path = dir.path().join("wrong-cpu.exe");
    std::fs::write(
        &path,
        stubfile::pe_bytes(0x014c, &Marker::for_target(&target).bytes()),
    )
    .unwrap();
    let error = stub::verify(&path, &target).unwrap_err();
    assert!(
        matches!(&error, StubError::ObjectMismatch { .. }),
        "{error}"
    );
    assert!(error.to_string().contains("no name"), "{error}");
}

#[test]
fn unsupported_native_machines_keep_their_actual_linkage_in_diagnostics() {
    let dir = tempfile::tempdir().unwrap();
    assert!(native::describe_object(dir.path()).unwrap().is_none());
    for (index, interp) in [None, Some("/vendor/custom-loader")]
        .into_iter()
        .enumerate()
    {
        let path = dir.path().join(format!("unsupported-{index}.so"));
        std::fs::write(&path, objects::shared_object(0xffff, interp)).unwrap();
        let description = native::describe_object(&path).unwrap().unwrap();
        let facts = description.facts.unwrap();
        assert!(facts.target.is_none());
        let line = native::facts_line(Some(&facts));
        assert!(
            line.contains(if interp.is_some() {
                "dynamic"
            } else {
                "static"
            }),
            "{line}"
        );
        assert!(line.contains("no target"));
    }
    assert!(matches!(
        native::inspect_object_bytes(b"ordinary configuration"),
        Err(native::ObjectError::NotAnObject)
    ));
}

fn native_artifact() -> NativeArtifact {
    NativeArtifact {
        package: "native_app".into(),
        rel_path: "native_app/priv/driver.so".into(),
        kind: NativeKind::SharedObject,
        object: None,
        size: 4,
        warning: Some("unreadable fixture".into()),
    }
}

#[test]
fn a_native_hook_with_a_missing_project_keeps_the_package_and_underlying_error() {
    let dir = tempfile::tempdir().unwrap();
    let target = Target::host();
    // Include discovery is part of constructing a real hook invocation. A
    // missing cwd makes the launch fail on both platforms, regardless of shell
    // availability; the POSIX hook executable itself is never substituted.
    for name in [
        "erl_interface-1/include",
        "erl_interface-2/include",
        "other/include",
    ] {
        std::fs::create_dir_all(dir.path().join("otp/lib").join(name)).unwrap();
    }
    let overrides = BTreeMap::new();
    let hooks = BTreeMap::from([(
        "native_app".to_owned(),
        "printf '%s' {target} {out_dir}".to_owned(),
    )]);
    let cfg = native::TargetNativeCfg {
        overrides: &overrides,
        hooks: &hooks,
    };
    let missing = dir.path().join("missing-project");
    let work = dir.path().join("work");
    let otp = dir.path().join("otp");
    let ctx = native::ReconcileCtx {
        target: &target,
        erts_nif_loading: true,
        cfg: &cfg,
        project_root: &missing,
        work_dir: &work,
        erts_root: &otp,
        erts_version: "17.0.5",
        otp_version: "29.0.5",
        allow_mismatch: false,
    };
    let error = native::reconcile(&[native_artifact()], &ctx).unwrap_err();
    assert!(
        matches!(&error, NativeError::HookProcess { package, .. } if package == "native_app"),
        "{error}"
    );
    assert!(std::error::Error::source(&error).is_some());
    assert!(native::hook_out_dir(&work, &target, "native_app").is_dir());
}

#[test]
fn a_hook_output_directory_collision_is_a_filesystem_error_before_any_child_runs() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("occupied");
    std::fs::write(&out, b"caller data").unwrap();
    let target = Target::host();
    let ctx = native::HookCtx {
        target: &target,
        out_dir: &out,
        project_root: dir.path(),
        erts_root: dir.path(),
        erts_version: "17",
        otp_version: "29",
    };
    let error = native::run_hook("native_app", "exit 0", &ctx).unwrap_err();
    assert!(matches!(error, NativeError::Io { path, .. } if path == out));
    assert_eq!(std::fs::read(out).unwrap(), b"caller data");
}

#[test]
fn an_unreadable_native_override_is_refused_with_the_artifact_identity() {
    let dir = tempfile::tempdir().unwrap();
    let target = Target::host();
    let hooks = BTreeMap::new();
    let overrides = BTreeMap::from([(
        "native_app/priv/driver.so".to_owned(),
        "replacement".to_owned(),
    )]);
    let cfg = native::TargetNativeCfg {
        overrides: &overrides,
        hooks: &hooks,
    };
    let ctx = native::ReconcileCtx {
        target: &target,
        erts_nif_loading: true,
        cfg: &cfg,
        project_root: dir.path(),
        work_dir: dir.path(),
        erts_root: dir.path(),
        erts_version: "17",
        otp_version: "29",
        allow_mismatch: false,
    };
    for contents in [b"not native code".as_slice(), b"\x7fELF".as_slice()] {
        std::fs::write(dir.path().join("replacement"), contents).unwrap();
        let error = native::reconcile(&[native_artifact()], &ctx).unwrap_err();
        assert!(
            matches!(&error, NativeError::OverrideMismatch { .. }),
            "{error}"
        );
        assert!(error.to_string().contains("driver.so"));
        assert!(error.to_string().contains("unreadable"));
    }
}
