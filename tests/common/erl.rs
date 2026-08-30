// SPDX-License-Identifier: MIT OR Apache-2.0
//! Running a staged root the way the launcher will.
//!
//! [`run_staged`] is a *hermetic subset* of the launch contract ADR 0003
//! records, and the difference matters enough to state first. The ADR describes
//! a launcher that inherits the user's environment and scrubs a denylist from
//! it — `ERL_LIBS`, `ERL_FLAGS`, `ERL_AFLAGS`, `ERL_ZFLAGS`, `ERL_OTP*_FLAGS`,
//! `ERL_ROOTDIR`, `ERL_EPMD_PORT` — and that sets `HOME` and `ERL_CRASH_DUMP`
//! only when the user has not. This function clears the environment instead and
//! sets both unconditionally, because a test that inherited the developer's
//! environment would assert on the machine it ran on. It also leaves out the
//! ADR's optional `+fnu`, `-args_file`, `-config` and flag-passing arguments,
//! which no staged tree carries yet.
//!
//! What it does pin is everything the two agree on: ginary does not ship the
//! `erl` shell script, does not shell out, and execs `erts-<vsn>/bin/erlexec`
//! directly with an argument vector that ends in `-extra`. When `src/launch.rs`
//! lands in A3, a difference between its `LaunchPlan` and this function *inside
//! that overlap* is a defect in one of the two, and `tests/stage_run.rs` — which
//! actually boots what `stage` wrote — is what says which. The denylist and the
//! "only when the user has not set it" rules are outside the overlap and need
//! their own tests over an inherited environment; a launch plan that agrees
//! with this function is not thereby finished.
//!
//! The five environment variables are the whole of what is set here. `erlexec` re-execs
//! `$BINDIR/$EMU` and refuses to start without `ROOTDIR` and `BINDIR`;
//! `PROGNAME` is what `init:get_argument(progname)` answers; `HOME` is where
//! the runtime writes anything it decides to write, and `ERL_CRASH_DUMP` points
//! inside it so that a crashing application never drops a dump into the
//! directory the user happened to be standing in. `PATH` is an empty directory
//! rather than absent, because an unset `PATH` makes a program search a
//! system-defined default rather than nothing at all.
//!
//! `+B` disables the break handler, `-noshell` keeps the runtime off the
//! terminal, and `-start_epmd false` stops the runtime spawning the port mapper
//! daemon: a packaged command line application is not a distributed node, and a
//! stray `epmd` outliving it would be a surprise.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::Duration;

use crate::common::bounded::run_bounded;

/// How long a staged `hello_ffi` gets to start, run and halt.
///
/// Generous next to the tenth of a second the fixture actually takes, and
/// finite, which is the point: a runtime that fails to halt is a defect this
/// helper must report rather than a hang the whole suite waits out.
pub const RUN_BUDGET: Duration = Duration::from_secs(60);

/// The directory [`run_staged`] runs the application in.
///
/// A fresh empty directory under `home`, so a test can assert on what the run
/// left behind — in particular that a crash wrote no `erl_crash.dump` here.
/// Passing a different `home` gives a different working directory, which is how
/// a test proves that running a staged root twice changes nothing in it.
pub fn run_cwd(home: &Path) -> PathBuf {
    home.join("cwd")
}

/// The empty directory [`run_staged`] passes as `PATH`.
pub fn empty_path_dir(home: &Path) -> PathBuf {
    home.join("empty-path")
}

/// Where a crash dump would land, if the runtime wrote one at all.
pub fn crash_dump_path(home: &Path) -> PathBuf {
    home.join("erl_crash.dump")
}

/// Launches `app` out of the staged root at `root`, exactly as ADR 0003 says.
///
/// `args` are the application's own arguments: they arrive after `-extra` and
/// come back out of `init:get_plain_arguments/0` unchanged. `home` is a
/// directory the test owns; the working directory, the `PATH` directory and
/// the crash dump path are all derived from it by the three functions above.
///
/// # Panics
///
/// If `root` holds no `erts-<vsn>` directory or more than one, if the working
/// directory cannot be created, if `erlexec` cannot be spawned, or if it does
/// not exit within [`RUN_BUDGET`]. None of those is a property of the
/// application under test.
pub fn run_staged(root: &Path, app: &str, args: &[&str], home: &Path) -> Output {
    let erts_bin = root.join(format!("erts-{}", erts_vsn(root))).join("bin");
    let cwd = run_cwd(home);
    let path_dir = empty_path_dir(home);
    for dir in [&cwd, &path_dir] {
        std::fs::create_dir_all(dir)
            .unwrap_or_else(|error| panic!("cannot create {}: {error}", dir.display()));
    }

    let mut command = Command::new(erts_bin.join("erlexec"));
    command
        .env_clear()
        .env("ROOTDIR", root)
        .env("BINDIR", &erts_bin)
        .env("EMU", "beam")
        .env("PROGNAME", app)
        .env("HOME", home)
        .env("PATH", &path_dir)
        .env("ERL_CRASH_DUMP", crash_dump_path(home))
        .current_dir(&cwd)
        .arg("-boot")
        .arg(root.join("bin/no_dot_erlang"))
        .args(["-noshell", "+B", "-start_epmd", "false"]);

    for ebin in code_path(root, app) {
        command.arg("-pa").arg(ebin);
    }

    command
        .arg("-eval")
        .arg(format!("'{app}@@main':run('{app}')"))
        .arg("-extra")
        .args(args);

    run_bounded(
        &mut command,
        RUN_BUDGET,
        &format!("the staged `{app}` under {}", erts_bin.display()),
    )
}

/// The `-pa` directories: `app` first, then every other shipment application.
///
/// A shipment application is staged as `lib/<name>`, an OTP one as
/// `lib/<name>-<vsn>`, and only the first kind needs a `-pa`: the boot file
/// already puts `kernel` and `stdlib` on the path, and the rest of the OTP
/// library is found by `code:lib_dir` under `$ROOTDIR`. The order is `app`
/// first and then the others sorted, so the vector is reproducible.
fn code_path(root: &Path, app: &str) -> Vec<PathBuf> {
    let lib = root.join("lib");
    let mut names: Vec<String> = std::fs::read_dir(&lib)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", lib.display()))
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| entry.file_name().to_str().map(str::to_owned))
        .filter(|name| !is_versioned(name))
        .filter(|name| name != app)
        .collect();
    names.sort();

    std::iter::once(app.to_owned())
        .chain(names)
        .map(|name| lib.join(name).join("ebin"))
        .collect()
}

/// Whether a `lib/` entry is an OTP `<name>-<vsn>` directory.
fn is_versioned(name: &str) -> bool {
    name.rsplit_once('-').is_some_and(|(stem, vsn)| {
        !stem.is_empty()
            && !vsn.is_empty()
            && vsn
                .split('.')
                .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
    })
}

/// The ERTS version of a staged root, from its one `erts-<vsn>` directory.
///
/// # Panics
///
/// If there is not exactly one. A staged root with none cannot be launched and
/// one with two is ambiguous; either way the test has nothing to run.
pub fn erts_vsn(root: &Path) -> String {
    let mut found: Vec<String> = std::fs::read_dir(root)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", root.display()))
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| entry.file_name().to_str().map(str::to_owned))
        .filter_map(|name| name.strip_prefix("erts-").map(str::to_owned))
        .collect();
    found.sort();
    match found.as_slice() {
        [only] => only.clone(),
        other => panic!(
            "expected exactly one `erts-*` directory under {}, found {other:?}",
            root.display()
        ),
    }
}
