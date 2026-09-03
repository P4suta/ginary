// SPDX-License-Identifier: MIT OR Apache-2.0
//! Removing debug information from a staged root.
//!
//! Two unrelated tools do the work, because a staged root holds two unrelated
//! kinds of binary and neither tool understands the other's.
//!
//! **ELF.** The runtime's programs and any NIF under `priv` are native code,
//! and `beam.smp` alone carries tens of megabytes of DWARF that no packaged
//! application will ever read. They go through `strip(1)`: `--strip-all` for an
//! executable, `--strip-unneeded` for a shared object, whose dynamic symbol
//! table is the reason it can be loaded at all. Files are found by their magic
//! bytes rather than by their names, because a NIF is not required to be called
//! `.so` and a `.so` is not required to be a NIF. `strip` is not part of the
//! Rust toolchain and not everywhere, so a missing one is
//! [`ElfOutcome::Skipped`] with a reason — a reported decision, never a silent
//! one — while a `strip` that runs and fails on a file is an error naming the
//! file and quoting what the tool said.
//!
//! **BEAM.** `beam_lib` is the only supported way to remove `Dbgi` and `Docs`
//! from compiled modules, and it lives inside the runtime, so stripping them
//! means running Erlang. ginary runs the OTP installation's own `bin/erl` by
//! absolute path rather than whatever `erl` is on `PATH`: the modules being
//! stripped came from that installation, and a different release rewriting them
//! is exactly the kind of mismatch that surfaces three phases later as a module
//! that will not load.
//!
//! The function is `beam_lib:strip_files/1` and not `strip_release/1`, which is
//! the same work with the file list decided somewhere else. `strip_release/1`
//! takes a directory and expands `<root>/lib/*/ebin/*.beam` through
//! `filelib:wildcard/1`, so the root is a *glob prefix* rather than a path: a
//! staged root named `build[1]` matches nothing and one named `build*` reaches
//! every sibling directory whose name starts `build`. ginary has already walked
//! the tree, so it passes the modules it walked and the two halves cannot
//! disagree — every module the report counts is one the runtime was handed, and
//! a `.beam` outside `lib/<app>/ebin` is stripped like any other.
//!
//! Neither tool is trusted to have done what it said. Every ELF is re-inspected
//! afterwards and has to still be an ELF of the same class and machine; every
//! staged `.beam` is re-read afterwards and has to have lost [`beam::DEBUG_INFO_CHUNK`]
//! and [`beam::DOCS_CHUNK`] and kept [`beam::CODE_CHUNK`]. `strip_files`
//! returning `{ok, _}` while leaving a `Dbgi` behind would otherwise cost the
//! artifact half its size for nothing and say so nowhere.
//!
//! What survives is deliberate, and ADR 0007 records it: the `Line` chunk stays,
//! so a crash in a packaged application still produces a stack trace with line
//! numbers, and the price is that `dialyzer`, `cover` and the debugger cannot be
//! run against the shipped modules.
//!
//! Stripping is idempotent. Running it twice over the same root leaves the tree
//! byte for byte identical, because the second pass finds nothing to remove —
//! which is what makes "identical input produces identical artifact bytes"
//! survive this phase.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;

use crate::beam::{self, BeamError};
use crate::elf::{self, ElfError, ElfInfo, ElfKind};
use crate::otp::OtpInfo;
use crate::process::{self, ProcessError};
use crate::target::Target;

/// The program that strips an ELF file.
pub const STRIP_PROGRAM: &str = "strip";

/// The arguments `strip` gets for an executable.
pub const STRIP_ALL_ARGS: [&str; 1] = ["--strip-all"];

/// The arguments `strip` gets for a shared object.
///
/// `--strip-unneeded` rather than `--strip-all`, because a shared object's
/// dynamic symbol table is what makes it loadable: stripping it produces a file
/// that is smaller and cannot be opened.
pub const STRIP_UNNEEDED_ARGS: [&str; 1] = ["--strip-unneeded"];

/// How long `strip` gets for one file.
pub const ELF_STRIP_TIMEOUT: Duration = Duration::from_secs(120);

/// How long `beam_lib:strip_files/1` gets for one batch of modules.
///
/// Generous next to the few seconds a real tree takes, because a batch rewrites
/// most of the modules in the artifact, and finite because a runtime that fails
/// to halt must be a reported failure rather than a hung build.
pub const BEAM_STRIP_TIMEOUT: Duration = Duration::from_secs(300);

