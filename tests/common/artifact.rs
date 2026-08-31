// SPDX-License-Identifier: MIT OR Apache-2.0
//! Hand-assembling a packaged application, without an Erlang installation.
//!
//! The launcher's contract is about *paths, permissions, argv and the
//! environment*. None of that needs a real BEAM, and a test suite that could
//! only check it with one would be a test suite that runs on three machines in
//! the world. So [`SyntheticArtifact`] builds the real thing out of parts a
//! test can read back:
//!
//! - a staging root whose `erts-<vsn>/bin` programs are `/bin/sh` scripts,
//!   with the launch program printing every variable the contract names and
//!   one line per argument it was given before exiting 7;
//! - the real [`ginary::payload::pack`], so the artifact holds a real
//!   deterministic tar inside a real zstd stream;
//! - a copy of this test run's own `ginary` binary as the stub, with the
//!   payload and a real [`ginary::trailer::Trailer`] appended.
//!
//! What comes out is an executable that a test runs like any other program,
//! and everything the launcher decides is then visible on its standard output.
//! The stub's exit code is [`STUB_EXIT`] rather than zero so that "the exit
//! code is mirrored" is a claim about a number nothing else in the system
//! produces.
//!
//! Every run is scrubbed: [`Run`]'s builder clears the environment, puts an
//! empty directory on `PATH`, and points `HOME` and `XDG_CACHE_HOME` inside
//! the artifact's own temporary tree. A launcher test that read the
//! developer's real cache would be a launcher test that passes on one machine.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use ginary::assemble::{Category, StageListing, StagedApp, StagedFile, StagedSource};
use ginary::manifest::{AppRef, LaunchSpec, Manifest};
use ginary::payload::Packed;
use ginary::target::Target;
use ginary::trailer::Trailer;

/// The ERTS version the synthetic runtime claims.
pub const ERTS_VSN: &str = "17.0.5";

/// The application name, and therefore the `<app>` component of every cache
/// path and the value of `PROGNAME`.
pub const APP: &str = "hello";

/// The OTP release the synthetic runtime claims.
pub const OTP_RELEASE: u32 = 29;

/// The OTP version the synthetic runtime claims.
pub const OTP_VERSION: &str = "29.0.5";

/// The one OTP application the synthetic runtime carries, with its version.
pub const OTP_APP: &str = "stdlib-8.0.3";

/// The exit code the `erlexec` stub leaves when it is not told otherwise.
pub const STUB_EXIT: i32 = 7;

/// The argument the stub reads an exit code from.
pub const EXIT_ARG: &str = "--exit";

/// The argument the stub kills itself with, so that a supervised run has a
/// signal to report rather than an exit code.
pub const SIGNAL_ARG: &str = "--signal";

/// The argument that makes the stub write `$ERL_CRASH_DUMP` before it exits.
///
/// The file it writes is not a real crash dump. It is three lines with a
/// `Slogan:` among them, which is exactly what `launch::supervise` reads and
/// all a test can assert without a running BEAM.
pub const DUMP_ARG: &str = "--dump";

/// The slogan the stub writes into the dump it is asked for.
pub const STUB_SLOGAN: &str = "Slogan: init terminating in do_boot (ginary test stub)";

/// The argument the stub stays alive for, in seconds.
///
/// The lock proof needs a runtime that is still running when the test looks at
/// the cache entry: nothing about a launcher that has already exited can say
/// whether the `flock` survived `execve`. `sleep` is a program rather than a
/// shell builtin, so a run that uses this must put a real `PATH` on the child
/// — see [`Runner::env`].
pub const SLEEP_ARG: &str = "--sleep";

/// The `-eval` expression the stub treats as a request to exit zero.
///
/// `GINARY_CMD=selftest` replaces the manifest's expression with this one, so
/// a stub that honours it is a stub that can be selftested. Nothing else in
/// the stub looks at `-eval`.
pub const HALT_EVAL: &str = "erlang:halt(0)";

/// The compression level the tests pack at.
///
/// One rather than nineteen: a launcher test packs a few kilobytes and runs
/// the packer once per test, and the artifact size that matters is measured
/// once, at level 19, by the toolchain-gated test at the end of
/// `tests/launcher.rs`.
pub const LEVEL: i32 = 1;

