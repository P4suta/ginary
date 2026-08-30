// SPDX-License-Identifier: MIT OR Apache-2.0
//! Finding an OTP installation and deciding whether ginary can use it.
//!
//! There are two ways in: ask the `erl` on `PATH` where it lives
//! ([`discover`] with no override), or point at a directory
//! ([`inspect_root`]). Both end in the same place, because [`inspect_root`] is
//! the only thing that decides whether a tree is usable. Whatever the ERTS came
//! from — the host, a tarball, the catalogue, a container — it is judged by
//! what is actually on disk, never by the metadata that came with it.
//!
//! The checks are the ones the launcher will depend on at run time: the four
//! ERTS binaries exist and are executable, `bin/no_dot_erlang.boot` is there,
//! and there is exactly one `kernel` and one `stdlib` in `lib/`.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::process::{NULL_DEVICE, ProcessOutput, find_in_path, run_with_timeout};

/// The oldest OTP release ginary will package.
///
/// OTP 26 is where `no_dot_erlang.boot` and the modern `erlexec` layout can be
/// relied on. Older releases are rejected rather than half-supported.
pub const MIN_RELEASE: u32 = 26;

/// How long the `erl` probe in [`discover`] may run before it is killed.
pub const DISCOVER_TIMEOUT: Duration = Duration::from_secs(20);

/// The programs the launcher execs or the emulator forks, in report order.
///
/// Every one of them must exist under `erts-<vsn>/bin` and be executable.
pub const REQUIRED_ERTS_BINARIES: [&str; 4] =
    ["beam.smp", "erlexec", "erl_child_setup", "inet_gethost"];

/// The Erlang program [`discover`] runs, printing three lines: the code root,
/// the OTP release and the ERTS version.
pub const DISCOVER_EVAL: &str = "io:format(\"~ts~n~ts~n~ts~n\",[code:root_dir(),erlang:system_info(otp_release),erlang:system_info(version)]),halt(0).";

/// A usable OTP installation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OtpInfo {
    /// The code root, the directory holding `bin`, `erts-*`, `lib`, `releases`.
    pub root: PathBuf,
    /// The major release, for example `29`.
    pub release: u32,
    /// The ERTS version, for example `17.0.5`.
    pub erts_vsn: String,
    /// The full version, for example `29.0.5`, or the release string when the
    /// installation carries no `OTP_VERSION` file.
    pub otp_version: String,
    /// `<root>/erts-<erts_vsn>/bin`.
    pub erts_bin: PathBuf,
    /// `<root>/lib`.
    pub lib: PathBuf,
}

