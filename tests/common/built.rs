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

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::Duration;

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
    pub fn app_dir(&self, app: &str) -> PathBuf {
        self.home.join("ginary").join(app)
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
            .env("HOME", &self.home)
            .env("PATH", &self.empty_path)
            .env("XDG_CACHE_HOME", &self.home)
            .current_dir(&self.cwd)
            .args(&self.args);
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