/// The `erlexec` stub.
///
/// It answers the two questions every launcher test asks. First, which of the
/// contract's variables were set and to what — `<unset>` is printed for one
/// that is absent, because "absent" is the assertion for `ERL_LIBS` and a
/// missing line would also be produced by a stub that failed. Second, the
/// arguments, one per line, printed with `printf` so that a byte which is not
/// valid UTF-8 arrives on standard output as itself.
const ERLEXEC_STUB: &str = r#"#!/bin/sh
for name in ROOTDIR BINDIR EMU PROGNAME HOME ERL_CRASH_DUMP ERL_LIBS ERL_FLAGS ERL_AFLAGS \
            HEART_COMMAND GINARY_ENV_ONE GINARY_ENV_TWO
do
  eval "value=\${$name-@unset@}"
  if [ "$value" = "@unset@" ]; then
    echo "env:$name=<unset>"
  else
    echo "env:$name=$value"
  fi
done
echo "cwd:$(pwd)"
code=7
signal=
dump=
nap=
previous=
for argument in "$@"
do
  case $previous in
    --exit) code=$argument ;;
    --signal) signal=$argument ;;
    --sleep) nap=$argument ;;
    -eval) case $argument in "erlang:halt(0)") code=0 ;; esac ;;
  esac
  if [ "$argument" = "--dump" ]; then dump=1; fi
  previous=$argument