/// Why an OTP installation could not be found or could not be used.
#[derive(Debug, thiserror::Error)]
pub enum OtpError {
    /// A path under the root could not be read.
    #[error("cannot read `{path}`: {source}")]
    Io {
        /// The path that could not be read.
        path: PathBuf,
        /// The underlying operating system error.
        #[source]
        source: std::io::Error,
    },
    /// `erl` is not on `PATH`.
    #[error("`erl` is not on PATH; install Erlang/OTP or pass an OTP root explicitly")]
    ErlNotFound,
    /// `erl` ran but did not print the three expected lines.
    #[error("`erl` did not report its code root; it printed: {output}")]
    ErlOutput {
        /// What the probe printed, for the report.
        output: String,
    },
    /// `erl` could not be run, or did not exit in time.
    #[error("cannot ask `erl` where it is installed: {message}")]
    ErlFailed {
        /// The underlying failure, already rendered.
        message: String,
    },
    /// The root is not a directory, or is not there at all.
    ///
    /// Distinct from [`OtpError::NoErts`] on purpose: a mistyped override path
    /// is the likeliest mistake of the two, and "has no `erts-*` directory" is
    /// the wrong thing to say about a directory that does not exist.
    #[error("`{root}` is not a directory, so it cannot be an OTP installation")]
    NoSuchRoot {
        /// The path that was given as a root.
        root: PathBuf,
    },
    /// The root holds no `erts-*` directory.
    #[error("`{root}` has no `erts-*` directory, so it is not an OTP installation")]
    NoErts {
        /// The root that was inspected.
        root: PathBuf,
    },
    /// The root holds more than one `erts-*` directory.
    #[error("`{root}` has {} `erts-*` directories ({}); ginary needs exactly one", .found.len(), .found.join(", "))]
    AmbiguousErts {
        /// The root that was inspected.
        root: PathBuf,
        /// The directory names found, sorted.
        found: Vec<String>,
    },
    /// One of [`REQUIRED_ERTS_BINARIES`] is missing.
    #[error("`{path}` is missing; the OTP installation is incomplete")]
    MissingErtsBinary {
        /// The binary that should have been there.
        path: PathBuf,
    },
    /// One of [`REQUIRED_ERTS_BINARIES`] is present but not executable.
    #[error("`{path}` is not executable; run `chmod +x` on it or reinstall Erlang/OTP")]
    ErtsBinaryNotExecutable {
        /// The binary that cannot be run.
        path: PathBuf,
    },
    /// `bin/no_dot_erlang.boot` is missing.
    #[error("`{path}` is missing; ginary boots the packaged runtime with it")]
    MissingBootFile {
        /// Where the boot file should have been.
        path: PathBuf,
    },
    /// `lib/` holds no versioned directory for a required application.
    #[error("`{root}/lib` has no `{name}-<version>` directory; the OTP installation is incomplete")]
    MissingLibApp {
        /// The root that was inspected.
        root: PathBuf,
        /// The application that is missing.
        name: &'static str,
    },
    /// `lib/` holds more than one versioned directory for an application.
    #[error("`{root}/lib` has {} `{name}-<version>` directories ({}); ginary needs exactly one", .found.len(), .found.join(", "))]
    AmbiguousLibApp {
        /// The root that was inspected.
        root: PathBuf,
        /// The application with several versions.
        name: &'static str,
        /// The directory names found, sorted.
        found: Vec<String>,
    },
    /// The release could not be read from `releases/`.
    #[error(
        "cannot tell which OTP release `{root}` is: `releases/start_erl.data` is unusable and `releases/` has no single numeric directory"
    )]
    UnknownRelease {
        /// The root that was inspected.
        root: PathBuf,
    },
    /// The installation is older than [`MIN_RELEASE`].
    #[error("OTP {release} is too old; ginary needs OTP {minimum} or newer")]
    ReleaseTooOld {
        /// The release that was found.
        release: u32,
        /// [`MIN_RELEASE`].
        minimum: u32,
    },
}

/// The applications an installation must hold before ginary will use it.
///
/// Every OTP release needs both to boot at all, so their absence is a broken
/// tree rather than a slimmed-down one.
const REQUIRED_LIB_APPS: [&str; 2] = ["kernel", "stdlib"];

/// Finds the OTP installation ginary should package.
///
/// With `override_root`, that directory is inspected and `erl` is never run.
/// Without it, the `erl` on `PATH` is asked for its code root, its release and
/// its ERTS version, under a [`DISCOVER_TIMEOUT`] budget, and the root it
/// reports is then inspected exactly as an override would be.
///
/// # Errors
///
/// [`OtpError::ErlNotFound`], [`OtpError::ErlFailed`] or
/// [`OtpError::ErlOutput`] when the probe cannot answer, and whatever
/// [`inspect_root`] says about the root it named.
pub fn discover(override_root: Option<&Path>) -> Result<OtpInfo, OtpError> {
    match override_root {
        Some(root) => inspect_root(root),
        None => inspect_root(&ask_erl_for_its_root()?),
    }
}

/// Runs [`DISCOVER_EVAL`] under the `erl` on `PATH` and returns its code root.
///
/// The probe prints three lines and all three are checked, but only the root is
/// carried out of here: the release and the ERTS version are read back from the
/// tree by [`inspect_root`], which is the single point of truth about a
/// runtime. Requiring them anyway is what makes this a probe rather than a
/// `code:root_dir()` call — a wrapper script that prints one path cannot pass
/// for an emulator.
fn ask_erl_for_its_root() -> Result<PathBuf, OtpError> {
    let erl = find_erl(std::env::var_os("PATH").as_deref())?;
    probe_root(&erl, DISCOVER_TIMEOUT)
}

/// Locates the `erl` a probe would run, given a `PATH` value.
///
/// Split out so that the "no Erlang at all" answer can be tested without
/// changing the process environment.
///
/// # Errors
///
/// [`OtpError::ErlNotFound`] when no entry of `path_var` holds an executable
/// `erl`.
fn find_erl(path_var: Option<&OsStr>) -> Result<PathBuf, OtpError> {
    find_in_path("erl", path_var).ok_or(OtpError::ErlNotFound)
}

