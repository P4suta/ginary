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
use std::process::{Command, ExitStatus, Stdio};
use std::time::Duration;

use crate::common::bounded::{run_bounded, wait_bounded};
use crate::common::hostpath::names_the_same_directory;

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

/// Whether [`REQUIRE_VAR`] forbids standing down.
///
/// One reading of the variable, so that every gate in this module — and the
/// fixtures that build a positive control for one — answers the same question
/// the same way. `1` and nothing else: a variable set to `0` or to `false` is
/// not a promise of a toolchain.
pub fn toolchain_required() -> bool {
    std::env::var_os(REQUIRE_VAR).is_some_and(|value| value == "1")
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
                let required = toolchain_required();
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
    let required = toolchain_required();
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

/// The PowerShell a `shell: pwsh` step runs under, or a reported skip.
///
/// E15's rule is about what PowerShell does with the exit code a step leaves
/// behind, and the only honest way to state such a rule is to measure it: the
/// Windows job's exit-code probe asserted `halt(3)` correctly, left `3` in
/// `$LASTEXITCODE`, and was then failed by the `exit $LASTEXITCODE` GitHub
/// appends to every `pwsh` step — a shape that fails identically under the
/// `pwsh` on a Linux machine. See
/// `tests/regressions/e15_a_pwsh_step_ended_with_the_code_it_asserted.rs`.
///
/// The probe *runs* the program rather than merely finding it, which
/// [`require_tools`] would. A name on `PATH` can be a version-manager shim
/// with no version selected: on the machine E15 was written on,
/// `~/.local/share/mise/shims/pwsh` is first on `PATH`, exits non-zero and
/// prints `mise ERROR No version is set for shim: pwsh`. That is not a
/// PowerShell, and a test that ran it would report a defect in this
/// repository for a fact about somebody's shims.
///
/// There is no escalation under [`REQUIRE_VAR`], for [`require_elf_stripper`]'s
/// reason: `pwsh` is not part of the toolchain an artifact is built with, and
/// the hosted runners that have one — `ubuntu-24.04` and `windows-2022` both
/// ship it — run the measurement without being told to.
///
/// The probe runs under [`PWSH_BUDGET`] through
/// [`run_bounded`](crate::common::bounded::run_bounded), like every other
/// child this suite starts: a `pwsh` that stops for a prompt — a profile
/// asking something, a module autoload — would otherwise hang the whole test
/// binary, and the gate that exists to keep a broken PowerShell from failing
/// this repository would be the thing that hung it.
pub fn require_working_pwsh() -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH");
    let Some(pwsh) = ginary::process::find_in_path(PWSH, path_var.as_deref()) else {
        eprintln!("skipping: {PWSH} not on PATH");
        return None;
    };
    let mut command = std::process::Command::new(&pwsh);
    command.args(["-NoProfile", "-Command", "exit 0"]);
    let output = run_bounded(&mut command, PWSH_BUDGET, "the pwsh health check");
    if output.status.success() {
        return Some(pwsh);
    }
    eprintln!(
        "skipping: `{} -NoProfile -Command \"exit 0\"` answered {}: {}",
        pwsh.display(),
        output.status,
        String::from_utf8_lossy(&output.stderr).trim()
    );
    None
}

/// The program [`require_working_pwsh`] looks for.
pub const PWSH: &str = "pwsh";

/// How long a `pwsh` this suite starts gets to finish.
///
/// Both children are seconds of work — `exit 0`, and a two-line script whose
/// slowest statement exits 3 — so this is the interpreter's own start-up on a
/// loaded runner with room to spare, and it is a deadline rather than an
/// expectation.
pub const PWSH_BUDGET: Duration = Duration::from_secs(60);

// ------------------------------------------------------ the work tree --

/// What one probe of one directory found.
///
/// Five answers rather than two, because "no answer" has four shapes and they
/// are four different things to report: there is no `git` to ask, there is a
/// `git` that will not run, there is a `git` that ran and *refused to look*,
/// and there is a `git` that ran and said no. Collapsing any of them into the
/// last is how a broken program, or a machine that will not let this uid read
/// this repository, comes to be reported as a directory that is not a
/// checkout — a confident diagnosis of the wrong thing, and one that points
/// the reader at `cargo mutants` for a line in `~/.gitconfig`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkTreeProbe {
    /// There is no `git` on `PATH`, so nothing was asked.
    NoGit,
    /// A `git` was found on `PATH` and could not be started at all.
    GitUnrunnable,
    /// `git` ran, failed, and did not say the directory belongs to no
    /// repository: it refused to look. The `safe.directory` ownership refusal
    /// (`fatal: detected dubious ownership in repository at ...`, git 2.35.2
    /// and later) and a broken global config (`fatal: bad config line ...`)
    /// both land here, and both reach every `git` child alike.
    GitRefused,
    /// `git` ran and did not name the directory as the top of its own work
    /// tree.
    NotItsOwnWorkTree,
    /// `git` ran and named the directory as the top of its own work tree.
    OwnWorkTree,
}

