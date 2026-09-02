// SPDX-License-Identifier: MIT OR Apache-2.0
//! The launcher's error type and the exit codes it maps to.
//!
//! A packaged application must never be confused with the application it
//! packages. Every failure that belongs to ginary rather than to the Gleam
//! program therefore leaves a numbered exit code between [`EXIT_SELF_EXE`] and
//! [`EXIT_EXEC`], and a one-line diagnostic on standard error that begins
//! `ginary: `. An operator who sees `121` knows the artifact never got as far
//! as reading itself; one who sees `125` knows the runtime was there and would
//! not start.
//!
//! | code | meaning |
//! |---|---|
//! | 121 | the running executable could not be opened, or ginary panicked |
//! | 122 | the trailer is unusable, or the manifest is a format this build does not read |
//! | 123 | the payload is corrupt |
//! | 124 | the cache could not be written or read |
//! | 125 | the runtime would not start |
//!
//! An error that a user can act on carries a [`LauncherError::hint`], rendered
//! by [`LauncherError::report`] as a second line beginning `hint: `. Nothing
//! else is printed: the launcher has no verbosity setting on the failure path,
//! because the failure path is the one place it must be predictable.
//!
//! See `docs/dev/debugging.md` for the operator-facing table and
//! `docs/adr/0008-launcher-exit-codes-and-env-protocol.md` for why the codes
//! start at 121.

use std::io::Write as _;
use std::path::PathBuf;

use crate::manifest::ManifestError;
use crate::payload::PayloadError;
use crate::trailer::TrailerError;

/// The running executable could not be opened, or ginary panicked.
pub const EXIT_SELF_EXE: u8 = 121;
/// The trailer, or the manifest's format version, is not one this build reads.
pub const EXIT_TRAILER: u8 = 122;
/// The payload is corrupt.
pub const EXIT_PAYLOAD: u8 = 123;
/// The cache could not be written or read.
pub const EXIT_CACHE: u8 = 124;
/// The runtime would not start.
pub const EXIT_EXEC: u8 = 125;
/// A panic on the launcher path. The same number as [`EXIT_SELF_EXE`], because
/// both mean the artifact never reached the application.
pub const EXIT_INTERNAL: u8 = 121;

/// The prefix every launcher diagnostic carries.
pub const PREFIX: &str = "ginary: ";

/// What [`LauncherError::report`] puts in front of a hint.
pub const HINT_PREFIX: &str = "hint: ";

/// The hint for an `ENOENT` from a program that is on disk.
pub const HINT_EXEC_ENOENT: &str = "the runtime is dynamically linked against glibc; the \
                                    interpreter it names (ld-linux) or one of its libraries is \
                                    missing on this machine";

/// The hint for an `EACCES` on a program whose execute bit is set.
pub const HINT_EXEC_EACCES: &str = "the cache filesystem may be mounted noexec; set \
                                    GINARY_CACHE_DIR to a directory programs may run from";

/// Every way a packaged application can fail before it reaches the Gleam code.
///
/// The variants are deliberately not [`Clone`] or [`PartialEq`]: each one
/// carries the cause it was built from, and a test asserts on the variant and
/// its fields rather than on the whole value.
#[derive(Debug)]
pub enum LauncherError {
    /// `/proc/self/exe` and [`std::env::current_exe`] both failed.
    SelfExe(std::io::Error),
    /// The last 64 bytes are not a trailer this build can use.
    Trailer(TrailerError),
    /// The payload does not unpack, or does not hash to what the trailer says.
    Payload(PayloadError),
    /// The cache could not be written or read.
    Cache {
        /// The path the operation was on.
        path: PathBuf,
        /// What the operation failed with.
        source: std::io::Error,
    },
    /// `execve` did not replace the process.
    Exec {
        /// The program that would not start.
        program: PathBuf,
        /// What `execve` failed with.
        source: std::io::Error,
        /// The advice, if this failure has any.
        hint: Option<&'static str>,
    },
}

impl LauncherError {
    /// Builds a [`LauncherError::Cache`] from a path and an I/O failure.
    pub fn cache(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Cache {
            path: path.into(),
            source,
        }
    }

    /// The process exit code this failure leaves.
    ///
    /// A payload error that is really a manifest this build cannot use is
    /// [`EXIT_TRAILER`] rather than [`EXIT_PAYLOAD`]: the bytes are intact and
    /// the *format* is the problem, which is the same fault the trailer's own
    /// version byte reports. Every other payload failure is corruption.
    pub fn exit_code(&self) -> u8 {
        match self {
            Self::SelfExe(_) => EXIT_SELF_EXE,
            Self::Trailer(_) | Self::Payload(PayloadError::Manifest(_)) => EXIT_TRAILER,
            Self::Payload(_) => EXIT_PAYLOAD,
            Self::Cache { .. } => EXIT_CACHE,
            Self::Exec { .. } => EXIT_EXEC,
        }
    }

