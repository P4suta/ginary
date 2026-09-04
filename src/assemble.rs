// SPDX-License-Identifier: MIT OR Apache-2.0
//! Building the staging root: the exact tree that becomes the payload.
//!
//! [`stage`] takes the bill of materials [`crate::closure`] produced and the
//! runtime [`crate::otp`] found, and writes the directory that will be tarred,
//! shipped inside the artifact, and extracted into the cache at first run.
//! What comes out is not a copy of an OTP installation with pieces missing; it
//! is a tree assembled from an allowlist, so a file is in the artifact because
//! something decided to put it there.
//!
//! ```text
//! <out>/
//!   bin/no_dot_erlang.boot         the only boot script; kernel and stdlib
//!   erts-<vsn>/bin/                four binaries, plus whatever --extra-bin named
//!     beam.smp erlexec erl_child_setup inet_gethost
//!   lib/<name>-<vsn>/{ebin,priv}   an application from the OTP library
//!   lib/<name>/{ebin,priv}         an application from the shipment
//!   ginary.stage.json              what was staged, why, and how big it is
//! ```
//!
//! Four rules shape it, and each one has a test that would fail without it.
//!
//! **The result is atomic.** Staging happens in a sibling `<out>.tmp-<pid>`
//! directory and is renamed onto `out` at the very end, so a failure half way
//! through leaves neither a partial `out` nor a temporary tree behind. A caller
//! that finds `out` finds it complete.
//!
//! **Nothing is copied by default.** Under `erts-<vsn>/bin` only
//! [`crate::otp::REQUIRED_ERTS_BINARIES`] and the names in
//! [`StageOptions::extra_bins`] are taken; every other program that was there
//! is recorded, with a reason, in [`StagedRoot::excluded_erts_bins`]. Under an
//! application only `ebin` and `priv` are taken and `*.appup` is dropped, so
//! [`EXCLUDED_APP_DIRS`] never travels — not under its own name, and not under
//! `ebin` or `priv` by way of a symlink, whether the link is inside them or is
//! one of them. A Windows runtime's `bin` is read by [`windows_required_bins`]
//! instead — the three names in [`WINDOWS_REQUIRED_BINS`] and every DLL beside
//! them but [`WINDOWS_DEBUG_EMULATOR_DLL`] — and which of the two lists applies
//! is read off the tree rather than off the target that was asked for.
//!
//! **The boot file is checked against the tree.** `no_dot_erlang.boot` names
//! the `kernel` and `stdlib` versions it was built against, as literal
//! `$ROOT/lib/<name>-<vsn>/ebin` paths. If the staged tree does not hold
//! exactly those directories the runtime will not boot, and it will not say
//! why, so [`AssembleError::BootReferencesMissingApp`] says it here instead.
//!
//! **The tree is described by a file inside it.** `ginary.stage.json` lists
//! every file with its size, its mode and its category. It is the precursor of
//! the artifact's `ginary.index.json` and it is what the size report reads;
//! nothing in it is a timestamp, so staging the same inputs twice produces the
//! same bytes.

#[cfg(feature = "cli")]
use std::collections::{BTreeMap, BTreeSet};
#[cfg(feature = "cli")]
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[cfg(feature = "cli")]
use crate::closure::{self, AppSet};
#[cfg(feature = "cli")]
use crate::otp::OtpInfo;

/// The name of the listing [`stage`] writes at the root of the staged tree.
pub const LISTING_NAME: &str = "ginary.stage.json";

/// The directories of an application that are never staged.
///
/// Sources, headers, documentation and build inputs are all present in a real
/// OTP `lib` and in some hex packages, and none of them is read at run time.
/// They are left behind structurally rather than by a filter: [`stage`] copies
/// an application's `ebin` and its `priv` and nothing else, so this list names
/// what that leaves in the source tree rather than what a pass deletes.
///
/// Nothing *inside* `ebin` or `priv` is pruned by name, and that is deliberate.
/// `snmp` ships its compiled MIBs as `priv/mibs/*.bin` and loads them at run
/// time; a filter that matched these names at any depth would drop them and
/// the application would fail only when it looked for one.
pub const EXCLUDED_APP_DIRS: [&str; 6] = ["src", "include", "doc", "examples", "c_src", "mibs"];

/// What a staged file is, for the size report and for `--explain`.
///
/// The order of the variants is the order [`StagedRoot::bytes_by_category`]
/// returns them in, and it is a reading order rather than an alphabetical one:
/// the runtime first, then the code, then the data.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    /// A program under `erts-<vsn>/bin`.
    ErtsBinary,
    /// `bin/no_dot_erlang.boot`.
    Boot,
    /// A `.beam` under an application that came from the OTP library.
    OtpBeam,
    /// A `.beam` under an application that came from the shipment.
    GleamBeam,
    /// Anything under an application's `priv`.
    Priv,
    /// An application's `.app` file.
    AppResource,
    /// Anything else that was staged.
    Other,
}

impl Category {
    /// The word this category prints as, matching its JSON spelling.
    pub fn label(self) -> &'static str {
        match self {
            Self::ErtsBinary => "erts_binary",
            Self::Boot => "boot",
            Self::OtpBeam => "otp_beam",
            Self::GleamBeam => "gleam_beam",
            Self::Priv => "priv",
            Self::AppResource => "app_resource",
            Self::Other => "other",
        }
    }
}

impl std::fmt::Display for Category {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// Which tree a staged application was copied from.
///
/// [`crate::closure::AppSource`] carries the OTP version as well, which the
/// listing already has in its own field; this is the one-word answer the
/// listing and the table print.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StagedSource {
    /// From the OTP library, staged as `lib/<name>-<vsn>`.
    Otp,
    /// From the shipment, staged as `lib/<name>`.
    Shipment,
}

impl StagedSource {
    /// The word this source prints as, matching its JSON spelling.
    pub fn label(self) -> &'static str {
        match self {
            Self::Otp => "otp",
            Self::Shipment => "shipment",
        }
    }
}

/// One file in the staged tree.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StagedFile {
    /// The path relative to the staged root, `/`-separated.
    ///
    /// Relative and slash-separated so that the listing is the same on every
    /// machine: the absolute path of the staging directory is not part of what
    /// was built.
    pub path: String,
    /// The exact size in bytes.
    pub size: u64,
    /// The Unix permission bits, `st_mode & 0o7777`, or zero without them.
    pub mode: u32,
    /// What the file is.
    pub category: Category,
}

/// One application in the staged tree.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StagedApp {
    /// The application name.
    pub name: String,
    /// The version from its `.app` file.
    pub vsn: String,
    /// Which tree it was copied from.
    pub source: StagedSource,
    /// Its directory relative to the staged root, `/`-separated.
    ///
    /// `lib/<name>-<vsn>` for an OTP application and `lib/<name>` for a
    /// shipment one, which is the difference the launcher's `-pa` turns on.
    pub dir: String,
    /// How many files were staged under it.
    pub files: usize,
    /// How many bytes were staged under it.
    pub bytes: u64,
}

/// A program that was in the runtime's `bin` and was deliberately not staged.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExcludedBin {
    /// The program's name.
    pub name: String,
    /// One line saying why it is not in the artifact.
    pub reason: String,
}

/// The contents of `ginary.stage.json`.
///
/// Deliberately not the whole of [`StagedRoot`]: the listing describes what was
/// built, not where it was built, so it holds no absolute path and no
/// timestamp. Two runs over the same inputs produce the same bytes.
///
/// The listing does not list itself. Its own size is not knowable until it has
/// been serialised, and a file whose contents depend on its own length is not a
/// file anyone can reproduce.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageListing {
    /// The ERTS version, and with it the `erts-<vsn>` directory name.
    pub erts_vsn: String,
    /// The OTP release, as `erlang:system_info(otp_release)` reports it.
    pub otp_release: u32,
    /// The full OTP version, as `erlang:system_info(version)` reports it.
    pub otp_version: String,
    /// The applications, in name order.
    pub apps: Vec<StagedApp>,
    /// Every staged file, sorted by [`StagedFile::path`].
    pub files: Vec<StagedFile>,
}

/// How [`stage`] should build the tree.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg(feature = "cli")]
pub struct StageOptions {
    /// Programs to stage from the runtime's `bin` beyond the required four.
    ///
    /// `heart` and `epmd` are the two an application can genuinely need, and
    /// neither is staged unless it is named here: `heart` only matters to an
    /// application that asks the runtime to supervise it, and `epmd` only to a
    /// distributed one.
    pub extra_bins: Vec<String>,
    /// Whether to delete the known-useless files listed in
    /// [`StagedRoot::junk_removed`]. On by default.
    pub remove_junk: bool,
    /// Whether to replace an output directory that is not empty.
    pub force: bool,
}

#[cfg(feature = "cli")]
impl Default for StageOptions {
    /// Nothing extra, junk removed, an existing output directory refused.
    fn default() -> Self {
        Self {
            extra_bins: Vec::new(),
            remove_junk: true,
            force: false,
        }
    }
}