/// The Erlang expression that strips the modules named after `-extra`.
///
/// The modules arrive as plain arguments rather than being interpolated into
/// the expression, so a directory name holding a quote or a backslash cannot
/// become Erlang source; and they are passed as *paths* to
/// `beam_lib:strip_files/1` rather than as a directory to `strip_release/1`,
/// which would expand them through `filelib:wildcard/1` and turn a name holding
/// `*`, `?`, `[`, `]`, `{` or `}` into a pattern matching somewhere else.
/// `halt/1` carries the outcome, and the failure term is printed on standard
/// error, which is where the caller reads it from.
pub const STRIP_FILES_EVAL: &str = "Files=init:get_plain_arguments(), \
case beam_lib:strip_files(Files) of {ok,_} -> halt(0); \
Err -> io:format(standard_error,\"~p~n\",[Err]), halt(1) end.";

/// How many bytes of module paths one `erl` gets.
///
/// `execve(2)` bounds the argument vector — two megabytes on a common Linux,
/// a quarter of that on macOS — and the whole of an OTP release is about
/// 130 kB of paths, so one call is what a real staged root takes. The bound is
/// here so that an artifact large enough to cross it is stripped in several
/// calls rather than failing with an error from the kernel that says nothing
/// about modules.
pub const MAX_ARGUMENT_BYTES: usize = 256 * 1024;

/// Which halves of the staged root to strip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StripOptions {
    /// Whether to run `strip` over the native binaries.
    pub elf: bool,
    /// Whether to run `beam_lib:strip_files/1` over the modules.
    pub beams: bool,
}

impl Default for StripOptions {
    /// Both, which is what `ginary stage` and `ginary build` do unless told
    /// otherwise.
    fn default() -> Self {
        Self {
            elf: true,
            beams: true,
        }
    }
}

/// What happened to the native binaries.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ElfOutcome {
    /// `strip` ran over `files` files.
    Stripped {
        /// How many files were stripped.
        files: usize,
        /// What they weighed before.
        before: u64,
        /// What they weigh now.
        after: u64,
    },
    /// The tree holds no ELF file at all.
    ///
    /// Not a failure: a staged root built from a fake runtime, or one whose
    /// ERTS came from a platform ginary does not strip, legitimately has
    /// nothing here.
    NothingToStrip,
    /// Stripping was asked for and could not be done.
    Skipped {
        /// One line saying why, for example that `strip` is not on `PATH`.
        reason: String,
    },
    /// Stripping was not asked for.
    Disabled,
}

/// What happened to the compiled modules.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum BeamOutcome {
    /// `beam_lib:strip_files/1` ran and every module was verified.
    Stripped {
        /// How many `.beam` files the tree holds.
        files: usize,
        /// What they weighed before.
        before: u64,
        /// What they weigh now.
        after: u64,
    },
    /// Stripping was asked for and could not be done.
    Skipped {
        /// One line saying why, for example that the OTP root holds no `erl`.
        reason: String,
    },
    /// Stripping was not asked for.
    Disabled,
}

/// One file the strip phase changed, or tried to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct StrippedFile {
    /// The path relative to the staged root, `/`-separated.
    pub path: String,
    /// The size before stripping.
    pub before: u64,
    /// The size after stripping.
    pub after: u64,
}

/// What the strip phase did, and what it cost.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct StripReport {
    /// What happened to the native binaries.
    pub elf: ElfOutcome,
    /// What happened to the compiled modules.
    pub beams: BeamOutcome,
    /// Every file that was measured, sorted by path.
    pub per_file: Vec<StrippedFile>,
    /// The total size of those files before stripping.
    pub before_total: u64,
    /// The total size of those files after stripping.
    pub after_total: u64,
    /// Anything the phase decided to leave alone, one line each.
    ///
    /// A file that starts like an ELF and cannot be parsed as one is the case
    /// this exists for: it is not a binary any tool can strip, and one odd file
    /// under `priv` must not cost the build. `src/report.rs` treats the same
    /// file the same way, and CLAUDE.md's rule holds in both — the skip is
    /// reported, never silent.
    pub warnings: Vec<String>,
}

impl StripReport {
    /// The report of a run that stripped nothing because it was told not to.
    pub fn disabled() -> Self {
        Self {
            elf: ElfOutcome::Disabled,
            beams: BeamOutcome::Disabled,
            per_file: Vec::new(),
            before_total: 0,
            after_total: 0,
            warnings: Vec::new(),
        }
    }

