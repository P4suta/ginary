// SPDX-License-Identifier: MIT OR Apache-2.0
//! The deep check of a packaged application.
//!
//! `ginary inspect --verify` answers one question — do the payload's bytes
//! still hash to what the trailer says — and stops there. That is the cheap
//! check, and it is the one the launcher itself makes. `ginary verify` is the
//! expensive one, and it answers four more:
//!
//! - **Is every file the one the index describes?** The payload is streamed a
//!   second time and every entry is hashed against
//!   [`crate::manifest::Index`]. A payload whose digest matches and whose
//!   index does not describe it is an artifact ginary built wrongly, which the
//!   payload hash alone cannot see.
//! - **Will the native code run on the machine this artifact targets?** Every
//!   entry whose first bytes name a container format
//!   ([`crate::platform::object_format_of`]) is read into memory — and only
//!   then, and only up to [`MAX_OBJECT_BYTES`] — and inspected, so a NIF built
//!   for another architecture is a build-time finding rather than a loader
//!   error on somebody else's machine. All three formats, because an artifact
//!   built on Windows carries no ELF at all and a check that read one format
//!   reported such an artifact as holding no native code.
//! - **Does it need anything the runtime does not carry?** A library outside
//!   the target platform's own floor ([`platform_allowlist`]) is one the
//!   artifact expects to find on the target and does not bring, which is the
//!   whole of the portability promise.
//! - **Will a launcher extract it at all?** Every rule
//!   [`crate::payload::unpack`] refuses a payload for is a rule this module
//!   reports: an entry that leaves the extracted root, one landing on a name
//!   the format reserves, one that is neither a file nor a directory. An
//!   artifact that fails at run time on somebody else's machine has to fail
//!   here first.
//!
//! Nothing here extracts anything and nothing here runs anything, exactly as
//! in [`crate::inspect`]: the payload is a stream, read twice, and the answer
//! is a report.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read as _, Seek as _, SeekFrom};
use std::path::{Path, PathBuf};

use serde::Serialize;
use sha2::Digest as _;

use crate::elf::ElfKind;
use crate::inspect::{self, ArtifactInfo, InspectError, Verification};
use crate::manifest::IndexFile;

use crate::payload::PayloadError;

/// Version of the `verify --json` schema.
pub const VERIFY_FORMAT_VERSION: u32 = 1;

/// The shared libraries a packaged application may name in `DT_NEEDED`.
///
/// Everything on this list is part of a glibc system's own runtime, which is
/// the one thing an artifact is allowed to assume is already there: it carries
/// its own BEAM and it cannot carry a libc. Anything else — a `libssl.so.3` a
/// NIF was linked against, a `libsqlite3.so.0` the build machine happened to
/// have — is a file the artifact expects to find on a stranger's machine and
/// does not bring, so it is reported.
///
/// The dynamic loader is matched by [`LOADER_PREFIX`] instead of by an exact
/// name, because its name carries the architecture: `ld-linux-x86-64.so.2` on
/// one target and `ld-linux-aarch64.so.1` on another.
pub const NEEDED_ALLOWLIST: [&str; 8] = [
    "libc.so.6",
    "libm.so.6",
    "libpthread.so.0",
    "libdl.so.2",
    "librt.so.1",
    "libgcc_s.so.1",
    "libstdc++.so.6",
    "libtinfo.so.6",
];

/// The libraries a macOS target guarantees.
///
/// Two spellings of one umbrella dylib, because a Mach-O's `LC_LOAD_DYLIB`
/// commands name it by absolute path and both paths are in use: macOS
/// re-exports `libm`, `libpthread`, `libdl` and the rest from that one file.
pub const MACOS_NEEDED_ALLOWLIST: [&str; 2] =
    ["/usr/lib/libSystem.B.dylib", "/usr/lib/libSystem.dylib"];

/// The libraries a Windows target guarantees.
///
/// The system DLLs that live in `%SystemRoot%\System32` on every supported
/// Windows, plus the two C runtimes a toolchain links there. The Universal
/// CRT ships as several dozen `api-ms-win-crt-*` forwarding libraries rather
/// than as one file, so [`WINDOWS_CRT_PREFIX`] matches that family the way
/// [`LOADER_PREFIX`] matches glibc's loader.
///
/// The list is deliberately conservative: a name it does not carry is
/// *reported*, which is the safe direction. An artifact that really does need
/// a stranger's machine to supply a DLL is the finding this check exists for,
/// and a system DLL missing from the list costs a reader one line to dismiss
/// rather than costing a user a program that will not start.
///
/// The Visual C++ redistributable is three files and it is named as three:
/// `VCRUNTIME140.dll`, the exception-handling half `VCRUNTIME140_1.dll` that
/// x64 splits out, and the C++ standard library `MSVCP140.dll`. The list
/// carried the first alone, so `ginary verify` reported two findings against
/// every healthy Windows artifact — a machine that has one of the three has
/// all three, and Erlang/OTP's own Windows installer requires the package.
/// The *debug* runtime is a different matter and is deliberately absent:
/// `MSVCP140D.dll`, `VCRUNTIME140D.dll`, `VCRUNTIME140_1D.dll` and
/// `ucrtbased.dll` are not redistributable and exist only where Visual Studio
/// is installed, so an artifact needing one is exactly the finding this check
/// is for. See
/// `tests/regressions/e12_the_windows_allowlist_carried_one_vc_runtime_of_three.rs`.
pub const WINDOWS_NEEDED_ALLOWLIST: [&str; 16] = [
    "ADVAPI32.dll",
    "bcrypt.dll",
    "CRYPT32.dll",
    "dbghelp.dll",
    "IPHLPAPI.DLL",
    "KERNEL32.dll",
    "MSVCP140.dll",
    "msvcrt.dll",
    "ole32.dll",
    "SHELL32.dll",
    "ucrtbase.dll",
    "USER32.dll",
    "VCRUNTIME140.dll",
    "VCRUNTIME140_1.dll",
    "WS2_32.dll",
    "WSOCK32.dll",
];