done
if [ $# -gt 0 ]; then printf 'argv:%s\n' "$@"; fi
if [ -n "$dump" ] && [ -n "$ERL_CRASH_DUMP" ]; then
  {
    echo "=erl_crash_dump:0.5"
    echo "Slogan: init terminating in do_boot (ginary test stub)"
    echo "System version: ginary test stub"
  } > "$ERL_CRASH_DUMP"
fi
if [ -n "$nap" ]; then sleep "$nap"; fi
if [ -n "$signal" ]; then kill -"$signal" $$; fi
exit "$code"
"#;

/// A program under the bindir that is not the launch program.
///
/// `preflight` only checks that these exist and are executable, so their body
/// is a refusal: a test that reaches one has found a launcher that started the
/// wrong program.
const OTHER_BIN_STUB: &str =
    "#!/bin/sh\necho \"ginary-test: $0 must not be executed\" >&2\nexit 99\n";

/// How the staged tree may be changed before it is packed.
#[derive(Clone, Debug, Default)]
pub struct ArtifactOptions {
    /// Files to leave out of the staging root, by staged path.
    pub omit: Vec<String>,
    /// The `format_version` the manifest carries.
    pub format_version: Option<u32>,
    /// The `app` the manifest carries, when it is not [`APP`].
    ///
    /// The application name is interpolated into every cache path, so a
    /// hostile one is a manifest a launcher must refuse before it creates a
    /// directory.
    pub app: Option<String>,
    /// The zstd level.
    pub level: Option<i32>,
    /// Extra programs to stage under the bindir, beyond the required four.
    ///
    /// `epmd` and `heart` are what a distribution or a `heart` artifact
    /// carries; each is written as the refusing stub, because nothing but
    /// their presence is asserted.
    pub erts_bins: Vec<String>,
    /// Extra files to stage: root-relative path, mode, contents, category.
    ///
    /// `releases/vm.args` and `releases/sys.config` arrive this way, so a
    /// launcher test can assert that the argument vector names a file that is
    /// actually in the extracted tree.
    pub extra_files: Vec<(String, u32, Vec<u8>, Category)>,
    /// A whole `launch` spec to put in the manifest instead of the canonical
    /// one.
    pub launch: Option<LaunchSpec>,
}

/// A packaged application built by hand.
#[derive(Debug)]
pub struct SyntheticArtifact {
    dir: PathBuf,
    path: PathBuf,
    manifest: Manifest,
    trailer: Trailer,
    stub_len: u64,
    packed: Packed,
}

impl SyntheticArtifact {
    /// Builds the default artifact in `dir`.
    ///
    /// `dir` must be a directory the test owns; the artifact, its staging
    /// root, the empty `PATH` directory, `HOME` and `XDG_CACHE_HOME` all live
    /// inside it.
    ///
    /// # Panics
    ///
    /// If any part of the assembly fails. Every one of them is a bug in the
    /// test tree rather than a property of the machine.
    pub fn build(dir: &Path) -> Self {
        Self::build_with(dir, &ArtifactOptions::default())
    }

    /// Builds an artifact with the staging root or the manifest changed.
    ///
    /// # Panics
    ///
    /// As [`SyntheticArtifact::build`].
    pub fn build_with(dir: &Path, options: &ArtifactOptions) -> Self {
        let staging = dir.join("staging");
        stage(&staging, options);
        let mut manifest = canonical_manifest();
        if let Some(version) = options.format_version {
            manifest.format_version = version;
        }
        if let Some(app) = &options.app {
            manifest.app.clone_from(app);
        }
        if let Some(launch) = &options.launch {
            manifest.launch.clone_from(launch);
        }

        let mut payload = Vec::new();
        let packed = ginary::payload::pack(
            &staging,
            &manifest,
            options.level.unwrap_or(LEVEL),
            &mut payload,
        )
        .unwrap_or_else(|error| panic!("cannot pack the synthetic payload: {error}"));

        let stub = std::fs::read(env!("CARGO_BIN_EXE_ginary"))
            .unwrap_or_else(|error| panic!("cannot read the ginary binary: {error}"));
        let trailer = Trailer {
            payload_offset: stub.len() as u64,
            payload_len: packed.len,
            payload_sha256: packed.sha256,
        };

        let path = dir.join(APP);
        let mut bytes = stub.clone();
        bytes.extend_from_slice(&payload);
        bytes.extend_from_slice(&trailer.to_bytes());
        write_executable(&path, &bytes);

        for name in ["home", "xdg", "emptybin"] {
            std::fs::create_dir_all(dir.join(name))
                .unwrap_or_else(|error| panic!("cannot create {name}: {error}"));
        }

        Self {
            dir: dir.to_path_buf(),
            path,
            manifest,
            trailer,
            stub_len: stub.len() as u64,
            packed,
        }
    }

    /// The artifact executable.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The temporary directory everything lives in.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The manifest that was packed.
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// The trailer that was appended.
    pub fn trailer(&self) -> &Trailer {
        &self.trailer
    }

    /// The payload's length and digest.
    pub fn packed(&self) -> &Packed {
        &self.packed
    }

    /// The stub length, which is the payload's offset.
    pub fn stub_len(&self) -> u64 {
        self.stub_len
    }

    /// The cache key the trailer produces.
    pub fn key(&self) -> String {
        self.trailer.cache_key()
    }

    /// The cache root a scrubbed run resolves, `<XDG_CACHE_HOME>/ginary`.
    pub fn cache_root(&self) -> PathBuf {
        self.dir.join("xdg").join("ginary")
    }

    /// `<cache>/<app>`, where a crash dump goes.
    pub fn app_dir(&self) -> PathBuf {
        self.cache_root().join(APP)
    }

    /// `<cache>/<app>/<key>`, the complete entry.
    pub fn key_dir(&self) -> PathBuf {
        self.app_dir().join(self.key())
    }

    /// The `HOME` a scrubbed run is given.
    pub fn home(&self) -> PathBuf {
        self.dir.join("home")
    }

    /// A run of the artifact, with the scrubbed environment already applied.
    pub fn run(&self) -> Runner<'_> {
        Runner::new(self)
    }

    /// Copies the artifact to `<dir>/<name>` and returns the new path.
    ///
    /// This is what the "a renamed artifact reuses its cache entry" test uses:
    /// the cache key comes from the payload's digest and nothing comes from
    /// the file's name.
    ///
    /// # Panics
    ///
    /// If the copy fails.
    pub fn copy_to(&self, name: &str) -> PathBuf {
        let target = self.dir.join(name);
        let bytes = std::fs::read(&self.path)
            .unwrap_or_else(|error| panic!("cannot read the artifact: {error}"));
        write_executable(&target, &bytes);
        target
    }

    /// Rewrites one byte of the artifact in place.
    ///
    /// # Panics
    ///
    /// If the artifact cannot be read back or written.
    pub fn poke(&self, offset: u64, value: u8) {
        let mut bytes = std::fs::read(&self.path)
            .unwrap_or_else(|error| panic!("cannot read the artifact: {error}"));
        let index = usize::try_from(offset).expect("an offset that fits");
        assert!(index < bytes.len(), "offset {offset} is past the artifact");
        bytes[index] ^= value;
        write_executable(&self.path, &bytes);
    }

    /// Flips a bit of the trailer's magic, so the file is no longer an
    /// artifact at all.
    pub fn break_magic(&self) {
        let length = self.file_len();
        self.poke(length - 64, 0xff);
    }

    /// Rewrites the trailer's `payload_len` so the geometry no longer
    /// describes the file.
    ///
    /// # Panics
    ///
    /// If the artifact cannot be read back or written.
    pub fn break_geometry(&self) {
        let mut bytes = std::fs::read(&self.path)
            .unwrap_or_else(|error| panic!("cannot read the artifact: {error}"));
        let start = bytes.len() - 64 + 16;
        let wrong = (self.packed.len + 1).to_le_bytes();
        bytes[start..start + 8].copy_from_slice(&wrong);
        write_executable(&self.path, &bytes);
    }

    /// Flips a byte in the middle of the compressed payload.
    pub fn break_payload(&self) {
        self.poke(self.stub_len + self.packed.len / 2, 0xff);
    }

    /// Flips a byte near the *end* of the compressed payload.
    ///
    /// The digest stops matching either way, which is all the launcher's
    /// exit-123 tests need. `ginary inspect` needs more: it reads entries 0
    /// and 1 — the manifest and the index — and stops, and it has to keep
    /// answering "what was this file supposed to be" about a file that would
    /// fail `--verify`. This artifact's staging tree is small enough that the
    /// *middle* of its compressed payload decompresses into the index, so
    /// [`SyntheticArtifact::break_payload`] would destroy the very entries the
    /// question is about. Sixteen bytes before the end is past both front
    /// entries in any tree, which is the same place
    /// `tests/e2e_hello.rs` damages a real artifact by hand.
    pub fn break_payload_tail(&self) {
        self.poke(self.stub_len + self.packed.len - 16, 0xff);
    }

    /// Shortens the artifact by `bytes` bytes, keeping the trailer at the end.
    ///
    /// The bytes come out of the *payload*, not off the end of the file,
    /// because `docs/format.md` rule 2 makes a tail truncation something else
    /// entirely: it takes the trailer with it, and what is left is a file with
    /// no magic — the ginary command line tool, which is what
    /// `the_magic_is_what_decides_the_mode` covers. The fault this models is
    /// the one that still carries a trailer and no longer matches it: a copy
    /// that stopped early, an installer that wrote a short file. The trailer
    /// is honest and the file is short, which is `TrailerError::Geometry` from
    /// the opposite direction to [`SyntheticArtifact::break_geometry`].
    ///
    /// # Panics
    ///
    /// If the artifact cannot be read back or written.
    pub fn truncate(&self, bytes: u64) {
        let all = std::fs::read(&self.path)
            .unwrap_or_else(|error| panic!("cannot read the artifact: {error}"));
        let end = all.len() - 64;
        let cut = usize::try_from(bytes).expect("a byte count that fits");
        assert!(
            cut < end,
            "{bytes} bytes is more payload than the artifact has"
        );
        let mut shortened = all[..end - cut].to_vec();
        shortened.extend_from_slice(&all[end..]);
        write_executable(&self.path, &shortened);
    }

    /// The artifact's length in bytes.
    ///
    /// # Panics
    ///
    /// If the artifact cannot be stat'd.
    pub fn file_len(&self) -> u64 {
        std::fs::metadata(&self.path)
            .unwrap_or_else(|error| panic!("cannot stat the artifact: {error}"))
            .len()
    }
}