    /// The advice this failure carries, if any.
    pub fn hint(&self) -> Option<&'static str> {
        match self {
            Self::Exec { hint, .. } => *hint,
            Self::SelfExe(_) | Self::Trailer(_) | Self::Payload(_) | Self::Cache { .. } => None,
        }
    }

    /// The whole diagnostic: the message, and a `hint: ` line when there is one.
    pub fn report(&self) -> String {
        match self.hint() {
            Some(hint) => format!("{self}\n{HINT_PREFIX}{hint}"),
            None => self.to_string(),
        }
    }
}

/// One line for an error and every cause behind it, joined with `: `.
///
/// [`TrailerError`] and [`PayloadError`] name the *layer* that failed and keep
/// the detail in a [`std::error::Error::source`], which is the right shape for
/// a library and the wrong one for a launcher: `the payload's manifest cannot
/// be used by this ginary` does not tell an operator which version they have.
/// The launcher has one line in which to say everything, so it says the chain.
fn chain(error: &dyn std::error::Error) -> String {
    let mut rendered = error.to_string();
    let mut cause = error.source();
    while let Some(next) = cause {
        rendered.push_str(": ");
        rendered.push_str(&next.to_string());
        cause = next.source();
    }
    rendered
}

impl std::fmt::Display for LauncherError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(PREFIX)?;
        match self {
            Self::SelfExe(source) => {
                write!(f, "cannot open the running executable: {source}")
            }
            Self::Trailer(source) => f.write_str(&chain(source)),
            Self::Payload(source) => f.write_str(&chain(source)),
            Self::Cache { path, source } => write!(
                f,
                "the runtime cache at {} is unusable: {source}",
                path.display()
            ),
            Self::Exec {
                program, source, ..
            } => write!(f, "cannot start {}: {source}", program.display()),
        }
    }
}

impl std::error::Error for LauncherError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::SelfExe(source) | Self::Cache { source, .. } | Self::Exec { source, .. } => {
                Some(source)
            }
            Self::Trailer(source) => Some(source),
            Self::Payload(source) => Some(source),
        }
    }
}

impl From<TrailerError> for LauncherError {
    fn from(error: TrailerError) -> Self {
        Self::Trailer(error)
    }
}

impl From<PayloadError> for LauncherError {
    fn from(error: PayloadError) -> Self {
        Self::Payload(error)
    }
}

impl From<ManifestError> for LauncherError {
    fn from(error: ManifestError) -> Self {
        Self::Payload(PayloadError::Manifest(error))
    }
}

/// Installs the launcher's panic hook.
///
/// The launcher promises never to panic, and a promise with no evidence behind
/// it is a promise a user finds out about through a Rust backtrace. The hook
/// turns a panic into the same shape as every other launcher failure — one
/// line beginning `ginary: internal error (this is a bug in ginary): ` — and
/// exits [`EXIT_INTERNAL`], so the artifact never prints a backtrace at a user
/// and never returns an exit code the application could have produced.
///
/// It is installed on the launcher path only. The command line half is a
/// developer tool and its panics are worth seeing in full.
pub fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        // `PanicHookInfo::payload_as_str` would say this in one call and is
        // newer than this crate's minimum supported Rust version.
        let payload = info.payload();
        let message = payload
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
            .unwrap_or("a panic whose payload is not a string");
        let _ = writeln!(std::io::stderr(), "{}", panic_line(message));
        std::process::exit(i32::from(EXIT_INTERNAL));
    }));
}