/// A staged tree, and the account of what went into it.
///
/// Every field is derived from the tree that was written, so a report built
/// from this describes the artifact rather than the intention.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg(feature = "cli")]
pub struct StagedRoot {
    /// The directory that was written.
    root: PathBuf,
    /// The ERTS version, and with it the `erts-<vsn>` directory name.
    erts_vsn: String,
    /// The OTP release the runtime came from.
    otp_release: u32,
    /// The full OTP version the runtime came from.
    otp_version: String,
    /// The applications, in name order.
    apps: Vec<StagedApp>,
    /// Every staged file, sorted by [`StagedFile::path`].
    files: Vec<StagedFile>,
    /// What junk removal deleted, as `(path relative to the root, bytes)`.
    ///
    /// A directory is one entry carrying the total of the files it held.
    junk_removed: Vec<(PathBuf, u64)>,
    /// The programs that were in the runtime's `bin` and were not staged.
    excluded_erts_bins: Vec<ExcludedBin>,
    /// The `<name>-<vsn>` directories the boot file names, in the order it
    /// names them. Every one of them was checked to exist in the staged tree.
    boot_refs: Vec<String>,
}

#[cfg(feature = "cli")]
impl StagedRoot {
    /// The directory that was written.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The ERTS version, and with it the `erts-<vsn>` directory name.
    pub fn erts_vsn(&self) -> &str {
        &self.erts_vsn
    }

    /// The OTP release the runtime came from.
    pub fn otp_release(&self) -> u32 {
        self.otp_release
    }

    /// The full OTP version the runtime came from.
    pub fn otp_version(&self) -> &str {
        &self.otp_version
    }

    /// The applications, in name order.
    pub fn apps(&self) -> &[StagedApp] {
        &self.apps
    }

    /// Every staged file, sorted by [`StagedFile::path`].
    pub fn files(&self) -> &[StagedFile] {
        &self.files
    }

    /// What junk removal deleted, as `(path relative to the root, bytes)`.
    pub fn junk_removed(&self) -> &[(PathBuf, u64)] {
        &self.junk_removed
    }

    /// The programs that were in the runtime's `bin` and were not staged.
    pub fn excluded_erts_bins(&self) -> &[ExcludedBin] {
        &self.excluded_erts_bins
    }

    /// The `<name>-<vsn>` directories the boot file names.
    pub fn boot_refs(&self) -> &[String] {
        &self.boot_refs
    }

    /// The total size of the staged tree, `ginary.stage.json` aside.
    pub fn total_bytes(&self) -> u64 {
        self.files.iter().map(|file| file.size).sum()
    }

    /// Bytes and file counts per category, in [`Category`] order.
    pub fn bytes_by_category(&self) -> BTreeMap<Category, (u64, usize)> {
        let mut totals: BTreeMap<Category, (u64, usize)> = BTreeMap::new();
        for file in &self.files {
            let entry = totals.entry(file.category).or_insert((0, 0));
            entry.0 += file.size;
            entry.1 += 1;
        }
        totals
    }

    /// The listing written to `ginary.stage.json`.
    pub fn listing(&self) -> StageListing {
        StageListing {
            erts_vsn: self.erts_vsn.clone(),
            otp_release: self.otp_release,
            otp_version: self.otp_version.clone(),
            apps: self.apps.clone(),
            files: self.files.clone(),
        }
    }

    /// Writes one more file into the staged tree and into the listing.
    ///
    /// This is how the runtime settings reach the artifact: `[tools.ginary]
    /// vm_args` and `sys_config` name files in the *project*, and the launcher
    /// needs them at fixed root-relative paths inside the extracted tree. They
    /// are not applications and not part of the runtime, so nothing in
    /// [`stage`] would have copied them.
    ///
    /// The listing is rewritten, because `ginary.stage.json` is what
    /// [`crate::payload::pack`] packs from and what it checks the tree
    /// against: a file the listing does not name would be either packed and
    /// undescribed or dropped without a word.
    ///
    /// `path` is root-relative and `/`-separated, and is the caller's to
    /// choose: it is not a value that comes from a project.
    ///
    /// # Errors
    ///
    /// [`AssembleError::Io`] when the file, its parent directory, its mode or
    /// the listing cannot be written.
    pub fn add_file(
        &mut self,
        path: &str,
        contents: &[u8],
        mode: u32,
        category: Category,
    ) -> Result<(), AssembleError> {
        let full = self.root.join(path);
        if let Some(parent) = full.parent() {
            create_dir(parent)?;
        }
        std::fs::write(&full, contents).map_err(|source| AssembleError::Io {
            path: full.clone(),
            source,
        })?;
        set_mode(&full, mode)?;

        let staged = StagedFile {
            path: path.to_owned(),
            size: contents.len() as u64,
            mode,
            category,
        };
        match self.files.iter_mut().find(|file| file.path == path) {
            Some(existing) => *existing = staged,
            None => self.files.push(staged),
        }
        self.files.sort_by(|left, right| left.path.cmp(&right.path));
        write_listing(&self.root.join(LISTING_NAME), &self.listing())
    }

    /// Re-reads the tree and rewrites `ginary.stage.json`.
    ///
    /// Stripping rewrites files in place, so every size this struct holds — and
    /// every size the listing on disk holds — is stale the moment
    /// `strip::strip` returns. Refreshing re-stats each file in
    /// [`StagedRoot::files`], recomputes the per-application totals, and writes
    /// the listing again, so the tree keeps describing itself.
    ///
    /// The account that is *not* derived from the tree — the excluded ERTS
    /// binaries, the junk that was removed, the boot references that were
    /// checked — is carried over unchanged, because it is a record of what
    /// staging decided and no later phase can re-derive it.
    ///
    /// # Errors
    ///
    /// [`AssembleError::Io`] when a staged file cannot be stat'd or the listing
    /// cannot be written.
    pub fn refresh(&self) -> Result<Self, AssembleError> {
        let mut files = self.files.clone();
        for file in &mut files {
            file.size = size_of(&self.root.join(&file.path))?;
        }
        let mut apps = self.apps.clone();
        count_apps(&mut apps, &files);

        let listing = StageListing {
            erts_vsn: self.erts_vsn.clone(),
            otp_release: self.otp_release,
            otp_version: self.otp_version.clone(),
            apps: apps.clone(),
            files: files.clone(),
        };
        write_listing(&self.root.join(LISTING_NAME), &listing)?;

        Ok(Self {
            apps,
            files,
            ..self.clone()
        })
    }

    /// Renders the whole account: sizes, applications, exclusions, junk, boot.
    ///
    /// This is what `ginary stage --explain` prints, and what
    /// `ginary build --explain` will fold into its own report. It answers the
    /// only question anyone asks about an artifact that is bigger than
    /// expected: what is in it, and what was left out.
    pub fn explain(&self) -> String {
        let mut text = String::new();

        let totals = self.bytes_by_category();
        let mut rows: Vec<[String; 3]> = totals
            .iter()
            .map(|(category, (bytes, files))| {
                [
                    category.label().to_owned(),
                    bytes.to_string(),
                    files.to_string(),
                ]
            })
            .collect();
        rows.push([
            "total".to_owned(),
            self.total_bytes().to_string(),
            self.files.len().to_string(),
        ]);
        text.push_str(&closure::render_table(
            ["category", "bytes", "files"],
            &rows,
        ));

        let rows: Vec<[String; 5]> = self
            .apps
            .iter()
            .map(|app| {
                [
                    app.name.clone(),
                    app.vsn.clone(),
                    app.source.label().to_owned(),
                    app.files.to_string(),
                    app.bytes.to_string(),
                ]
            })
            .collect();
        text.push('\n');
        text.push_str(&closure::render_table(
            ["app", "vsn", "source", "files", "bytes"],
            &rows,
        ));

        if !self.excluded_erts_bins.is_empty() {
            text.push_str("\nexcluded erts binaries:\n");
            let width = self
                .excluded_erts_bins
                .iter()
                .map(|bin| bin.name.len())
                .max()
                .unwrap_or(0);
            for bin in &self.excluded_erts_bins {
                text.push_str(&format!(
                    "  {name:width$}  {reason}\n",
                    name = bin.name,
                    reason = bin.reason
                ));
            }
        }

        if !self.junk_removed.is_empty() {
            text.push_str("\njunk removed:\n");
            for (path, bytes) in &self.junk_removed {
                text.push_str(&format!("  {} ({bytes} bytes)\n", path.display()));
            }
        }

        if !self.boot_refs.is_empty() {
            text.push_str("\nboot references checked:\n");
            for reference in &self.boot_refs {
                text.push_str(&format!("  {reference}\n"));
            }
        }

        text
    }
}