/// A pending run of an artifact.
#[derive(Debug)]
pub struct Runner<'a> {
    artifact: &'a SyntheticArtifact,
    program: PathBuf,
    args: Vec<OsString>,
    env: BTreeMap<OsString, OsString>,
}

impl<'a> Runner<'a> {
    fn new(artifact: &'a SyntheticArtifact) -> Self {
        let mut env = BTreeMap::new();
        env.insert(
            OsString::from("PATH"),
            artifact.dir.join("emptybin").into_os_string(),
        );
        env.insert(OsString::from("HOME"), artifact.home().into_os_string());
        env.insert(
            OsString::from("XDG_CACHE_HOME"),
            artifact.dir.join("xdg").into_os_string(),
        );
        Self {
            artifact,
            program: artifact.path.clone(),
            args: Vec::new(),
            env,
        }
    }

    /// Runs a different copy of the artifact, such as a renamed one.
    pub fn program(mut self, path: &Path) -> Self {
        self.program = path.to_path_buf();
        self
    }

    /// Appends one argument.
    pub fn arg(mut self, argument: impl AsRef<OsStr>) -> Self {
        self.args.push(argument.as_ref().to_os_string());
        self
    }

    /// Appends an argument that is not valid UTF-8.
    pub fn raw_arg(mut self, bytes: &[u8]) -> Self {
        self.args.push(OsString::from_vec(bytes.to_vec()));
        self
    }

