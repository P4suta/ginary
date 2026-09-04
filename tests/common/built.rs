// SPDX-License-Identifier: MIT OR Apache-2.0
//! `ginary build` over a fixture project, and running what it produced.
//!
//! This is the only helper that drives the *real* command end to end:
//! [`crate::common::fixture::FixtureProject`] copies the project,
//! [`BuiltProject::build`] runs this test run's own `ginary` binary in it, and
//! [`BuiltProject::run`] executes the artifact under a scrubbed environment.
//!
//! "Scrubbed" is the load-bearing word and it is the same discipline
//! `tests/common/artifact.rs` follows: `env_clear()`, a `PATH` that is an
//! empty directory, and `HOME` and `XDG_CACHE_HOME` inside the test's own
//! tree. A packaged application that ran because the developer had Erlang
//! installed would prove nothing, and a cache written into the developer's
//! `~/.cache` would leak between test runs.
//!
//! Both children are bounded. A build is minutes on a cold machine and a run
//! is milliseconds, but neither may hang the suite: a `gleam` waiting on a
//! lock and a launcher deadlocked on its cache are exactly the failures these
//! tests exist to catch, and an unbounded wait reports either one as a stalled
//! job with no diagnosis.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::Duration;

use ginary::cache;
use ginary::target::Os;

use crate::common::bounded::run_bounded;
use crate::common::fixture::FixtureProject;

/// How long one `ginary build` gets.
///
/// Wide: it runs `gleam export erlang-shipment`, stages a whole OTP
/// installation, runs `strip` over it and packs the result at zstd 19.
pub const BUILD_BUDGET: Duration = Duration::from_secs(900);

/// How long one run of a built artifact gets, cold cache included.
pub const RUN_BUDGET: Duration = Duration::from_secs(120);

/// The `SOURCE_DATE_EPOCH` the determinism test pins the clock to.
///
/// 2023-11-14T22:13:20Z. Any fixed value would do; a fixed one is the point,
/// because `created_at` is in the manifest and the manifest is in the payload
/// the artifact's digest is taken over.
pub const PINNED_EPOCH: &str = "1700000000";

/// This test run's own `ginary` binary.
pub fn ginary_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_ginary"))
}

/// A copied fixture project, its temporary root, and the runs made in it.
#[derive(Debug)]
pub struct BuiltProject {
    dir: tempfile::TempDir,
    project: FixtureProject,
    app: String,
}

impl BuiltProject {
    /// Copies `tests/fixtures/<name>` into a temporary directory.
    ///
    /// # Panics
    ///
    /// If the temporary directory or the copy fails.
    pub fn copy(name: &str) -> Self {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let project = FixtureProject::copy(name, dir.path());
        Self {
            dir,
            project,
            app: name.to_owned(),
        }
    }

    /// The temporary root, which holds the project and the run directories.
    pub fn dir(&self) -> &Path {
        self.dir.path()
    }

    /// The project root, the directory holding `gleam.toml`.
    pub fn root(&self) -> &Path {
        self.project.dir()
    }

    /// The application name, which is the fixture's directory name.
    pub fn app(&self) -> &str {
        &self.app
    }

    /// Where the default `ginary build` writes: `build/ginary/<app>`.
    pub fn artifact(&self) -> PathBuf {
        self.root().join("build/ginary").join(&self.app)
    }