/// Why a staging root could not be built.
#[derive(Debug, thiserror::Error)]
#[cfg(feature = "cli")]
pub enum AssembleError {
    /// The output directory exists and is not empty.
    #[error(
        "`{path}` already exists and is not empty; pass --force to replace it, or name a directory that does not exist"
    )]
    OutputNotEmpty {
        /// The directory that was in the way.
        path: PathBuf,
    },
    /// One of [`crate::otp::REQUIRED_ERTS_BINARIES`] is not in the runtime.
    #[error("the ERTS binary `{name}` is missing; looked for it at `{searched}`")]
    MissingErtsBinary {
        /// The binary that should have been there.
        name: String,
        /// The path that was looked at.
        searched: PathBuf,
    },
    /// One of [`WINDOWS_REQUIRED_BINS`] is in the runtime under a spelling
    /// that differs from it only in case.
    ///
    /// Its own answer rather than a longer [`Self::MissingErtsBinary`]: a
    /// tree that never had the file and a tree that has it under another name
    /// are two problems, and only one of them is fixed by renaming. See
    /// [`windows_required_bins`] for why the three names are matched exactly.
    #[error(
        "the ERTS binary `{name}` is not in the runtime under that name; the tree spells it `{found}`, and ginary names this file exactly — the flavour test, the launcher's preflight and the artifact index all spell it `{name}` — so rename it in the tree, or use a runtime that spells it that way; looked for it at `{searched}`"
    )]
    ErtsBinaryNamedInAnotherCase {
        /// The name ginary needs, exactly.
        name: String,
        /// The name the tree really spells it with.
        found: String,
        /// The path that was looked at.
        searched: PathBuf,
    },
    /// A binary named in [`StageOptions::extra_bins`] is not in the runtime.
    #[error(
        "the extra ERTS binary `{name}` is not in the runtime's `bin` directory; drop it from --extra-bin or use a runtime that has it"
    )]
    MissingExtraBinary {
        /// The binary that was asked for and is not there.
        name: String,
    },
    /// A name in [`StageOptions::extra_bins`] is not a program name.
    ///
    /// The name is joined onto the runtime's `bin` directory and onto the
    /// staged one, so a name holding a separator or `..` would read and write
    /// outside both trees. Checked here as well as in
    /// [`crate::config::ToolsConfig::validate`], because staging is what
    /// performs the copy and a rule enforced only by its caller is a rule one
    /// caller can forget.
    #[error(
        "`{name}` is not the name of a program in the runtime's `bin` directory: a name is a file name, not a path"
    )]
    UnusableExtraBinary {
        /// The name that was refused.
        name: String,
    },
    /// The boot file names a library directory the staged tree does not hold.
    ///
    /// A boot script hardcodes the `kernel` and `stdlib` versions it was
    /// generated against. Booting against a tree that holds different ones
    /// fails inside the runtime with no useful message, so the mismatch is
    /// refused here, where both halves can be named.
    #[error(
        "the boot file `{boot}` requires `$ROOT/lib/{dir}/ebin`, and the staged tree holds {} for that application; a boot file hardcodes the kernel and stdlib versions it was generated against, so the runtime it came from and the applications being staged have to be the same installation",
        or_nothing(.staged)
    )]
    BootReferencesMissingApp {
        /// The `<name>-<vsn>` directory the boot file named.
        dir: String,
        /// The directories the staged tree holds for the same application,
        /// sorted. Usually exactly one — the version the closure resolved —
        /// which is the half of the mismatch the boot file cannot say.
        staged: Vec<String>,
        /// The boot file that named it.
        boot: PathBuf,
    },
    /// A symlink in an application points outside its own directory, or at
    /// nothing at all.
    ///
    /// Staging dereferences symlinks, so a link out of the application
    /// directory would pull an arbitrary file of the build machine into the
    /// artifact, and a dangling one would abort the copy half way through.
    #[error(
        "the symlink `{path}` points to `{target}`, which is outside the application directory or does not exist"
    )]
    UnsafeSymlink {
        /// The link that was found.
        path: PathBuf,
        /// Where it pointed, as resolved as it could be.
        target: PathBuf,
    },
    /// An application's `ebin` or `priv` is itself a symlink that reaches a
    /// directory staging deliberately leaves behind.
    ///
    /// The exclusion of [`EXCLUDED_APP_DIRS`] is structural, and a structural
    /// rule a symlink can step around is not a rule. A link to a directory
    /// found *inside* an `ebin` or a `priv` is held to that `ebin` or `priv`,
    /// which is what stops `ebin/sources -> ../src`. The `ebin` and the `priv`
    /// have no enclosing subtree to be held to, so they are held to the
    /// exclusion directly: an application whose `priv` *is* a link to its own
    /// `src` would otherwise stage the whole of `src` under another name,
    /// silently, because the target is inside the application and the weaker
    /// boundary is satisfied.
    #[error(
        "the symlink `{path}` points to `{target}`, which is `{excluded}` or is inside it; `{excluded}` is one of the directories that never travel, so following the link would stage it under another name"
    )]
    ExcludedSymlinkTarget {
        /// The `ebin` or `priv` that was a link.
        path: PathBuf,
        /// The directory it resolved to, inside the application.
        target: PathBuf,
        /// The excluded directory name the target is, or is inside.
        excluded: String,
    },
    /// A symlink in an application points at a directory that contains it.
    ///
    /// Staging dereferences symlinks, so `priv/loop -> .` describes a tree of
    /// infinite depth. The copy stops at the link that closes the loop and
    /// names it, rather than recursing until the destination path is longer
    /// than the filesystem allows and blaming whichever file it was copying.
    #[error(
        "the symlink `{path}` points to `{target}`, which already contains it; following it would never end"
    )]
    SymlinkCycle {
        /// The link that closed the loop.
        path: PathBuf,
        /// The directory it pointed back at.
        target: PathBuf,
    },
    /// A file's name is not valid UTF-8.
    ///
    /// Every path in `ginary.stage.json`, and later in the payload index, is
    /// text. A name that cannot be spelled as text could be copied but not
    /// described, and an artifact holding a file its own listing does not
    /// mention is worse than one that was never built.
    #[error(
        "cannot stage `{path}`: its name is not valid UTF-8, and every path ginary records is text; rename it, or leave it out of the application"
    )]
    NonUtf8Name {
        /// The path that could not be named.
        path: PathBuf,
    },
    /// A file could not be copied into the staged tree.
    ///
    /// Distinct from [`AssembleError::Io`] because both halves matter: a copy
    /// fails as often for what the destination cannot take as for what the
    /// source will not give.
    #[error("cannot copy `{from}` to `{to}`: {source}")]
    Copy {
        /// The file that was being read.
        from: PathBuf,
        /// The path that was being written.
        to: PathBuf,
        /// The underlying operating system error.
        #[source]
        source: std::io::Error,
    },
    /// A path could not be read, written or removed.
    #[error("cannot stage `{path}`: {source}")]
    Io {
        /// The path that could not be used.
        path: PathBuf,
        /// The underlying operating system error.
        #[source]
        source: std::io::Error,
    },
}

