// SPDX-License-Identifier: MIT OR Apache-2.0
//! Reading a packaged application from the outside.
//!
//! `ginary inspect` is the answer to "what is in this file, and is it intact".
//! It reads the trailer, then the payload's first two entries — the manifest
//! and the index — and stops: an artifact is tens of megabytes and a question
//! about what it holds costs a few kilobytes.
//!
//! Three things it deliberately does *not* do. It never extracts, so
//! inspecting a stranger's artifact writes nothing. It never runs it, so
//! `--launch-plan` prints the plan against a placeholder root rather than
//! against a real cache entry. And `--verify` streams the payload past a
//! hasher rather than reading it into memory, because the file is a file and
//! not a buffer.

use std::fs::File;
use std::io::{Read as _, Seek as _, SeekFrom};
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::cache::Env;
use crate::error::LauncherError;
use crate::launch::LaunchPlan;
use crate::manifest::{Index, IndexFile, Manifest};
use crate::payload::PayloadError;
use crate::trailer::{Trailer, TrailerError};

/// Version of the `ginary inspect --json` schema.
pub const INSPECT_FORMAT_VERSION: u32 = 1;

/// The extracted root `--launch-plan` prints against.
///
/// The shape of a real one, `<cache>/<app>/<key>`, with the three parts that
/// vary left as their own names: the plan is for reading, and a plan naming
/// this machine's cache would be a plan about this machine.
pub const PLACEHOLDER_ROOT: &str = "<cache>/<app>/<key>";

/// The application directory `--launch-plan` points `ERL_CRASH_DUMP` into.
pub const PLACEHOLDER_APP_DIR: &str = "<cache>/<app>";

/// How many of the index's files the text report lists.
pub const LARGEST_FILES: usize = 10;

/// The width the text report's labels are padded to.
///
/// `applications:` is the longest, so every value starts in the same column.
pub const LABEL_WIDTH: usize = 15;

/// What one artifact says about itself.
///
/// Not [`Serialize`]: the JSON form is [`InspectReport`], which flattens the
/// trailer into the three numbers a reader wants and carries what `--verify`
/// and `--launch-plan` added.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtifactInfo {
    /// The file, as it was named on the command line.
    pub path: PathBuf,
    /// The last 64 bytes.
    pub trailer: Trailer,
    /// Entry 0 of the payload.
    pub manifest: Manifest,
    /// Entry 1 of the payload.
    pub index: Index,
    /// The stub's length, which is the payload's offset.
    pub stub_len: u64,
    /// The payload's length.
    pub payload_len: u64,
    /// The file's length.
    pub total_len: u64,
}

impl ArtifactInfo {
    /// The `count` largest files of the index.
    ///
    /// Largest first; two files of one size are in path order, so the list is
    /// the same on every machine that reads the same artifact.
    pub fn largest_files(&self, count: usize) -> Vec<&IndexFile> {
        let mut files: Vec<&IndexFile> = self.index.files.iter().collect();
        files.sort_by(|left, right| {
            right
                .size
                .cmp(&left.size)
                .then_with(|| left.path.cmp(&right.path))
        });
        files.truncate(count);
        files
    }

    /// The human-readable report.
    ///
    /// ```text
    /// app:           hello 1.2.3
    /// target:        linux-x86_64-gnu
    /// otp:           29.0.5 (release 29, erts 17.0.5)
    /// gleam:         1.18.1
    /// created:       2026-08-31T00:00:00Z
    /// ginary:        0.1.0
    /// size:          1564 bytes = 1000 stub + 500 payload + 64 trailer
    /// applications:  1 otp, 2 gleam
    /// files:         3
    ///
    /// size  path
    /// 600   erts-17.0.5/bin/beam.smp
    /// ```
    ///
    /// A field the manifest leaves empty prints `-` rather than nothing, so a
    /// reader can tell an absent value from a missing line.
    pub fn render_text(&self) -> String {
        let manifest = &self.manifest;
        let mut text = String::new();
        let mut field = |label: &str, value: &str| {
            text.push_str(&format!(
                "{:<width$}{value}\n",
                format!("{label}:"),
                width = LABEL_WIDTH
            ));
        };

        field(
            "app",
            &format!("{} {}", manifest.app, or_dash(&manifest.app_version)),
        );
        field("target", &manifest.target.to_string());
        field(
            "otp",
            &format!(
                "{} (release {}, erts {})",
                or_dash(&manifest.otp_version),
                manifest.otp_release,
                or_dash(&manifest.erts_version)
            ),
        );
        field(
            "gleam",
            or_dash(manifest.gleam_version.as_deref().unwrap_or("")),
        );
        field("created", or_dash(&manifest.created_at));
        field("ginary", or_dash(&manifest.ginary_version));
        field(
            "size",
            &format!(
                "{} bytes = {} stub + {} payload + {} trailer",
                self.total_len,
                self.stub_len,
                self.payload_len,
                crate::trailer::TRAILER_LEN
            ),
        );
        field(
            "applications",
            &format!(
                "{} otp, {} gleam",
                manifest.otp_applications.len(),
                manifest.gleam_applications.len()
            ),
        );
        field("files", &self.index.files.len().to_string());

        let rows: Vec<[String; 2]> = self
            .largest_files(LARGEST_FILES)
            .into_iter()
            .map(|file| [file.size.to_string(), file.path.clone()])
            .collect();
        text.push('\n');
        text.push_str(&crate::closure::render_table(["size", "path"], &rows));
        text
    }
}

