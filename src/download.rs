// SPDX-License-Identifier: MIT OR Apache-2.0
//! Fetching one file over HTTPS, with a checksum, a retry and an atomic
//! rename.
//!
//! Two things in a ginary build come off the network and neither may be
//! half-written: the prebuilt OTP tarball a catalogue entry names, and the
//! upstream release asset `ginary otp repack` verifies and re-packages. Both
//! go through [`fetch`], which streams the body into `<dest>.part-<pid>`
//! while hashing it, verifies what the caller expected, and only then renames
//! the file onto `dest`. A failure at any point removes the part file, so a
//! destination either does not exist or is the whole, verified file.
//!
//! [`Net`] carries the two things a fetch has to be told rather than read:
//! whether this build may talk to the network at all, and which bases have
//! been redirected at a mirror. `GINARY_OFFLINE=1` and `GINARY_GITHUB_BASE_URL`
//! are the two spellings, and [`Net::from_vars`] takes the variables rather
//! than reading the process environment, so the rules are testable in parallel.
//!
//! Proxies are the transport's business: the honoured variables are named in
//! [`PROXY_VARS`] and are read by the HTTP client itself, so nothing here
//! re-implements `NO_PROXY` matching. `ureq`'s default configuration calls
//! `Proxy::try_from_env`, which reads `ALL_PROXY`, `HTTPS_PROXY` and
//! `HTTP_PROXY` in both spellings and honours `NO_PROXY`, and the client this module
//! builds leaves that setting alone rather than clearing it.
//!
//! Nothing on the launcher path reaches this module: it is behind the `cli`
//! feature, and a packaged application never fetches anything.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use sha2::{Digest, Sha256};

/// How many times [`fetch`] asks for one URL before giving up.
///
/// Three: one attempt plus two retries. A transport error and a 5xx are
/// retried, a 4xx is not — a body that is not there will not be there on the
/// third ask, and retrying it only slows the error down.
pub const MAX_ATTEMPTS: u32 = 3;

/// The first backoff, doubled at each further attempt by [`backoff`].
pub const BACKOFF_BASE: Duration = Duration::from_millis(200);

/// How long one attempt may take to open its connection.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// How long one attempt may take to produce its response headers.
///
/// The *body* is deliberately not bounded: a runtime tarball is tens of
/// megabytes and a slow link is not a failure.
pub const RESPONSE_TIMEOUT: Duration = Duration::from_secs(60);

/// The buffer one body is copied through.
const COPY_BUFFER: usize = 64 * 1024;

/// The variable that forbids every fetch.
pub const OFFLINE_VAR: &str = "GINARY_OFFLINE";

/// The variable that points the GitHub API at a mirror.
pub const GITHUB_BASE_VAR: &str = "GINARY_GITHUB_BASE_URL";

/// The base [`GITHUB_BASE_VAR`] replaces.
pub const GITHUB_API_BASE: &str = "https://api.github.com";

/// The proxy variables the HTTP client is expected to honour.
///
/// Recorded here rather than implemented here: the client reads them, and this
/// constant is what a test holds the choice to, so a future client swap that
/// silently stops honouring `NO_PROXY` fails a test rather than a user's
/// firewall.
pub const PROXY_VARS: [&str; 4] = ["HTTPS_PROXY", "NO_PROXY", "https_proxy", "no_proxy"];

/// What the caller knows about the file before it is fetched.
///
/// Both fields are optional because the two callers know different things: a
/// catalogue entry carries a digest and a length, and the GitHub release asset
/// the repack pipeline fetches is verified against the digest the API reported
/// for it, which is read in the same request.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Expect {
    /// The SHA-256 the body must hash to.
    pub sha256: Option<[u8; 32]>,
    /// The number of bytes the body must have.
    pub size: Option<u64>,
}

impl Expect {
    /// Expecting nothing: whatever the server sends is the file.
    pub const fn anything() -> Self {
        Self {
            sha256: None,
            size: None,
        }
    }

    /// Expecting one digest and one length.
    pub const fn exactly(sha256: [u8; 32], size: u64) -> Self {
        Self {
            sha256: Some(sha256),
            size: Some(size),
        }
    }
}