/// Renders a list of directory names, or `nothing` when it is empty.
///
/// [`AssembleError::BootReferencesMissingApp`] names both halves of the
/// mismatch, and "holds nothing for that application" is a different and more
/// useful sentence than "holds []".
#[cfg(feature = "cli")]
fn or_nothing(names: &[String]) -> String {
    if names.is_empty() {
        "nothing".to_owned()
    } else {
        names
            .iter()
            .map(|name| format!("`{name}`"))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// One line saying why a program in the runtime's `bin` is not staged.
///
/// The reasons are a policy, not a description, which is why they live in one
/// place: `ginary stage --explain` prints them, and a reader who disagrees with
/// one of them is disagreeing with the decision rather than with an accident.
#[cfg(feature = "cli")]
pub fn excluded_reason(name: &str) -> &'static str {
    // The Windows name first, and case-insensitively, for the reason
    // [`is_windows_library`] compares its suffix that way: a Windows
    // filesystem does not distinguish the spellings and the zip this tree came
    // out of is under no obligation to pick one. The unix names below are
    // compared exactly, because there the two spellings are two files.
    if name.eq_ignore_ascii_case(WINDOWS_DEBUG_EMULATOR_DLL) {
        return DEBUG_EMULATOR_REASON;
    }
    match name {
        "erl" => "the shell wrapper; the launcher execs erlexec directly",
        "erlc" | "escript" | "dialyzer" | "typer" | "ct_run" | "yielding_c_fun" => {
            "a build-time tool, never run by a packaged application"
        }
        "run_erl" | "to_erl" | "start" | "start_erl" => {
            "a detached-console helper; a packaged application runs in the foreground"
        }
        "erl_call" => "a distribution client; the packaged runtime starts no node",
        "epmd" => "the port mapper daemon; add it with --extra-bin for a distributed application",
        "heart" => "the heartbeat supervisor; add it with --extra-bin if the application uses it",
        "dyn_erl" | "erl.src" | "start.src" | "start_erl.src" => {
            "an installation-time template, not a program"
        }
        "beam.debug.smp" => DEBUG_EMULATOR_REASON,
        _ => "not on the launcher's allowlist",
    }
}

/// Why [`WINDOWS_DEBUG_EMULATOR_DLL`] and its unix counterpart are left
/// behind.
///
/// One sentence, named, because the decline and the reason are one policy
/// read from two places and a spelling that reached one of them and not the
/// other reported the file as "not on the launcher's allowlist" when the
/// answer is that nothing on a user's machine could load it.
#[cfg(feature = "cli")]
const DEBUG_EMULATOR_REASON: &str =
    "the debug emulator; nothing loads it and it needs a C runtime no user's machine has";

/// Builds the staging root at `out` from a closure and a runtime.
///
/// `out` must not exist, or must be an empty directory, unless
/// [`StageOptions::force`] is set. The tree is built in a sibling
/// `<out>.tmp-<pid>` directory and renamed onto `out` once it is complete, so
/// `out` either does not exist or is finished; a failure removes the temporary
/// directory rather than leaving it behind.
///
/// # Errors
///
/// [`AssembleError::OutputNotEmpty`] when `out` is in the way;
/// [`AssembleError::MissingErtsBinary`] and
/// [`AssembleError::MissingExtraBinary`] when the runtime does not hold a
/// program that has to be staged; [`AssembleError::BootReferencesMissingApp`]
/// when the boot file and the applications disagree;
/// [`AssembleError::UnsafeSymlink`] when an application holds a link that
/// leaves it; [`AssembleError::ExcludedSymlinkTarget`] when its `ebin` or
/// `priv` is a link to a directory that never travels;
/// [`AssembleError::SymlinkCycle`] when one loops back on itself;
/// [`AssembleError::NonUtf8Name`] when a file cannot be named as text; and
/// [`AssembleError::Copy`] or [`AssembleError::Io`] for anything the
/// filesystem refuses.
#[cfg(feature = "cli")]
pub fn stage(
    set: &AppSet,
    otp: &OtpInfo,
    opts: &StageOptions,
    out: &Path,
) -> Result<StagedRoot, AssembleError> {
    prepare_output(out, opts.force)?;

    let temp = temp_root_for(out);
    if let Some(parent) = temp.parent()
        && !parent.as_os_str().is_empty()
    {
        create_dir(parent)?;
    }
    if temp.exists() {
        // A previous run of this process id died between creating the
        // temporary tree and renaming it. Reusing it would stage into a tree
        // that already holds files, so it goes.
        remove_dir(&temp)?;
    }

    let staged = match build(set, otp, opts, &temp) {
        Ok(staged) => staged,
        Err(error) => {
            let _ = std::fs::remove_dir_all(&temp);
            return Err(error);
        }
    };

    match publish(&temp, out) {
        Ok(()) => Ok(StagedRoot {
            root: out.to_path_buf(),
            ..staged
        }),
        Err(error) => {
            let _ = std::fs::remove_dir_all(&temp);
            Err(error)
        }
    }
}

/// The sibling directory the tree is built in before it is renamed onto `out`.
///
/// The process id keeps two concurrent stagings of the same output out of each
/// other's way, and keeps the name predictable enough that a leftover from a
/// killed run is recognisable rather than mysterious.
#[cfg(feature = "cli")]
fn temp_root_for(out: &Path) -> PathBuf {
    let mut name = out
        .file_name()
        .unwrap_or(TEMP_FALLBACK.as_ref())
        .to_os_string();
    name.push(format!(".tmp-{}", std::process::id()));
    match out.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.join(name),
        _ => PathBuf::from(name),
    }
}

/// The name a temporary directory falls back to for an output path with none.
///
/// Only a path that is a root or ends in `..` has no final component, and
/// neither is a directory `stage` can write; the fallback exists so that
/// building the temporary name needs no panic.
#[cfg(feature = "cli")]
const TEMP_FALLBACK: &str = "ginary-stage";

/// The boot script staged as the artifact's only one.
#[cfg(feature = "cli")]
const BOOT_NAME: &str = "no_dot_erlang.boot";

/// Checks `out`, and clears it when [`StageOptions::force`] says to.
///
/// A path that does not exist and an empty directory are both fine, and both
/// leave the filesystem alone: the rename at the end of staging is what
/// creates `out`, so a failure in between must not have destroyed anything.
#[cfg(feature = "cli")]
fn prepare_output(out: &Path, force: bool) -> Result<(), AssembleError> {
    let existing = match std::fs::symlink_metadata(out) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(source) => {
            return Err(AssembleError::Io {
                path: out.to_path_buf(),
                source,
            });
        }
    };

    if existing.is_dir() && is_empty_dir(out)? {
        return Ok(());
    }
    if !force {
        return Err(AssembleError::OutputNotEmpty {
            path: out.to_path_buf(),
        });
    }
    if existing.is_dir() {
        remove_dir(out)
    } else {
        std::fs::remove_file(out).map_err(|source| AssembleError::Io {
            path: out.to_path_buf(),
            source,
        })
    }
}

/// Whether a directory holds no entries at all.
#[cfg(feature = "cli")]
fn is_empty_dir(dir: &Path) -> Result<bool, AssembleError> {
    let mut entries = read_dir(dir)?;
    Ok(entries.next().is_none())
}

/// Renames the finished temporary tree onto `out`.
///
/// `out` is either absent or the empty directory [`prepare_output`] accepted;
/// the empty one is removed here rather than earlier, so that a staging that
/// fails leaves even that untouched.
#[cfg(feature = "cli")]
fn publish(temp: &Path, out: &Path) -> Result<(), AssembleError> {
    if out.exists() {
        std::fs::remove_dir(out).map_err(|source| AssembleError::Io {
            path: out.to_path_buf(),
            source,
        })?;
    }
    std::fs::rename(temp, out).map_err(|source| AssembleError::Io {
        path: out.to_path_buf(),
        source,
    })
}

/// Writes the whole tree under `root`, which is the temporary directory.
///
/// The order is the order the errors are worth reporting in: the runtime
/// first, because a missing ERTS binary is a broken installation and nothing
/// else matters; then the applications; then the boot file's cross-check,
/// which can only run once the applications are on disk; then junk removal and
/// the listing, which describe what is there rather than deciding it.
#[cfg(feature = "cli")]
fn build(
    set: &AppSet,
    otp: &OtpInfo,
    opts: &StageOptions,
    root: &Path,
) -> Result<StagedRoot, AssembleError> {
    create_dir(root)?;

    let erts_dir = format!("erts-{}", otp.erts_vsn);
    let staged_bin = root.join(&erts_dir).join("bin");
    let mut staged_bins = stage_erts_bins(otp, opts, &staged_bin)?;
    // Belt and braces. The Windows probe never contributes `erl.ini` — it is
    // neither one of the required names nor a DLL — but `--extra-bin erl.ini`
    // would put one back, and an `erl.ini` in the artifact points `erl.exe` at
    // the build machine's `Rootdir`. It is removed from the staged tree rather
    // than from the runtime the build read, which is somebody's installation.
    //
    // Before the complement is computed, and the name comes out of the staged
    // set with it: a file that was copied and then deleted is not a staged
    // file, and a report that called it one would name it as staged, omit it
    // from the index and list it under junk in the same breath.
    let staged_erl_ini = remove_windows_erl_ini(&staged_bin)?;
    if staged_erl_ini.is_some() {
        staged_bins.remove(WINDOWS_ERL_INI);
    }
    let excluded_erts_bins = excluded_bins(&otp.erts_bin, &staged_bins)?;

    let boot = otp.root.join("bin").join(BOOT_NAME);
    let boot_bytes = read(&boot)?;
    create_dir(&root.join("bin"))?;
    copy_file(&boot, &root.join("bin").join(BOOT_NAME))?;

    let mut apps = stage_apps(set, root)?;
    let boot_refs = check_boot_refs(&boot, &boot_bytes, root, &apps)?;

    let mut junk_removed = if opts.remove_junk {
        remove_junk(root, &apps)?
    } else {
        Vec::new()
    };
    // In the account whether or not junk removal was asked for: this one is
    // not junk, it is a file that would have made the artifact resolve its
    // root against another machine, and a removal nobody can see the cost of
    // is a removal nobody can explain.
    if let Some(size) = staged_erl_ini {
        // Relative to the staged root, which is what every other entry in the
        // account is: an absolute path here would name the temporary directory
        // this build happened to use.
        junk_removed.push((
            listed_relative(
                root,
                &staged_bin.join(WINDOWS_ERL_INI),
                crate::platform::HOST,
            ),
            size,
        ));
        junk_removed.sort();
    }

    let sources: BTreeMap<String, StagedSource> = apps
        .iter()
        .map(|app| (dir_name(&app.dir).to_owned(), app.source))
        .collect();
    let files = list_files(root, &sources)?;
    count_apps(&mut apps, &files);

    let listing = StageListing {
        erts_vsn: otp.erts_vsn.clone(),
        otp_release: otp.release,
        otp_version: otp.otp_version.clone(),
        apps: apps.clone(),
        files: files.clone(),
    };
    write_listing(&root.join(LISTING_NAME), &listing)?;

    Ok(StagedRoot {
        root: root.to_path_buf(),
        erts_vsn: otp.erts_vsn.clone(),
        otp_release: otp.release,
        otp_version: otp.otp_version.clone(),
        apps,
        files,
        junk_removed,
        excluded_erts_bins,
        boot_refs,
    })
}

/// The program a Windows runtime is started with, and the one file a Windows
/// `erts-<vsn>/bin` must hold.
pub const WINDOWS_LAUNCH_BINARY: &str = crate::target::WINDOWS_LAUNCH_PROGRAM;