/// What the git work-tree gate decides, from what it found.
///
/// The decision is a pure function of the probe and one variable rather than
/// of this machine, so the rule the gate follows can be asserted row by row
/// without a test mutating the environment out from under its neighbours. See
/// [`work_tree_gate`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkTreeGate {
    /// `git` is on `PATH` and the directory is the top of its own work tree:
    /// the test runs and makes its claim.
    Open,
    /// There is no `git` on `PATH`: a reported skip, exactly as
    /// [`require_tools`] gives one.
    SkipNoGit,
    /// There is no `git` on `PATH` and [`REQUIRE_VAR`] is `1`: a failure,
    /// exactly as [`require_tools`] gives one.
    FailNoGit,
    /// The `git` on `PATH` could not be run: a reported skip that names the
    /// program and the error, rather than the directory.
    SkipGitUnrunnable,
    /// The `git` on `PATH` could not be run and [`REQUIRE_VAR`] is `1`: a
    /// failure, for [`FailNoGit`](Self::FailNoGit)'s reason.
    FailGitUnrunnable,
    /// The `git` on `PATH` ran and refused to look: a reported skip that
    /// quotes what it said, rather than one that names the directory.
    SkipGitRefused,
    /// The `git` on `PATH` ran and refused to look, and [`REQUIRE_VAR`] is
    /// `1`: a failure. Unlike a directory that is not a checkout, a refusal
    /// is something a job can make untrue — `git config --global --add
    /// safe.directory` is one line of a workflow — so it escalates.
    FailGitRefused,
    /// `git` ran and answered that the directory is not the top of its own
    /// work tree: a reported skip, whatever [`REQUIRE_VAR`] says.
    SkipNotAWorkTree,
}

/// The rule the git work-tree gate follows.
///
/// Three of the four ways there is no answer behave the way every other gate
/// in this module behaves, and the fourth deliberately does not.
///
/// A `git` that is missing, that is there and will not start, or that runs and
/// refuses to look, escalates under [`REQUIRE_VAR`], because that variable is
/// a claim somebody has to be able to *make true*: a job that promises a
/// complete toolchain and has no working `git` is broken, and saying so is the
/// whole point of the variable. An ownership refusal is the same kind of
/// thing — one `git config --global --add safe.directory` in the job makes it
/// untrue.
///
/// A directory that is not the top of its own work tree never escalates, for
/// [`require_elf_stripper`]'s reason: nobody can install being a checkout. The
/// case is not hypothetical and it is not a broken machine either —
/// `cargo mutants` runs the suite inside a copy of the tree that carries no
/// `.git`, and every mutation-testing shard of the Nightly workflow failed
/// there on a test that asked `git check-ignore` what this repository ignores.
/// See
/// `tests/regressions/e19_a_repository_property_test_could_not_answer_in_a_copy_of_the_tree.rs`.
pub fn work_tree_gate(probe: WorkTreeProbe, toolchain_required: bool) -> WorkTreeGate {
    match probe {
        // Nothing asked, because there was nothing to ask with, or nothing
        // that would run. Both are the ordinary toolchain gate and both
        // escalate exactly as `require_tools` does.
        WorkTreeProbe::NoGit if toolchain_required => WorkTreeGate::FailNoGit,
        WorkTreeProbe::NoGit => WorkTreeGate::SkipNoGit,
        WorkTreeProbe::GitUnrunnable if toolchain_required => WorkTreeGate::FailGitUnrunnable,
        WorkTreeProbe::GitUnrunnable => WorkTreeGate::SkipGitUnrunnable,
        // `git` ran and would not look. Nobody can install being a checkout,
        // but an ownership refusal or a broken global config is something a
        // job can fix, so this side of the asymmetry escalates too.
        WorkTreeProbe::GitRefused if toolchain_required => WorkTreeGate::FailGitRefused,
        WorkTreeProbe::GitRefused => WorkTreeGate::SkipGitRefused,
        // `git` answered, and its answer was that this directory is not the
        // top of a work tree of its own. `toolchain_required` is deliberately
        // not consulted.
        WorkTreeProbe::NotItsOwnWorkTree => WorkTreeGate::SkipNotAWorkTree,
        WorkTreeProbe::OwnWorkTree => WorkTreeGate::Open,
    }
}