    /// Sets a variable, replacing the scrubbed default if there is one.
    pub fn env(mut self, key: &str, value: impl AsRef<OsStr>) -> Self {
        self.env
            .insert(OsString::from(key), value.as_ref().to_os_string());
        self
    }

    /// Removes a variable the scrubbed default would have set.
    pub fn without(mut self, key: &str) -> Self {
        self.env.remove(OsStr::new(key));
        self
    }

    /// The command this run would spawn.
    pub fn command(&self) -> Command {
        let mut command = Command::new(&self.program);
        command.args(&self.args).env_clear();
        for (key, value) in &self.env {
            command.env(key, value);
        }
        command.current_dir(&self.artifact.dir);
        command
    }

    /// Spawns the run without waiting for it.
    ///
    /// # Panics
    ///
    /// If the artifact cannot be spawned.
    pub fn spawn(self) -> Child {
        let mut command = self.command();
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        retrying("spawn the artifact", || command.spawn())
    }

    /// Runs to completion.
    ///
    /// # Panics
    ///
    /// If the artifact cannot be run.
    pub fn output(self) -> Run {
        let child = self.spawn();
        let output = super::bounded::wait_bounded(child, RUN_BUDGET, "the artifact");
        Run {
            code: output.status.code(),
            stdout: output.stdout,
            stderr: output.stderr,
        }
    }
}

/// How long one run of an artifact gets before the test kills it and fails.
///
/// A launcher run is milliseconds; the budget is generous because eight of
/// them race on one machine and because `mise run mutants` runs the suite
/// under load. What it buys is that a launcher which deadlocks on the cache —
/// the failure this whole module exists to catch — is a failed test rather
/// than a stalled CI job.
pub const RUN_BUDGET: std::time::Duration = std::time::Duration::from_secs(120);

/// `ETXTBSY`: the file is open for writing somewhere.
const ETXTBSY: i32 = 26;

/// How long a spawn is retried before `ETXTBSY` is treated as real.
const SPAWN_BUDGET: std::time::Duration = std::time::Duration::from_secs(10);

/// Spawns, retrying while the kernel says the program is open for writing.
///
/// This is a property of the test harness rather than of the launcher. Cargo
/// runs these tests as threads of one process, and a `fork` for one test's
/// spawn inherits every descriptor another thread has open — including the one
/// it is writing the *next* artifact through. Until that child reaches
/// `execve` and `O_CLOEXEC` closes the inherited copy, the file the other
/// thread has just finished writing still has an open writer, and executing it
/// is `ETXTBSY` (rust-lang/rust#39189). Nothing about the artifact is wrong,
/// so the answer is to try again rather than to serialise the suite.
///
/// # Panics
///
/// On any other failure, and on `ETXTBSY` that outlasts [`SPAWN_BUDGET`].
fn retrying<T>(what: &str, mut attempt: impl FnMut() -> std::io::Result<T>) -> T {
    let deadline = std::time::Instant::now() + SPAWN_BUDGET;
    loop {
        match attempt() {
            Ok(value) => return value,
            Err(error)
                if error.raw_os_error() == Some(ETXTBSY)
                    && std::time::Instant::now() < deadline =>
            {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(error) => panic!("cannot {what}: {error}"),
        }
    }
}

/// What one run of an artifact produced.
#[derive(Clone, Debug)]
pub struct Run {
    /// The exit code, or [`None`] when a signal ended the process.
    pub code: Option<i32>,
    /// Standard output, as bytes: an argument that is not valid UTF-8 comes
    /// back through here.
    pub stdout: Vec<u8>,
    /// Standard error, as bytes.
    pub stderr: Vec<u8>,
}

impl Run {
    /// The exit code.
    ///
    /// # Panics
    ///
    /// If the process was killed by a signal, which no test expects.
    pub fn code(&self) -> i32 {
        match self.code {
            Some(code) => code,
            None => panic!(
                "the run was killed by a signal\n--- stdout ---\n{}\n--- stderr ---\n{}",
                self.stdout_text(),
                self.stderr_text()
            ),
        }
    }

    /// Standard output, lossily converted.
    pub fn stdout_text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }

    /// Standard error, lossily converted.
    pub fn stderr_text(&self) -> String {
        String::from_utf8_lossy(&self.stderr).into_owned()
    }

    /// The `env:<NAME>=<VALUE>` lines the stub printed.
    ///
    /// A variable the stub found unset is `<unset>`, so a test can tell an
    /// absent variable from a stub that never ran.
    pub fn env(&self) -> BTreeMap<String, String> {
        self.stdout_text()
            .lines()
            .filter_map(|line| line.strip_prefix("env:"))
            .filter_map(|line| line.split_once('='))
            .map(|(key, value)| (key.to_owned(), value.to_owned()))
            .collect()
    }

    /// The working directory the stub was started in.
    pub fn cwd(&self) -> Option<String> {
        self.stdout_text()
            .lines()
            .find_map(|line| line.strip_prefix("cwd:").map(str::to_owned))
    }

    /// The `argv:` lines the stub printed, as raw bytes.
    pub fn argv(&self) -> Vec<Vec<u8>> {
        self.stdout
            .split(|byte| *byte == b'\n')
            .filter_map(|line| line.strip_prefix(b"argv:".as_slice()))
            .map(<[u8]>::to_vec)
            .collect()
    }

    /// The `argv:` lines, lossily converted, for a readable assertion.
    pub fn argv_text(&self) -> Vec<String> {
        self.argv()
            .iter()
            .map(|argument| String::from_utf8_lossy(argument).into_owned())
            .collect()
    }
}

