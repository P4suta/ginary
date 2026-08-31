// SPDX-License-Identifier: MIT OR Apache-2.0
//! The launcher half of Windows support, held to what a Linux machine can
//! honestly check.
//!
//! Everything here is a pure function: where the cache goes, what the `\\?\`
//! prefix does to a path, which share mode each lock means, what exit code a
//! spawned child produces and which program a target's launch spec names. None
//! of it needs a Windows machine, and all of it is what a Windows launcher is
//! made of — so a defect in any of these rules is a defect this suite can
//! catch on the machine ginary is developed on.
//!
//! What is *not* here, and cannot be: starting `erl.exe`, the job object, the
//! console control handler and the exit code of a real runtime. Those are the
//! GitHub Actions milestone; `docs/dev/log/D2.md` says so in as many words.
//!
//! The file is not gated on the `cli` feature. Every module it reads is one a
//! launcher-only stub carries, so the stub flavor of the suite asserts these
//! rules too — which is the point, since the stub is the binary a Windows
//! artifact is made of.

mod common;

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use common::artifact::canonical_manifest;

use ginary::cache::{self, Env, LOCALAPPDATA_VAR, Origin, UNKNOWN_USER};
use ginary::cache_lock::{FILE_SHARE_DELETE, FILE_SHARE_READ, LockKind, windows_share_mode};
use ginary::launch::{self, LaunchPlan, NO_EXIT_CODE};
use ginary::manifest::Manifest;
use ginary::target::{Arch, Libc, Os, Target};
use ginary::winpath::{self, LONG_PATH_PREFIX, UNC_LONG_PATH_PREFIX};

/// A user name for the fallback root. It is never a real one: the tests are
/// about the rule, not about this machine.
const USER: &str = "ada";

/// A Windows home a cache hangs off.
const LOCAL_APP_DATA: &str = r"C:\Users\ada\AppData\Local";

/// A Windows temporary directory.
const TEMP: &str = r"C:\Users\ada\AppData\Local\Temp";

fn env(pairs: &[(&str, &str)]) -> Env {
    Env::from_pairs(
        pairs
            .iter()
            .map(|(key, value)| (OsString::from(*key), OsString::from(*value))),
    )
}

// ------------------------------------------------ the cache root --

#[test]
fn the_windows_cache_root_is_the_override_when_one_is_set() {
    let resolved = cache::resolve_windows(
        &env(&[
            ("GINARY_CACHE_DIR", r"D:\ginary"),
            (LOCALAPPDATA_VAR, LOCAL_APP_DATA),
        ]),
        USER,
    );
    assert_eq!(
        resolved.root,
        PathBuf::from(r"D:\ginary"),
        "GINARY_CACHE_DIR is used verbatim, with no `ginary` appended"
    );
    assert_eq!(resolved.origin, Origin::GinaryCacheDir);
    assert!(
        !resolved.is_fallback,
        "an override is a first choice, not a fallback"
    );
}

#[test]
fn the_windows_cache_root_is_ginary_under_local_app_data() {
    let resolved = cache::resolve_windows(&env(&[(LOCALAPPDATA_VAR, LOCAL_APP_DATA)]), USER);
    assert_eq!(
        resolved.root,
        Path::new(LOCAL_APP_DATA).join(cache::DIR_NAME),
        "%LOCALAPPDATA% is the Windows XDG_CACHE_HOME, with `ginary` appended"
    );
    assert_eq!(resolved.origin, Origin::LocalAppData);
    assert!(!resolved.is_fallback);
}

#[test]
fn the_windows_cache_root_falls_back_to_temp_named_after_the_user() {
    let resolved = cache::resolve_windows(&env(&[("TEMP", TEMP)]), USER);
    assert_eq!(
        resolved.root,
        Path::new(TEMP).join("ginary-ada"),
        "a temporary directory is shared, so the user is in the name"
    );
    assert_eq!(resolved.origin, Origin::WindowsFallback);
    assert!(
        resolved.is_fallback,
        "the caller has to know this root is the fallback: it warns about it"
    );
}

