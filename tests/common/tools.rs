// SPDX-License-Identifier: MIT OR Apache-2.0
//! Gating a test on the external programs it needs.
//!
//! A test that needs `erl` cannot run on a machine without Erlang, and a test
//! that quietly passes on such a machine is worse than one that does not run at
//! all. [`require_tools`] makes the choice explicit: it skips, loudly, unless
//! [`REQUIRE_VAR`] says the toolchain is supposed to be there, and then it
//! fails instead.
//!
//! [`REQUIRE_VAR`] is a claim about *the toolchain an artifact is built with*:
//! `gleam`, `erl`, `strip`, `docker`. It is not a claim about every program a
//! test could want, and reading it as one is how `actionlint` — a lint, wanted
//! by one job and installed by no runner — failed three CI jobs that had a
//! complete toolchain. So this module holds a second gate,
//! [`require_actionlint`], with a variable of its own, exactly as E6 split
//! `GINARY_REQUIRE_STUBS` out for the cross-built stubs. The rule for adding a
//! third is the rule these two follow: a gate is a claim somebody has to be
//! able to *make true*, so it belongs to whichever job installs the thing.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The variable that turns a skip into a failure.
///
/// CI sets it on the job that installs Erlang and Gleam, so a broken toolchain
/// there cannot look like a green run.
pub const REQUIRE_VAR: &str = "GINARY_REQUIRE_TOOLCHAIN";

/// The programs a gated test asked for, with the path each was found at.
#[derive(Clone, Debug)]
pub struct Toolchain {
    /// Program name to the absolute path `PATH` resolved it to.
    tools: BTreeMap<String, PathBuf>,
}

impl Toolchain {
    /// The path of a program the test asked for.
    ///
    /// # Panics
    ///
    /// If `name` was not in the list passed to [`require_tools`]. That is a bug
    /// in the test, not a property of the machine.
    pub fn path(&self, name: &str) -> &Path {
        match self.tools.get(name) {
            Some(path) => path,
            None => panic!("`{name}` was not requested from require_tools"),
        }
    }
}

/// Finds every named program on `PATH`, or reports a skip.
///
/// Returns `Some(Toolchain)` when all of them are present. When one is missing
/// it prints `skipping: <tool> not on PATH` on standard error and returns
/// `None`, so the caller returns and the test passes without pretending to have
/// covered anything — unless `GINARY_REQUIRE_TOOLCHAIN=1`, in which case the
/// missing program is a panic.
///
/// # Panics
///
/// If a program is missing and [`REQUIRE_VAR`] is `1`.
pub fn require_tools(names: &[&str]) -> Option<Toolchain> {
    let path_var = std::env::var_os("PATH");
    let mut tools = BTreeMap::new();

    for name in names {
        match ginary::process::find_in_path(name, path_var.as_deref()) {
            Some(path) => {
                tools.insert((*name).to_owned(), path);
            }
            None => {
                let required = std::env::var_os(REQUIRE_VAR).is_some_and(|value| value == "1");
                assert!(
                    !required,
                    "`{name}` is not on PATH and {REQUIRE_VAR}=1 forbids skipping"
                );
                eprintln!("skipping: {name} not on PATH");
                return None;
            }
        }
    }

    Some(Toolchain { tools })
}

/// Reports whether this machine has the POSIX shell a test needs, or prints a
/// skip naming it.
///
/// Two tests run a shell script rather than reasoning about one, which is the
/// right way round in both cases: `c4_a_hook_token_was_pasted_unquoted` runs
/// a build hook to prove that a directory holding a space arrives as one
/// argument, and `the_notice_script_exits_zero_under_the_shell_that_runs_it`
/// runs the Release workflow's notice block to prove it exits zero. Neither
/// can run where there is no such shell, and on the Windows runner both said
/// so in their own way:
///
/// ```text
/// the hook runs: HookProcess { package: "esqlite", source: Spawn {
///   program: "/bin/sh", source: Os { code: 3, kind: NotFound } } }
///
/// the notice script exits Some(1) under `bash -e -o pipefail`, so the
/// Release workflow of a repository with no credentials is red. stderr:
/// ```
///
/// (`Windows build and exit-code propagation`
/// <https://github.com/P4suta/ginary/actions/runs/33751715516/job/100636537290>.)
///
/// The shell asked about is [`POSIX_SHELL`], by absolute path, because that is
/// the program the hook rule names — see
/// `tests/regressions/c4_the_hook_shell_was_cmd_on_a_windows_host.rs` for why
/// it is `/bin/sh` on every host. A name looked up on `PATH` would not do:
/// `bash` resolves on a Windows runner to `C:\Windows\System32\bash.exe`, the
/// Windows Subsystem for Linux launcher, which exits `1` with nothing on
/// either stream when no distribution is installed. That is what the notice
/// test actually ran.
///
/// This is a genuine platform inability rather than a gate being lowered: the
/// claim is about what a POSIX shell does with a line, and a machine with no
/// POSIX shell cannot answer it. The skip is printed and, like
/// [`require_tools`], is a panic under [`REQUIRE_VAR`].
///
/// # Panics
///
/// If there is no such shell and [`REQUIRE_VAR`] is `1`.
pub fn require_posix_shell() -> Option<&'static Path> {
    let shell = Path::new(POSIX_SHELL);
    if shell.is_file() {
        return Some(shell);
    }
    let required = std::env::var_os(REQUIRE_VAR).is_some_and(|value| value == "1");
    assert!(
        !required,
        "there is no `{}` on this machine and {REQUIRE_VAR}=1 forbids skipping",
        shell.display()
    );
    eprintln!("skipping: no POSIX shell at {}", shell.display());
    None
}