/// The environment variables that decide which repository a `git` answers
/// about, whatever directory it was pointed at.
///
/// Each of them changes the answer whatever `-C` says: `GIT_DIR=/elsewhere git
/// -C here status` reports on `/elsewhere`, `GIT_CEILING_DIRECTORIES` stops
/// the search for a repository partway up, and the rest redirect the work
/// tree, the index and the object store. A question about *this* repository
/// asked by a child that inherited any of them is a question about whichever
/// repository the caller's shell was last in, and a `git init` that inherited
/// `GIT_DIR` creates its repository somewhere the test never looks. So they
/// are removed rather than trusted; see [`git_command`].
pub const GIT_REDIRECTING_VARS: [&str; 6] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_COMMON_DIR",
    "GIT_OBJECT_DIRECTORY",
    "GIT_CEILING_DIRECTORIES",
];

/// A `git` child that answers about the repository it is pointed at.
///
/// Every `git` this suite starts goes through here, so that the answer does
/// not depend on the environment the suite was started from. The variables
/// removed are [`GIT_REDIRECTING_VARS`] and the reason is written there.
pub fn git_command(git: &Path) -> Command {
    let mut command = Command::new(git);
    for name in GIT_REDIRECTING_VARS {
        command.env_remove(name);
    }
    // And in one language, because [`probe_own_work_tree`] reads what `git`
    // says to tell "this directory belongs to no repository" from "I will not
    // look at it". `LC_ALL=C` is the whole of it: glibc's gettext ignores
    // `LANGUAGE` once the locale is `C`, so a caller's translated `git` cannot
    // turn a refusal into a directory that is not a checkout.
    command.env("LC_ALL", "C");
    command
}

/// The sentence `git` prints, in the `C` locale, where the directory it was
/// pointed at belongs to no repository at all.
///
/// The one failure that is an *answer* rather than a refusal. Everything else
/// `git` fails with — an ownership refusal, a broken config, a corrupt object
/// store — is [`WorkTreeProbe::GitRefused`], because the question was never
/// reached.
pub const NOT_A_REPOSITORY: &str = "not a git repository";

/// Why a `git` that is on `PATH` could not be asked anything.
///
/// The program's path and the operating system's own words, because a `git`
/// that will not start is a fact about the program and reporting it as a
/// directory that is not a checkout would send the reader to `cargo mutants`
/// for a file mode.
pub fn git_unrunnable_reason(git: &Path, error: &std::io::Error) -> String {
    format!(
        "`{}` is on PATH and could not be run: {error}. That is the program rather than the \
         directory, so what this repository tracks or ignores was never asked",
        git.display()
    )
}

/// What one `git` said about one directory, and its own words when it would
/// not say.
///
/// The probe is the row [`work_tree_gate`] decides on. `detail` is the
/// sentence the skip or the failure quotes, and it is empty wherever nothing
/// went wrong: for [`WorkTreeProbe::GitUnrunnable`] it is
/// [`git_unrunnable_reason`] and for [`WorkTreeProbe::GitRefused`] it is
/// [`git_refusal_reason`]. Carrying it out of the probe is the point — the one
/// line that would tell a reader to run `git config --global --add
/// safe.directory` is `git`'s, and a probe that returns a bare `false` throws
/// it away and lets the caller blame `cargo mutants` for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkTreeAnswer {
    /// Which of the five things happened.
    pub probe: WorkTreeProbe,
    /// What to print, in `git`'s or the operating system's own words. Empty
    /// unless `probe` is `GitUnrunnable` or `GitRefused`.
    pub detail: String,
}

/// Why a `git` that ran would not answer about `directory`.
///
/// Its exit status and its own words, and the one line that fixes the common
/// cause. Reported as [`work_tree_skip_message`] instead, this reads as a
/// directory that is not a checkout and sends the reader to `cargo mutants`
/// for a line in `~/.gitconfig` or a bind-mounted checkout owned by another
/// uid.
pub fn git_refusal_reason(
    git: &Path,
    directory: &Path,
    status: ExitStatus,
    stderr: &str,
) -> String {
    format!(
        "`{}` ran and would not answer about {}: it exited with {status} and said `{}`. That is \
         the program or this machine rather than a directory that is not a checkout; an ownership \
         refusal is undone with `git config --global --add safe.directory {}`",
        git.display(),
        directory.display(),
        stderr.trim(),
        directory.display()
    )
}