/// The prefix of the Universal CRT's forwarding libraries.
///
/// Matched case-insensitively, like every name in
/// [`WINDOWS_NEEDED_ALLOWLIST`]: a PE import table spells `KERNEL32.dll` and
/// `kernel32.dll` for one file, because the platform's own file names are
/// case-insensitive.
pub const WINDOWS_CRT_PREFIX: &str = "api-ms-win-crt-";

/// The libraries the target platform `os` guarantees, which an artifact may
/// expect to find and need not carry.
///
/// The allowlist is a statement about a *target*, not about the machine
/// verifying. Reading one container format and holding it to glibc's floor is
/// what let a Windows artifact verify as one holding no objects at all; now
/// that all three formats are read, each is held to its own platform's floor.
pub const fn platform_allowlist(os: crate::target::Os) -> &'static [&'static str] {
    match os {
        crate::target::Os::Linux => &NEEDED_ALLOWLIST,
        crate::target::Os::Macos => &MACOS_NEEDED_ALLOWLIST,
        crate::target::Os::Windows => &WINDOWS_NEEDED_ALLOWLIST,
    }
}

/// The prefix every glibc dynamic loader's name begins with.
pub const LOADER_PREFIX: &str = "ld-linux-";

/// The library whose presence on an allowlist admits the dynamic loader.
///
/// The loader cannot be listed by name, because its soname carries the
/// architecture — `ld-linux-x86-64.so.2` on one target and
/// `ld-linux-aarch64.so.1` on another — so [`LOADER_PREFIX`] matches it
/// instead. That rule belongs to one C runtime and not to allowlists in
/// general: the loader *is* glibc, so an allowlist that admits glibc's own
/// `libc` admits its loader, and one that does not admits neither. An empty
/// allowlist therefore means what it says — nothing about the target machine
/// is assumed, the loader included.
pub const LOADER_COMPANION: &str = "libc.so.6";

/// How many of an entry's first bytes are read to decide whether it is an
/// object.
///
/// Four: the longest of the three magics [`crate::platform::object_format_of`]
/// reads. A PE's is two and an ELF's and a Mach-O's are four, so four is
/// enough for all three and is still nothing beside the entry itself.
const OBJECT_MAGIC_BYTES: usize = 4;

/// The largest payload entry that is read into memory to be inspected.
///
/// An entry is only read at all once its first bytes name a container format
/// [`crate::platform::object_format_of`] knows, so the bound is about a
/// *hostile* artifact rather than about a large one: a tar header can claim
/// any length, and a verifier that believed it would be killed rather than
/// report anything. The largest object a real artifact
/// carries is `beam.smp`, two orders of magnitude below this.
pub const MAX_OBJECT_BYTES: u64 = 100 * 1024 * 1024;

/// How the allowlist is chosen, so that a test can narrow it.
///
/// The allowlist is the one part of verification whose *content* cannot be
/// exercised from outside: every ELF a test can produce on the machine it runs
/// on links against libraries that are on the list, so a bug that accepted
/// everything and a correct implementation would agree. Injecting the list is
/// what makes "this name is refused" a test rather than a hope.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VerifyOptions<'a> {
    /// The names that are not reported, or [`None`] for the artifact's own
    /// target platform's list.
    ///
    /// [`None`] rather than a default array, because the answer depends on
    /// what the artifact targets and the artifact has not been opened yet
    /// when the options are built: [`platform_allowlist`] is consulted once
    /// the manifest names a target. A test that wants to narrow the list
    /// passes [`Some`] and gets exactly what it named, the loader rule
    /// included.
    pub allowlist: Option<&'a [&'a str]>,
    /// The bound on an entry that is read into memory to be inspected.
    ///
    /// Injectable for the same reason as the allowlist and not for a
    /// different one: [`MAX_OBJECT_BYTES`] is a hundred megabytes, and a test
    /// that produced an entry that large would be a test nobody runs. Lowering
    /// it is how "an object this verifier will not hold is reported rather
    /// than held" becomes an assertion.
    pub max_object_bytes: u64,
}

impl Default for VerifyOptions<'_> {
    fn default() -> Self {
        Self {
            allowlist: None,
            max_object_bytes: MAX_OBJECT_BYTES,
        }
    }
}

/// Whether `name` is a library the artifact may expect the target to have.
///
/// An exact match against `allowlist`, plus the [`LOADER_PREFIX`] rule for an
/// allowlist that names [`LOADER_COMPANION`].
pub fn needed_is_allowed(name: &str, allowlist: &[&str]) -> bool {
    allowlist.contains(&name)
        || (name.starts_with(LOADER_PREFIX) && allowlist.contains(&LOADER_COMPANION))
        || windows_name_is_allowed(name, allowlist)
}

/// The case-insensitive half of [`needed_is_allowed`], for a PE import name.
///
/// Only a list that names a Windows library at all takes this arm, so an
/// allowlist a test narrowed to `&[]` admits nothing here either and the seam
/// stays a seam.
fn windows_name_is_allowed(name: &str, allowlist: &[&str]) -> bool {
    allowlist.iter().any(|known| {
        // The suffix is asked about case-insensitively too, or the one entry
        // spelled `IPHLPAPI.DLL` never reaches the comparison beside it and is
        // admitted in its own spelling alone. See
        // `tests/regressions/e11_a_dll_the_import_table_spelt_in_lower_case_was_unexpected.rs`.
        known.to_ascii_lowercase().ends_with(WINDOWS_LIBRARY_SUFFIX)
            && known.eq_ignore_ascii_case(name)
    }) || (name.to_ascii_lowercase().starts_with(WINDOWS_CRT_PREFIX)
        && allowlist.contains(&WINDOWS_CRT_COMPANION))
}

/// The suffix that makes a name on an allowlist a Windows library.
const WINDOWS_LIBRARY_SUFFIX: &str = ".dll";

