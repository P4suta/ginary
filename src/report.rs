// SPDX-License-Identifier: MIT OR Apache-2.0
//! The size and dependency account of a staged root.
//!
//! Two questions decide whether an artifact is shippable, and neither is
//! answered by the tree alone.
//!
//! **How big is it, and where did the size go?** [`SizeReport::categories`]
//! carries a before and an after for each of [`Category`]'s buckets, so
//! "the artifact is 40 MB" becomes "the ERTS binaries are 12 MB of it, and
//! stripping already took 30 MB off them". A number without a breakdown gives
//! nobody anything to act on.
//!
//! **Where will it not run?** An artifact carries its own BEAM but not its own
//! libc. Every ELF file in the tree is inspected, the union of their
//! `DT_NEEDED` entries and the highest `GLIBC_x.y` any of them requires become
//! the `needs:` line, and that line is the artifact's portability floor stated
//! at build time rather than discovered by a user whose loader refuses it.
//!
//! Nothing here is fatal. A file that cannot be inspected, or that the listing
//! names and the tree does not hold, lands in [`SizeReport::warnings`] and the
//! rest of the report is still produced: a report that refuses to print because
//! one file is odd is worse than one that prints and says which file was odd.
//! The rule the rest of the crate follows applies here too — a skip is a
//! reported decision, never a default.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read as _;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::assemble::{Category, StagedRoot};
use crate::closure;
use crate::elf;
use crate::strip::StripReport;

/// The prefix the `needs:` line puts back in front of a glibc version.
pub const GLIBC_PREFIX: &str = "GLIBC_";

/// One category's contribution to the artifact.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct CategorySize {
    /// How many files are in this category.
    pub files: usize,
    /// What they weighed before stripping.
    pub bytes_before: u64,
    /// What they weigh now.
    pub bytes_after: u64,
}

/// What one ELF file in the tree needs from the machine that runs it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ElfDep {
    /// The path relative to the staged root, `/`-separated.
    pub path: String,
    /// Its `DT_NEEDED` entries, in the order the file lists them.
    pub needed: Vec<String>,
    /// The highest `GLIBC_x.y` it requires, without the prefix.
    pub glibc_max: Option<String>,
    /// Its program interpreter, if it has one.
    pub interp: Option<String>,
    /// The machine it was built for.
    pub machine: String,
}

/// What the artifact as a whole needs.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct NeedsSummary {
    /// Every shared library any file in the tree names, deduplicated.
    ///
    /// A set rather than a list: `libc.so.6` is named by every binary in the
    /// artifact, and the question is which libraries the machine has to have,
    /// not how many times each was asked for.
    pub needed: BTreeSet<String>,
    /// The highest `GLIBC_x.y` any file requires, without the prefix.
    pub glibc_max: Option<String>,
}

/// The size and dependency account of a staged root.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct SizeReport {
    /// Bytes and file counts per category, in [`Category`] order.
    pub categories: BTreeMap<Category, CategorySize>,
    /// The whole tree's size before stripping.
    pub total_before: u64,
    /// The whole tree's size now.
    pub total_after: u64,
    /// One entry per ELF file in the tree, sorted by path.
    pub elf_deps: Vec<ElfDep>,
    /// The union of what those files need.
    pub needs_summary: NeedsSummary,
    /// Anything that could not be measured, one line each.
    pub warnings: Vec<String>,
}

/// Why a report could not be produced.
#[derive(Debug, thiserror::Error)]
pub enum ReportError {
    /// The staged root could not be read.
    #[error("cannot read `{path}`: {source}")]
    Io {
        /// The path that could not be read.
        path: PathBuf,
        /// The underlying operating system error.
        #[source]
        source: std::io::Error,
    },
}

impl SizeReport {
    /// How many bytes stripping removed from the whole tree.
    ///
    /// Saturating: a tree that grew is a warning to print, not a panic.
    pub fn saved(&self) -> u64 {
        self.total_before.saturating_sub(self.total_after)
    }