/// What `git` says about `directory`: is it the top of its own work tree?
///
/// `git -C <directory> rev-parse --show-toplevel`, which prints the root of
/// the work tree `directory` belongs to and fails with `fatal: not a git
/// repository` where it belongs to none.
///
/// Deliberately `--show-toplevel` compared against `directory` rather than
/// `--is-inside-work-tree`. Being *inside* a work tree is not being one: a
/// copy of this tree unpacked under any directory that is itself a checkout —
/// and `cargo mutants` unpacks under `TMPDIR`, which nothing says is outside
/// every checkout — is inside a work tree that knows nothing about it. There
/// `git ls-files` lists nothing and succeeds, and `git check-ignore` answers
/// about the enclosing repository, which is E19's failure with the gate wide
/// open.
///
/// The cost is stated rather than hidden: a checkout of this repository that
/// is itself a subdirectory of a larger one reads as "cannot answer" and its
/// repository-property tests stand down, out loud, instead of answering from
/// the enclosing repository. A reported skip in a layout nobody here uses is
/// the safe side of that trade.
///
/// Deliberately not `git status` or a probe for a `.git` entry either. A work
/// tree added with `git worktree add` carries a `.git` *file* rather than a
/// directory, and a submodule's carries a pointer; `rev-parse` is the one
/// question that answers correctly for all three.
///
/// A failure is not one thing. `git` exits `128` both where the directory
/// belongs to no repository and where it refused to look at one, so the two
/// are told apart by what it said — [`NOT_A_REPOSITORY`], which
/// [`git_command`] pins to one language — and only the first is an answer.
///
/// The probe runs under [`WORK_TREE_BUDGET`] through
/// [`wait_bounded`](crate::common::bounded::wait_bounded), like every other
/// child this suite starts. It is spawned by hand rather than through
/// [`run_bounded`](crate::common::bounded::run_bounded) for one reason:
/// `run_bounded` turns a program that cannot be spawned into a panic, and here
/// that is an answer rather than a defect.
pub fn probe_own_work_tree(git: &Path, directory: &Path) -> WorkTreeAnswer {
    let spawned = git_command(git)
        .arg("-C")
        .arg(directory)
        .args(["rev-parse", "--show-toplevel"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let child = match spawned {
        Ok(child) => child,
        Err(error) => {
            return WorkTreeAnswer {
                probe: WorkTreeProbe::GitUnrunnable,
                detail: git_unrunnable_reason(git, &error),
            };
        }
    };
    let output = wait_bounded(child, WORK_TREE_BUDGET, "the git work-tree probe");
    // A failure is two different things. Outside a repository `rev-parse`
    // fails saying so, and reading *that* as an answer is right; reading every
    // other failure as one is how a `git` that refused to look — an ownership
    // refusal, a broken global config — comes to be reported as the copy of
    // the tree `cargo mutants` made.
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !stderr.contains(NOT_A_REPOSITORY) {
            return WorkTreeAnswer {
                probe: WorkTreeProbe::GitRefused,
                detail: git_refusal_reason(git, directory, output.status, &stderr),
            };
        }
        return WorkTreeAnswer {
            probe: WorkTreeProbe::NotItsOwnWorkTree,
            detail: String::new(),
        };
    }
    // Inside somebody else's repository `rev-parse` succeeds and prints
    // *their* root, and reading that success as an answer about this directory
    // is the other half of reading a failure as one.
    let printed = String::from_utf8_lossy(&output.stdout);
    let printed = printed.trim();
    let probe = if !printed.is_empty() && names_the_same_directory(printed, directory) {
        WorkTreeProbe::OwnWorkTree
    } else {
        WorkTreeProbe::NotItsOwnWorkTree
    };
    WorkTreeAnswer {
        probe,
        detail: String::new(),
    }
}

/// How long the `git rev-parse` probe gets to finish.
///
/// It is one process doing no work, so this is a deadline against a wedged
/// child — a `git` waiting on a credential helper, a filesystem that has
/// stopped answering — rather than an expectation of how long it takes.
pub const WORK_TREE_BUDGET: Duration = Duration::from_secs(60);

/// The skip a repository-property test prints where it cannot answer.
///
/// Rendered rather than printed inline so the sentence is a value a test can
/// hold to a snapshot: a skip that does not say *which* directory could not
/// answer, or why the run it happened in has no `.git`, sends the reader
/// looking for a broken machine instead of at the copy `cargo mutants` made.
pub fn work_tree_skip_message(directory: &Path) -> String {
    format!(
        "skipping: {} is not the top of a git work tree of its own, so what this repository \
         tracks or ignores cannot be asked here. `cargo mutants` runs the suite in a copy of the \
         tree that carries no `.git`; this claim is made from a checkout.",
        directory.display()
    )
}

/// The `git` a question about this repository needs, or a reported skip.
///
/// Two conditions, and the second is the one [`require_tools`] cannot state.
/// `git` has to be on `PATH` and has to run, which is the ordinary gate and
/// escalates under [`REQUIRE_VAR`]; and `directory` has to be the top of a git
/// work tree of its own, because `git check-ignore` and `git ls-files` are
/// questions about a repository and a directory that is not in one — or is in
/// somebody else's — has no answer to give about this one.
///
/// The second condition is a genuine inability rather than a gate lowered, and
/// the skip announces itself with [`work_tree_skip_message`]. A `git` that ran
/// and *refused* to look is neither condition and is reported as itself, in
/// its own words, through [`git_refusal_reason`]. The rule every half follows
/// is [`work_tree_gate`], written there as a table.
///
/// # Panics
///
/// If `git` is missing, unrunnable, or refuses to answer, and [`REQUIRE_VAR`]
/// is `1`.
pub fn require_git_work_tree(directory: &Path) -> Option<Toolchain> {
    let path_var = std::env::var_os("PATH");
    let git = ginary::process::find_in_path(GIT, path_var.as_deref());
    // Only asked when there is something to ask with; `NoGit` is the answer
    // when there is not.
    let answer = match git.as_deref() {
        None => WorkTreeAnswer {
            probe: WorkTreeProbe::NoGit,
            detail: String::new(),
        },
        Some(git) => probe_own_work_tree(git, directory),
    };
    let detail = answer.detail;
    let required = toolchain_required();

    match work_tree_gate(answer.probe, required) {
        WorkTreeGate::Open => git.map(|git| Toolchain {
            tools: BTreeMap::from([(GIT.to_owned(), git)]),
        }),
        WorkTreeGate::SkipNoGit => {
            eprintln!("skipping: {GIT} not on PATH");
            None
        }
        WorkTreeGate::FailNoGit => {
            panic!("`{GIT}` is not on PATH and {REQUIRE_VAR}=1 forbids skipping")
        }
        WorkTreeGate::SkipGitUnrunnable | WorkTreeGate::SkipGitRefused => {
            eprintln!("skipping: {detail}");
            None
        }
        WorkTreeGate::FailGitUnrunnable | WorkTreeGate::FailGitRefused => {
            panic!("{detail}, and {REQUIRE_VAR}=1 forbids skipping")
        }
        WorkTreeGate::SkipNotAWorkTree => {
            eprintln!("{}", work_tree_skip_message(directory));
            None
        }
    }
}

/// How long a `git` that answers a question about this repository gets.
///
/// `git ls-files` over four directories and `git check-ignore` over one path
/// are both index reads, so this is a deadline against a wedged child rather
/// than an expectation, exactly as [`WORK_TREE_BUDGET`] is.
pub const LS_FILES_BUDGET: Duration = Duration::from_secs(120);

/// The program [`require_git_work_tree`] looks for.
pub const GIT: &str = "git";

/// The `git` on `PATH`, if there is one and it will answer.
///
/// [`require_tools`] admits a program because it is on `PATH`. That is the
/// wrong question for `git`: a `git` can be perfectly present and still refuse
/// every question this suite asks it — `detected dubious ownership` is the
/// common shape, and it exits 128 without looking at the repository. A test
/// that asserts what a probe *found* then fails for a reason that has nothing
/// to do with the code, on a machine whose only fault is a `safe.directory`
/// setting.
///
/// So this asks the same question [`require_working_pwsh`] asks: not "is it
/// there" but "does it work". A `git` that cannot report its own version is
/// one this suite stands down in front of, saying which `git` and what it
/// said.
///
/// It is deliberately *not* used by the over-correction guards, which have to
/// fail rather than skip when a working `git` reports itself unable — a
/// fixture builder that called every `git` unusable would stand every guard in
/// the file down and report a clean run, which is the check deleted rather
/// than gated.
pub fn require_working_git() -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH");
    let Some(git) = ginary::process::find_in_path(GIT, path_var.as_deref()) else {
        eprintln!("skipping: {GIT} not on PATH");
        return None;
    };
    let mut command = git_command(&git);
    command.arg("--version");
    let output = run_bounded(&mut command, WORK_TREE_BUDGET, "the git health check");
    if output.status.success() {
        return Some(git);
    }
    eprintln!(
        "skipping: `{} --version` answered {}: {}",
        git.display(),
        output.status,
        String::from_utf8_lossy(&output.stderr).trim()
    );
    None
}