/// The emulator a Windows `erl.exe` loads into its own process.
///
/// The unix tree's `beam.smp` is a program `erlexec` execs; the Windows tree's
/// is a DLL `erl.exe` loads, so it is required for the same reason and found
/// by a different rule.
#[cfg(feature = "cli")]
pub const WINDOWS_EMULATOR_DLL: &str = crate::target::WINDOWS_EMULATOR_DLL;

/// The port program a Windows runtime resolves host names with.
///
/// `inet_gethost` under the spelling a Windows tree gives it. It is one of
/// [`crate::otp::REQUIRED_ERTS_BINARIES`] on unix — "every one of them must
/// exist under `erts-<vsn>/bin`" — and it is required here for exactly that
/// reason: an artifact without it starts a runtime that cannot resolve a host
/// name, and finds that out the first time the application opens a socket.
#[cfg(feature = "cli")]
pub const WINDOWS_RESOLVER_PROGRAM: &str = crate::target::WINDOWS_RESOLVER_PROGRAM;

/// The file deleted beside [`WINDOWS_LAUNCH_BINARY`] during assembly.
///
/// `erl.ini` holds the absolute `Rootdir` of the machine the runtime was
/// installed on, and `erl.exe` prefers it to its own location. An artifact
/// that carried one would look for its runtime wherever the build machine
/// happened to keep it, so it is removed and `erl.exe` is left to locate
/// itself: its own directory is the `bin`, and two levels above it is the
/// root.
#[cfg(feature = "cli")]
pub const WINDOWS_ERL_INI: &str = "erl.ini";

/// The three names a Windows `erts-<vsn>/bin` must hold, whatever else is in
/// it.
///
/// The launch program, the emulator it loads, and the resolver every runtime
/// needs. A tree missing any of them is not a runtime, and finding that out at
/// extraction time on somebody else's machine is what
/// [`windows_required_bins`] exists to prevent. [`crate::launch::preflight`]
/// holds the extracted artifact to the same names.
#[cfg(feature = "cli")]
pub const WINDOWS_REQUIRED_BINS: [&str; 3] = [
    WINDOWS_LAUNCH_BINARY,
    WINDOWS_EMULATOR_DLL,
    WINDOWS_RESOLVER_PROGRAM,
];

/// The debug build of the emulator, which ships beside the real one and never
/// travels.
///
/// A unix `erts-<vsn>/bin` holds `beam.debug.smp` beside `beam.smp` and
/// [`crate::otp::REQUIRED_ERTS_BINARIES`] leaves it behind by not naming it.
/// The Windows rule cannot be a fixed list — see [`windows_required_bins`] —
/// so the debug emulator is left behind by name instead, for two reasons that
/// are each sufficient. Nothing in a packaged artifact loads it: `erl.exe`
/// loads [`WINDOWS_EMULATOR_DLL`] unless it is asked for the debug emulator,
/// which a packaged application never is. And it could not be loaded if
/// something tried: it needs the *debug* C runtime — `MSVCP140D.dll`,
/// `VCRUNTIME140D.dll`, `VCRUNTIME140_1D.dll` and `ucrtbased.dll` — which is
/// not redistributable and exists only where Visual Studio is installed, so
/// `ginary verify` reported four findings against every Windows artifact for
/// a file that was dead weight in all of them. See
/// `tests/regressions/e12_a_windows_artifact_carried_the_debug_emulator.rs`.
#[cfg(feature = "cli")]
pub const WINDOWS_DEBUG_EMULATOR_DLL: &str = "beam.debug.smp.dll";

/// The names a Windows `erts-<vsn>/bin` contributes to the artifact.
///
/// Data-driven rather than a fixed list, because a Windows ERTS tree is a zip
/// somebody else built and the set of DLLs in it moves between releases:
/// [`WINDOWS_REQUIRED_BINS`] and every `*.dll` beside them travel, and every
/// other program — `werl.exe`, `erlsrv.exe`, `dialyzer.exe`, `Install.exe` —
/// is left behind the same way [`crate::otp::REQUIRED_ERTS_BINARIES`] leaves
/// the unix tree's spare programs behind. The answer is sorted, so a staged
/// tree does not depend on the order a directory was read back in.
///
/// [`WINDOWS_DEBUG_EMULATOR_DLL`] is the one DLL the rule declines, and it is
/// declined *after* the required names are checked: a tree whose only
/// emulator is the debug build is not a runtime, and refusing it by the name
/// that is missing is a better answer than staging it and finding out on
/// somebody else's machine. The name is compared the way the `.dll` rule
/// beside it compares its suffix — case-insensitively — because a zip
/// spelling it `BEAM.DEBUG.SMP.DLL` would otherwise pass the decline and be
/// admitted as a library by the rule on the next line.
///
/// The three required names are the other way round, and deliberately: they
/// are matched *exactly*, because every other gate that names these files
/// names them literally. [`is_windows_erts_bin`] decides a tree's flavour by
/// joining [`WINDOWS_LAUNCH_BINARY`] onto it,
/// [`crate::launch::WINDOWS_REQUIRED_BINARIES`] checks the extracted tree by
/// name, and the index a Linux host verifies a Windows artifact against
/// carries the name the tree was staged under. A tree admitted here under
/// another spelling would satisfy this one rule and fail those on any
/// case-sensitive host, so it is refused instead — but it is refused by
/// [`AssembleError::ErtsBinaryNamedInAnotherCase`], which names the spelling
/// the tree really uses, rather than by a message telling a user a file they
/// can see is not there.
///
/// What is declined here is reported: it
/// is not in the answer, so [`StagedRoot::excluded_erts_bins`] carries it with
/// the reason [`excluded_reason`] gives it.
///
/// # Errors
///
/// [`AssembleError::MissingErtsBinary`] naming the first of
/// [`WINDOWS_REQUIRED_BINS`] that is not there: a tree missing any of them is
/// not a runtime, and finding that out at extraction time on somebody else's
/// machine is exactly what this check exists to prevent.
/// [`AssembleError::ErtsBinaryNamedInAnotherCase`] when that name is in the
/// tree and differs from the one ginary needs only in case.
/// [`AssembleError::Io`] when the directory cannot be listed.
#[cfg(feature = "cli")]
pub fn windows_required_bins(erts_bin: &Path) -> Result<Vec<String>, AssembleError> {
    let mut present: BTreeSet<String> = BTreeSet::new();
    for entry in read_dir(erts_bin)? {
        let entry = entry.map_err(|source| AssembleError::Io {
            path: erts_bin.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        if path.is_dir() {
            continue;
        }
        present.insert(file_name_of(&path)?);
    }

    // All three by name, and all three before anything is collected: a tree
    // missing any of them is not a runtime, and a staged tree that is missing
    // one is found out at extraction time on somebody else's machine. The
    // comparison is exact, for the reason this function's documentation
    // gives; a tree that holds the file under another case is told so rather
    // than told the file is not there.
    for required in WINDOWS_REQUIRED_BINS {
        if present.contains(required) {
            continue;
        }
        let found = present
            .iter()
            .find(|name| name.eq_ignore_ascii_case(required));
        return Err(match found {
            Some(found) => AssembleError::ErtsBinaryNamedInAnotherCase {
                name: required.to_owned(),
                found: found.clone(),
                searched: erts_bin.join(required),
            },
            None => AssembleError::MissingErtsBinary {
                name: required.to_owned(),
                searched: erts_bin.join(required),
            },
        });
    }

    Ok(present
        .into_iter()
        .filter(|name| !name.eq_ignore_ascii_case(WINDOWS_DEBUG_EMULATOR_DLL))
        .filter(|name| WINDOWS_REQUIRED_BINS.contains(&name.as_str()) || is_windows_library(name))
        .collect())
}

/// Whether `name` is a DLL, whatever case the tree spells the suffix in.
///
/// `erl.exe` loads its emulator and every driver beside it out of its own
/// directory, so a DLL there is part of the runtime rather than a program
/// somebody could have run. The suffix is compared case-insensitively because
/// a Windows filesystem does not distinguish the two spellings and an upstream
/// zip is under no obligation to pick one.
#[cfg(feature = "cli")]
fn is_windows_library(name: &str) -> bool {
    std::path::Path::new(name)
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("dll"))
}

/// Gives a staged file the permission bits the listing records for it.
#[cfg(all(unix, feature = "cli"))]
fn set_mode(path: &Path, mode: u32) -> Result<(), AssembleError> {
    use std::os::unix::fs::PermissionsExt as _;

    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).map_err(|source| {
        AssembleError::Io {
            path: path.to_path_buf(),
            source,
        }
    })
}

/// Nothing, on a platform with no permission bits to give.
///
/// The same decision [`crate::payload`] and [`crate::cache`] make, for the same
/// reason: a Windows file has an ACL and a read-only flag, and a POSIX mode
/// word maps onto neither. The `mode` the caller passed is still recorded in
/// the listing and in `ginary.index.json`, where it is informational.
#[cfg(all(windows, feature = "cli"))]
fn set_mode(path: &Path, mode: u32) -> Result<(), AssembleError> {
    let _ = (path, mode);
    Ok(())
}