#[test]
fn an_empty_windows_cache_variable_counts_as_unset() {
    let over_local = cache::resolve_windows(
        &env(&[("GINARY_CACHE_DIR", ""), (LOCALAPPDATA_VAR, LOCAL_APP_DATA)]),
        USER,
    );
    assert_eq!(
        over_local.origin,
        Origin::LocalAppData,
        "an exported-but-empty override is a shell accident, not a root of `\\`"
    );

    let over_temp = cache::resolve_windows(&env(&[(LOCALAPPDATA_VAR, ""), ("TEMP", TEMP)]), USER);
    assert_eq!(over_temp.origin, Origin::WindowsFallback);
}

#[test]
fn the_windows_fallback_prefers_temp_then_tmp_then_the_machine_directory() {
    assert_eq!(
        cache::windows_fallback_root(&env(&[("TEMP", TEMP), ("TMP", r"C:\other")]), USER),
        Path::new(TEMP).join("ginary-ada"),
        "%TEMP% is the modern spelling and wins"
    );
    assert_eq!(
        cache::windows_fallback_root(&env(&[("TMP", r"C:\other")]), USER),
        Path::new(r"C:\other").join("ginary-ada"),
        "%TMP% is read when %TEMP% is not set"
    );
    assert_eq!(
        cache::windows_fallback_root(&env(&[]), USER),
        Path::new(cache::WINDOWS_DEFAULT_TEMP).join("ginary-ada"),
        "a process with neither is a scrubbed environment, not a reason to give up"
    );
}

#[test]
fn the_windows_user_name_comes_from_username_and_is_unknown_otherwise() {
    assert_eq!(cache::current_user(&env(&[("USERNAME", "ada")])), "ada");
    assert_eq!(
        cache::current_user(&env(&[("USERNAME", "")])),
        UNKNOWN_USER,
        "an empty value is not a user name"
    );
    assert_eq!(
        cache::current_user(&env(&[])),
        UNKNOWN_USER,
        "a service account may have no %USERNAME% at all"
    );
    assert_eq!(
        cache::current_user(&env(&[("USERNAME", r"..\..\public")])),
        UNKNOWN_USER,
        "the name is joined onto a temporary directory, so it has to be one component"
    );
}

#[test]
fn the_windows_cache_provenance_says_which_rule_produced_the_root() {
    let cases: [(&str, Env); 5] = [
        (
            "GINARY_CACHE_DIR set",
            env(&[("GINARY_CACHE_DIR", r"D:\ginary")]),
        ),
        (
            "GINARY_CACHE_DIR empty, LOCALAPPDATA set",
            env(&[("GINARY_CACHE_DIR", ""), (LOCALAPPDATA_VAR, LOCAL_APP_DATA)]),
        ),
        (
            "LOCALAPPDATA set",
            env(&[(LOCALAPPDATA_VAR, LOCAL_APP_DATA)]),
        ),
        (
            "LOCALAPPDATA empty, TEMP set",
            env(&[(LOCALAPPDATA_VAR, ""), ("TEMP", TEMP)]),
        ),
        ("nothing set", env(&[])),
    ];

    // The roots themselves are not in the table on purpose: they are built
    // with `Path::join`, so their separator is the separator of whatever
    // machine ran the test, and a snapshot of them would pin the machine
    // rather than the rule. The provenance is what a user reads out of
    // `ginary cache dir`, and it is the same text everywhere.
    let mut table = String::new();
    for (name, env) in cases {
        let resolved = cache::resolve_windows(&env, USER);
        table.push_str(&format!(
            "{name}\n  provenance: {}\n  fallback: {}\n",
            resolved.origin.describe(),
            resolved.is_fallback
        ));
    }
    insta::assert_snapshot!("windows_cache_provenance", table);
}

// ------------------------------------------------ the long-path prefix --

#[test]
fn a_drive_absolute_path_gets_the_long_path_prefix() {
    assert_eq!(
        winpath::long_path_str(r"C:\Users\ada\AppData\Local\ginary"),
        r"\\?\C:\Users\ada\AppData\Local\ginary"
    );
    assert!(
        winpath::long_path_str(r"C:\a").starts_with(LONG_PATH_PREFIX),
        "the prefix constant is the one the helper applies"
    );
}