    /// The work directories `ginary build` left under `build/ginary`.
    ///
    /// Sorted, and empty when the build cleaned up after itself, which is what
    /// every test but the `--keep-staging` one asserts.
    ///
    /// # Panics
    ///
    /// Never: a `build/ginary` that is not there is no work directories.
    pub fn work_dirs(&self) -> Vec<PathBuf> {
        let Ok(entries) = std::fs::read_dir(self.root().join("build/ginary")) else {
            return Vec::new();
        };
        let mut dirs: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(ginary::bundle::WORK_DIR_PREFIX)
            })
            .map(|entry| entry.path())
            .collect();
        dirs.sort();
        dirs
    }

    /// Runs `ginary build` in the project with no extra flags.
    ///
    /// # Panics
    ///
    /// If the binary cannot be started or does not finish within
    /// [`BUILD_BUDGET`].
    pub fn build(&self) -> Output {
        self.build_with(&[], &[])
    }

    /// Runs `ginary build` with extra arguments and extra variables.
    ///
    /// The environment is inherited rather than cleared: the build needs
    /// `PATH` to find `gleam`, `erl` and `strip`, which is the whole point of
    /// gating these tests on the toolchain.
    ///
    /// # Panics
    ///
    /// If the binary cannot be started or does not finish within
    /// [`BUILD_BUDGET`].
    pub fn build_with(&self, args: &[&str], env: &[(&str, &str)]) -> Output {
        let mut command = Command::new(ginary_bin());
        command.arg("build").args(args).current_dir(self.root());
        for (key, value) in env {
            command.env(key, value);
        }
        run_bounded(&mut command, BUILD_BUDGET, "`ginary build`")
    }

    /// Runs `ginary` itself with the given arguments, from the project root.
    ///
    /// # Panics
    ///
    /// If the binary cannot be started or does not finish within
    /// [`RUN_BUDGET`].
    pub fn ginary(&self, args: &[&OsStr]) -> Output {
        let mut command = Command::new(ginary_bin());
        command.args(args).current_dir(self.root());
        run_bounded(&mut command, RUN_BUDGET, "`ginary`")
    }

    /// Runs the built artifact under a scrubbed environment.
    ///
    /// `name` labels this run's directories, so several runs of one build have
    /// their own `HOME`, cache and working directory and cannot be confused
    /// with each other.
    pub fn run(&self, name: &str) -> ArtifactRun {
        ArtifactRun::new(self.dir.path(), self.artifact(), name)
    }

    /// Runs a *copy* of the artifact, such as one a test has damaged.
    pub fn run_program(&self, name: &str, program: &Path) -> ArtifactRun {
        ArtifactRun::new(self.dir.path(), program.to_path_buf(), name)
    }
}

/// The variables that pin a built artifact's cache root inside `home` on
/// `os`, so that one test run's cache is nobody else's.
///
/// The rule five `tests/e2e_hello.rs` failures turned on. [`ArtifactRun`]
/// clears the environment and then sets `HOME` and `XDG_CACHE_HOME` — the two
/// unix conventions — so on Windows nothing was set that
/// `ginary::cache::resolve_windows` reads, every run fell through to
/// `%TEMP%\ginary-<user>` with no `%TEMP%` and no `%USERNAME%` either, and
/// every run of every test in the binary shared one directory:
///
/// ```text
/// ginary: the runtime cache at \\?\C:\Windows\Temp\ginary-unknown\hello_ffi\
///   .4459c55fbab39b91.tmp-2352\lib\kernel-11.0.3\ebin\heart.beam is unusable:
///   The system cannot find the file specified. (os error 2)
///
/// the printed entry must be under this run's own cache:
///   \\?\C:\Windows\Temp\ginary-unknown\hello_ffi\5e6351992db98538
///
/// a warm run must not write a second entry
///   left: 0
///  right: 1
/// ```
///
/// (`Windows build and exit-code propagation`
/// <https://github.com/P4suta/ginary/actions/runs/33751715516/job/100636537290>.)
///
/// Two concurrent runs extracting the same key into one shared entry is what
/// produced the missing `.beam` files and the `os error 2 while canonicalizing`
/// the tar reader reported: one run's sweep of the leftovers removed the other
/// run's half-finished tree.
///
/// `GINARY_CACHE_DIR` is deliberately not the answer. It is the override, and
/// a run that used it would prove that the override works rather than that the
/// platform's own rule lands inside this run's `home`, which is what these
/// tests are for. So the pairs are the platform's ordinary ones, and
/// [`isolated_cache_root`] checks that they are read as such.
pub fn isolating_cache_env(os: Os, home: &Path) -> Vec<(&'static str, PathBuf)> {
    match os {
        Os::Linux | Os::Macos => vec![
            (cache::HOME_VAR, home.to_path_buf()),
            (cache::XDG_CACHE_HOME_VAR, home.to_path_buf()),
        ],
        // `%LOCALAPPDATA%` is what `cache::resolve_windows` reads, and
        // `%USERNAME%` is set beside it so that the *fallback* would also be
        // this run's own rather than the machine's shared
        // `%TEMP%\ginary-unknown` — which is the directory every run of every
        // test in the binary was sharing. The name is the run's own directory,
        // so two runs never collide even if the fallback is ever reached.
        Os::Windows => vec![
            (cache::LOCALAPPDATA_VAR, home.to_path_buf()),
            (cache::USERNAME_VAR, PathBuf::from(run_user_name(home))),
        ],
    }
}