/// Whether `erts_bin` is a Windows runtime's `bin` rather than a unix one.
///
/// The launch program is the marker, because it is the one file the two trees
/// cannot share: a Windows tree has no `erlexec` and a unix tree has no
/// `erl.exe`. Reading the flavour off the tree rather than off the requested
/// target is what keeps the two lists from crossing — a unix runtime staged
/// for a Windows target is refused by name for a missing `erl.exe` instead of
/// being staged as if the request had been right.
///
/// It is the one flavour test there is. [`crate::otp::inspect_root`] asks it
/// which list of required programs a tree is measured against, and
/// [`crate::erts_source::resolve`] asks it which object file is the evidence,
/// so a tree cannot be a Windows one to assembly and a unix one to the
/// resolver.
#[cfg(feature = "cli")]
pub fn is_windows_erts_bin(erts_bin: &Path) -> bool {
    erts_bin.join(WINDOWS_LAUNCH_BINARY).is_file()
}

/// Removes [`WINDOWS_ERL_INI`] from a staged Windows `bin`, if it is there.
///
/// Returns the size of the file that was removed, or [`None`] when there was
/// none — a tree that never had one is the ordinary case for a runtime built
/// from source, and it is not a failure.
///
/// # Errors
///
/// [`AssembleError::Io`] when the file is there and cannot be removed. A
/// staged `erl.ini` left behind is not a smaller problem than a failed build:
/// the artifact would resolve its root against the build machine.
#[cfg(feature = "cli")]
pub fn remove_windows_erl_ini(bin: &Path) -> Result<Option<u64>, AssembleError> {
    let path = bin.join(WINDOWS_ERL_INI);
    let Ok(metadata) = std::fs::symlink_metadata(&path) else {
        return Ok(None);
    };
    let size = metadata.len();
    std::fs::remove_file(&path).map_err(|source| AssembleError::Io {
        path: path.clone(),
        source,
    })?;
    Ok(Some(size))
}

/// Whether `name` may be staged out of the runtime's `bin` directory.
///
/// A program name is one file name: not empty, not `.` or `..`, and holding
/// neither a path separator nor a NUL. The rule is the one
/// [`crate::closure`] applies to an application name and for the same reason —
/// the value is interpolated into a path — and it is checked here as well as
/// in [`crate::config::ToolsConfig::validate`], because this is the function
/// that performs the copy.
#[cfg(feature = "cli")]
pub fn is_erts_bin_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\', '\0'])
}

/// Copies the required binaries and the extra ones into `bin`.
///
/// Returns the names that were staged, which is what
/// [`StagedRoot::excluded_erts_bins`] is the complement of.
#[cfg(feature = "cli")]
fn stage_erts_bins(
    otp: &OtpInfo,
    opts: &StageOptions,
    bin: &Path,
) -> Result<BTreeSet<String>, AssembleError> {
    create_dir(bin)?;
    let mut staged = BTreeSet::new();

    // Which names are required is a property of the tree rather than of the
    // target that was asked for: a Windows runtime has no `erlexec` and a unix
    // one has no `erl.exe`, so the flavour is read off the directory and the
    // build cannot stage one list out of the other's tree.
    let required: Vec<String> = if is_windows_erts_bin(&otp.erts_bin) {
        windows_required_bins(&otp.erts_bin)?
    } else {
        crate::otp::REQUIRED_ERTS_BINARIES
            .iter()
            .map(|name| (*name).to_owned())
            .collect()
    };
    for name in required {
        let from = otp.erts_bin.join(&name);
        if !from.is_file() {
            return Err(AssembleError::MissingErtsBinary {
                name: name.clone(),
                searched: from,
            });
        }
        copy_file(&from, &bin.join(&name))?;
        staged.insert(name);
    }

    for name in &opts.extra_bins {
        // Before either join: the name is interpolated into the path a file is
        // read from *and* into the path it is written to, so a name holding a
        // separator or `..` would read and write outside both trees.
        if !is_erts_bin_name(name) {
            return Err(AssembleError::UnusableExtraBinary { name: name.clone() });
        }
        let from = otp.erts_bin.join(name);
        if !from.is_file() {
            return Err(AssembleError::MissingExtraBinary { name: name.clone() });
        }
        copy_file(&from, &bin.join(name))?;
        staged.insert(name.clone());
    }

    Ok(staged)
}

/// Every program in the runtime's `bin` that was not staged, with its reason.
#[cfg(feature = "cli")]
fn excluded_bins(
    erts_bin: &Path,
    staged: &BTreeSet<String>,
) -> Result<Vec<ExcludedBin>, AssembleError> {
    let mut names: Vec<String> = Vec::new();
    for entry in read_dir(erts_bin)? {
        let entry = entry.map_err(|source| AssembleError::Io {
            path: erts_bin.to_path_buf(),
            source,
        })?;
        let name = file_name_of(&entry.path())?;
        if staged.contains(&name) || entry.path().is_dir() {
            continue;
        }
        names.push(name);
    }
    names.sort();

    Ok(names
        .into_iter()
        .map(|name| ExcludedBin {
            reason: excluded_reason(&name).to_owned(),
            name,
        })
        .collect())
}

/// Copies every application's `ebin` and `priv` into `root/lib`.
///
/// The file and byte counts are filled in later by [`count_apps`], because
/// junk removal happens after the copy and an application's size is what
/// survived it.
#[cfg(feature = "cli")]
fn stage_apps(set: &AppSet, root: &Path) -> Result<Vec<StagedApp>, AssembleError> {
    let lib = root.join("lib");
    create_dir(&lib)?;

    let mut apps = Vec::new();
    for app in set.iter() {
        let (source, dir) = match &app.source {
            closure::AppSource::Shipment => (StagedSource::Shipment, app.name.clone()),
            closure::AppSource::Otp { vsn } => (StagedSource::Otp, format!("{}-{}", app.name, vsn)),
        };
        let target = lib.join(&dir);
        create_dir(&target)?;

        let app_root = canonical(app.ebin.parent().unwrap_or(&app.ebin))?;
        copy_subtree(&app_root, &app.ebin, &target.join("ebin"), Filter::Ebin)?;
        if let Some(priv_dir) = &app.priv_dir {
            copy_subtree(&app_root, priv_dir, &target.join("priv"), Filter::Priv)?;
        }

        apps.push(StagedApp {
            name: app.name.clone(),
            vsn: app.vsn.clone(),
            source,
            dir: format!("lib/{dir}"),
            files: 0,
            bytes: 0,
        });
    }
    Ok(apps)
}

/// What a recursive copy is allowed to take.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg(feature = "cli")]
enum Filter {
    /// An `ebin`: everything but `*.appup`, which is a release upgrade
    /// instruction and is never read at run time.
    Ebin,
    /// A `priv`: everything.
    Priv,
}

#[cfg(feature = "cli")]
impl Filter {
    /// Whether a file of this name is copied.
    fn keeps(self, name: &str) -> bool {
        match self {
            Self::Ebin => !name.ends_with(".appup"),
            Self::Priv => true,
        }
    }
}

/// Copies one application's `ebin` or `priv` into the staged tree.
///
/// `from` is checked before anything is read out of it, against both of the
/// boundaries a link inside it would face. An `ebin` or a `priv` that is
/// *itself* a symlink is the same defect as a link inside one, and `read_dir`
/// would follow it without a word: an application whose `priv` points at some
/// other directory of the build machine would have that directory copied into
/// the artifact whole, and one whose `priv` points at its own `src` would have
/// the sources [`EXCLUDED_APP_DIRS`] exists to leave behind staged under the
/// name `priv`.
#[cfg(feature = "cli")]
fn copy_subtree(
    app_root: &Path,
    from: &Path,
    to: &Path,
    filter: Filter,
) -> Result<(), AssembleError> {
    let metadata = symlink_metadata(from)?;
    let subtree = if metadata.is_symlink() {
        let resolved = resolve_link(app_root, from)?;
        if let Some(excluded) = excluded_component(app_root, &resolved) {
            return Err(AssembleError::ExcludedSymlinkTarget {
                path: from.to_path_buf(),
                target: resolved,
                excluded,
            });
        }
        resolved
    } else {
        canonical(from)?
    };

    TreeCopy {
        app_root,
        subtree: subtree.clone(),
        filter,
        entered: Vec::new(),
    }
    .directory(&subtree, to)
}

/// One recursive copy, and the two things it has to remember.
///
/// The boundaries are deliberately different for a file and for a directory. A
/// link to a file may point anywhere inside the application, which is how an
/// application that keeps one copy of a data file and links to it from `priv`
/// still stages. A link to a *directory* may not leave the `ebin` or `priv` it
/// was found in, because `ebin/sources -> ../src` would otherwise stage the
/// sources that [`EXCLUDED_APP_DIRS`] exists to leave behind: the exclusion is
/// structural, and a structural rule that a symlink can step around is not one.
///
/// `entered` is the stack of canonical directories the copy is inside, which
/// is what makes `priv/loop -> .` an [`AssembleError::SymlinkCycle`] naming the
/// link rather than a recursion that ends when the operating system runs out of
/// path.
#[cfg(feature = "cli")]
struct TreeCopy<'a> {
    /// The canonical application directory: the boundary for a link to a file.
    app_root: &'a Path,
    /// The canonical `ebin` or `priv`: the boundary for a link to a directory.
    subtree: PathBuf,
    /// What this copy is allowed to take.
    filter: Filter,
    /// The canonical directories this copy is currently inside.
    entered: Vec<PathBuf>,
}