/// The single line the panic hook prints.
fn panic_line(payload: &str) -> String {
    format!("{PREFIX}internal error (this is a bug in ginary): {payload}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn io(code: i32) -> std::io::Error {
        std::io::Error::from_raw_os_error(code)
    }

    #[test]
    fn opening_the_running_executable_is_121() {
        let error = LauncherError::SelfExe(io(2));
        assert_eq!(error.exit_code(), 121);
        assert_eq!(
            error.to_string(),
            "ginary: cannot open the running executable: No such file or directory (os error 2)"
        );
    }

    #[test]
    fn a_trailer_failure_is_122() {
        let error = LauncherError::Trailer(TrailerError::Geometry {
            expected: 4096,
            actual: 4095,
        });
        assert_eq!(error.exit_code(), 122);
        assert_eq!(
            error.to_string(),
            "ginary: the trailer says the file is 4096 bytes long and it is 4095, so it was \
             truncated or something was appended to it"
        );
    }

    #[test]
    fn a_payload_failure_is_123() {
        let error = LauncherError::Payload(PayloadError::ChecksumMismatch {
            expected: "aa".to_owned(),
            actual: "bb".to_owned(),
        });
        assert_eq!(error.exit_code(), 123);
        assert_eq!(
            error.to_string(),
            "ginary: the payload hashes to bb and the trailer says aa"
        );
    }

    #[test]
    fn a_manifest_version_this_build_does_not_read_is_122_and_not_123() {
        // The bytes are intact; the format is the problem, and that is the same
        // fault the trailer's version byte reports.
        let error =
            LauncherError::Payload(PayloadError::Manifest(ManifestError::UnsupportedVersion {
                found: 2,
                supported: 1,
            }));
        assert_eq!(error.exit_code(), 122);
    }

    #[test]
    fn a_cache_failure_is_124_and_names_the_path() {
        let error = LauncherError::cache("/var/cache/ginary/hello", io(13));
        assert_eq!(error.exit_code(), 124);
        assert_eq!(
            error.to_string(),
            "ginary: the runtime cache at /var/cache/ginary/hello is unusable: Permission denied \
             (os error 13)"
        );
    }

    #[test]
    fn an_exec_failure_is_125_and_names_the_program() {
        let error = LauncherError::Exec {
            program: PathBuf::from("/c/hello/k/erts-17.0.5/bin/erlexec"),
            source: io(2),
            hint: None,
        };
        assert_eq!(error.exit_code(), 125);
        assert_eq!(
            error.to_string(),
            "ginary: cannot start /c/hello/k/erts-17.0.5/bin/erlexec: No such file or directory \
             (os error 2)"
        );
    }

    #[test]
    fn every_message_begins_with_the_ginary_prefix() {
        let errors = [
            LauncherError::SelfExe(io(2)),
            LauncherError::Trailer(TrailerError::EmptyPayload),
            LauncherError::Payload(PayloadError::UnsafePath {
                path: "../x".to_owned(),
            }),
            LauncherError::cache("/c", io(13)),
            LauncherError::Exec {
                program: PathBuf::from("/p"),
                source: io(13),
                hint: None,
            },
        ];
        for error in &errors {
            let rendered = error.to_string();
            assert!(
                rendered.starts_with(PREFIX),
                "`{rendered}` does not begin with `{PREFIX}`"
            );
            assert!(
                !rendered.contains('\n'),
                "`{rendered}` is more than one line"
            );
        }
    }

    #[test]
    fn a_hint_is_rendered_as_a_second_line() {
        let error = LauncherError::Exec {
            program: PathBuf::from("/p"),
            source: io(13),
            hint: Some(HINT_EXEC_EACCES),
        };
        assert_eq!(error.hint(), Some(HINT_EXEC_EACCES));
        assert_eq!(
            error.report(),
            format!("{error}\n{HINT_PREFIX}{HINT_EXEC_EACCES}")
        );
    }

    #[test]
    fn an_error_without_a_hint_reports_one_line() {
        let error = LauncherError::cache("/c", io(13));
        assert_eq!(error.hint(), None);
        assert_eq!(error.report(), error.to_string());
    }

    #[test]
    fn every_variant_reports_its_cause_as_the_error_source() {
        use std::error::Error as _;

        // Each variant that wraps a cause must expose it through `source`, so a
        // caller that walks the chain reaches the underlying failure rather
        // than stopping at the launcher's own message.
        let self_exe = LauncherError::SelfExe(io(2));
        assert!(
            self_exe.source().is_some(),
            "a self-exe failure must carry its io cause"
        );

        let trailer = LauncherError::Trailer(TrailerError::EmptyPayload);
        assert!(
            trailer.source().is_some(),
            "a trailer failure must carry its trailer cause"
        );

        let payload = LauncherError::Payload(PayloadError::UnsafePath {
            path: "../x".to_owned(),
        });
        assert!(
            payload.source().is_some(),
            "a payload failure must carry its payload cause"
        );

        let cache = LauncherError::cache("/c", io(13));
        assert!(
            cache.source().is_some(),
            "a cache failure must carry its io cause"
        );

        let exec = LauncherError::Exec {
            program: PathBuf::from("/p"),
            source: io(2),
            hint: None,
        };
        assert!(
            exec.source().is_some(),
            "an exec failure must carry its io cause"
        );
    }

    #[test]
    fn the_panic_hook_line_names_the_bug_and_the_message() {
        assert_eq!(
            panic_line("index out of bounds"),
            "ginary: internal error (this is a bug in ginary): index out of bounds"
        );
    }

    #[test]
    fn the_five_codes_are_the_documented_ones() {
        assert_eq!(
            [
                EXIT_SELF_EXE,
                EXIT_TRAILER,
                EXIT_PAYLOAD,
                EXIT_CACHE,
                EXIT_EXEC
            ],
            [121, 122, 123, 124, 125]
        );
        assert_eq!(EXIT_INTERNAL, EXIT_SELF_EXE);
    }
}