    /// Renders the aligned table, the `needs:` line and any warnings.
    ///
    /// ```text
    /// category      files  before    after     saved
    /// erts_binary   4      41675352  11742936  29932416
    /// ...
    /// total         360    51125137  14821649  36303488
    ///
    /// needs: libc.so.6 (GLIBC_2.38), libgcc_s.so.1, libm.so.6
    ///
    /// warnings:
    ///   lib/notify/priv/ghost.txt is in the listing and not in the tree
    /// ```
    ///
    /// The rows are in [`Category`] order and a category the tree does not
    /// hold has no row; the `total` row closes the table. The warnings block is
    /// absent when there are none, and the `needs:` line never is.
    pub fn render_text(&self) -> String {
        let mut rows: Vec<[String; 5]> = self
            .categories
            .iter()
            .map(|(category, size)| {
                [
                    category.label().to_owned(),
                    size.files.to_string(),
                    size.bytes_before.to_string(),
                    size.bytes_after.to_string(),
                    size.bytes_before
                        .saturating_sub(size.bytes_after)
                        .to_string(),
                ]
            })
            .collect();
        rows.push([
            "total".to_owned(),
            self.categories
                .values()
                .map(|size| size.files)
                .sum::<usize>()
                .to_string(),
            self.total_before.to_string(),
            self.total_after.to_string(),
            self.saved().to_string(),
        ]);

        let mut text =
            closure::render_table(["category", "files", "before", "after", "saved"], &rows);
        text.push('\n');
        text.push_str(&self.needs_line());
        text.push('\n');

        if !self.warnings.is_empty() {
            text.push_str("\nwarnings:\n");
            for warning in &self.warnings {
                text.push_str(&format!("  {warning}\n"));
            }
        }

        text
    }

    /// The `needs:` line on its own, without the table.
    ///
    /// `ginary stage --explain` prints the whole report; a caller that only
    /// wants the portability floor gets it here rather than by slicing the
    /// rendered text apart. The libraries are in sorted order, the glibc floor
    /// is in brackets after `libc.so.6` because that is the one entry whose
    /// *version* decides whether the artifact runs, and an artifact that needs
    /// nothing says `needs: (none)` rather than nothing at all. No trailing
    /// newline.
    pub fn needs_line(&self) -> String {
        if self.needs_summary.needed.is_empty() {
            return match &self.needs_summary.glibc_max {
                Some(floor) => format!("needs: ({GLIBC_PREFIX}{floor})"),
                None => "needs: (none)".to_owned(),
            };
        }

        // The floor belongs to the C library, so it is printed against that
        // entry. A tree that requires a `GLIBC_` version without naming a
        // `libc.so.*` is not a shape any toolchain produces, and the floor is
        // still printed rather than dropped, because a number nobody sees is
        // the one failure mode this line exists to prevent.
        let mut libc_named = false;
        let mut entries: Vec<String> = Vec::with_capacity(self.needs_summary.needed.len());
        for name in &self.needs_summary.needed {
            match &self.needs_summary.glibc_max {
                Some(floor) if !libc_named && name.starts_with("libc.so") => {
                    libc_named = true;
                    entries.push(format!("{name} ({GLIBC_PREFIX}{floor})"));
                }
                _ => entries.push(name.clone()),
            }
        }
        if let Some(floor) = &self.needs_summary.glibc_max
            && !libc_named
        {
            entries.push(format!("({GLIBC_PREFIX}{floor})"));
        }

        format!("needs: {}", entries.join(", "))
    }
}