/// One line of a `GINARY_TRACE` file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraceRecord {
    /// Microseconds since the recorder was built.
    pub t_us: u128,
    /// The phase name.
    pub phase: String,
    /// The phase's facts.
    pub kv: BTreeMap<String, String>,
    /// How long the phase took, when it was a phase rather than a fact.
    pub elapsed_us: Option<u128>,
}

/// Reads a `GINARY_TRACE` file as records, failing on anything that is not
/// JSON Lines of the documented shape.
///
/// # Panics
///
/// If the file cannot be read, if a line is not a JSON object, or if an object
/// is missing `t_us`, `phase` or `kv`. The trace is a documented format and a
/// test that shrugged at a malformed line would be a test that stopped
/// checking it.
pub fn read_trace(path: &Path) -> Vec<TraceRecord> {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("cannot read the trace {}: {error}", path.display()));
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let value: serde_json::Value = serde_json::from_str(line)
                .unwrap_or_else(|error| panic!("the trace line `{line}` is not JSON: {error}"));
            let Some(object) = value.as_object() else {
                panic!("the trace line `{line}` is not a JSON object");
            };
            let Some(t_us) = object.get("t_us").and_then(serde_json::Value::as_u64) else {
                panic!("the trace line `{line}` has no `t_us`");
            };
            let Some(phase) = object.get("phase").and_then(serde_json::Value::as_str) else {
                panic!("the trace line `{line}` has no `phase`");
            };
            let Some(kv) = object.get("kv").and_then(serde_json::Value::as_object) else {
                panic!("the trace line `{line}` has no `kv`");
            };
            TraceRecord {
                t_us: u128::from(t_us),
                phase: phase.to_owned(),
                kv: kv
                    .iter()
                    .map(|(key, value)| {
                        let Some(value) = value.as_str() else {
                            panic!("the trace line `{line}` has a non-string value for `{key}`");
                        };
                        (key.clone(), value.to_owned())
                    })
                    .collect(),
                elapsed_us: object
                    .get("elapsed_us")
                    .and_then(serde_json::Value::as_u64)
                    .map(u128::from),
            }
        })
        .collect()
}

/// The names of the phases a trace holds, in the order they were written.
pub fn trace_phases(records: &[TraceRecord]) -> Vec<String> {
    records
        .iter()
        .map(|record| record.phase.clone())
        .collect::<Vec<_>>()
}

/// Every path under `root`, relative and `/`-separated, sorted; directories
/// carry a trailing `/`.
pub fn tree(root: &Path) -> Vec<String> {
    let mut found = Vec::new();
    walk(root, root, &mut found);
    found.sort();
    found
}