/// A SHA-256 written as 64 lower-case hexadecimal digits.
///
/// Returns [`None`] for anything else, including upper case: the catalogue
/// writes lower case and a comparison that accepted both would make two
/// spellings of one digest, which is a difference nothing downstream could
/// explain.
pub fn parse_sha256(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    if text.bytes().any(|byte| byte.is_ascii_uppercase()) {
        return None;
    }
    let mut digest = [0_u8; 32];
    hex::decode_to_slice(text, &mut digest).ok()?;
    Some(digest)
}

/// Whether this build may fetch, and where the bases point.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Net {
    /// `--offline`, or `GINARY_OFFLINE=1`.
    pub offline: bool,
    /// Base URL to replacement, longest base first when they overlap.
    pub base_overrides: BTreeMap<String, String>,
}

impl Net {
    /// A network that may be used and redirects nothing.
    pub fn online() -> Self {
        Self::default()
    }

    /// A network nothing may be fetched over.
    pub fn offline() -> Self {
        Self {
            offline: true,
            ..Self::default()
        }
    }

    /// The two variables this module reads, taken rather than read.
    ///
    /// `offline_flag` is the command line's own `--offline`, which
    /// [`OFFLINE_VAR`] can only turn on: a build asked to stay offline is not
    /// put back on the network by an environment.
    pub fn from_vars(offline_flag: bool, vars: &BTreeMap<String, String>) -> Self {
        let offline = offline_flag || vars.get(OFFLINE_VAR).is_some_and(|value| value == "1");
        let mut base_overrides = BTreeMap::new();
        if let Some(base) = vars.get(GITHUB_BASE_VAR).filter(|value| !value.is_empty()) {
            base_overrides.insert(GITHUB_API_BASE.to_owned(), base.clone());
        }
        Self {
            offline,
            base_overrides,
        }
    }

    /// The variables [`Net::from_vars`] reads, taken from this process.
    pub fn env_vars() -> BTreeMap<String, String> {
        [OFFLINE_VAR, GITHUB_BASE_VAR]
            .into_iter()
            .filter_map(|name| {
                std::env::var(name)
                    .ok()
                    .map(|value| (name.to_owned(), value))
            })
            .collect()
    }

    /// `url` with the longest matching base replaced.
    ///
    /// A URL that matches no base is returned unchanged, and a base is matched
    /// as a prefix, so one override redirects a whole host.
    pub fn rewrite(&self, url: &str) -> String {
        let matched = self
            .base_overrides
            .iter()
            .filter(|(base, _)| url.starts_with(base.as_str()))
            .max_by_key(|(base, _)| base.len());
        match matched {
            Some((base, replacement)) => format!("{replacement}{}", &url[base.len()..]),
            None => url.to_owned(),
        }
    }
}

/// The part file [`fetch`] streams into: `<dest>.part-<pid>`.
///
/// The process id is in the name so that two ginary processes fetching the
/// same entry into the same cache cannot write one another's bytes; the rename
/// at the end is what makes the last one to finish the winner.
pub fn part_path(dest: &Path) -> PathBuf {
    let mut name = dest.as_os_str().to_os_string();
    name.push(format!(".part-{}", std::process::id()));
    PathBuf::from(name)
}

/// How long to wait after attempt `attempt` has failed, counting from 1.
///
/// [`BACKOFF_BASE`] doubled per attempt, so nothing is waited before the first
/// request and the two retries are preceded by 200 ms and 400 ms.
pub fn backoff(attempt: u32) -> Duration {
    BACKOFF_BASE * 2_u32.saturating_pow(attempt.saturating_sub(1))
}

/// Whether a status code is worth asking again.
///
/// 5xx only. A 4xx is the server saying the request was wrong, and asking the
/// same wrong question twice more is not a retry.
pub fn retryable(status: u16) -> bool {
    (500..600).contains(&status)
}