/// The library whose presence on an allowlist admits the Universal CRT's
/// `api-ms-win-crt-*` family.
///
/// [`LOADER_COMPANION`]'s counterpart: the family *is* the Universal CRT, so
/// an allowlist that admits `ucrtbase.dll` admits its forwarding libraries and
/// one that does not admits neither.
pub const WINDOWS_CRT_COMPANION: &str = "ucrtbase.dll";

/// One native object found in the payload.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ObjectInfo {
    /// The path inside the artifact, relative to the extracted root.
    pub path: String,
    /// The machine, as [`crate::elf::ElfInfo::machine`] spells it.
    pub machine: String,
    /// The ELF class, `32` or `64`.
    pub class: u8,
    /// What kind of object the file is.
    pub kind: ElfKind,
    /// The program interpreter, when the file has one.
    pub interp: Option<String>,
    /// The `DT_NEEDED` entries, in the order the dynamic section lists them.
    pub needed: Vec<String>,
    /// The highest `GLIBC_x.y` the file requires, without the prefix.
    pub glibc_max: Option<String>,
    /// The issues this object raised, in the order they were found.
    pub issues: Vec<Issue>,
}

/// Something about an artifact that a reader has to be told.
///
/// Each variant is one sentence, because the table `ginary verify` prints is
/// one line per issue and the JSON form carries the same fields.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, thiserror::Error)]
#[serde(tag = "issue", rename_all = "snake_case")]
pub enum Issue {
    /// A native object is for a machine the artifact does not target.
    #[error("{path}: built for {found}, and the artifact targets {expected}")]
    MachineMismatch {
        /// The object's path inside the artifact.
        path: String,
        /// The machine the object was built for.
        found: String,
        /// The machine the manifest's target names.
        expected: String,
    },
    /// A native object needs a library the artifact does not carry.
    #[error("{path}: needs `{needed}`, which the artifact does not carry")]
    UnexpectedNeeded {
        /// The object's path inside the artifact.
        path: String,
        /// The `DT_NEEDED` name that is not on the allowlist.
        needed: String,
    },
    /// A file begins with the ELF magic and cannot be inspected as one.
    ///
    /// Skipping it would be the one thing this module may not do: an entry
    /// that looks like native code and is not readable as native code is
    /// either a damaged artifact or a hostile one, and either way it is the
    /// reader who has to decide. Its bytes are still checked against the
    /// index, so a file that is *only* damaged is named twice.
    #[error("{path}: begins with the ELF magic and cannot be read as one ({message})")]
    UnreadableObject {
        /// The file's path inside the artifact.
        path: String,
        /// Why it could not be read.
        message: String,
    },
    /// A file's bytes are not the ones the index describes.
    #[error("{path}: sha256 {actual}, and the index says {expected}")]
    IndexMismatch {
        /// The file's path inside the artifact.
        path: String,
        /// The digest the index carries.
        expected: String,
        /// The digest the payload's bytes produce.
        actual: String,
    },
    /// A file's length is not the one the index describes.
    ///
    /// The index's `size` is the file's own length and so is the tar header's,
    /// so the two are equal in an artifact that describes itself. A row whose
    /// digest is right and whose length is wrong is not a damaged file; it is
    /// an index that cannot be used to plan an extraction, and a reader who
    /// sizes a cache entry from it would be sizing it from a number nothing
    /// checked.
    #[error("{path}: {actual} bytes in the payload, and the index says {expected}")]
    IndexSizeMismatch {
        /// The file's path inside the artifact.
        path: String,
        /// The length the index carries.
        expected: u64,
        /// The length the payload entry has.
        actual: u64,
    },
    /// A file's permission bits are not the ones the index describes.
    ///
    /// `docs/format.md` fixes the relation: the index records the staged
    /// file's own `st_mode & 0o7777`, and the header carries the
    /// normalisation of it — `0755` when the staged mode has the user execute
    /// bit and `0644` otherwise. That relation, rather than equality, is what
    /// is checked, because the two columns are documented to differ for a tree
    /// whose modes are neither. A row promising an executable over an entry
    /// the launcher will extract `0644` describes a file the artifact does not
    /// carry.
    ///
    /// `actual` is `unreadable` for a header whose mode field is not an octal
    /// number, which is a header that agrees with no row at all.
    #[error(
        "{path}: entry mode {actual}, and the index row's mode {indexed} normalises to {expected}"
    )]
    IndexModeMismatch {
        /// The file's path inside the artifact.
        path: String,
        /// The staged mode the index carries, in octal.
        indexed: String,
        /// The header mode that row implies, in octal.
        expected: String,
        /// The mode the payload entry's header carries, in octal.
        actual: String,
    },
    /// A file is in the payload and not in the index.
    #[error("{path}: in the payload and not in the index")]
    IndexOrphan {
        /// The file's path inside the artifact.
        path: String,
    },
    /// A file is in the index and not in the payload.
    #[error("{path}: in the index and not in the payload")]
    IndexMissing {
        /// The file's path inside the artifact.
        path: String,
    },
    /// An entry's path does not stay under the extracted root.
    ///
    /// The rule [`crate::payload::PayloadError::UnsafePath`] states, applied
    /// to a report rather than to an extraction: an absolute name, a name
    /// holding `..`, or one that normalises to nothing is an entry
    /// [`crate::payload::unpack`] refuses outright, and an artifact carrying
    /// one is not an artifact any launcher will run. It is raised before the
    /// path is matched against the index, because an entry that may not be
    /// written is not a file the index can account for, whatever row an
    /// artifact's author put there to make it look accounted for.
    #[error("{path}: does not stay under the extracted root")]
    UnsafePath {
        /// The path the entry's header carries, as it was written.
        path: String,
    },
    /// An entry after the front matter lands on a name the format reserves.
    ///
    /// `ginary.json` and `ginary.index.json` are entries 0 and 1 and nothing
    /// else, so a later entry on either name — as the name itself, or as a
    /// directory holding a file — is a payload
    /// [`crate::payload::unpack`] refuses outright. Skipping it here by name
    /// rather than by position would let an artifact no launcher can extract
    /// verify clean.
    #[error("{path}: lands on a name the format reserves for entry {fixed} of the payload")]
    ReservedEntry {
        /// The path the entry would land on.
        path: String,
        /// The position the format fixes that first component at.
        fixed: usize,
    },
    /// An entry is neither a regular file nor a directory.
    ///
    /// The same set [`crate::payload::PayloadError::UnsupportedEntry`] names,
    /// and the same vocabulary for `kind`. `unpack` refuses the payload for
    /// one of these; `verify` reports every one of them, because a report that
    /// stopped at the first would describe less of the artifact than it could.
    #[error("{path}: is a {kind}, and a payload holds only files and directories")]
    UnsupportedEntry {
        /// The entry's path inside the artifact.
        path: String,
        /// What the entry is instead.
        kind: String,
    },
    /// `manifest.native` names a file the index does not hold.
    ///
    /// The manifest's list is what a reader is handed when they ask what
    /// native code an artifact carries, and a row naming nothing is either a
    /// build that listed a file it did not pack or a manifest somebody
    /// rewrote. Either way the answer to the question is wrong.
    #[error("{path}: the manifest lists it as native code, and the index has no such file")]
    NativeRowMissing {
        /// The path the manifest's row named.
        path: String,
    },
    /// A `manifest.native` row records a machine the object does not have.
    #[error("{path}: the manifest records machine {recorded}, and the object is {actual}")]
    NativeMachineLie {
        /// The object's path inside the artifact.
        path: String,
        /// The machine the manifest's row recorded.
        recorded: String,
        /// The machine the object in the payload really has.
        actual: String,
    },
}