/// Measures the staged root at `root` against what it was before stripping.
///
/// `before` is the [`StagedRoot`] staging returned, whose file sizes are the
/// pre-strip ones; `strip` is what the strip phase reported; `root` is the tree
/// on disk now, which is where the after sizes and every ELF dependency are
/// read from. Nothing is written.
///
/// # Errors
///
/// [`ReportError::Io`] when the staged root cannot be read at all. A single
/// unreadable *file* is a warning rather than an error — see
/// [`SizeReport::warnings`].
pub fn measure(
    before: &StagedRoot,
    strip: &StripReport,
    root: &Path,
) -> Result<SizeReport, ReportError> {
    // The strip report is an input to the account and never a source for it:
    // whatever a tool claimed, the sizes come from the files.
    let _ = strip;

    let mut categories: BTreeMap<Category, CategorySize> = BTreeMap::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut total_before = 0u64;
    let mut total_after = 0u64;

    for file in before.files() {
        let entry = categories.entry(file.category).or_default();
        entry.files += 1;
        entry.bytes_before = entry.bytes_before.saturating_add(file.size);
        total_before = total_before.saturating_add(file.size);

        match std::fs::metadata(root.join(&file.path)) {
            Ok(metadata) => {
                entry.bytes_after = entry.bytes_after.saturating_add(metadata.len());
                total_after = total_after.saturating_add(metadata.len());
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                warnings.push(format!(
                    "{} is in the listing and not in the tree",
                    file.path
                ));
            }
            Err(error) => {
                warnings.push(format!("{} cannot be measured: {error}", file.path));
            }
        }
    }

    let mut elf_deps = Vec::new();
    collect_elf_deps(root, root, &mut elf_deps, &mut warnings)?;
    elf_deps.sort_by(|left, right| left.path.cmp(&right.path));

    let mut needs_summary = NeedsSummary::default();
    for dep in &elf_deps {
        for name in &dep.needed {
            needs_summary.needed.insert(name.clone());
        }
    }
    // The per-file floors come back without the prefix and
    // `elf::max_glibc_version` reads them with it, because that is the shape
    // `.gnu.version_r` holds. Putting it back is cheaper than a second
    // comparison that could disagree with the first.
    let floors: Vec<String> = elf_deps
        .iter()
        .filter_map(|dep| dep.glibc_max.as_deref())
        .map(|version| format!("{GLIBC_PREFIX}{version}"))
        .collect();
    needs_summary.glibc_max = elf::max_glibc_version(floors.iter().map(String::as_str));

    Ok(SizeReport {
        categories,
        total_before,
        total_after,
        elf_deps,
        needs_summary,
        warnings,
    })
}

/// Walks `dir`, inspecting every file whose first bytes are the ELF magic.
///
/// A file that starts like an ELF and cannot be parsed is a warning rather than
/// a failure: one odd file in a staged tree must not cost the reader the whole
/// account.
fn collect_elf_deps(
    root: &Path,
    dir: &Path,
    into: &mut Vec<ElfDep>,
    warnings: &mut Vec<String>,
) -> Result<(), ReportError> {
    let entries = std::fs::read_dir(dir).map_err(|source| ReportError::Io {
        path: dir.to_path_buf(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| ReportError::Io {
            path: dir.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        if path.is_dir() {
            collect_elf_deps(root, &path, into, warnings)?;
            continue;
        }
        let listed = match path.strip_prefix(root).ok().and_then(slash_path) {
            Some(listed) => listed,
            None => continue,
        };
        match starts_like_an_elf(&path) {
            Ok(false) => continue,
            Ok(true) => {}
            Err(error) => {
                warnings.push(format!("{listed} cannot be read: {error}"));
                continue;
            }
        }
        match elf::inspect(&path) {
            Ok(info) => into.push(ElfDep {
                path: listed,
                needed: info.needed,
                glibc_max: info.glibc_max,
                interp: info.interp,
                machine: info.machine,
            }),
            Err(error) => warnings.push(format!("{listed} cannot be inspected: {error}")),
        }
    }
    Ok(())
}

/// Whether a file's first four bytes are the ELF magic.
///
/// Only the header is read: a staged tree holds a `beam.smp` of tens of
/// megabytes, and asking whether it is an ELF must not cost the price of
/// reading it.
fn starts_like_an_elf(path: &Path) -> Result<bool, std::io::Error> {
    let mut file = std::fs::File::open(path)?;
    let mut magic = [0u8; 4];
    let mut read = 0;
    while read < magic.len() {
        match file.read(&mut magic[read..])? {
            0 => return Ok(false),
            count => read += count,
        }
    }
    Ok(elf::is_elf(&magic))
}

/// A path relative to the staged root, `/`-separated, or `None` when a
/// component is not text.
fn slash_path(path: &Path) -> Option<String> {
    let mut text = String::new();
    for component in path.components() {
        if !text.is_empty() {
            text.push('/');
        }
        text.push_str(component.as_os_str().to_str()?);
    }
    Some(text)
}