/// Why a file could not be fetched.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DownloadError {
    /// This build is offline and the file is not on the machine.
    #[error(
        "offline: {url} would be fetched to {dest_hint}; fetch it on a connected machine, or \
         point `--catalog` at a local copy"
    )]
    Offline {
        /// The URL that would have been fetched.
        url: String,
        /// Where the file would have gone.
        dest_hint: PathBuf,
    },
    /// The server answered a status that is not worth asking again.
    #[error("{url} answered HTTP {status}")]
    Status {
        /// The URL that was asked for.
        url: String,
        /// What it answered.
        status: u16,
    },
    /// Every attempt failed.
    #[error("{url} failed {attempts} times; the last was: {last}")]
    Exhausted {
        /// The URL that was asked for.
        url: String,
        /// How many attempts were made.
        attempts: u32,
        /// What the last one said.
        last: String,
    },
    /// The body is not the file the caller expected.
    #[error("{url} has sha256 {actual}, and {expected} was expected")]
    ChecksumMismatch {
        /// The URL that was fetched.
        url: String,
        /// The digest the caller expected, in lower-case hexadecimal.
        expected: String,
        /// The digest the body actually has.
        actual: String,
    },
    /// The body is not the length the caller expected.
    #[error("{url} is {actual} bytes, and {expected} were expected")]
    SizeMismatch {
        /// The URL that was fetched.
        url: String,
        /// The length the caller expected.
        expected: u64,
        /// The length the body actually has.
        actual: u64,
    },
    /// The part file, or the rename onto the destination, failed.
    #[error("cannot write {path}: {message}")]
    Io {
        /// The file that could not be written.
        path: PathBuf,
        /// What the operating system said.
        message: String,
    },
}

/// Fetches `url` into `dest`, verified against `expect`.
///
/// Streams into [`part_path`] while hashing, retries a transport error and a
/// 5xx up to [`MAX_ATTEMPTS`] times with [`backoff`] between them, verifies the
/// digest and the length, and renames the part file onto `dest`. Every failure
/// removes the part file: a `dest` either does not exist or is whole.
///
/// # Errors
///
/// [`DownloadError`], naming the URL in every variant. A mismatch names both
/// the expected and the actual value, because "checksum failed" tells the
/// reader nothing about which of the two is wrong.
pub fn fetch(url: &str, dest: &Path, expect: &Expect, net: &Net) -> Result<(), DownloadError> {
    let url = net.rewrite(url);
    if net.offline {
        return Err(DownloadError::Offline {
            url,
            dest_hint: dest.to_path_buf(),
        });
    }

    if let Some(parent) = dest
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(|error| DownloadError::Io {
            path: parent.to_path_buf(),
            message: error.to_string(),
        })?;
    }

    let part = part_path(dest);
    let agent = agent();
    let mut last = String::new();

    for attempt in 1..=MAX_ATTEMPTS {
        match attempt_once(&agent, &url, &part, expect) {
            Ok(()) => {
                return std::fs::rename(&part, dest).map_err(|error| {
                    remove(&part);
                    DownloadError::Io {
                        path: dest.to_path_buf(),
                        message: error.to_string(),
                    }
                });
            }
            Err(Attempt::Fatal(error)) => {
                remove(&part);
                return Err(error);
            }
            Err(Attempt::Retryable(message)) => {
                remove(&part);
                last = message;
                if attempt < MAX_ATTEMPTS {
                    std::thread::sleep(backoff(attempt));
                }
            }
        }
    }

    Err(DownloadError::Exhausted {
        url,
        attempts: MAX_ATTEMPTS,
        last,
    })
}

/// The body of one small document, such as a release description.
///
/// The same offline rule, the same rewriting and the same retry policy as
/// [`fetch`], and a limit on the answer: this is for API documents, and a
/// megabyte of JSON is already a release nobody expected. A body is *not*
/// streamed to a file, so nothing here can leave a part behind.
///
/// # Errors
///
/// [`DownloadError`], as [`fetch`].
pub fn get_text(url: &str, net: &Net) -> Result<String, DownloadError> {
    let url = net.rewrite(url);
    if net.offline {
        return Err(DownloadError::Offline {
            url,
            dest_hint: PathBuf::from("(read into memory)"),
        });
    }

    let agent = agent();
    let mut last = String::new();
    for attempt in 1..=MAX_ATTEMPTS {
        match text_once(&agent, &url) {
            Ok(text) => return Ok(text),
            Err(Attempt::Fatal(error)) => return Err(error),
            Err(Attempt::Retryable(message)) => {
                last = message;
                if attempt < MAX_ATTEMPTS {
                    std::thread::sleep(backoff(attempt));
                }
            }
        }
    }
    Err(DownloadError::Exhausted {
        url,
        attempts: MAX_ATTEMPTS,
        last,
    })
}