/// Runs [`DISCOVER_EVAL`] under one specific `erl` and returns the root.
///
/// Takes the program and the budget rather than reading `PATH` and a constant,
/// which is what lets the failure paths — a program that says nothing, one that
/// prints one line, one whose release is not a number, one that never exits —
/// be tested against stub programs.
///
/// # Errors
///
/// [`OtpError::ErlFailed`] when the program cannot be run or outlives
/// `timeout`, and [`OtpError::ErlOutput`] when it runs but does not answer.
fn probe_root(erl: &Path, timeout: Duration) -> Result<PathBuf, OtpError> {
    let output = run_with_timeout(
        erl,
        &[
            "-noshell",
            // A broken installation can dump core on start-up, and a probe must
            // not leave an `erl_crash.dump` in the user's working directory.
            "-env",
            "ERL_CRASH_DUMP",
            NULL_DEVICE,
            "-eval",
            DISCOVER_EVAL,
        ],
        timeout,
    )
    .map_err(|error| OtpError::ErlFailed {
        message: error.to_string(),
    })?;

    if !output.success {
        return Err(OtpError::ErlOutput {
            output: describe_output(&output),
        });
    }

    let lines: Vec<&str> = output
        .stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    let [root, release, erts_vsn] = lines.as_slice() else {
        return Err(OtpError::ErlOutput {
            output: describe_output(&output),
        });
    };
    if root.is_empty() || release.parse::<u32>().is_err() || erts_vsn.is_empty() {
        return Err(OtpError::ErlOutput {
            output: describe_output(&output),
        });
    }
    Ok(PathBuf::from(root))
}

/// The most a report can quote of what a probe printed.
///
/// A misconfigured `erl` can print pages; the message is a sentence in a
/// report, so it is cut and the cut is marked.
const MAX_QUOTED_OUTPUT: usize = 400;

/// Quotes what a probe printed, on either stream, for an error message.
///
/// Standard error is included because a program that fails writes its diagnosis
/// there and nothing at all on standard output: quoting only standard output
/// would end the sentence "it printed:" with nothing after it.
fn describe_output(output: &ProcessOutput) -> String {
    let stdout = truncate(output.stdout.trim());
    let stderr = truncate(output.stderr.trim());
    match (stdout.is_empty(), stderr.is_empty()) {
        (true, true) => "nothing at all".to_owned(),
        (false, true) => stdout,
        (true, false) => format!("nothing on standard output, and on standard error: {stderr}"),
        (false, false) => format!("{stdout} (and on standard error: {stderr})"),
    }
}

/// Cuts `text` to [`MAX_QUOTED_OUTPUT`] characters, marking the cut.
fn truncate(text: &str) -> String {
    match text.char_indices().nth(MAX_QUOTED_OUTPUT) {
        Some((end, _)) => format!("{}...", &text[..end]),
        None => text.to_owned(),
    }
}

/// Judges one directory: is it an OTP installation ginary can package?
///
/// This is the single point of truth about a runtime. Every field of the
/// returned [`OtpInfo`] is read from the tree, and every check that the
/// launcher will later depend on is made here, where the error can still name
/// the file and say what to do about it.
///
/// # Errors
///
/// One [`OtpError`] variant per way a tree can fail to be an installation: a
/// root that is not a directory at all, no
/// or several `erts-*` directories, a missing or non-executable ERTS binary, a
/// missing boot file, a missing or duplicated `kernel` or `stdlib`, a release
/// that cannot be read, and a release older than [`MIN_RELEASE`].
pub fn inspect_root(root: &Path) -> Result<OtpInfo, OtpError> {
    if !root.is_dir() {
        return Err(OtpError::NoSuchRoot {
            root: root.to_path_buf(),
        });
    }
    let erts_vsn = single_erts_version(root)?;
    let erts_bin = root.join(format!("erts-{erts_vsn}")).join("bin");
    check_erts_binaries(&erts_bin)?;
    check_boot_file(root)?;

    let lib = root.join("lib");
    let lib_dirs = directory_names(&lib)?;
    for name in REQUIRED_LIB_APPS {
        check_lib_app(root, &lib_dirs, name)?;
    }

    let release = read_release(root)?;
    if release < MIN_RELEASE {
        return Err(OtpError::ReleaseTooOld {
            release,
            minimum: MIN_RELEASE,
        });
    }
    let otp_version = read_otp_version(root, release);

    Ok(OtpInfo {
        root: root.to_path_buf(),
        release,
        erts_vsn,
        otp_version,
        erts_bin,
        lib,
    })
}