/// What `--verify` found.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Verification {
    /// The digest the trailer carries, in lower-case hexadecimal.
    pub expected: String,
    /// The digest the payload's bytes actually produce.
    pub actual: String,
}

impl Verification {
    /// Whether the payload is the one the trailer describes.
    pub fn ok(&self) -> bool {
        self.expected == self.actual
    }
}

/// Opens an artifact and reads its trailer, manifest and index.
///
/// # Errors
///
/// [`InspectError::NoTrailer`] when the file is not a packaged application —
/// the ginary command line tool itself is the common case —
/// [`InspectError::Trailer`] when the last 64 bytes begin the magic and then
/// do not describe the file, [`InspectError::Payload`] when the payload does
/// not hold the two front entries, and [`InspectError::Io`] when the file
/// could not be opened or read at all, a directory included.
pub fn open(path: &Path) -> Result<ArtifactInfo, InspectError> {
    let file = File::open(path).map_err(|source| InspectError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let total_len = file
        .metadata()
        .map_err(|source| InspectError::Io {
            path: path.to_path_buf(),
            source,
        })?
        .len();

    let trailer = match Trailer::read_from(&file) {
        Ok(Some(trailer)) => trailer,
        Ok(None) => {
            return Err(InspectError::NoTrailer {
                path: path.to_path_buf(),
            });
        }
        // An IO failure is not a statement about the file's contents: a
        // directory, or an EIO, would otherwise be reported as a damaged
        // artifact, which sends a reader looking for damage that is not there.
        Err(TrailerError::Io(source)) => {
            return Err(InspectError::Io {
                path: path.to_path_buf(),
                source,
            });
        }
        Err(source) => {
            return Err(InspectError::Trailer {
                path: path.to_path_buf(),
                source,
            });
        }
    };

    let (manifest, index) = payload_reader(path, &trailer).and_then(|reader| {
        crate::payload::read_index(reader).map_err(|source| InspectError::Payload {
            path: path.to_path_buf(),
            source,
        })
    })?;

    Ok(ArtifactInfo {
        path: path.to_path_buf(),
        stub_len: trailer.payload_offset,
        payload_len: trailer.payload_len,
        total_len,
        trailer,
        manifest,
        index,
    })
}

/// A reader over exactly the payload region of `path`.
///
/// The file is opened again rather than shared, because the two readers this
/// module builds — the front-entry read and `--verify` — each seek, and a
/// descriptor whose offset another caller moved is a bug that only shows up
/// under `--verify --launch-plan`.
fn payload_reader(path: &Path, trailer: &Trailer) -> Result<std::io::Take<File>, InspectError> {
    let mut file = File::open(path).map_err(|source| InspectError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    file.seek(SeekFrom::Start(trailer.payload_offset))
        .map_err(|source| InspectError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    Ok(file.take(trailer.payload_len))
}

/// Streams the payload past a hasher and compares the result to the trailer.
///
/// # Errors
///
/// [`InspectError::Io`] when the file cannot be read. A digest that does not
/// match is a [`Verification`] rather than an error: the caller decides what
/// a mismatch means, and `ginary inspect --verify` makes it exit 1.
pub fn verify(info: &ArtifactInfo) -> Result<Verification, InspectError> {
    use sha2::Digest as _;

    let mut reader = payload_reader(&info.path, &info.trailer)?;
    let mut hasher = sha2::Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|source| InspectError::Io {
                path: info.path.clone(),
                source,
            })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }

    Ok(Verification {
        expected: hex::encode(info.trailer.payload_sha256),
        actual: hex::encode(hasher.finalize()),
    })
}

/// The plan the launcher would build, against a placeholder root.
///
/// # Errors
///
/// [`InspectError::Launch`] when the manifest holds a path that would not stay
/// under the extracted root — the same check the launcher makes, made here so
/// that an artifact nobody can run can be diagnosed without running it.
pub fn launch_plan(
    info: &ArtifactInfo,
    root: &Path,
    crash_dump_dir: &Path,
) -> Result<LaunchPlan, InspectError> {
    // An empty environment rather than this process': the plan is about the
    // artifact, and a `GINARY_ERL_FLAGS` or a `HOME` that happens to be set
    // where the inspection ran is not part of it.
    crate::launch::plan(
        root,
        &info.manifest,
        &[],
        &Env::default(),
        crash_dump_dir,
        &info.path,
    )
    .map_err(|source| InspectError::Launch {
        path: info.path.clone(),
        source,
    })
}

/// Renders a plan as `program:`, `argv:`, `set:` and `remove:` blocks.
///
/// One item per line, two spaces of indent, in the order the plan holds them,
/// so that the argument vector reads as the argument vector rather than as a
/// sorted set.
pub fn render_launch_plan(plan: &LaunchPlan) -> String {
    let mut text = format!("program: {}\n", plan.program.display());
    text.push_str("argv:\n");
    for argument in &plan.args {
        text.push_str(&format!("  {}\n", argument.to_string_lossy()));
    }
    text.push_str("set:\n");
    for (key, value) in &plan.set {
        text.push_str(&format!(
            "  {}={}\n",
            key.to_string_lossy(),
            value.to_string_lossy()
        ));
    }
    text.push_str("remove:\n");
    for name in &plan.remove {
        text.push_str(&format!("  {}\n", name.to_string_lossy()));
    }
    text
}

/// `value`, or `-` when the manifest left it empty.
///
/// A reader has to be able to tell an absent value from a missing line, and a
/// line that just stops after its label reads as the second.
fn or_dash(value: &str) -> &str {
    if value.is_empty() { "-" } else { value }
}

/// The payload of `ginary inspect --json`.
///
/// One object with everything the text report prints, plus whatever the flags
/// asked for. `verify` and `launch_plan` are absent rather than null when the
/// flag was not given, so a consumer can tell "not asked" from "asked and
/// empty".
#[derive(Debug, Serialize)]
pub struct InspectReport {
    /// Version of this schema; see [`INSPECT_FORMAT_VERSION`].
    pub format_version: u32,
    /// The file, as it was named on the command line.
    pub path: String,
    /// The payload's absolute offset, which is the stub's length.
    pub payload_offset: u64,
    /// The payload's length.
    pub payload_len: u64,
    /// The file's length.
    pub total_len: u64,
    /// The digest the trailer carries, in lower-case hexadecimal.
    pub payload_sha256: String,
    /// Entry 0 of the payload.
    pub manifest: Manifest,
    /// Entry 1 of the payload.
    pub index: Index,
    /// What `--verify` found, when it was asked for.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verify: Option<Verification>,
    /// What `--launch-plan` found, when it was asked for.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub launch_plan: Option<LaunchPlanReport>,
}

/// The `launch_plan` member of an [`InspectReport`].
///
/// Lossily converted to text, because the plan is for reading: an argument
/// that is not valid UTF-8 cannot reach it — every one of them comes from the
/// manifest, which is JSON — and the user's own arguments are not in a plan
/// nobody ran.
#[derive(Debug, Serialize)]
pub struct LaunchPlanReport {
    /// The program the launcher would exec.
    pub program: String,
    /// The argument vector, in order.
    pub argv: Vec<String>,
    /// The variables the launcher sets, in order, as `NAME=VALUE`.
    pub set: Vec<String>,
    /// The variables the launcher removes, in order.
    pub remove: Vec<String>,
}

/// Why an artifact could not be inspected.
#[derive(Debug, thiserror::Error)]
pub enum InspectError {
    /// The file carries no trailer, so it is not a packaged application.
    #[error("{path}: no ginary trailer")]
    NoTrailer {
        /// The file that was read.
        path: PathBuf,
    },
    /// The trailer is there and does not describe the file.
    #[error("{path}: the ginary trailer is damaged")]
    Trailer {
        /// The file that was read.
        path: PathBuf,
        /// What is wrong with the last 64 bytes.
        #[source]
        source: TrailerError,
    },
    /// The payload does not hold the manifest and the index.
    #[error("{path}: the payload cannot be read")]
    Payload {
        /// The file that was read.
        path: PathBuf,
        /// What is wrong with the payload.
        #[source]
        source: PayloadError,
    },
    /// The manifest holds a path the launcher would refuse.
    #[error("{path}: the manifest would not launch")]
    Launch {
        /// The file that was read.
        path: PathBuf,
        /// What the launcher's own check said.
        #[source]
        source: LauncherError,
    },
    /// The file could not be opened or read.
    #[error("cannot read {path}")]
    Io {
        /// The file that was read.
        path: PathBuf,
        /// What the operating system said.
        #[source]
        source: std::io::Error,
    },
}