impl Issue {
    /// The path the issue is about.
    pub fn path(&self) -> &str {
        match self {
            Self::MachineMismatch { path, .. }
            | Self::UnexpectedNeeded { path, .. }
            | Self::IndexMismatch { path, .. }
            | Self::IndexSizeMismatch { path, .. }
            | Self::IndexModeMismatch { path, .. }
            | Self::IndexOrphan { path }
            | Self::IndexMissing { path }
            | Self::UnsafePath { path }
            | Self::ReservedEntry { path, .. }
            | Self::UnsupportedEntry { path, .. }
            | Self::NativeRowMissing { path }
            | Self::NativeMachineLie { path, .. }
            | Self::UnreadableObject { path, .. } => path,
        }
    }

    /// Where the issue sorts among the issues of one path.
    ///
    /// The order the variants are declared in, which is the order
    /// [`VerifyReport::issues`] documents: what the object *is*, then what it
    /// needs, then whether it could be read at all, then what the index says
    /// about its bytes, and last what the entry itself is. A reader who has
    /// one path's findings in front of them meets the file before the
    /// bookkeeping about it.
    fn rank(&self) -> u8 {
        match self {
            Self::MachineMismatch { .. } => 0,
            Self::UnexpectedNeeded { .. } => 1,
            Self::UnreadableObject { .. } => 2,
            Self::IndexMismatch { .. } => 3,
            Self::IndexSizeMismatch { .. } => 4,
            Self::IndexModeMismatch { .. } => 5,
            Self::IndexOrphan { .. } => 6,
            Self::IndexMissing { .. } => 7,
            Self::UnsafePath { .. } => 8,
            Self::ReservedEntry { .. } => 9,
            Self::UnsupportedEntry { .. } => 10,
            Self::NativeRowMissing { .. } => 11,
            Self::NativeMachineLie { .. } => 12,
        }
    }
}

/// What `ginary verify` found.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct VerifyReport {
    /// Version of this schema; see [`VERIFY_FORMAT_VERSION`].
    pub format_version: u32,
    /// The file, as it was named on the command line.
    pub path: String,
    /// The payload digest, as `inspect --verify` reports it.
    pub payload: Verification,
    /// How many payload entries were hashed against the index.
    pub files_checked: usize,
    /// Every native object the payload holds, in path order.
    pub objects: Vec<ObjectInfo>,
    /// Every issue, in path order and then in the order above.
    pub issues: Vec<Issue>,
}

impl VerifyReport {
    /// Whether the artifact is intact and raised nothing.
    ///
    /// This is what decides the exit code: zero when it is true, one when it
    /// is not.
    pub fn ok(&self) -> bool {
        self.payload.ok() && self.issues.is_empty()
    }

    /// The human-readable report.
    ///
    /// ```text
    /// payload:  ok
    /// files:    9 checked against the index
    /// objects:  1
    ///
    /// path                        machine  needed
    /// lib/hello/priv/bin/tool     x86_64   libc.so.6
    ///
    /// issues:
    ///   lib/hello/priv/bin/tool: needs `libssl.so.3`, which the artifact does not carry
    /// ```
    ///
    /// The `issues:` block is absent when there are none, because a heading
    /// with nothing under it reads as a finding rather than as its absence.
    pub fn render_text(&self) -> String {
        let mut text = String::new();
        let mut field = |label: &str, value: &str| {
            text.push_str(&format!("{:<LABEL_WIDTH$}{value}\n", format!("{label}:")));
        };

        field(
            "payload",
            &if self.payload.ok() {
                "ok".to_owned()
            } else {
                format!(
                    "MISMATCH (expected {}, actual {})",
                    self.payload.expected, self.payload.actual
                )
            },
        );
        field(
            "files",
            &format!("{} checked against the index", self.files_checked),
        );
        field("objects", &self.objects.len().to_string());

        if !self.objects.is_empty() {
            let rows: Vec<[String; 5]> = self
                .objects
                .iter()
                .map(|object| {
                    [
                        object.path.clone(),
                        object.machine.clone(),
                        object.class.to_string(),
                        object.glibc_max.clone().unwrap_or_else(|| DASH.to_owned()),
                        or_dash(&object.needed.join(", ")).to_owned(),
                    ]
                })
                .collect();
            text.push('\n');
            text.push_str(&crate::closure::render_table(
                ["path", "machine", "class", "glibc", "needed"],
                &rows,
            ));
        }

        if !self.issues.is_empty() {
            text.push_str("\nissues:\n");
            for issue in &self.issues {
                text.push_str(&format!("  {issue}\n"));
            }
        }
        text
    }
}