    /// How many bytes stripping removed.
    ///
    /// Saturating, because a strip that made a file bigger is a defect to
    /// report rather than an arithmetic overflow to panic on.
    pub fn saved(&self) -> u64 {
        self.before_total.saturating_sub(self.after_total)
    }

    /// How many files the two halves between them measured.
    ///
    /// The sum of the halves rather than the length of
    /// [`StripReport::per_file`], because a half that did not run contributes
    /// nothing and a reader comparing the total line against the two above it
    /// has to find the same arithmetic.
    pub fn counted_files(&self) -> usize {
        let elf = match self.elf {
            ElfOutcome::Stripped { files, .. } => files,
            _ => 0,
        };
        let beams = match self.beams {
            BeamOutcome::Stripped { files, .. } => files,
            _ => 0,
        };
        elf.saturating_add(beams)
    }
}

/// The width the three labels are padded to, so the numbers line up.
const LABEL_WIDTH: usize = 7;

/// What a half that did not run because nobody asked it to prints.
const NOT_ASKED_FOR: &str = "not asked for";

/// One half's numbers, or the total's.
fn measured(files: usize, before: u64, after: u64) -> String {
    format!(
        "{files} files, {before} -> {after} bytes, {} saved",
        before.saturating_sub(after)
    )
}

impl fmt::Display for StripReport {
    /// The three-line table `ginary stage` prints after staging.
    ///
    /// One line per half and one for the total, each label padded to seven
    /// columns so the numbers line up:
    ///
    /// ```text
    /// elf:   4 files, 41675352 -> 11742936 bytes, 29932416 saved
    /// beams: 312 files, 9382144 -> 3011072 bytes, 6371072 saved
    /// total: 316 files, 51057496 -> 14754008 bytes, 36303488 saved
    /// ```
    ///
    /// A file the phase decided to leave alone adds a `warning:` line under
    /// the table, so that a skip is read rather than inferred from a count.
    ///
    /// A half that did not run says why in place of its numbers —
    /// `nothing to strip`, `skipped: <reason>`, `not asked for` — and the
    /// total is still the arithmetic, so a reader never has to work out
    /// whether a missing line meant zero or meant nothing happened. The file
    /// count on the total line is the sum of the two halves, not the length of
    /// [`StripReport::per_file`], which holds only the files that were
    /// individually measured.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let elf = match &self.elf {
            ElfOutcome::Stripped {
                files,
                before,
                after,
            } => measured(*files, *before, *after),
            ElfOutcome::NothingToStrip => "nothing to strip".to_owned(),
            ElfOutcome::Skipped { reason } => format!("skipped: {reason}"),
            ElfOutcome::Disabled => NOT_ASKED_FOR.to_owned(),
        };
        let beams = match &self.beams {
            BeamOutcome::Stripped {
                files,
                before,
                after,
            } => measured(*files, *before, *after),
            BeamOutcome::Skipped { reason } => format!("skipped: {reason}"),
            BeamOutcome::Disabled => NOT_ASKED_FOR.to_owned(),
        };
        let total = measured(self.counted_files(), self.before_total, self.after_total);

        writeln!(f, "{:<LABEL_WIDTH$}{elf}", "elf:")?;
        writeln!(f, "{:<LABEL_WIDTH$}{beams}", "beams:")?;
        writeln!(f, "{:<LABEL_WIDTH$}{total}", "total:")?;
        for warning in &self.warnings {
            writeln!(f, "warning: {warning}")?;
        }
        Ok(())
    }
}