#[cfg(feature = "cli")]
impl TreeCopy<'_> {
    /// Copies one directory, recursing into the directories under it.
    fn directory(&mut self, from: &Path, to: &Path) -> Result<(), AssembleError> {
        self.entered.push(from.to_path_buf());
        create_dir(to)?;

        for path in entries_of(from)? {
            let name = file_name_of(&path)?;
            let linked = symlink_metadata(&path)?.is_symlink();
            let resolved = if linked {
                resolve_link(self.app_root, &path)?
            } else {
                path.clone()
            };

            if resolved.is_dir() {
                let dir = canonical(&resolved)?;
                if linked && !dir.starts_with(&self.subtree) {
                    return Err(AssembleError::UnsafeSymlink { path, target: dir });
                }
                if self.entered.contains(&dir) {
                    return Err(AssembleError::SymlinkCycle { path, target: dir });
                }
                self.directory(&dir, &to.join(&name))?;
            } else if self.filter.keeps(&name) {
                copy_file(&resolved, &to.join(&name))?;
            }
        }

        self.entered.pop();
        Ok(())
    }
}

/// The excluded directory a path inside the application reaches, if any.
///
/// The comparison is on the path components below `app_root`, so a link to
/// `src`, to `src/data`, or to anything deeper answers `src`, and a link to a
/// directory that merely *holds* a `doc` answers nothing: the exclusion applies
/// at the top level of an application, and a real `priv/doc` stages.
#[cfg(feature = "cli")]
fn excluded_component(app_root: &Path, target: &Path) -> Option<String> {
    let relative = target.strip_prefix(app_root).ok()?;
    relative.components().find_map(|component| {
        let name = component.as_os_str().to_str()?;
        EXCLUDED_APP_DIRS.contains(&name).then(|| name.to_owned())
    })
}

/// Resolves a symlink, refusing one that dangles or leaves the application.
#[cfg(feature = "cli")]
fn resolve_link(app_root: &Path, path: &Path) -> Result<PathBuf, AssembleError> {
    let Ok(resolved) = std::fs::canonicalize(path) else {
        return Err(AssembleError::UnsafeSymlink {
            path: path.to_path_buf(),
            target: link_target(path),
        });
    };
    if !resolved.starts_with(app_root) {
        return Err(AssembleError::UnsafeSymlink {
            path: path.to_path_buf(),
            target: resolved,
        });
    }
    Ok(resolved)
}

/// Where a link points, as far as it can be told without resolving it.
///
/// A dangling link cannot be canonicalised, and the error is more useful for
/// naming what it pointed at than for saying nothing at all.
#[cfg(feature = "cli")]
fn link_target(path: &Path) -> PathBuf {
    match std::fs::read_link(path) {
        Ok(target) if target.is_absolute() => target,
        Ok(target) => match path.parent() {
            Some(parent) => parent.join(target),
            None => target,
        },
        Err(_) => path.to_path_buf(),
    }
}

/// Checks every library directory the boot file names against the staged tree.
///
/// Returns the directories that were checked, in the order the boot file names
/// them, so that `--explain` can show the cross-check was done.
#[cfg(feature = "cli")]
fn check_boot_refs(
    boot: &Path,
    bytes: &[u8],
    root: &Path,
    apps: &[StagedApp],
) -> Result<Vec<String>, AssembleError> {
    let refs = crate::otp::boot_lib_dirs(bytes);
    for dir in &refs {
        if root.join("lib").join(dir).join("ebin").is_dir() {
            continue;
        }
        let name = dir.rsplit_once('-').map_or(dir.as_str(), |(name, _)| name);
        let mut staged: Vec<String> = apps
            .iter()
            .filter(|app| app.name == name)
            .map(|app| dir_name(&app.dir).to_owned())
            .collect();
        staged.sort();
        return Err(AssembleError::BootReferencesMissingApp {
            dir: dir.clone(),
            staged,
            boot: boot.to_path_buf(),
        });
    }
    Ok(refs)
}

/// Deletes the known-useless files under every application's `priv`.
///
/// Three shapes, and each one is a file a build produced and a run never
/// reads: OpenSSL's test engine, the static archives beside the shared objects
/// that were linked from them, and the object directory the shared objects
/// were built in. A removed directory is one entry carrying the total of what
/// it held.
#[cfg(feature = "cli")]
fn remove_junk(root: &Path, apps: &[StagedApp]) -> Result<Vec<(PathBuf, u64)>, AssembleError> {
    let mut removed: Vec<(PathBuf, u64)> = Vec::new();
    for app in apps {
        let priv_dir = root.join(&app.dir).join("priv");
        if !priv_dir.is_dir() {
            continue;
        }

        let obj = priv_dir.join("obj");
        if obj.is_dir() {
            let bytes = tree_bytes(&obj)?;
            remove_dir(&obj)?;
            removed.push((listed_relative(root, &obj, crate::platform::HOST), bytes));
        }

        let lib = priv_dir.join("lib");
        if lib.is_dir() {
            for path in entries_of(&lib)? {
                let name = file_name_of(&path)?;
                if !path.is_file() || !is_junk_file(&name, &app.name) {
                    continue;
                }
                let bytes = size_of(&path)?;
                std::fs::remove_file(&path).map_err(|source| AssembleError::Io {
                    path: path.clone(),
                    source,
                })?;
                removed.push((listed_relative(root, &path, crate::platform::HOST), bytes));
            }
        }
    }
    removed.sort();
    Ok(removed)
}

/// Whether one file directly under an application's `priv/lib` is junk.
///
/// A `.a` is the static half of the shared object beside it and is linked into
/// nothing at run time. `otp_test_engine.so` is named for `crypto` alone,
/// because that is the application that ships it — it is OpenSSL's test engine
/// — and a file of that name under any other application would be somebody's
/// own.
#[cfg(feature = "cli")]
fn is_junk_file(name: &str, app: &str) -> bool {
    name.ends_with(".a") || (app == "crypto" && name == "otp_test_engine.so")
}

/// Every file under a directory, as sorted `/`-separated relative paths with
/// their size and permission bits.
#[cfg(feature = "cli")]
fn list_files(
    root: &Path,
    sources: &BTreeMap<String, StagedSource>,
) -> Result<Vec<StagedFile>, AssembleError> {
    let mut found: Vec<(String, u64, u32)> = Vec::new();
    walk(root, root, &mut found)?;
    found.sort();

    Ok(found
        .into_iter()
        .map(|(path, size, mode)| StagedFile {
            category: categorise(&path, sources),
            path,
            size,
            mode,
        })
        .collect())
}