/// Verifies the artifact at `path` against the default allowlist.
///
/// # Errors
///
/// [`VerifyError`] when the file is not a packaged application, when its
/// payload cannot be read, or when it cannot be read at all. A *finding* is
/// never an error: an artifact with issues is a [`VerifyReport`] whose
/// [`VerifyReport::ok`] is false, and the caller decides what that means.
pub fn verify(path: &Path) -> Result<VerifyReport, VerifyError> {
    verify_with(path, &VerifyOptions::default())
}

/// Verifies the artifact at `path` against the allowlist `options` names.
///
/// # Errors
///
/// As [`verify`].
pub fn verify_with(path: &Path, options: &VerifyOptions<'_>) -> Result<VerifyReport, VerifyError> {
    let info = inspect::open(path)?;
    let payload = inspect::verify(&info)?;
    if !payload.ok() {
        // A payload whose digest is not the trailer's is not a payload to
        // read. Every entry past the damage is bytes nobody wrote, and a table
        // of findings about them would describe the damage rather than the
        // artifact — including a stream that stops decompressing part-way,
        // which is what a truncated payload actually does. The digest is the
        // finding, and it is the one a reader has to act on first.
        return Ok(VerifyReport {
            format_version: VERIFY_FORMAT_VERSION,
            path: path.display().to_string(),
            payload,
            files_checked: 0,
            objects: Vec::new(),
            issues: Vec::new(),
        });
    }

    // Everything the index says, removed as the payload accounts for it: what
    // is left at the end is what the artifact promised and did not carry.
    let mut expected: BTreeMap<&str, &IndexFile> = info
        .index
        .files
        .iter()
        .map(|file| (file.path.as_str(), file))
        .collect();

    let mut issues = Vec::new();
    let mut objects = Vec::new();
    let mut files_checked = 0usize;

    let mut archive = tar::Archive::new(
        zstd::stream::read::Decoder::new(payload_reader(path, &info)?).map_err(payload_io(path))?,
    );
    for (position, entry) in archive.entries().map_err(payload_io(path))?.enumerate() {
        let mut entry = entry.map_err(payload_io(path))?;
        let raw = entry_name(&entry);
        let size = entry.size();
        let kind = entry.header().entry_type();
        // Read before the entry is streamed, because the header is what the
        // index row is checked against and the reader moves past it.
        let mode = entry.header().mode().ok().map(|mode| mode & 0o7777);

        if position < FRONT_ENTRIES {
            // Entries 0 and 1 are the artifact's own description of itself and
            // are not in the index they carry. `inspect::open` has already
            // read both and checked that they are the two the format fixes.
            continue;
        }
        let Some(name) = destination(&entry) else {
            // First of the three, and before the index is consulted: an entry
            // that leaves the extracted root is one `payload::unpack` refuses
            // outright, so it is not a file the index can account for however
            // truthful a row an artifact's author wrote for it.
            issues.push(Issue::UnsafePath { path: raw });
            continue;
        };
        if let Some(fixed) = crate::payload::reserved_first_component(&name) {
            // By position and never by name: an entry that lands on a
            // front-matter name here is one `unpack` refuses the whole payload
            // for, so passing over it because it is *called* `ginary.json`
            // would let an artifact no launcher can extract verify clean.
            issues.push(Issue::ReservedEntry {
                path: name.clone(),
                fixed,
            });
            continue;
        }
        if kind == tar::EntryType::Directory {
            // The format permits one, for a directory that would otherwise be
            // lost, and the index lists files only. It carries no bytes, so
            // there is nothing to check it against.
            continue;
        }
        if kind != tar::EntryType::Regular {
            issues.push(Issue::UnsupportedEntry {
                path: name.clone(),
                kind: entry_kind(kind).to_owned(),
            });
            continue;
        }

        let scan =
            read_entry(&mut entry, size, options.max_object_bytes).map_err(payload_io(path))?;
        files_checked = files_checked.saturating_add(1);

        match expected.remove(name.as_str()) {
            // Every column the row carries, not only the digest: a row can
            // hold the right hash and the wrong metadata, because nothing in a
            // packer recomputes one of them from the other.
            Some(row) => issues.extend(row_issues(&name, row, scan.len, mode, &scan.sha256)),
            None => issues.push(Issue::IndexOrphan { path: name.clone() }),
        }

        match scan.object {
            Some(Ok(bytes)) => match crate::native::inspect_object_bytes(&bytes) {
                Ok(needs) => objects.push(describe(&name, &needs, &info, options)),
                Err(error) => issues.push(Issue::UnreadableObject {
                    path: name.clone(),
                    message: error.to_string(),
                }),
            },
            Some(Err(message)) => issues.push(Issue::UnreadableObject {
                path: name.clone(),
                message,
            }),
            None => {}
        }
    }

    for path in expected.into_keys() {
        issues.push(Issue::IndexMissing {
            path: path.to_owned(),
        });
    }

    objects.sort_by(|left, right| left.path.cmp(&right.path));
    issues.extend(native_issues(&info, &objects));
    for object in &objects {
        issues.extend(object.issues.iter().cloned());
    }
    // Stable, so the `DT_NEEDED` entries of one object keep the order the
    // dynamic section lists them in.
    issues.sort_by(|left, right| {
        left.path()
            .cmp(right.path())
            .then_with(|| left.rank().cmp(&right.rank()))
    });

    Ok(VerifyReport {
        format_version: VERIFY_FORMAT_VERSION,
        path: path.display().to_string(),
        payload,
        files_checked,
        objects,
        issues,
    })
}