/// Why a staged root could not be stripped.
#[derive(Debug, thiserror::Error)]
pub enum StripError {
    /// The staged root could not be walked.
    #[error("cannot read `{path}`: {source}")]
    Io {
        /// The path that could not be read.
        path: PathBuf,
        /// The underlying operating system error.
        #[source]
        source: std::io::Error,
    },
    /// `strip` ran on one file and failed.
    #[error("`strip` failed on `{path}`: {stderr}")]
    StripFailed {
        /// The file it was run on.
        path: PathBuf,
        /// What `strip` wrote to standard error, trimmed.
        stderr: String,
    },
    /// `strip` could not be run at all on one file.
    #[error("cannot run `strip` on `{path}`: {source}")]
    StripProcess {
        /// The file it would have been run on.
        path: PathBuf,
        /// Why the process failed.
        #[source]
        source: ProcessError,
    },
    /// A file that was an ELF before stripping is not one afterwards.
    #[error("`strip` left `{path}`, which was an ELF file, unreadable as one")]
    NotElfAfterStrip {
        /// The file that was destroyed.
        path: PathBuf,
    },
    /// A stripped file is a different class or machine than it was.
    #[error("`strip` changed `{path}` from {before} to {after}")]
    ElfChanged {
        /// The file that changed.
        path: PathBuf,
        /// The `<class>/<machine>` it was.
        before: String,
        /// The `<class>/<machine>` it is now.
        after: String,
    },
    /// An ELF file could not be inspected.
    #[error("cannot inspect `{path}`")]
    Elf {
        /// The file that could not be inspected.
        path: PathBuf,
        /// Why not.
        #[source]
        source: ElfError,
    },
    /// `beam_lib:strip_files/1` reported a failure.
    #[error("beam_lib:strip_files/1 failed: {stderr}")]
    BeamStripFailed {
        /// The Erlang term the runtime printed, trimmed.
        stderr: String,
    },
    /// The runtime could not be run at all.
    #[error("cannot run `{erl}`: {source}")]
    BeamStripProcess {
        /// The `erl` that would have been run.
        erl: PathBuf,
        /// Why the process failed.
        #[source]
        source: ProcessError,
    },
    /// A staged `.beam` could not be read back.
    #[error("cannot read the stripped `{path}`")]
    Beam {
        /// The module that could not be read.
        path: PathBuf,
        /// Why not.
        #[source]
        source: BeamError,
    },
    /// A staged module still holds a chunk stripping had to remove.
    ///
    /// `strip_files` answering `{ok, _}` and leaving the chunk in place is
    /// the failure this exists to catch: the build would otherwise ship the
    /// debug information it just reported removing.
    #[error(
        "`{path}` still holds the `{chunk}` chunk after beam_lib:strip_files/1 reported success"
    )]
    BeamStillHasChunk {
        /// The module relative to the staged root.
        path: String,
        /// The chunk that is still there.
        chunk: String,
    },
    /// A staged module lost its byte code.
    #[error("`{path}` has no `Code` chunk after beam_lib:strip_files/1; the module is destroyed")]
    BeamLostCode {
        /// The module relative to the staged root.
        path: String,
    },
}

/// Strips the staged root at `root` in place.
///
/// `otp` names the installation whose `bin/erl` runs
/// `beam_lib:strip_files/1`. The staged root is read back afterwards, so what
/// the report says is what the tree holds rather than what the tools claimed.
///
/// The sizes in `<root>/ginary.stage.json` are stale once this returns; the
/// caller refreshes them with [`crate::assemble::StagedRoot::refresh`].
///
/// # Errors
///
/// [`StripError::Io`] when the tree cannot be walked;
/// [`StripError::StripFailed`] and [`StripError::StripProcess`] when `strip`
/// fails on a file; [`StripError::NotElfAfterStrip`] and
/// [`StripError::ElfChanged`] when it damages one;
/// [`StripError::BeamStripFailed`] and [`StripError::BeamStripProcess`] when
/// the runtime fails; and [`StripError::BeamStillHasChunk`],
/// [`StripError::BeamLostCode`] or [`StripError::Beam`] when the modules do not
/// come out of it in the state they must be in.
pub fn strip(root: &Path, otp: &OtpInfo, opts: &StripOptions) -> Result<StripReport, StripError> {
    let mut per_file: Vec<StrippedFile> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    let staged = if opts.elf || opts.beams {
        walk(root, root)?
    } else {
        Vec::new()
    };

    let elf = if opts.elf {
        strip_elf(&staged, &mut per_file, &mut warnings)?
    } else {
        ElfOutcome::Disabled
    };
    let beams = if opts.beams {
        strip_beams(otp, &staged, &mut per_file)?
    } else {
        BeamOutcome::Disabled
    };

    per_file.sort_by(|left, right| left.path.cmp(&right.path));
    let before_total = per_file.iter().map(|file| file.before).sum();
    let after_total = per_file.iter().map(|file| file.after).sum();

    Ok(StripReport {
        elf,
        beams,
        per_file,
        before_total,
        after_total,
        warnings,
    })
}

/// One file of the staged tree, named both ways the phase needs it.
#[derive(Debug)]
struct Staged {
    /// The path on disk.
    path: PathBuf,
    /// The path relative to the staged root, `/`-separated, which is what the
    /// report prints and what a later phase can reproduce.
    listed: String,
}