/// The recursive half of [`list_files`].
#[cfg(feature = "cli")]
fn walk(root: &Path, dir: &Path, found: &mut Vec<(String, u64, u32)>) -> Result<(), AssembleError> {
    for entry in read_dir(dir)? {
        let entry = entry.map_err(|source| AssembleError::Io {
            path: dir.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        if path.is_dir() {
            walk(root, &path, found)?;
            continue;
        }
        let metadata = std::fs::metadata(&path).map_err(|source| AssembleError::Io {
            path: path.clone(),
            source,
        })?;
        let Some(listed) = slash_path(&relative(root, &path)) else {
            return Err(AssembleError::NonUtf8Name { path });
        };
        found.push((listed, metadata.len(), mode_of(&metadata)));
    }
    Ok(())
}

/// What a staged file is, from its path and the application it is under.
#[cfg(feature = "cli")]
fn categorise(path: &str, sources: &BTreeMap<String, StagedSource>) -> Category {
    let parts: Vec<&str> = path.split('/').collect();
    match parts.as_slice() {
        [first, ..] if first.starts_with("erts-") => Category::ErtsBinary,
        ["bin", name] if *name == BOOT_NAME => Category::Boot,
        ["lib", _, "priv", ..] => Category::Priv,
        ["lib", app, "ebin", ..] => {
            let Some(name) = parts.last() else {
                return Category::Other;
            };
            if name.ends_with(".app") {
                Category::AppResource
            } else if name.ends_with(".beam") {
                match sources.get(*app) {
                    Some(StagedSource::Otp) => Category::OtpBeam,
                    Some(StagedSource::Shipment) => Category::GleamBeam,
                    None => Category::Other,
                }
            } else {
                Category::Other
            }
        }
        _ => Category::Other,
    }
}

/// Fills in each application's file count and byte total from the listing.
#[cfg(feature = "cli")]
fn count_apps(apps: &mut [StagedApp], files: &[StagedFile]) {
    for app in apps.iter_mut() {
        let prefix = format!("{}/", app.dir);
        app.files = 0;
        app.bytes = 0;
        for file in files.iter().filter(|file| file.path.starts_with(&prefix)) {
            app.files += 1;
            app.bytes += file.size;
        }
    }
}

/// Writes `ginary.stage.json`, pretty-printed and newline-terminated.
#[cfg(feature = "cli")]
fn write_listing(path: &Path, listing: &StageListing) -> Result<(), AssembleError> {
    let mut json = match serde_json::to_string_pretty(listing) {
        Ok(json) => json,
        Err(error) => {
            return Err(AssembleError::Io {
                path: path.to_path_buf(),
                source: std::io::Error::other(error),
            });
        }
    };
    json.push('\n');
    std::fs::write(path, json).map_err(|source| AssembleError::Io {
        path: path.to_path_buf(),
        source,
    })
}

/// The total size of every file under a directory.
#[cfg(feature = "cli")]
fn tree_bytes(dir: &Path) -> Result<u64, AssembleError> {
    let mut total = 0;
    for entry in read_dir(dir)? {
        let entry = entry.map_err(|source| AssembleError::Io {
            path: dir.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        if path.is_dir() {
            total += tree_bytes(&path)?;
        } else {
            total += size_of(&path)?;
        }
    }
    Ok(total)
}

/// The size of one file.
#[cfg(feature = "cli")]
fn size_of(path: &Path) -> Result<u64, AssembleError> {
    std::fs::metadata(path)
        .map(|metadata| metadata.len())
        .map_err(|source| AssembleError::Io {
            path: path.to_path_buf(),
            source,
        })
}

/// The permission bits of a file, as the listing records them.
///
/// `st_mode & 0o7777` where the filesystem has a mode word, and
/// [`crate::platform::modeless_mode`] where it has none. Zero was the old
/// answer on such a platform and it was wrong twice over: it is not a mode any
/// file has, and it disagreed with the two other producers of the same column
/// — [`crate::manifest::Index::from_staged`] and the payload's `tar` header —
/// so `ginary verify` reported a mismatch nobody had introduced. One rule, one
/// answer; `docs/dev/log/E8.md` records the run that found it.
#[cfg(feature = "cli")]
fn mode_of(metadata: &std::fs::Metadata) -> u32 {
    #[cfg(unix)]
    let raw_mode = {
        use std::os::unix::fs::PermissionsExt as _;
        metadata.permissions().mode() & 0o7777
    };
    // A modeless filesystem has no `st_mode` to read; `recorded_mode` discards
    // this value there, so `0` stands for "unread".
    #[cfg(not(unix))]
    let raw_mode = 0u32;
    crate::platform::recorded_mode(
        crate::platform::has_unix_modes(crate::platform::HOST),
        raw_mode,
        metadata.is_dir(),
    )
}

/// A path relative to the staged root, or the path itself if it is not under
/// it. Only paths built from the root are passed, so the fallback never fires.
#[cfg(feature = "cli")]
fn relative(root: &Path, path: &Path) -> PathBuf {
    path.strip_prefix(root).unwrap_or(path).to_path_buf()
}

/// A path relative to the staged root, spelled the way the listing carries it.
///
/// The junk table is read against `ginary.stage.json`, and that document is
/// `/`-separated on every platform. A path joined out of an application's own
/// `/`-separated directory and a walked `OsStr` carries the platform's
/// separator into the middle of it — the first Windows runner produced
/// `lib/crypto-5.9.2\priv\obj` — so the report is respelled once, here,
/// rather than at each of the three places that push a row.
///
/// [`crate::winpath::slash_path_str`] is the rule, and it is applied to a
/// Windows path only: `\` is an ordinary character in a unix file name, and
/// rewriting one would name a different file rather than respell this one.
#[cfg(feature = "cli")]
fn listed_relative(root: &Path, path: &Path, os: crate::target::Os) -> PathBuf {
    let relative = relative(root, path);
    if !crate::platform::separates_paths_with_backslash(os) {
        return relative;
    }
    match relative.to_str() {
        Some(text) => PathBuf::from(crate::winpath::slash_path_str(text)),
        // Not text, so there is no listing path to respell it into: the caller
        // is given the walked spelling rather than a guess.
        None => relative,
    }
}

/// A relative path as the listing writes it: `/`-separated, whatever the
/// platform's own separator is, or `None` when a component is not text.
///
/// The listing is JSON and the payload index will be too, so a path that
/// cannot be spelled is a path that cannot be recorded. Dropping the component
/// would name a different file; the caller raises
/// [`AssembleError::NonUtf8Name`] instead.
#[cfg(feature = "cli")]
fn slash_path(path: &Path) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    for component in path.components() {
        parts.push(component.as_os_str().to_str()?);
    }
    Some(parts.join("/"))
}

/// The final component of a path, as text.
///
/// # Errors
///
/// [`AssembleError::NonUtf8Name`] when the name is not valid UTF-8, and when
/// there is no final component at all — neither can be staged, and neither is
/// a thing to pass over in silence.
#[cfg(feature = "cli")]
fn file_name_of(path: &Path) -> Result<String, AssembleError> {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(str::to_owned)
        .ok_or_else(|| AssembleError::NonUtf8Name {
            path: path.to_path_buf(),
        })
}

/// The metadata of a path, without following a symlink at the end of it.
#[cfg(feature = "cli")]
fn symlink_metadata(path: &Path) -> Result<std::fs::Metadata, AssembleError> {
    std::fs::symlink_metadata(path).map_err(|source| AssembleError::Io {
        path: path.to_path_buf(),
        source,
    })
}

/// The last component of a `lib/<dir>` path.
#[cfg(feature = "cli")]
fn dir_name(dir: &str) -> &str {
    dir.rsplit_once('/').map_or(dir, |(_, name)| name)
}

/// Copies one file, keeping its permission bits.
#[cfg(feature = "cli")]
fn copy_file(from: &Path, to: &Path) -> Result<(), AssembleError> {
    std::fs::copy(from, to)
        .map(|_| ())
        .map_err(|source| AssembleError::Copy {
            from: from.to_path_buf(),
            to: to.to_path_buf(),
            source,
        })
}

/// Reads a file whole.
#[cfg(feature = "cli")]
fn read(path: &Path) -> Result<Vec<u8>, AssembleError> {
    std::fs::read(path).map_err(|source| AssembleError::Io {
        path: path.to_path_buf(),
        source,
    })
}

/// Creates a directory and every directory above it.
#[cfg(feature = "cli")]
fn create_dir(path: &Path) -> Result<(), AssembleError> {
    std::fs::create_dir_all(path).map_err(|source| AssembleError::Io {
        path: path.to_path_buf(),
        source,
    })
}

/// Removes a directory and everything under it.
#[cfg(feature = "cli")]
fn remove_dir(path: &Path) -> Result<(), AssembleError> {
    std::fs::remove_dir_all(path).map_err(|source| AssembleError::Io {
        path: path.to_path_buf(),
        source,
    })
}

/// The entries of a directory, as paths sorted by name.
///
/// Sorted so that a copy visits the same files in the same order on every
/// machine: `read_dir` answers in whatever order the filesystem holds, and a
/// tree that was built in a different order is a tree that is harder to
/// compare with the one built beside it.
#[cfg(feature = "cli")]
fn entries_of(dir: &Path) -> Result<Vec<PathBuf>, AssembleError> {
    let mut entries: Vec<PathBuf> = Vec::new();
    for entry in read_dir(dir)? {
        let entry = entry.map_err(|source| AssembleError::Io {
            path: dir.to_path_buf(),
            source,
        })?;
        entries.push(entry.path());
    }
    entries.sort();
    Ok(entries)
}

/// Reads a directory.
#[cfg(feature = "cli")]
fn read_dir(path: &Path) -> Result<std::fs::ReadDir, AssembleError> {
    std::fs::read_dir(path).map_err(|source| AssembleError::Io {
        path: path.to_path_buf(),
        source,
    })
}

/// The canonical form of a path, for comparing a symlink's target against it.
#[cfg(feature = "cli")]
fn canonical(path: &Path) -> Result<PathBuf, AssembleError> {
    std::fs::canonicalize(path).map_err(|source| AssembleError::Io {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(all(test, feature = "cli"))]
mod tests {
    use super::{listed_relative, relative};
    use crate::target::Os;
    use std::path::{Path, PathBuf};

    #[test]
    fn listed_relative_respells_a_windows_row_and_leaves_a_unix_backslash_name_alone() {
        // `strip_prefix` fails for a path that is not under `root`, so
        // `relative` hands the path straight through and only the respelling
        // is under test.
        let root = Path::new("/staged/root");

        let windows_row = Path::new(r"lib/crypto-5.9.2\priv\lib\libcrypto_static.a");
        assert_eq!(
            listed_relative(root, windows_row, Os::Windows),
            PathBuf::from("lib/crypto-5.9.2/priv/lib/libcrypto_static.a"),
            "a Windows-spelled row is respelled the way ginary.stage.json carries it"
        );

        let unix_name = Path::new(r"lib/odd\name/thing.beam");
        assert_eq!(
            listed_relative(root, unix_name, Os::Linux),
            PathBuf::from(r"lib/odd\name/thing.beam"),
            "a backslash is an ordinary character in a unix file name; respelling it would name \
             a different file"
        );

        // `relative` itself is spelling-blind: it only strips the root.
        assert_eq!(relative(root, windows_row), windows_row.to_path_buf());
    }
}