/// Every way `manifest.native` disagrees with the artifact it describes.
///
/// The manifest's list is what a reader is handed when they ask what native
/// code an artifact carries, and nothing else in the archive is derived from
/// it: the index is computed from the tree the packer walked and the objects
/// are read out of the payload, so a row that names a file nobody packed, or
/// that records a machine the object does not have, is a claim only this
/// cross-check can deny. Both are the same class of defect — a manifest
/// somebody rewrote, or a build that listed what it did not ship — and neither
/// is visible from the digest, which describes the bytes and not what they were
/// said to be.
///
/// A row whose object is not an ELF is checked for its *presence* only: the
/// machine of a PE or a Mach-O is not read here, so there is nothing to hold
/// the manifest to.
fn native_issues(info: &ArtifactInfo, objects: &[ObjectInfo]) -> Vec<Issue> {
    let mut issues = Vec::new();
    for row in &info.manifest.native {
        if !info.index.files.iter().any(|file| file.path == row.path) {
            issues.push(Issue::NativeRowMissing {
                path: row.path.clone(),
            });
            continue;
        }
        let Some(recorded) = row.machine.as_deref() else {
            continue;
        };
        if let Some(object) = objects.iter().find(|object| object.path == row.path)
            && object.machine != recorded
        {
            issues.push(Issue::NativeMachineLie {
                path: row.path.clone(),
                recorded: recorded.to_owned(),
                actual: object.machine.clone(),
            });
        }
    }
    issues
}

/// Every way one payload entry disagrees with the index row that names it.
///
/// The three columns the row carries beside the path, in the order
/// [`Issue::rank`] gives them: the bytes, the length, then the permission
/// bits. They are independent — a row can hold the right digest and the wrong
/// length, because nothing recomputes one from the other — so all three are
/// checked and each one that fails is its own finding.
fn row_issues(
    name: &str,
    row: &IndexFile,
    len: u64,
    mode: Option<u32>,
    sha256: &str,
) -> Vec<Issue> {
    let mut issues = Vec::new();
    if row.sha256 != sha256 {
        issues.push(Issue::IndexMismatch {
            path: name.to_owned(),
            expected: row.sha256.clone(),
            actual: sha256.to_owned(),
        });
    }
    if row.size != len {
        issues.push(Issue::IndexSizeMismatch {
            path: name.to_owned(),
            expected: row.size,
            actual: len,
        });
    }
    let normalised = header_mode(row.mode);
    if mode != Some(normalised) {
        issues.push(Issue::IndexModeMismatch {
            path: name.to_owned(),
            indexed: octal(row.mode),
            expected: octal(normalised),
            actual: mode.map_or_else(|| "unreadable".to_owned(), octal),
        });
    }
    issues
}

/// The header mode a staged mode implies, as `docs/format.md` fixes it.
///
/// `HeaderMode::Deterministic` propagates the user execute bit and nothing
/// else, so `0755` for a staged mode that has it and `0644` for one that does
/// not. Comparing the columns for equality instead would report every
/// artifact staged from a tree whose modes are neither, which the format
/// permits.
fn header_mode(staged: u32) -> u32 {
    if staged & 0o100 == 0 { 0o644 } else { 0o755 }
}

/// Permission bits as the four octal digits a reader recognises.
fn octal(mode: u32) -> String {
    format!("{mode:04o}")
}

/// How many entries the format fixes at the front of the payload.
///
/// `ginary.json` and `ginary.index.json`, at positions 0 and 1. Everything
/// after them is a file of the application, and every rule below is about
/// *position* rather than about a name.
const FRONT_ENTRIES: usize = 2;

/// What an entry that is neither a file nor a directory is called.
///
/// The vocabulary [`crate::payload::PayloadError::UnsupportedEntry`] uses, so
/// the two commands name the same shapes the same way.
fn entry_kind(kind: tar::EntryType) -> &'static str {
    use tar::EntryType;

    match kind {
        EntryType::Continuous => "contiguous file",
        EntryType::Symlink => "symlink",
        EntryType::Link => "hardlink",
        EntryType::Char => "character device",
        EntryType::Block => "block device",
        EntryType::Fifo => "fifo",
        EntryType::GNULongName => "gnu long name",
        EntryType::GNULongLink => "gnu long link name",
        EntryType::GNUSparse => "gnu sparse",
        EntryType::XGlobalHeader | EntryType::XHeader => "pax",
        _ => "other",
    }
}

/// The width the report's labels are padded to.
///
/// `objects:` and `payload:` are the longest, so every value starts in the
/// same column.
const LABEL_WIDTH: usize = 10;

/// What an empty column prints as.
const DASH: &str = "-";

/// `value`, or [`DASH`] when it is empty.
fn or_dash(value: &str) -> &str {
    if value.is_empty() { DASH } else { value }
}

/// Builds one object's row, with the issues it raised.
fn describe(
    name: &str,
    object: &crate::native::ObjectNeeds,
    artifact: &ArtifactInfo,
    options: &VerifyOptions<'_>,
) -> ObjectInfo {
    let expected = artifact.manifest.target.arch.as_str();
    let allowlist = options
        .allowlist
        .unwrap_or_else(|| platform_allowlist(artifact.manifest.target.os));
    let mut issues = Vec::new();
    if object.machine != expected {
        issues.push(Issue::MachineMismatch {
            path: name.to_owned(),
            found: object.machine.clone(),
            expected: expected.to_owned(),
        });
    }
    for needed in &object.needed {
        if !needed_is_allowed(needed, allowlist) {
            issues.push(Issue::UnexpectedNeeded {
                path: name.to_owned(),
                needed: needed.clone(),
            });
        }
    }

    ObjectInfo {
        path: name.to_owned(),
        machine: object.machine.clone(),
        class: object.class,
        kind: object.kind,
        interp: object.interp.clone(),
        needed: object.needed.clone(),
        glibc_max: object.glibc_max.clone(),
        issues,
    }
}