/// A user name unique to the run that owns `home`.
///
/// `home` is a fresh temporary directory per run, so its own file name is
/// already unique; anything a separator could hide in is replaced, because
/// `cache::current_user` refuses a name holding one and would fall back to the
/// shared `unknown`.
fn run_user_name(home: &Path) -> String {
    let name = home.file_name().map_or_else(
        || String::from("run"),
        |name| name.to_string_lossy().into_owned(),
    );
    let cleaned: String = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '-'
            }
        })
        .collect();
    if cleaned.is_empty() {
        String::from("run")
    } else {
        cleaned
    }
}

/// The cache root [`isolating_cache_env`]'s pairs resolve to on `os`.
///
/// Derived by running `ginary::cache`'s own resolver over those pairs rather
/// than by rebuilding the layout here, so the helper cannot claim an isolation
/// the product does not perform. A root the resolver calls a *fallback* is one
/// the pairs failed to pin, and that is a panic rather than a silent share.
///
/// # Panics
///
/// If the pairs do not pin a root, or pin one outside `home`.
pub fn isolated_cache_root(os: Os, home: &Path) -> PathBuf {
    let env = cache::Env::from_pairs(
        isolating_cache_env(os, home)
            .into_iter()
            .map(|(key, value)| (OsString::from(key), OsString::from(value))),
    );
    let dirs = match os {
        Os::Windows => cache::resolve_windows(&env, "tester"),
        Os::Linux | Os::Macos => cache::resolve(&env, 0),
    };
    assert!(
        !dirs.is_fallback,
        "the variables for {os:?} did not pin a cache root; {:?} is the fallback",
        dirs.origin
    );
    assert!(
        dirs.root.starts_with(home),
        "a run's cache has to live under the directory the run owns: {} is not under {}",
        dirs.root.display(),
        home.display()
    );
    dirs.root
}

/// A pending run of a built artifact.
#[derive(Debug)]
pub struct ArtifactRun {
    program: PathBuf,
    home: PathBuf,
    cwd: PathBuf,
    empty_path: PathBuf,
    trace: PathBuf,
    args: Vec<std::ffi::OsString>,
    env: Vec<(String, std::ffi::OsString)>,
}

impl ArtifactRun {
    /// Creates the four directories one run needs.
    ///
    /// # Panics
    ///
    /// If any of them cannot be created.
    fn new(root: &Path, program: PathBuf, name: &str) -> Self {
        let home = root.join(format!("{name}-home"));
        let cwd = root.join(format!("{name}-cwd"));
        let empty_path = root.join(format!("{name}-path"));
        for directory in [&home, &cwd, &empty_path] {
            std::fs::create_dir_all(directory).expect("a run directory");
        }
        Self {
            program,
            trace: root.join(format!("{name}-trace.jsonl")),
            home,
            cwd,
            empty_path,
            args: Vec::new(),
            env: Vec::new(),
        }
    }

    /// Appends one argument for the packaged application.
    #[must_use]
    pub fn arg(mut self, argument: impl AsRef<OsStr>) -> Self {
        self.args.push(argument.as_ref().to_os_string());
        self
    }

    /// Appends several arguments.
    #[must_use]
    pub fn args<I, S>(mut self, arguments: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.args
            .extend(arguments.into_iter().map(|a| a.as_ref().to_os_string()));
        self
    }

    /// Sets a variable on top of the scrubbed environment.
    #[must_use]
    pub fn env(mut self, key: &str, value: impl AsRef<OsStr>) -> Self {
        self.env
            .push((key.to_owned(), value.as_ref().to_os_string()));
        self
    }

    /// Turns `GINARY_TRACE` on, writing to this run's own trace file.
    #[must_use]
    pub fn traced(mut self) -> Self {
        let trace = self.trace.clone();
        self.env.push(("GINARY_TRACE".to_owned(), trace.into()));
        self
    }

    /// The working directory this run is started in.
    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    /// `<cache>/<app>`, the application directory under this run's cache.
    ///
    /// The layout the *platform's* resolver produces, not the unix one:
    /// see [`isolating_cache_env`].
    pub fn app_dir(&self, app: &str) -> PathBuf {
        isolated_cache_root(ginary::platform::HOST, &self.home).join(app)
    }