fn walk(root: &Path, dir: &Path, found: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .into_owned();
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.is_dir() {
            found.push(format!("{relative}/"));
            walk(root, &path, found);
        } else {
            found.push(relative);
        }
    }
}

/// The names directly under `dir`, sorted.
///
/// # Panics
///
/// If `dir` cannot be listed.
pub fn names_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("cannot list {}: {error}", dir.display()))
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// A file's permission bits.
///
/// # Panics
///
/// If the file cannot be stat'd.
pub fn mode_of(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::symlink_metadata(path)
        .unwrap_or_else(|error| panic!("cannot stat {}: {error}", path.display()))
        .permissions()
        .mode()
        & 0o7777
}

// ------------------------------------------------------- the staged tree --

/// The manifest [`stage`] writes a staging root for.
///
/// It is public because two things need the same value: the artifact builder,
/// which packs it, and `tests/launch.rs`, whose whole subject is the plan this
/// manifest produces. A launch plan asserted against one manifest and a
/// running artifact asserted against another would agree by luck.
pub fn canonical_manifest() -> Manifest {
    Manifest {
        format_version: ginary::manifest::FORMAT_VERSION,
        app: APP.to_owned(),
        app_version: "1.2.3".to_owned(),
        gleam_version: Some("1.18.1".to_owned()),
        otp_release: OTP_RELEASE,
        otp_version: OTP_VERSION.to_owned(),
        erts_version: ERTS_VSN.to_owned(),
        target: "linux-x86_64-gnu"
            .parse::<Target>()
            .unwrap_or_else(|error| panic!("the host target must parse: {error}")),
        otp_applications: vec![AppRef {
            name: "stdlib".to_owned(),
            vsn: "8.0.3".to_owned(),
        }],
        gleam_applications: vec![APP.to_owned()],
        launch: LaunchSpec {
            program: "erlexec".to_owned(),
            bindir: format!("erts-{ERTS_VSN}/bin"),
            boot: "bin/no_dot_erlang".to_owned(),
            pa: vec![
                format!("lib/{APP}/ebin"),
                "lib/stdlib-8.0.3/ebin".to_owned(),
            ],
            eval: format!("'{APP}@@main':run('{APP}')"),
            // Not `+fnu`: the filename encoding is its own launch field now
            // and the launcher builds that flag itself, so a manifest flag
            // that happened to be the same word could not tell the two apart.
            erl_flags: vec!["+SDio".to_owned(), "4".to_owned()],
            args_file: None,
            config: None,
            distribution: false,
            filename_encoding: ginary::config::DEFAULT_FILENAME_ENCODING.to_owned(),
            heart: false,
            env: BTreeMap::new(),
        },
        native: Vec::new(),
        created_at: "2026-08-31T00:00:00Z".to_owned(),
        ginary_version: "0.1.0".to_owned(),
        extra: BTreeMap::new(),
    }
}