/// What one streamed entry turned out to be.
struct Scan {
    /// The digest of every byte the entry held.
    sha256: String,
    /// How many bytes the entry held, which is what the index's `size`
    /// describes. It is counted rather than taken from the header, so the
    /// number a finding reports is the one the payload really carries.
    len: u64,
    /// The bytes, when the entry began with the ELF magic; `Err` when it did
    /// and this verifier would not hold it.
    object: Option<Result<Vec<u8>, String>>,
}

/// Streams one entry past a hasher, keeping it only when it is an object.
///
/// The first bytes decide: nothing is held for a file that is not native code,
/// and an entry whose header claims more than `bound` is refused before a byte
/// of it is kept rather than after.
///
/// Which bytes name an object is [`crate::platform::object_format_of`], and
/// not [`crate::elf::is_elf`]. Asking only the ELF question is what let a
/// Windows artifact — every object of which is a PE — verify as one holding
/// no objects at all, and the anti-vacuity guard of
/// `tests/verify.rs::a_real_artifact_verifies_clean` is what caught it.
fn read_entry(entry: &mut impl std::io::Read, size: u64, bound: u64) -> std::io::Result<Scan> {
    let mut hasher = sha2::Sha256::new();
    let mut len = 0u64;

    let mut head = Vec::with_capacity(OBJECT_MAGIC_BYTES);
    let mut byte = [0u8; 1];
    while head.len() < OBJECT_MAGIC_BYTES {
        if entry.read(&mut byte)? == 0 {
            break;
        }
        head.push(byte[0]);
    }
    hasher.update(&head);
    len = len.saturating_add(head.len() as u64);

    let mut object = if crate::platform::object_format_of(&head).is_some() {
        if size > bound {
            Some(Err(format!(
                "it is {size} bytes, and this verifier reads at most {bound}"
            )))
        } else {
            Some(Ok(head.clone()))
        }
    } else {
        None
    };

    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read = entry.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        len = len.saturating_add(read as u64);
        if let Some(Ok(bytes)) = &mut object {
            // The tar reader stops at the header's own length, and the header
            // was checked above; this is the second bound, in case it lied.
            if bytes.len().saturating_add(read) as u64 > bound {
                object = Some(Err(format!(
                    "it is longer than the {bound} bytes this verifier reads"
                )));
                continue;
            }
            bytes.extend_from_slice(&buffer[..read]);
        }
    }

    Ok(Scan {
        sha256: hex::encode(hasher.finalize()),
        len,
        object,
    })
}

/// One entry's path, lossily, exactly as its header carries it.
///
/// This is the name a *report* uses for an entry it will not accept, because
/// the point of such a report is what the artifact actually says. Everything
/// that is accepted is named by [`destination`] instead.
fn entry_name(entry: &tar::Entry<'_, impl std::io::Read>) -> String {
    String::from_utf8_lossy(&entry.path_bytes()).into_owned()
}

/// Where an entry would land, relative to the extracted root.
///
/// [`crate::payload::destined_path`], which is the rule `unpack` applies:
/// `None` for a name that is absolute, holds `..`, or normalises to nothing.
/// The `Some` is what the tar crate would create — `./ginary.json` and
/// `ginary.json` are one destination — so the name matched against the index
/// is the name the launcher would write.
fn destination(entry: &tar::Entry<'_, impl std::io::Read>) -> Option<String> {
    let path = entry.path().ok()?;
    crate::payload::destined_path(&path)
}