    /// Runs it to completion.
    ///
    /// # Panics
    ///
    /// If the artifact cannot be started, or does not exit within
    /// [`RUN_BUDGET`].
    pub fn output(self) -> ArtifactOutput {
        let mut command = Command::new(&self.program);
        command
            .env_clear()
            .env("PATH", &self.empty_path)
            .current_dir(&self.cwd)
            .args(&self.args);
        for (key, value) in isolating_cache_env(ginary::platform::HOST, &self.home) {
            command.env(key, value);
        }
        super::coverage::preserve_coverage_env(&mut command);
        for (key, value) in &self.env {
            command.env(key, value);
        }
        let output = run_bounded(&mut command, RUN_BUDGET, "the built artifact");
        ArtifactOutput {
            output,
            cwd: self.cwd,
            trace: self.trace,
            home: self.home,
        }
    }
}

/// What one run of a built artifact produced.
#[derive(Debug)]
pub struct ArtifactOutput {
    /// The child's exit status and both streams.
    pub output: Output,
    /// The working directory it ran in, so a test can list what it left there.
    pub cwd: PathBuf,
    /// The `GINARY_TRACE` file, which exists only for a [`ArtifactRun::traced`]
    /// run.
    pub trace: PathBuf,
    /// The `HOME` it was given, which is also its `XDG_CACHE_HOME`.
    pub home: PathBuf,
}

impl ArtifactOutput {
    /// `<cache>/<app>`, the application directory under this run's cache.
    ///
    /// The cache root a scrubbed run resolves is `<HOME>/ginary`, because the
    /// run's `XDG_CACHE_HOME` is its `HOME`.
    pub fn app_dir(&self, app: &str) -> PathBuf {
        self.home.join("ginary").join(app)
    }

    /// The exit code.
    ///
    /// # Panics
    ///
    /// If a signal ended the process, which no test expects.
    pub fn code(&self) -> i32 {
        self.output.status.code().unwrap_or_else(|| {
            panic!(
                "the artifact was killed by a signal\n--- stdout ---\n{}\n--- stderr ---\n{}",
                self.stdout(),
                self.stderr()
            )
        })
    }

    /// Standard output, lossily converted.
    pub fn stdout(&self) -> String {
        String::from_utf8_lossy(&self.output.stdout).into_owned()
    }

    /// Standard error, lossily converted.
    pub fn stderr(&self) -> String {
        String::from_utf8_lossy(&self.output.stderr).into_owned()
    }

    /// The trace file's contents, or the empty string when there is none.
    pub fn trace_text(&self) -> String {
        std::fs::read_to_string(&self.trace).unwrap_or_default()
    }

    /// The names in the working directory, sorted.
    pub fn cwd_entries(&self) -> Vec<String> {
        names_in(&self.cwd)
    }
}

/// The entries of a directory, sorted; an empty list when it is not there.
pub fn names_in(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// The SHA-256 of a file, in lower-case hexadecimal.
///
/// # Panics
///
/// If the file cannot be read.
pub fn sha256_of(path: &Path) -> String {
    let bytes = std::fs::read(path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    crate::common::payload::sha256_hex(&bytes)
}

/// Copies `path` to `<path>-<suffix>` and flips one byte at `offset`.
///
/// # Panics
///
/// If the copy or the rewrite fails, or if `offset` is past the end.
pub fn corrupt_copy(path: &Path, suffix: &str, offset: u64) -> PathBuf {
    use std::io::{Read as _, Seek as _, SeekFrom, Write as _};

    let target = path.with_file_name(format!(
        "{}-{suffix}",
        path.file_name().unwrap_or_default().to_string_lossy()
    ));
    std::fs::copy(path, &target).expect("copy the artifact");

    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&target)
        .expect("open the copy");
    let len = file.metadata().expect("stat the copy").len();
    assert!(offset < len, "offset {offset} is past the end of {len}");
    file.seek(SeekFrom::Start(offset)).expect("seek");
    let mut byte = [0u8; 1];
    file.read_exact(&mut byte).expect("read the byte");
    file.seek(SeekFrom::Start(offset)).expect("seek back");
    file.write_all(&[byte[0] ^ 0xff]).expect("write the byte");
    file.sync_all().expect("sync the copy");
    target
}