/// The version of the single `erts-*` directory under `root`.
fn single_erts_version(root: &Path) -> Result<String, OtpError> {
    let mut found: Vec<String> = directory_names(root)?
        .into_iter()
        .filter(|name| {
            name.strip_prefix("erts-")
                .is_some_and(|vsn| !vsn.is_empty())
        })
        .collect();
    found.sort();

    if found.len() > 1 {
        return Err(OtpError::AmbiguousErts {
            root: root.to_path_buf(),
            found,
        });
    }
    let Some(name) = found.first() else {
        return Err(OtpError::NoErts {
            root: root.to_path_buf(),
        });
    };
    Ok(name.strip_prefix("erts-").unwrap_or(name).to_owned())
}

/// Checks that every program the launcher needs is there and can be run.
fn check_erts_binaries(erts_bin: &Path) -> Result<(), OtpError> {
    for name in REQUIRED_ERTS_BINARIES {
        let path = erts_bin.join(name);
        let metadata = match std::fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(OtpError::MissingErtsBinary { path });
            }
            Err(source) => return Err(OtpError::Io { path, source }),
        };
        if !metadata.is_file() {
            return Err(OtpError::MissingErtsBinary { path });
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            if metadata.permissions().mode() & 0o111 == 0 {
                return Err(OtpError::ErtsBinaryNotExecutable { path });
            }
        }
    }
    Ok(())
}

/// Checks that `bin/no_dot_erlang.boot` is there.
///
/// It is the boot script ginary starts the packaged runtime with, so an
/// installation without it cannot be packaged even though `erl` runs.
fn check_boot_file(root: &Path) -> Result<(), OtpError> {
    let path = root.join("bin").join("no_dot_erlang.boot");
    if path.is_file() {
        Ok(())
    } else {
        Err(OtpError::MissingBootFile { path })
    }
}

/// Checks that `lib/` holds exactly one versioned directory for `name`.
fn check_lib_app(root: &Path, lib_dirs: &[String], name: &'static str) -> Result<(), OtpError> {
    let mut found: Vec<String> = lib_dirs
        .iter()
        .filter(|dir| is_versioned_dir(dir, name))
        .cloned()
        .collect();
    found.sort();

    match found.len() {
        0 => Err(OtpError::MissingLibApp {
            root: root.to_path_buf(),
            name,
        }),
        1 => Ok(()),
        _ => Err(OtpError::AmbiguousLibApp {
            root: root.to_path_buf(),
            name,
            found,
        }),
    }
}

/// Whether `dir` is `<name>-<version>` with a numeric version.
///
/// The version has to be numeric because a documentation install puts
/// `kernel-doc` next to `kernel-11.0.3`, and a `kernel-*` glob would call that
/// two copies of `kernel`.
fn is_versioned_dir(dir: &str, name: &str) -> bool {
    dir.strip_prefix(name)
        .and_then(|rest| rest.strip_prefix('-'))
        .is_some_and(is_version)
}

/// Whether `text` is a dotted run of digits, for example `11.0.3`.
fn is_version(text: &str) -> bool {
    !text.is_empty()
        && text
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

/// Reads the major release from `releases/`.
///
/// `start_erl.data` is `<erts version> <release>`, and its second field is the
/// answer when the file is there. Otherwise the single all-digits directory
/// under `releases/` is the release; `RELEASES` and `backup` sit next to it and
/// are neither directories nor digits.
fn read_release(root: &Path) -> Result<u32, OtpError> {
    let releases = root.join("releases");
    if let Some(release) = release_from_start_erl_data(&releases) {
        return Ok(release);
    }

    let numeric: Vec<u32> = directory_names(&releases)?
        .iter()
        .filter_map(|name| name.parse::<u32>().ok())
        .collect();
    match numeric.as_slice() {
        [release] => Ok(*release),
        _ => Err(OtpError::UnknownRelease {
            root: root.to_path_buf(),
        }),
    }
}

/// The release from the second field of `releases/start_erl.data`.
///
/// An unreadable or unparsable file is not an error here: the directory scan is
/// the documented fallback, and it gives a better message when it fails too.
fn release_from_start_erl_data(releases: &Path) -> Option<u32> {
    let text = std::fs::read_to_string(releases.join("start_erl.data")).ok()?;
    text.split_whitespace().nth(1)?.parse().ok()
}

/// Reads `releases/<release>/OTP_VERSION`, falling back to the release itself.
///
/// A source build or a trimmed installation may not carry the file, and `29` is
/// a true if less precise answer than `29.0.5`.
fn read_otp_version(root: &Path, release: u32) -> String {
    std::fs::read_to_string(
        root.join("releases")
            .join(release.to_string())
            .join("OTP_VERSION"),
    )
    .ok()
    .map(|text| text.trim().to_owned())
    .filter(|version| !version.is_empty())
    .unwrap_or_else(|| release.to_string())
}

/// The names of the directories directly under `dir`, unsorted.
///
/// A directory that is not there reads as empty, because every caller turns
/// that into a better error than "not found": no `erts-*`, no `kernel`, no
/// release. Any other failure is reported as it happened.
fn directory_names(dir: &Path) -> Result<Vec<String>, OtpError> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(OtpError::Io {
                path: dir.to_path_buf(),
                source,
            });
        }
    };

    let mut names = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| OtpError::Io {
            path: dir.to_path_buf(),
            source,
        })?;
        if entry.path().is_dir()
            && let Some(name) = entry.file_name().to_str()
        {
            names.push(name.to_owned());
        }
    }
    Ok(names)
}