/// A reader over exactly the payload region of `path`.
fn payload_reader(path: &Path, info: &ArtifactInfo) -> Result<std::io::Take<File>, VerifyError> {
    let mut file = File::open(path).map_err(|source| VerifyError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    file.seek(SeekFrom::Start(info.trailer.payload_offset))
        .map_err(|source| VerifyError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    Ok(file.take(info.trailer.payload_len))
}

/// Turns a stream failure into [`VerifyError::Payload`] for `path`.
fn payload_io(path: &Path) -> impl Fn(std::io::Error) -> VerifyError {
    move |source| VerifyError::Payload {
        path: path.to_path_buf(),
        source: PayloadError::Io(source),
    }
}

/// Why an artifact could not be verified.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum VerifyError {
    /// The file is not a packaged application, or its front matter is broken.
    #[error(transparent)]
    Artifact(#[from] InspectError),
    /// The payload could not be streamed a second time.
    #[error("{path}: the payload cannot be read")]
    Payload {
        /// The file that was read.
        path: PathBuf,
        /// What is wrong with the payload.
        #[source]
        source: PayloadError,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_loader_is_matched_by_prefix_and_only_beside_its_own_libc() {
        assert!(needed_is_allowed("ld-linux-x86-64.so.2", &NEEDED_ALLOWLIST));
        assert!(needed_is_allowed(
            "ld-linux-aarch64.so.1",
            &NEEDED_ALLOWLIST
        ));
        assert!(
            !needed_is_allowed("ld-linux-x86-64.so.2", &["libm.so.6"]),
            "an allowlist that does not admit glibc does not admit its loader"
        );
        assert!(!needed_is_allowed("ld-linux-x86-64.so.2", &[]));
    }

    #[test]
    fn a_name_that_is_only_a_prefix_of_an_allowed_one_is_not_allowed() {
        assert!(!needed_is_allowed("libc.so", &NEEDED_ALLOWLIST));
        assert!(!needed_is_allowed("libc.so.6.1", &NEEDED_ALLOWLIST));
    }

    /// One of every [`Issue`] variant, on `path`, in the order declared.
    ///
    /// The exhaustive match is the reminder: a variant added to `Issue` stops
    /// this file compiling until it has a row here too, and the assertion then
    /// checks that `rank` gave it a place of its own.
    fn every_issue(path: &str) -> Vec<Issue> {
        let issues = vec![
            Issue::MachineMismatch {
                path: path.to_owned(),
                found: "aarch64".to_owned(),
                expected: "x86_64".to_owned(),
            },
            Issue::UnexpectedNeeded {
                path: path.to_owned(),
                needed: "libssl.so.3".to_owned(),
            },
            Issue::UnreadableObject {
                path: path.to_owned(),
                message: "truncated".to_owned(),
            },
            Issue::IndexMismatch {
                path: path.to_owned(),
                expected: "a".repeat(64),
                actual: "b".repeat(64),
            },
            Issue::IndexSizeMismatch {
                path: path.to_owned(),
                expected: 1,
                actual: 2,
            },
            Issue::IndexModeMismatch {
                path: path.to_owned(),
                indexed: "0700".to_owned(),
                expected: "0755".to_owned(),
                actual: "0644".to_owned(),
            },
            Issue::IndexOrphan {
                path: path.to_owned(),
            },
            Issue::IndexMissing {
                path: path.to_owned(),
            },
            Issue::UnsafePath {
                path: path.to_owned(),
            },
            Issue::ReservedEntry {
                path: path.to_owned(),
                fixed: 0,
            },
            Issue::UnsupportedEntry {
                path: path.to_owned(),
                kind: "symlink".to_owned(),
            },
            Issue::NativeRowMissing {
                path: path.to_owned(),
            },
            Issue::NativeMachineLie {
                path: path.to_owned(),
                recorded: "x86_64".to_owned(),
                actual: "aarch64".to_owned(),
            },
        ];
        for issue in &issues {
            match issue {
                Issue::MachineMismatch { .. }
                | Issue::UnexpectedNeeded { .. }
                | Issue::UnreadableObject { .. }
                | Issue::IndexMismatch { .. }
                | Issue::IndexSizeMismatch { .. }
                | Issue::IndexModeMismatch { .. }
                | Issue::IndexOrphan { .. }
                | Issue::IndexMissing { .. }
                | Issue::UnsafePath { .. }
                | Issue::ReservedEntry { .. }
                | Issue::UnsupportedEntry { .. }
                | Issue::NativeRowMissing { .. }
                | Issue::NativeMachineLie { .. } => {}
            }
        }
        let ranks: Vec<u8> = issues.iter().map(Issue::rank).collect();
        let places: Vec<u8> = (0..u8::try_from(issues.len()).expect("thirteen variants")).collect();
        assert_eq!(
            ranks, places,
            "`rank` is the declaration order, one place each"
        );
        issues
    }

    /// The order [`VerifyReport::issues`] is documented to be in.
    fn issue_order(left: &Issue, right: &Issue) -> std::cmp::Ordering {
        left.path()
            .cmp(right.path())
            .then_with(|| left.rank().cmp(&right.rank()))
    }

    #[test]
    fn the_issues_of_one_path_sort_in_the_order_they_are_declared() {
        let declared = every_issue("a");
        let mut shuffled: Vec<Issue> = declared.iter().rev().cloned().collect();

        shuffled.sort_by(issue_order);

        assert_eq!(
            shuffled, declared,
            "sorting the whole set backwards has to give the declaration order back"
        );
    }

    #[test]
    fn the_path_decides_before_the_kind_does() {
        // The last variant on the first path against the first variant on the
        // second: only a comparison that reads the path first orders these.
        let mut issues: Vec<Issue> = vec![
            every_issue("b").swap_remove(0),
            every_issue("a").pop().expect("thirteen variants"),
        ];

        issues.sort_by(issue_order);

        assert_eq!(issues[0].path(), "a");
        assert!(matches!(issues[0], Issue::NativeMachineLie { .. }));
        assert_eq!(issues[1].path(), "b");
    }

    /// The bytes of an entry that begins with the ELF magic.
    fn elf_bytes(length: usize) -> Vec<u8> {
        let mut bytes = crate::elf::ELF_MAGIC.to_vec();
        bytes.resize(length.max(crate::elf::ELF_MAGIC.len()), 0u8);
        bytes
    }

    #[test]
    fn an_entry_whose_header_over_claims_is_reported_rather_than_held() {
        let bytes = elf_bytes(32);
        let claimed = 4096;

        let scan = read_entry(&mut bytes.as_slice(), claimed, 64).expect("the entry streams");

        let Some(Err(message)) = scan.object else {
            panic!("expected a refusal, got {:?}", scan.object)
        };
        assert!(message.contains("4096"), "{message}");
        assert!(message.contains("64"), "{message}");
        assert_eq!(
            scan.sha256,
            hex::encode(sha2::Sha256::digest(&bytes)),
            "the entry is still hashed against the index"
        );
    }

    #[test]
    fn an_entry_longer_than_its_header_claimed_is_refused_on_the_way_past() {
        // The tar reader stops at the header's own length, so this is the
        // second bound: it exists for a reader that does not, and a unit test
        // is the only place it can be reached.
        let bytes = elf_bytes(4096);

        let scan = read_entry(&mut bytes.as_slice(), 8, 64).expect("the entry streams");

        let Some(Err(message)) = scan.object else {
            panic!("expected a refusal, got {:?}", scan.object)
        };
        assert!(message.contains("longer than"), "{message}");
        assert_eq!(scan.sha256, hex::encode(sha2::Sha256::digest(&bytes)));
    }

    #[test]
    fn an_entry_that_is_not_an_elf_is_hashed_and_never_held() {
        let bytes = b"#!/bin/sh\nexit 0\n".to_vec();

        let scan =
            read_entry(&mut bytes.as_slice(), bytes.len() as u64, 64).expect("the entry streams");

        assert!(scan.object.is_none(), "{:?}", scan.object);
        assert_eq!(scan.sha256, hex::encode(sha2::Sha256::digest(&bytes)));
    }

    #[test]
    fn an_entry_shorter_than_the_magic_is_not_an_elf() {
        let bytes = vec![0x7f, b'E'];

        let scan =
            read_entry(&mut bytes.as_slice(), bytes.len() as u64, 64).expect("the entry streams");

        assert!(scan.object.is_none(), "{:?}", scan.object);
    }

    #[test]
    fn an_empty_column_prints_a_dash() {
        assert_eq!(or_dash(""), DASH);
        assert_eq!(or_dash("libc.so.6"), "libc.so.6");
    }
}