/// Every file under `dir`, with its path relative to `root`.
///
/// Sorted, so that `strip` visits the tree in the same order on every machine
/// and a failure names the same file twice in a row.
fn walk(root: &Path, dir: &Path) -> Result<Vec<Staged>, StripError> {
    let mut found = Vec::new();
    collect(root, dir, &mut found)?;
    found.sort_by(|left: &Staged, right: &Staged| left.listed.cmp(&right.listed));
    Ok(found)
}

/// The recursive half of [`walk`].
fn collect(root: &Path, dir: &Path, into: &mut Vec<Staged>) -> Result<(), StripError> {
    let entries = std::fs::read_dir(dir).map_err(|source| StripError::Io {
        path: dir.to_path_buf(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| StripError::Io {
            path: dir.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        if path.is_dir() {
            collect(root, &path, into)?;
            continue;
        }
        let Some(listed) = path.strip_prefix(root).ok().and_then(slash_path) else {
            // A name that is not text cannot be reported, and staging refuses
            // to write one, so nothing that reaches here can be reached.
            continue;
        };
        into.push(Staged { path, listed });
    }
    Ok(())
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

/// Runs `strip` over every ELF file in the staged tree.
fn strip_elf(
    staged: &[Staged],
    per_file: &mut Vec<StrippedFile>,
    warnings: &mut Vec<String>,
) -> Result<ElfOutcome, StripError> {
    let mut natives: Vec<(&Staged, ElfInfo, u64)> = Vec::new();
    for file in staged {
        if !starts_like_an_elf(&file.path)? {
            continue;
        }
        // A file that begins with the magic and is not a whole ELF is not
        // something `strip` can work on, and it is not a reason to abandon the
        // build either: four bytes of `\x7fELF` under `priv` is inert data, a
        // fixture or a truncated download. `report::measure` reaches the same
        // decision about the same file, and both of them say so.
        let info = match elf::inspect(&file.path) {
            Ok(info) => info,
            Err(source) => {
                warnings.push(format!(
                    "{} starts like an ELF file and cannot be inspected, so it was left alone: \
                     {source}",
                    file.listed
                ));
                continue;
            }
        };
        natives.push((file, info, size_of(&file.path)?));
    }

    if natives.is_empty() {
        return Ok(ElfOutcome::NothingToStrip);
    }

    // A cross build stages binaries for another machine, and `strip` on this
    // one cannot read them: binutils is built for a set of architectures and
    // answers `Unable to recognise the format of the input file` for anything
    // outside it. Left alone and *said* rather than attempted, because a
    // failure here would stop a build over files that upstream already ships
    // stripped.
    let host = Target::host().arch.as_str();
    let foreign = natives
        .iter()
        .filter(|(_, info, _)| info.machine != host)
        .count();
    if foreign == natives.len() {
        return Ok(ElfOutcome::Skipped {
            reason: format!(
                "{foreign} native {} for another machine than this one ({}), and `strip` here \
                 reads {host}; they were left as the runtime shipped them",
                if foreign == 1 { "file is" } else { "files are" },
                natives
                    .iter()
                    .map(|(_, info, _)| info.machine.clone())
                    .collect::<std::collections::BTreeSet<_>>()
                    .into_iter()
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
        });
    }
    if foreign > 0 {
        warnings.push(format!(
            "{foreign} native {} for another machine and `strip` here reads {host}, so they were \
             left as the runtime shipped them",
            if foreign == 1 { "file is" } else { "files are" }
        ));
        natives.retain(|(_, info, _)| info.machine == host);
    }

    let Some(program) = process::find_in_path(STRIP_PROGRAM, std::env::var_os("PATH").as_deref())
    else {
        return Ok(ElfOutcome::Skipped {
            reason: format!(
                "`{STRIP_PROGRAM}` is not on PATH, so {} native {} kept their debug information; \
                 install binutils, or pass --no-strip to ask for this on purpose",
                natives.len(),
                if natives.len() == 1 { "file" } else { "files" }
            ),
        });
    };

    let mut before_total = 0u64;
    let mut after_total = 0u64;
    for (file, before_info, before) in &natives {
        let target = as_argument(&file.path)?;
        let mut argv: Vec<&str> = strip_arguments(&file.path, before_info).to_vec();
        argv.push(target);
        let output =
            process::run_with_timeout(&program, &argv, ELF_STRIP_TIMEOUT).map_err(|source| {
                StripError::StripProcess {
                    path: file.path.clone(),
                    source,
                }
            })?;
        if !output.success {
            return Err(StripError::StripFailed {
                path: file.path.clone(),
                stderr: said(&output.stderr, &output.stdout),
            });
        }

        // A tool that produced something smaller and unloadable is worse than
        // one that failed, because only the second says so.
        let after_info = match elf::inspect(&file.path) {
            Ok(info) => info,
            Err(ElfError::NotElf) => {
                return Err(StripError::NotElfAfterStrip {
                    path: file.path.clone(),
                });
            }
            Err(source) => {
                return Err(StripError::Elf {
                    path: file.path.clone(),
                    source,
                });
            }
        };
        if after_info.class != before_info.class || after_info.machine != before_info.machine {
            return Err(StripError::ElfChanged {
                path: file.path.clone(),
                before: shape(before_info),
                after: shape(&after_info),
            });
        }

        let after = size_of(&file.path)?;
        before_total = before_total.saturating_add(*before);
        after_total = after_total.saturating_add(after);
        per_file.push(StrippedFile {
            path: file.listed.clone(),
            before: *before,
            after,
        });
    }

    Ok(ElfOutcome::Stripped {
        files: natives.len(),
        before: before_total,
        after: after_total,
    })
}

/// Runs `beam_lib:strip_files/1` over every staged module and verifies them.
fn strip_beams(
    otp: &OtpInfo,
    staged: &[Staged],
    per_file: &mut Vec<StrippedFile>,
) -> Result<BeamOutcome, StripError> {
    let modules: Vec<&Staged> = staged
        .iter()
        .filter(|file| file.listed.ends_with(BEAM_SUFFIX))
        .collect();

    let erl = otp.root.join("bin").join(ERL_PROGRAM);
    if !erl.is_file() {
        return Ok(BeamOutcome::Skipped {
            reason: format!(
                "the OTP installation has no `{}`, and `beam_lib:strip_files/1` can only be \
                 run by the runtime the modules came from; {} module{} kept their debug \
                 information",
                erl.display(),
                modules.len(),
                if modules.len() == 1 { "" } else { "s" }
            ),
        });
    }

    if modules.is_empty() {
        // Nothing to hand over, and a runtime started to be told so is a
        // second of build time and a process that can fail for its own
        // reasons.
        return Ok(BeamOutcome::Stripped {
            files: 0,
            before: 0,
            after: 0,
        });
    }

    let mut sizes: Vec<u64> = Vec::with_capacity(modules.len());
    for module in &modules {
        sizes.push(size_of(&module.path)?);
    }

    for batch in batches(&modules, MAX_ARGUMENT_BYTES) {
        let mut arguments: Vec<&str> = vec![
            "-noshell",
            "-env",
            "ERL_CRASH_DUMP",
            process::NULL_DEVICE,
            "-eval",
            STRIP_FILES_EVAL,
            "-extra",
        ];
        for module in batch {
            arguments.push(as_argument(&module.path)?);
        }
        let output =
            process::run_with_timeout(&erl, &arguments, BEAM_STRIP_TIMEOUT).map_err(|source| {
                StripError::BeamStripProcess {
                    erl: erl.clone(),
                    source,
                }
            })?;
        if !output.success {
            return Err(StripError::BeamStripFailed {
                stderr: said(&output.stderr, &output.stdout),
            });
        }
    }

    let mut before_total = 0u64;
    let mut after_total = 0u64;
    for (module, before) in modules.iter().zip(sizes) {
        verify(module)?;
        let after = size_of(&module.path)?;
        before_total = before_total.saturating_add(before);
        after_total = after_total.saturating_add(after);
        per_file.push(StrippedFile {
            path: module.listed.clone(),
            before,
            after,
        });
    }

    Ok(BeamOutcome::Stripped {
        files: modules.len(),
        before: before_total,
        after: after_total,
    })
}

/// Splits `modules` into runs whose paths fit in `limit` bytes of argument.
///
/// A module whose path alone is longer than `limit` travels in a batch of its
/// own rather than in none: a bound that could produce an empty batch would
/// either lose a module or spin.
fn batches<'a>(modules: &[&'a Staged], limit: usize) -> Vec<Vec<&'a Staged>> {
    let mut batches: Vec<Vec<&Staged>> = Vec::new();
    let mut current: Vec<&Staged> = Vec::new();
    let mut used = 0usize;
    for module in modules {
        // One for the separator every argument costs the kernel.
        let cost = module.path.as_os_str().len().saturating_add(1);
        if !current.is_empty() && used.saturating_add(cost) > limit {
            batches.push(std::mem::take(&mut current));
            used = 0;
        }
        used = used.saturating_add(cost);
        current.push(module);
    }
    if !current.is_empty() {
        batches.push(current);
    }
    batches
}

/// Reads one stripped module back and holds it to what stripping promised.
fn verify(module: &Staged) -> Result<(), StripError> {
    let bytes = std::fs::read(&module.path).map_err(|source| StripError::Io {
        path: module.path.clone(),
        source,
    })?;
    let chunks = beam::chunks(&bytes).map_err(|source| StripError::Beam {
        path: module.path.clone(),
        source,
    })?;

    for chunk in &chunks {
        if is_removed_chunk(&chunk.id) {
            return Err(StripError::BeamStillHasChunk {
                path: module.listed.clone(),
                chunk: chunk.id_str(),
            });
        }
    }
    if !chunks.iter().any(|chunk| chunk.id == beam::CODE_CHUNK) {
        return Err(StripError::BeamLostCode {
            path: module.listed.clone(),
        });
    }
    Ok(())
}

/// The name of a compiled module.
const BEAM_SUFFIX: &str = ".beam";

/// The runtime's own launcher, under `<otp root>/bin`, as this host spells it.
///
/// `erl.exe` on Windows. The name was the constant `"erl"` until the first
/// Windows runner reported the whole beam half skipped — "the OTP installation
/// has no `d:/a/_temp/.setup-beam/otp\bin\erl`" — with the installation and
/// the program both present. [`crate::platform::erl_program`] is the rule, so
/// that the spelling is a value a Linux machine can assert rather than one
/// only a Windows runner can find.
const ERL_PROGRAM: &str = crate::platform::erl_program(crate::platform::HOST);

/// Whether a file's first four bytes are the ELF magic.
///
/// Only the header is read: a staged tree holds a `beam.smp` of tens of
/// megabytes, and asking whether it is native code must not cost the price of
/// reading it.
fn starts_like_an_elf(path: &Path) -> Result<bool, StripError> {
    use std::io::Read as _;

    let mut file = std::fs::File::open(path).map_err(|source| StripError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut magic = [0u8; 4];
    let mut read = 0;
    while read < magic.len() {
        let count = file
            .read(&mut magic[read..])
            .map_err(|source| StripError::Io {
                path: path.to_path_buf(),
                source,
            })?;
        if count == 0 {
            return Ok(false);
        }
        read += count;
    }
    Ok(elf::is_elf(&magic))
}

/// The arguments `strip` gets for one file.
///
/// [`STRIP_UNNEEDED_ARGS`] for a shared object and [`STRIP_ALL_ARGS`] for
/// everything else. Public because it is the one decision the ELF half makes
/// that nothing downstream can observe afterwards: `strip` leaves no record of
/// which arguments it was given, so a test that wants to hold this rule has to
/// ask it directly.
pub fn strip_arguments(path: &Path, info: &ElfInfo) -> &'static [&'static str] {
    if is_shared_object(path, info) {
        &STRIP_UNNEEDED_ARGS
    } else {
        &STRIP_ALL_ARGS
    }
}

/// Whether a file is a shared object rather than a program.
///
/// Being an `ET_DYN` is necessary and not sufficient, because a
/// position-independent *executable* is one too and must still be fully
/// stripped. What separates the two is either half of the rest: a library is
/// named `*.so` or `*.so.<version>`, or it has no program interpreter. A PIE
/// program has an interpreter — that is how the kernel starts it — and is not
/// named after a library, so it matches neither.
///
/// Both halves are needed. A NIF is not required to be called `.so`, so the
/// name alone would miss one; and a real library may well carry an
/// interpreter — glibc's `libc.so.6` runs as a program and prints its version
/// — so the interpreter alone would miss that. Getting this backwards costs a
/// loadable NIF its dynamic symbol table.
fn is_shared_object(path: &Path, info: &ElfInfo) -> bool {
    if info.kind != ElfKind::SharedObject {
        return false;
    }
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    name.ends_with(SHARED_OBJECT_SUFFIX)
        || name.contains(SHARED_OBJECT_INFIX)
        || info.interp.is_none()
}

/// What a shared object's name ends with.
const SHARED_OBJECT_SUFFIX: &str = ".so";

/// What a versioned shared object's name holds, as in `libc.so.6`.
const SHARED_OBJECT_INFIX: &str = ".so.";

/// The `<class>/<machine>` shape a stripped file has to keep.
fn shape(info: &ElfInfo) -> String {
    format!("{}-bit {}", info.class, info.machine)
}

/// What a failing program said, preferring standard error.
///
/// A program that fails writes its diagnosis to standard error and usually
/// nothing at all to standard output, but a report that quoted an empty string
/// would leave the reader with no explanation, so the other pipe is the
/// fallback.
fn said(stderr: &str, stdout: &str) -> String {
    let stderr = stderr.trim();
    if stderr.is_empty() {
        stdout.trim().to_owned()
    } else {
        stderr.to_owned()
    }
}

/// The size of one file.
fn size_of(path: &Path) -> Result<u64, StripError> {
    std::fs::metadata(path)
        .map(|metadata| metadata.len())
        .map_err(|source| StripError::Io {
            path: path.to_path_buf(),
            source,
        })
}

/// A path as a command line argument.
///
/// Both programs stripping runs take text arguments, and staging already
/// refuses to write a file whose name is not valid UTF-8, so this can only fail
/// for a staging *directory* the caller named — which is worth saying rather
/// than passing a lossy path to a tool that would then rewrite the wrong file.
fn as_argument(path: &Path) -> Result<&str, StripError> {
    path.to_str().ok_or_else(|| StripError::Io {
        path: path.to_path_buf(),
        source: std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "the path is not valid UTF-8, and `strip` and `erl` take text arguments",
        ),
    })
}

/// Whether a chunk identifier is one stripping has to have removed.
///
/// Public so that the verification rule and the ADR that records it cannot
/// drift apart, and so that a test asserts against the same list the code uses.
pub fn is_removed_chunk(id: &[u8; 4]) -> bool {
    *id == beam::DEBUG_INFO_CHUNK || *id == beam::DOCS_CHUNK
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A staged file at `listed`, for the batching tests.
    fn staged(listed: &str) -> Staged {
        Staged {
            path: PathBuf::from(listed),
            listed: listed.to_owned(),
        }
    }

    #[test]
    fn every_module_fits_in_one_batch_when_the_paths_are_short() {
        let modules = [staged("a.beam"), staged("b.beam")];
        let borrowed: Vec<&Staged> = modules.iter().collect();

        let batches = batches(&borrowed, MAX_ARGUMENT_BYTES);

        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].len(), 2);
    }

    #[test]
    fn a_tree_larger_than_one_argument_vector_is_stripped_in_several_calls() {
        // `execve(2)` bounds the argument vector, and a staged root large
        // enough to cross it has to be stripped in batches rather than fail
        // with an error from the kernel that says nothing about modules.
        let modules = [
            staged("aaaa.beam"),
            staged("bbbb.beam"),
            staged("cccc.beam"),
        ];
        let borrowed: Vec<&Staged> = modules.iter().collect();

        let batches = batches(&borrowed, 20);

        assert_eq!(
            batches
                .iter()
                .map(|batch| batch.iter().map(|module| module.listed.clone()).collect())
                .collect::<Vec<Vec<String>>>(),
            vec![
                vec!["aaaa.beam".to_owned(), "bbbb.beam".to_owned()],
                vec!["cccc.beam".to_owned()],
            ]
        );
    }

    #[test]
    fn a_path_longer_than_the_whole_bound_travels_alone_rather_than_not_at_all() {
        let modules = [staged("a.beam"), staged("a_very_long_name_indeed.beam")];
        let borrowed: Vec<&Staged> = modules.iter().collect();

        let batches = batches(&borrowed, 4);

        assert_eq!(batches.len(), 2, "no module may be dropped: {batches:?}");
        assert!(batches.iter().all(|batch| !batch.is_empty()));
    }

    #[test]
    fn no_modules_at_all_is_no_call_at_all() {
        assert!(batches(&[], MAX_ARGUMENT_BYTES).is_empty());
    }

    #[test]
    fn dbgi_and_docs_are_the_chunks_stripping_removes() {
        assert!(is_removed_chunk(&beam::DEBUG_INFO_CHUNK));
        assert!(is_removed_chunk(&beam::DOCS_CHUNK));
        assert!(!is_removed_chunk(&beam::CODE_CHUNK));
        assert!(!is_removed_chunk(&beam::LINE_CHUNK));
    }
}