/// How large an answer [`get_text`] will read.
pub const MAX_TEXT_BYTES: u64 = 4 * 1024 * 1024;

/// One request for a small document.
fn text_once(agent: &ureq::Agent, url: &str) -> Result<String, Attempt> {
    let response = agent
        .get(url)
        .header("accept", "application/vnd.github+json")
        .call()
        .map_err(|error| Attempt::Retryable(error.to_string()))?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(if retryable(status) {
            Attempt::Retryable(format!("HTTP {status}"))
        } else {
            Attempt::Fatal(DownloadError::Status {
                url: url.to_owned(),
                status,
            })
        });
    }
    response
        .into_body()
        .into_with_config()
        .limit(MAX_TEXT_BYTES)
        .read_to_string()
        .map_err(|error| Attempt::Retryable(error.to_string()))
}

/// How one attempt ended, when it did not end with a verified part file.
enum Attempt {
    /// Worth asking again: a transport failure or a 5xx.
    Retryable(String),
    /// Not worth asking again: the answer, or the file, was wrong.
    Fatal(DownloadError),
}

/// The client every attempt is made through.
///
/// `http_status_as_error` is turned off so that a status is read from the
/// response rather than out of an error, which is what makes [`retryable`] the
/// one place the policy lives. The proxy configuration is left at its default,
/// which is `Proxy::try_from_env`; see [`PROXY_VARS`].
fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_connect(Some(CONNECT_TIMEOUT))
        .timeout_recv_response(Some(RESPONSE_TIMEOUT))
        .build()
        .into()
}

/// One request, streamed into `part` and verified.
fn attempt_once(
    agent: &ureq::Agent,
    url: &str,
    part: &Path,
    expect: &Expect,
) -> Result<(), Attempt> {
    let response = agent
        .get(url)
        .call()
        .map_err(|error| Attempt::Retryable(error.to_string()))?;

    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(if retryable(status) {
            Attempt::Retryable(format!("HTTP {status}"))
        } else {
            Attempt::Fatal(DownloadError::Status {
                url: url.to_owned(),
                status,
            })
        });
    }

    let mut body = response.into_body().into_reader();
    let (size, digest) = stream_to_file(&mut body, part)?;

    if let Some(wanted) = expect.sha256 {
        let actual = hex::encode(digest);
        if digest != wanted {
            return Err(Attempt::Fatal(DownloadError::ChecksumMismatch {
                url: url.to_owned(),
                expected: hex::encode(wanted),
                actual,
            }));
        }
    }
    if let Some(wanted) = expect.size
        && size != wanted
    {
        return Err(Attempt::Fatal(DownloadError::SizeMismatch {
            url: url.to_owned(),
            expected: wanted,
            actual: size,
        }));
    }
    Ok(())
}

/// Copies `body` into `part`, hashing as it goes.
///
/// A read failure is the transport's and is worth another attempt; a write
/// failure is this machine's and is not.
fn stream_to_file(body: &mut impl Read, part: &Path) -> Result<(u64, [u8; 32]), Attempt> {
    let mut file = std::fs::File::create(part).map_err(|error| {
        Attempt::Fatal(DownloadError::Io {
            path: part.to_path_buf(),
            message: error.to_string(),
        })
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; COPY_BUFFER];
    let mut size = 0_u64;

    loop {
        let read = match body.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => read,
            Err(error) => return Err(Attempt::Retryable(error.to_string())),
        };
        hasher.update(&buffer[..read]);
        size = size.saturating_add(read as u64);
        file.write_all(&buffer[..read]).map_err(|error| {
            Attempt::Fatal(DownloadError::Io {
                path: part.to_path_buf(),
                message: error.to_string(),
            })
        })?;
    }

    file.flush().map_err(|error| {
        Attempt::Fatal(DownloadError::Io {
            path: part.to_path_buf(),
            message: error.to_string(),
        })
    })?;
    Ok((size, hasher.finalize().into()))
}

/// Removes a part file, ignoring one that is already gone.
fn remove(part: &Path) {
    let _ = std::fs::remove_file(part);
}