/// Writes the staging root and its `ginary.stage.json`.
///
/// Public because `tests/launch.rs` checks `preflight` against the same tree
/// the artifact carries, with pieces taken out of it.
///
/// # Panics
///
/// If the tree cannot be written.
pub fn stage(root: &Path, options: &ArtifactOptions) -> StageListing {
    let bindir = format!("erts-{ERTS_VSN}/bin");
    let contents: Vec<(String, u32, Vec<u8>, Category)> = vec![
        (
            "bin/no_dot_erlang.boot".to_owned(),
            0o644,
            b"boot script bytes".to_vec(),
            Category::Boot,
        ),
        (
            format!("{bindir}/erlexec"),
            0o755,
            ERLEXEC_STUB.as_bytes().to_vec(),
            Category::ErtsBinary,
        ),
        (
            format!("{bindir}/beam.smp"),
            0o755,
            OTHER_BIN_STUB.as_bytes().to_vec(),
            Category::ErtsBinary,
        ),
        (
            format!("{bindir}/erl_child_setup"),
            0o755,
            OTHER_BIN_STUB.as_bytes().to_vec(),
            Category::ErtsBinary,
        ),
        (
            format!("{bindir}/inet_gethost"),
            0o755,
            OTHER_BIN_STUB.as_bytes().to_vec(),
            Category::ErtsBinary,
        ),
        (
            format!("lib/{APP}/ebin/{APP}.app"),
            0o644,
            format!("{{application, {APP}, [{{vsn, \"1.2.3\"}}]}}.\n").into_bytes(),
            Category::AppResource,
        ),
        (
            format!("lib/{APP}/ebin/{APP}.beam"),
            0o644,
            b"FOR1\0\0\0\x04BEAM".to_vec(),
            Category::GleamBeam,
        ),
        (
            format!("lib/{APP}/priv/greeting.txt"),
            0o644,
            b"hello, world\n".to_vec(),
            Category::Priv,
        ),
        (
            "lib/stdlib-8.0.3/ebin/stdlib.app".to_owned(),
            0o644,
            b"{application, stdlib, [{vsn, \"8.0.3\"}]}.\n".to_vec(),
            Category::AppResource,
        ),
    ];

    let mut contents = contents;
    for name in &options.erts_bins {
        contents.push((
            format!("{bindir}/{name}"),
            0o755,
            OTHER_BIN_STUB.as_bytes().to_vec(),
            Category::ErtsBinary,
        ));
    }
    contents.extend(options.extra_files.iter().cloned());

    let mut files = Vec::new();
    for (path, mode, data, category) in contents {
        if options.omit.contains(&path) {
            continue;
        }
        let full = root.join(&path);
        let Some(parent) = full.parent() else {
            panic!("{path} has no parent");
        };
        std::fs::create_dir_all(parent)
            .unwrap_or_else(|error| panic!("cannot create {}: {error}", parent.display()));
        std::fs::write(&full, &data)
            .unwrap_or_else(|error| panic!("cannot write {}: {error}", full.display()));
        set_mode(&full, mode);
        files.push(StagedFile {
            path,
            size: data.len() as u64,
            mode,
            category,
        });
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));

    let apps = vec![
        StagedApp {
            name: APP.to_owned(),
            vsn: "1.2.3".to_owned(),
            source: StagedSource::Shipment,
            dir: format!("lib/{APP}"),
            files: files
                .iter()
                .filter(|file| file.path.starts_with(&format!("lib/{APP}/")))
                .count(),
            bytes: files
                .iter()
                .filter(|file| file.path.starts_with(&format!("lib/{APP}/")))
                .map(|file| file.size)
                .sum(),
        },
        StagedApp {
            name: "stdlib".to_owned(),
            vsn: "8.0.3".to_owned(),
            source: StagedSource::Otp,
            dir: "lib/stdlib-8.0.3".to_owned(),
            files: files
                .iter()
                .filter(|file| file.path.starts_with("lib/stdlib-8.0.3/"))
                .count(),
            bytes: files
                .iter()
                .filter(|file| file.path.starts_with("lib/stdlib-8.0.3/"))
                .map(|file| file.size)
                .sum(),
        },
    ];

    let listing = StageListing {
        erts_vsn: ERTS_VSN.to_owned(),
        otp_release: OTP_RELEASE,
        otp_version: OTP_VERSION.to_owned(),
        apps,
        files,
    };
    let json = serde_json::to_string_pretty(&listing)
        .unwrap_or_else(|error| panic!("cannot serialise the listing: {error}"));
    std::fs::write(root.join("ginary.stage.json"), format!("{json}\n"))
        .unwrap_or_else(|error| panic!("cannot write the listing: {error}"));
    listing
}

fn set_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .unwrap_or_else(|error| panic!("cannot chmod {}: {error}", path.display()));
}

fn write_executable(path: &Path, bytes: &[u8]) {
    // Written to a sibling and renamed: a test that spawns the artifact while
    // another thread still holds a write descriptor on the same inode gets
    // ETXTBSY, and a rename gives every spawn a file nobody is writing.
    let temporary = path.with_extension("writing");
    {
        let mut file = std::fs::File::create(&temporary)
            .unwrap_or_else(|error| panic!("cannot create {}: {error}", temporary.display()));
        file.write_all(bytes)
            .unwrap_or_else(|error| panic!("cannot write {}: {error}", temporary.display()));
    }
    set_mode(&temporary, 0o755);
    std::fs::rename(&temporary, path)
        .unwrap_or_else(|error| panic!("cannot rename onto {}: {error}", path.display()));
}

/// The bytes of a path, for an assertion about an argument that is not valid
/// UTF-8.
pub fn path_bytes(path: &Path) -> Vec<u8> {
    path.as_os_str().as_bytes().to_vec()
}