/// Lists the `<name>-<version>` library directories a boot file refers to.
///
/// A boot script is an Erlang term encoded with `term_to_binary`, and the
/// library paths in it are plain byte strings of the form
/// `$ROOT/lib/<name>-<version>/ebin`. Scanning for that shape avoids decoding
/// the external term format, and it is enough for what the caller needs: the
/// set of applications the boot file will look for, so that `assemble` can
/// prove every one of them is actually staged.
///
/// The returned names are the `<name>-<version>` components, in order of first
/// appearance, without repeats.
pub fn boot_lib_dirs(boot: &[u8]) -> Vec<String> {
    const PREFIX: &[u8] = b"$ROOT/lib/";
    const SUFFIX: &[u8] = b"/ebin";

    let mut found: Vec<String> = Vec::new();
    let mut index = 0;
    while let Some(offset) = find(&boot[index..], PREFIX) {
        index += offset + PREFIX.len();
        let rest = &boot[index..];
        let Some(end) = rest.iter().position(|byte| *byte == b'/') else {
            continue;
        };
        if !rest[end..].starts_with(SUFFIX) {
            continue;
        }
        let Ok(dir) = std::str::from_utf8(&rest[..end]) else {
            continue;
        };
        if !dir
            .rsplit_once('-')
            .is_some_and(|(name, vsn)| !name.is_empty() && is_version(vsn))
        {
            continue;
        }
        if !found.iter().any(|held| held == dir) {
            found.push(dir.to_owned());
        }
    }
    found
}