#[test]
fn forward_slashes_become_backslashes_before_the_prefix_is_added() {
    assert_eq!(
        winpath::long_path_str("C:/Users/ada/ginary"),
        r"\\?\C:\Users\ada\ginary",
        "a verbatim path is not normalised, so a slash left in it would be part of a name"
    );
}

#[test]
fn a_unc_path_gets_the_unc_long_path_prefix() {
    assert_eq!(
        winpath::long_path_str(r"\\server\share\ginary"),
        r"\\?\UNC\server\share\ginary"
    );
    assert!(
        winpath::long_path_str(r"\\server\share").starts_with(UNC_LONG_PATH_PREFIX),
        "the UNC prefix constant is the one the helper applies"
    );
}

#[test]
fn a_path_that_already_carries_the_prefix_is_left_alone() {
    assert_eq!(
        winpath::long_path_str(r"\\?\C:\ginary"),
        r"\\?\C:\ginary",
        "applying the prefix twice would name a directory called `?`"
    );
    assert_eq!(
        winpath::long_path_str(r"\\?\UNC\server\share"),
        r"\\?\UNC\server\share"
    );
}

#[test]
fn a_relative_path_is_left_alone_because_the_prefix_needs_a_full_path() {
    assert_eq!(
        winpath::long_path_str(r"lib\kernel-11.0.3\ebin"),
        r"lib\kernel-11.0.3\ebin",
        "`\\\\?\\lib` names a device called `lib`, not a directory here"
    );
    assert_eq!(
        winpath::long_path_str(""),
        "",
        "an empty path is not a path to prefix"
    );
}

#[cfg(unix)]
#[test]
fn long_path_is_the_identity_on_unix() {
    let path = Path::new("/cache/ginary/hello/0123456789abcdef");
    let answered = winpath::long_path(path);
    assert_eq!(
        answered.as_ref(),
        path,
        "the helper exists so the call sites are one code path, not two"
    );
    assert!(
        matches!(answered, std::borrow::Cow::Borrowed(_)),
        "the unix answer is the argument itself: nothing is copied on a path the cache joins \
         thousands of times"
    );
}

// ------------------------------------------------ the ordinary spelling --

#[test]
fn a_verbatim_path_loses_its_prefix_and_an_ordinary_one_is_untouched() {
    let table: [(&str, &str); 5] = [
        (r"\\?\C:\a\b", r"C:\a\b"),
        (r"\\?\UNC\srv\share\a", r"\\srv\share\a"),
        (r"C:\a\b", r"C:\a\b"),
        ("/home/ada/.cache/ginary", "/home/ada/.cache/ginary"),
        ("", ""),
    ];
    for (given, expected) in table {
        assert_eq!(
            winpath::plain_path_str(given),
            expected,
            "`{given}` is what ginary opens; `{expected}` is what `erl.exe` is handed"
        );
    }
}

#[test]
fn a_verbatim_device_path_keeps_its_prefix_because_it_has_no_other_spelling() {
    for device in [r"\\?\Volume{9f4b}\a", r"\\?\GLOBALROOT\Device\Harddisk0"] {
        assert_eq!(
            winpath::plain_path_str(device),
            device,
            "removing the prefix here would not shorten the path, it would name another object"
        );
    }
}

#[test]
fn the_two_prefix_rules_are_each_other_s_inverse() {
    for path in [r"C:\Users\ada\AppData\Local\ginary", r"\\srv\share\ginary"] {
        assert_eq!(
            winpath::plain_path_str(&winpath::long_path_str(path)),
            path,
            "what the extraction writes under and what the runtime is handed are one path \
             spelled two ways"
        );
    }
}

#[test]
fn plain_path_borrows_the_path_it_leaves_alone() {
    let path = Path::new("/cache/ginary/hello/0123456789abcdef");
    let answered = winpath::plain_path(path);
    assert_eq!(answered.as_ref(), path);
    assert!(
        matches!(answered, std::borrow::Cow::Borrowed(_)),
        "no unix path begins `\\\\?\\`, so the rule is compiled everywhere and copies nothing \
         here"
    );
}