/// The `strip` a test can hand a real ELF to, or a reported skip naming why
/// this machine has none.
///
/// Two conditions, and the second is the one [`require_tools`] cannot state.
/// `strip` has to be on `PATH`, which is the ordinary gate; and the host's own
/// executables have to *be* ELF files, because the fixture every ELF-stripping
/// test plants is a real binary this machine wrote — the running test binary,
/// or the host's own `libc.so.6` — and [`ginary::strip`]'s ELF phase reads
/// what a linker put there rather than a header written by hand.
///
/// On a Windows runner the first condition holds and the second does not: the
/// image carries a GNU `strip`, so the gate opened, and the file the test then
/// planted where `beam.smp` goes was a PE:
///
/// ```text
/// ---- a_native_binary_in_the_staged_tree_is_stripped_and_stays_the_same_machine ----
/// thread '…' panicked at tests\strip.rs:718:53:
/// the copy is an ELF file: NotElf
/// ```
///
/// (`Windows build and exit-code propagation`
/// <https://github.com/P4suta/ginary/actions/runs/33751715516/job/100636537290>.)
///
/// This is a genuine platform inability and not a gate lowered. The claim is
/// about what `strip --strip-all` does to an ELF, and a machine that writes no
/// ELF cannot make it. What such a machine *can* be held to is the other half
/// of the same rule — that a tree of objects the stripper cannot read is a
/// reported skip rather than silence — and
/// `tests/regressions/e11_a_tree_of_objects_the_stripper_cannot_read_was_silent.rs`
/// is where that is asserted, on every host at once.
///
/// Unlike [`require_tools`] there is no escalation under [`REQUIRE_VAR`]: that
/// variable is a claim somebody can make true by installing something, and
/// nobody can install being a platform whose objects are ELF.
#[cfg(feature = "cli")]
pub fn require_elf_stripper() -> Option<Toolchain> {
    use ginary::platform::{HOST, ObjectFormat, object_format};

    let format = object_format(HOST);
    if format != ObjectFormat::Elf {
        eprintln!(
            "skipping: this host's own objects are {}, and the ELF stripper reads ELF",
            format.as_str()
        );
        return None;
    }
    require_tools(&["strip"])
}

/// The POSIX shell [`require_posix_shell`] probes for.
///
/// The same string as [`ginary::native::HOOK_SHELL`], spelled here because
/// that constant lives in a `cli`-only module and one of the two tests this
/// gate serves — `the_notice_script_exits_zero_under_the_shell_that_runs_it`
/// — is compiled into both feature flavours. The two are held equal by
/// `tests/regressions/e11_a_shell_script_test_ran_on_a_host_with_no_posix_shell.rs`,
/// which asserts that what this gate answers *is* the program the hook rule
/// names, so a change to one and not the other fails a test rather than
/// drifting.
pub const POSIX_SHELL: &str = "/bin/sh";

/// The variable that says `actionlint` is supposed to be on this machine, so
/// a missing one is a failure rather than a skip.
///
/// Deliberately *not* [`REQUIRE_VAR`]. `GINARY_REQUIRE_TOOLCHAIN` says the
/// toolchain a runtime is packaged with is installed — `gleam`, `erl`,
/// `strip`, `docker` — and `actionlint` is none of those: it is a lint over
/// the workflow files, it has nothing to do with whether a runtime can be
/// packaged, and no hosted runner ships it. Three jobs set the toolchain
/// variable and ran the `regressions` target; all three panicked on a machine
/// whose toolchain was complete. See
/// `tests/regressions/e7_actionlint_was_required_of_every_toolchain_job.rs`.
///
/// Exactly one job sets this one: `lint` in `.github/workflows/ci.yml`, which
/// installs the tool and selects the gated test by name. A gate no job can
/// satisfy is a test that never runs; a gate every job claims is a job that
/// fails for a tool it was never given.
pub const REQUIRE_ACTIONLINT_VAR: &str = "GINARY_REQUIRE_ACTIONLINT";

/// The `actionlint` binary, or a reported skip.
///
/// The same rule [`require_tools`] follows, against [`REQUIRE_ACTIONLINT_VAR`]
/// rather than [`REQUIRE_VAR`]: `Some(path)` when the program is on `PATH`, a
/// printed `skipping:` and `None` when it is not, and a panic when it is not
/// and the caller's job promised it.
///
/// # Panics
///
/// If `actionlint` is missing and [`REQUIRE_ACTIONLINT_VAR`] is `1`.
pub fn require_actionlint() -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH");
    match ginary::process::find_in_path(ACTIONLINT, path_var.as_deref()) {
        Some(path) => Some(path),
        None => {
            let required =
                std::env::var_os(REQUIRE_ACTIONLINT_VAR).is_some_and(|value| value == "1");
            assert!(
                !required,
                "`{ACTIONLINT}` is not on PATH and {REQUIRE_ACTIONLINT_VAR}=1 forbids skipping. \
                 The job that sets it is the job that installs the tool"
            );
            eprintln!("skipping: {ACTIONLINT} not on PATH");
            None
        }
    }
}

/// The program [`require_actionlint`] looks for.
pub const ACTIONLINT: &str = "actionlint";