/// The offset of the first occurrence of `needle` in `haystack`.
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.len() > haystack.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    use crate::process::test_support::script;

    /// A budget long enough for a stub script and short enough for a test.
    #[cfg(unix)]
    const TEST_TIMEOUT: Duration = Duration::from_secs(10);

    /// The error from probing `body` as an `erl`, which must fail.
    #[cfg(unix)]
    fn probe_failure(dir: &Path, body: &str) -> OtpError {
        let erl = script(dir, "erl", body);
        match probe_root(&erl, TEST_TIMEOUT) {
            Ok(root) => panic!(
                "`{body}` should not answer, but reported {}",
                root.display()
            ),
            Err(error) => error,
        }
    }

    #[test]
    fn an_erl_that_is_not_on_the_path_is_reported_as_absent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path_var = std::env::join_paths([dir.path()]).expect("join paths");
        let error = find_erl(Some(&path_var)).expect_err("an empty directory holds no erl");
        assert!(matches!(error, OtpError::ErlNotFound), "{error:?}");
        assert!(
            error.to_string().contains("`erl` is not on PATH"),
            "{error}"
        );
    }

    #[test]
    fn an_absent_path_variable_finds_no_erl() {
        let error = find_erl(None).expect_err("no PATH holds no erl");
        assert!(matches!(error, OtpError::ErlNotFound), "{error:?}");
    }

    #[cfg(unix)]
    #[test]
    fn a_probe_that_answers_all_three_lines_yields_the_root() {
        let dir = tempfile::tempdir().expect("tempdir");
        let erl = script(dir.path(), "erl", "printf '/opt/otp\\n29\\n17.0.5\\n'");
        let root = probe_root(&erl, TEST_TIMEOUT).expect("three lines is an answer");
        assert_eq!(root, PathBuf::from("/opt/otp"));
    }

    /// The probe demands all three fields even though it carries only the root
    /// out: a wrapper script that prints one path is not an emulator, and
    /// `inspect_root` would then be handed a root nothing vouched for.
    #[cfg(unix)]
    #[test]
    fn a_probe_that_prints_only_a_root_is_not_an_answer() {
        let dir = tempfile::tempdir().expect("tempdir");
        let error = probe_failure(dir.path(), "printf '/opt/otp\\n'");
        assert!(matches!(error, OtpError::ErlOutput { .. }), "{error:?}");
        assert!(error.to_string().contains("/opt/otp"), "{error}");
    }

    /// A non-zero exit means the emulator did not finish the program, so the
    /// three lines before it are not an answer however well formed they look.
    #[cfg(unix)]
    #[test]
    fn a_probe_that_fails_after_printing_an_answer_is_not_trusted() {
        let dir = tempfile::tempdir().expect("tempdir");
        let error = probe_failure(dir.path(), "printf '/opt/otp\n29\n17.0.5\n'; exit 1");
        assert!(matches!(error, OtpError::ErlOutput { .. }), "{error:?}");
    }

    #[cfg(unix)]
    #[test]
    fn a_probe_whose_release_is_not_a_number_is_not_an_answer() {
        let dir = tempfile::tempdir().expect("tempdir");
        let error = probe_failure(dir.path(), "printf '/opt/otp\\nRelease 29\\n17.0.5\\n'");
        assert!(matches!(error, OtpError::ErlOutput { .. }), "{error:?}");
        assert!(error.to_string().contains("Release 29"), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn a_probe_that_prints_nothing_at_all_says_so() {
        let dir = tempfile::tempdir().expect("tempdir");
        let error = probe_failure(dir.path(), "exit 0");
        assert_eq!(
            error.to_string(),
            "`erl` did not report its code root; it printed: nothing at all"
        );
    }

    /// Regression for the A1a review: a failing probe was reported with only
    /// its standard output, which a crashing `erl` leaves empty, so the message
    /// ended in a colon and nothing.
    #[cfg(unix)]
    #[test]
    fn a_failing_probe_is_reported_with_what_it_wrote_to_standard_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let error = probe_failure(dir.path(), "echo 'erlexec: not found' >&2; exit 127");
        assert!(matches!(error, OtpError::ErlOutput { .. }), "{error:?}");
        assert!(
            error.to_string().contains("erlexec: not found"),
            "the diagnosis on standard error must survive: {error}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_probe_that_never_exits_is_killed_and_reported() {
        let dir = tempfile::tempdir().expect("tempdir");
        let erl = script(dir.path(), "erl", "sleep 60");
        let started = std::time::Instant::now();
        let error = probe_root(&erl, Duration::from_millis(200))
            .expect_err("a probe that never exits cannot answer");
        assert!(matches!(error, OtpError::ErlFailed { .. }), "{error:?}");
        assert!(error.to_string().contains("did not exit within"), "{error}");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the budget must bound the call, not the child: {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn the_discover_budget_is_the_documented_twenty_seconds() {
        assert_eq!(DISCOVER_TIMEOUT, Duration::from_secs(20));
    }

    #[test]
    fn quoting_a_probe_covers_both_streams_and_is_bounded() {
        let both = ProcessOutput {
            success: false,
            stdout: "out\n".to_owned(),
            stderr: "err\n".to_owned(),
        };
        assert_eq!(describe_output(&both), "out (and on standard error: err)");

        let only_stderr = ProcessOutput {
            success: false,
            stdout: String::new(),
            stderr: "boom".to_owned(),
        };
        assert_eq!(
            describe_output(&only_stderr),
            "nothing on standard output, and on standard error: boom"
        );

        let silent = ProcessOutput {
            success: false,
            stdout: "  \n".to_owned(),
            stderr: String::new(),
        };
        assert_eq!(describe_output(&silent), "nothing at all");

        let chatty = ProcessOutput {
            success: false,
            stdout: "x".repeat(10_000),
            stderr: String::new(),
        };
        let quoted = describe_output(&chatty);
        assert_eq!(quoted.chars().count(), MAX_QUOTED_OUTPUT + 3);
        assert!(quoted.ends_with("..."), "{quoted}");
    }
}