// ------------------------------------------------ the exit code --

#[test]
fn a_windows_child_exit_code_becomes_the_launchers_own() {
    let table: [(Option<i32>, u8); 7] = [
        (Some(0), 0),
        (Some(3), 3),
        (Some(7), 7),
        (Some(255), 255),
        // A code that does not fit is `u8::MAX`, never its low byte: a
        // runtime that exited 256 did not exit 0.
        (Some(256), 255),
        // What a Windows access violation looks like as an `i32`.
        (Some(-1_073_741_819), 255),
        (None, NO_EXIT_CODE),
    ];
    for (code, expected) in table {
        assert_eq!(
            launch::windows_exit_code(code),
            expected,
            "a child that ended {code:?} makes the launcher exit {expected}"
        );
    }
    assert_eq!(
        NO_EXIT_CODE, 1,
        "a child with no code of its own is a failure, and 1 is how a parent says so"
    );
}

// ------------------------------------------------ the lock --

#[test]
fn the_two_windows_share_modes_are_what_the_locks_mean_there() {
    assert_eq!(
        windows_share_mode(LockKind::Shared),
        FILE_SHARE_READ,
        "a runtime holds its entry open for writing and lets readers in, which is what makes a \
         second runtime's shared lock succeed and a prune's exclusive open fail"
    );
    assert_eq!(
        windows_share_mode(LockKind::Exclusive),
        FILE_SHARE_DELETE,
        "a prune shares no reading and no writing — an entry it can open is an entry nobody is \
         running out of — and shares deletion, because its own next step is to rename the entry \
         directory this handle is open inside, which Windows refuses otherwise"
    );
    assert_eq!(
        windows_share_mode(LockKind::Exclusive) & FILE_SHARE_READ,
        0,
        "and a runtime's shared handle is still what refuses it"
    );
    assert_eq!(
        (FILE_SHARE_READ, FILE_SHARE_DELETE),
        (1, 4),
        "the Win32 values, not names for a guess"
    );
}

// ------------------------------------------------ the launch program --

#[test]
fn every_target_names_the_program_its_runtime_is_started_with() {
    for target in ginary::target::ALL {
        let expected = if target.os == Os::Windows {
            "erl.exe"
        } else {
            "erlexec"
        };
        assert_eq!(
            target.launch_program(),
            expected,
            "{} starts its runtime with {expected}",
            target.name()
        );
    }
}

#[test]
fn a_windows_launch_plan_is_the_unix_one_with_erl_exe_in_front() {
    let root = Path::new("/cache/hello/0123456789abcdef");
    let dumps = Path::new("/cache/hello");
    let exe = Path::new("/opt/bin/hello");
    let env = env(&[]);
    let args: Vec<OsString> = vec![OsString::from("--name"), OsString::from("world")];

    let unix = canonical_manifest();
    let mut windows = unix.clone();
    windows.target = Target::new(Os::Windows, Arch::X86_64, Libc::None);
    windows.launch.program = windows.target.launch_program().to_owned();

    let plan_of = |m: &Manifest| -> LaunchPlan {
        match launch::plan(root, m, &args, &env, dumps, exe) {
            Ok(plan) => plan,
            Err(error) => panic!("the manifest must produce a plan: {error}"),
        }
    };
    let unix_plan = plan_of(&unix);
    let windows_plan = plan_of(&windows);

    assert_eq!(
        windows_plan.program.file_name(),
        Some(std::ffi::OsStr::new("erl.exe")),
        "the Windows runtime has no `erlexec`: `erl.exe` locates its own bindir and root"
    );
    assert_eq!(
        unix_plan.program.file_name(),
        Some(std::ffi::OsStr::new("erlexec"))
    );
    assert_eq!(
        windows_plan.program.parent(),
        unix_plan.program.parent(),
        "both live in the manifest's bindir; only the name differs"
    );
    assert_eq!(
        windows_plan.args, unix_plan.args,
        "the argument vector is the same on both platforms, which is what makes one plan \
         function serve both launchers"
    );
    assert_eq!(windows_plan.set, unix_plan.set);
    assert_eq!(windows_plan.remove, unix_plan.remove);
}
