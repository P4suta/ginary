// SPDX-License-Identifier: MIT OR Apache-2.0
//! Native code in the shipment, matched against the target being built for.
//!
//! A Gleam application that depends on a NIF ships the compiled object in its
//! `priv` directory, and that object was built for the machine the developer
//! is standing on. A cross build has to notice: an artifact for
//! `linux-aarch64-musl` carrying an x86_64 glibc `.so` is one the loader will
//! refuse at run time, and refusing it at *build* time is the whole of this
//! module's job.
//!
//! Three questions, in order:
//!
//! 1. [`scan_shipment`] — what native code is in the shipment at all? Found by
//!    magic and never by extension, because a `.so` under `priv` may be a
//!    shell wrapper and a NIF may be called anything.
//! 2. [`reconcile`] — for one target, is each object usable, replaced by a
//!    configured override, replaced by a build hook, or a mismatch nobody
//!    declared? [`NativeError::Mismatch`] is the last of those, and it names
//!    the exact `gleam.toml` keys that would fix it.
//! 3. [`apply`] — copy the replacements over the staged tree, before the
//!    artifact is packed.
//!
//! [`NativeError::StaticRuntime`] is the one refusal `--allow-native-mismatch`
//! does not lift. A statically linked emulator has no dynamic loader in it, so
//! a `.so` beside it can never be opened however much its architecture agrees;
//! an artifact built anyway is one that fails on the user's machine instead of
//! on the developer's.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;

use crate::target::{Linkage, Target};

/// The directory a shipment application is staged under.
///
/// A shipment-relative `esqlite/priv/x.so` is staged as
/// `lib/esqlite/priv/x.so`; see `docs/dev/architecture.md`.
pub const STAGED_LIB_DIR: &str = "lib";

/// The directory component under which an application keeps native code.
pub const PRIV_DIR: &str = "priv";

/// How long one build hook may run before it is killed.
pub const HOOK_TIMEOUT: Duration = Duration::from_secs(600);

/// The token a hook command's canonical target name is substituted for.
pub const HOOK_TARGET_TOKEN: &str = "{target}";

/// The token a hook command's output directory is substituted for.
pub const HOOK_OUT_DIR_TOKEN: &str = "{out_dir}";

/// The work-directory component one package's hook output goes under.
///
/// The whole directory is `<work>/native/<target>/<package>/`, which is inside
/// the build's own work directory and therefore removed with it. The target is
/// in the path because the work directory is the *build*'s and a build makes
/// as many artifacts as it was given targets: a `make`-style hook that decides
/// its output is up to date and writes nothing would otherwise have the
/// previous target's object accepted in its place, and a static object — the
/// shape this module accepts for any target of its machine — would pass every
/// check on the way out.
pub const HOOK_OUT_COMPONENT: &str = "native";

/// The environment variables every hook is run with, in the order this module
/// sets them.
///
/// `ERL_INTERFACE_INCLUDE_DIR` is in the list and is the one that may be
/// *unset*: an OTP installation that ships no `erl_interface` has no directory
/// to name, and naming one that does not exist would be worse than saying
/// nothing.
pub const HOOK_ENV: [&str; 6] = [
    "GINARY_TARGET",
    "GINARY_TARGET_TRIPLE",
    "OUT_DIR",
    "ERTS_INCLUDE_DIR",
    "ERL_INTERFACE_INCLUDE_DIR",
    "OTP_VERSION",
];

/// The object file format a native artifact is in.
///
/// Defined by [`crate::platform`] and re-exported here, where the scanner
/// that names one lives: the container format is a fact about an operating
/// system — [`crate::platform::object_format`] — before it is a fact about a
/// file, and the launcher-side half of the suite has to be able to ask
/// without the `cli` feature's scanner in scope.
pub use crate::platform::ObjectFormat;

/// What kind of native artifact a file is.
///
/// The distinction that decides [`NativeError::StaticRuntime`]: a shared
/// object is loaded by the emulator through `dlopen`, and a program under
/// `priv/bin` is executed by the application itself and needs no loader inside
/// the runtime at all.
///
/// An ELF `e_type` does not answer it on its own — a position-independent
/// program is an `ET_DYN` like every shared library — so the ELF branch reads
/// `DF_1_PIE` as well; see [`crate::elf::ElfInfo::is_pie`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeKind {
    /// A shared object: a NIF, a driver, or a library one of them needs.
    SharedObject,
    /// A program the application runs as a child process.
    Executable,
    /// An unlinked object file: an ELF `ET_REL`, or the same shape in another
    /// format.
    ///
    /// Nothing loads one, and it is named rather than folded into
    /// [`NativeKind::Unknown`] because a build system that left a `.o` under
    /// `priv` is a different thing to fix than a file nobody could read.
    Relocatable,
    /// A core dump: an ELF `ET_CORE`, or the same shape in another format.
    Core,
    /// An ELF whose `e_type` is none of the above, carrying the number the
    /// header held.
    ///
    /// A processor-specific or operating-system-specific type, or a header
    /// holding a number no standard assigns. The number travels because it is
    /// the only thing anybody can act on.
    ElfType(u16),
    /// A file whose magic says it is an object and which will not parse.
    Unknown,
}

/// What an ELF's `e_type` and its `DF_1_PIE` flag add up to.
///
/// The rule in one place, because two readers apply it: the ELF arm of
/// [`describe_object`] and `ginary doctor`'s own walk over a shipment. A table that read `e_type`
/// itself would call a port program a shared object in one column and reach
/// the verdict in the next from the other answer, which is the shape of
/// report that contradicts itself about the fact deciding
/// [`NativeError::StaticRuntime`].
///
/// `ET_DYN` is both shapes — every program a modern toolchain links is one —
/// and `DF_1_PIE`, which a shared library does not carry, is what separates
/// them; see [`crate::elf::ElfInfo::is_pie`].
#[must_use]
pub const fn kind_of_elf(kind: crate::elf::ElfKind, is_pie: bool) -> NativeKind {
    match kind {
        crate::elf::ElfKind::SharedObject if is_pie => NativeKind::Executable,
        crate::elf::ElfKind::SharedObject => NativeKind::SharedObject,
        crate::elf::ElfKind::Executable => NativeKind::Executable,
        crate::elf::ElfKind::Relocatable => NativeKind::Relocatable,
        crate::elf::ElfKind::Core => NativeKind::Core,
        // Not [`NativeKind::Unknown`]: the header said what it was, and a
        // column that prints `unknown` over a number the file states is a
        // report that knows more than it says.
        crate::elf::ElfKind::Other(e_type) => NativeKind::ElfType(e_type),
    }
}

impl Serialize for NativeKind {
    /// One string, always, whatever the header held.
    ///
    /// `Serialize` by hand rather than derived, because the derive spells
    /// [`NativeKind::ElfType`] as `{"elf_type": 65024}` and every other variant
    /// as a string: `ginary doctor --json`'s `native[].kind` would change type
    /// depending on the file it describes, which is the one thing a
    /// machine-readable report may not do. The words are the ones
    /// [`NativeKind`]'s [`Display`] prints in the table beside it, so the two
    /// renderings of the same report cannot disagree — except in the four
    /// two-word names, which keep the `snake_case` spelling the derive gave
    /// them because a field a reader already parses may not be renamed for
    /// tidiness.
    ///
    /// [`Display`]: std::fmt::Display
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match *self {
            Self::SharedObject => serializer.serialize_str("shared_object"),
            Self::Executable => serializer.serialize_str("executable"),
            Self::Relocatable => serializer.serialize_str("relocatable"),
            Self::Core => serializer.serialize_str("core"),
            Self::ElfType(e_type) => serializer.serialize_str(&format!("e_type {e_type}")),
            Self::Unknown => serializer.serialize_str("unknown"),
        }
    }
}

impl std::fmt::Display for NativeKind {
    /// The words this kind prints as in a table.
    ///
    /// [`NativeKind::ElfType`] prints as `e_type <n>`, which is the header
    /// field spelled the way `readelf` spells it, because the number is the
    /// whole of what is known about such a file.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Self::SharedObject => f.write_str("shared object"),
            Self::Executable => f.write_str("executable"),
            Self::Relocatable => f.write_str("relocatable"),
            Self::Core => f.write_str("core"),
            Self::ElfType(e_type) => write!(f, "e_type {e_type}"),
            Self::Unknown => f.write_str("unknown"),
        }
    }
}

/// What was read out of an object's header.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ObjectFacts {
    /// The container format.
    pub format: ObjectFormat,
    /// The machine, spelled the way [`crate::target::Arch`] spells it.
    pub machine: String,
    /// The target the object names, when its header names a whole one.
    ///
    /// [`None`] for a Linux object with no `PT_INTERP`, whose C library is not
    /// written down anywhere in the file, and for a machine ginary has no
    /// target for.
    pub target: Option<Target>,
    /// How the object resolves its C library.
    pub linkage: Linkage,
}

/// One native file found under a shipment application's `priv`.
///
/// Or, in one case, a directory: [`MAX_PRIV_DEPTH`] stops the walk, and the
/// directory it stopped at is listed with [`NativeKind::Unknown`] and a
/// warning rather than dropped, because a build that read nothing below a
/// directory has to say so.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct NativeArtifact {
    /// The application it belongs to, which is the first path component.
    pub package: String,
    /// The path relative to the shipment, `/`-separated:
    /// `esqlite/priv/esqlite3_nif.so`.
    pub rel_path: String,
    /// What kind of artifact it is.
    pub kind: NativeKind,
    /// What its header said, or [`None`] when it could not be read.
    pub object: Option<ObjectFacts>,
    /// Its length on disk, in bytes.
    pub size: u64,
    /// Why this artifact could not be read, when it could not.
    ///
    /// Carried on the artifact rather than returned beside the list because a
    /// warning that is not attached to the row it is about is one a reader has
    /// to match up by hand. A scan never fails on one file: the build has not
    /// decided anything yet, and a `priv` directory holding four bytes of
    /// `\x7fELF` is a fixture far more often than it is a defect.
    pub warning: Option<String>,
}

impl NativeArtifact {
    /// Where this artifact lives in the staged tree.
    ///
    /// [`STAGED_LIB_DIR`] joined onto [`NativeArtifact::rel_path`], because
    /// assembly stages a shipment application at `lib/<name>` and the scan
    /// walked the shipment, where it is `<name>`.
    pub fn staged_path(&self) -> String {
        format!("{STAGED_LIB_DIR}/{}", self.rel_path)
    }
}

/// The native settings of one target, as `gleam.toml` holds them.
#[derive(Clone, Copy, Debug)]
pub struct TargetNativeCfg<'a> {
    /// `[tools.ginary.target.<name>.native]`: a shipment-relative artifact
    /// path against the project-relative file that replaces it.
    pub overrides: &'a BTreeMap<String, String>,
    /// `[tools.ginary.native.<package>] build`: a package name against the
    /// command that builds its native code.
    pub hooks: &'a BTreeMap<String, String>,
}

/// Everything [`reconcile`] needs beyond the artifacts themselves.
///
/// A struct rather than eight arguments: the values travel together, three of
/// them are only used to run a hook, and a positional list of that length is
/// one a caller gets wrong silently.
#[derive(Clone, Copy, Debug)]
pub struct ReconcileCtx<'a> {
    /// The target being built for.
    pub target: &'a Target,
    /// Whether the runtime resolved for that target can load a NIF.
    pub erts_nif_loading: bool,
    /// The overrides and hooks configured for it.
    pub cfg: &'a TargetNativeCfg<'a>,
    /// The project root, which every override path is relative to.
    pub project_root: &'a Path,
    /// The build's work directory; a hook writes under
    /// `<work>/native/<target>/<package>`.
    pub work_dir: &'a Path,
    /// The resolved runtime root, which `ERTS_INCLUDE_DIR` is derived from.
    pub erts_root: &'a Path,
    /// That runtime's ERTS version, which names the `erts-<vsn>` directory.
    pub erts_version: &'a str,
    /// The OTP version a hook is told about.
    pub otp_version: &'a str,
    /// Whether `--allow-native-mismatch` was given.
    pub allow_mismatch: bool,
}

/// Where a replacement object came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReplacementSource {
    /// A file the target's `native` table named, resolved against the project.
    Override(PathBuf),
    /// A file a package's build hook wrote.
    Hook {
        /// The package whose hook ran.
        package: String,
        /// The file the hook produced.
        out_path: PathBuf,
    },
}

impl ReplacementSource {
    /// The word this source is recorded as in the manifest and in `doctor`.
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Override(_) => "override",
            Self::Hook { .. } => "hook",
        }
    }

    /// The file whose bytes replace the artifact.
    pub fn path(&self) -> &Path {
        match self {
            Self::Override(path) => path,
            Self::Hook { out_path, .. } => out_path,
        }
    }
}

/// One staged artifact and the file whose bytes replace it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Replacement {
    /// The shipment-relative path of the artifact being replaced.
    pub artifact_rel_path: String,
    /// Where the replacement came from.
    pub source: ReplacementSource,
}

/// What [`reconcile`] decided.
///
/// The warnings are part of the answer rather than a side channel: an
/// `--allow-native-mismatch` build is *defined* by the sentences it prints,
/// and a caller that had to reconstruct them would print something else.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Reconciliation {
    /// The replacements to apply, in artifact path order.
    pub replacements: Vec<Replacement>,
    /// What the reconciliation decided that is not an error.
    pub warnings: Vec<String>,
}

/// One row of the table [`NativeError::Mismatch`] renders.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MismatchRow {
    /// The application the artifact belongs to.
    pub package: String,
    /// The artifact, relative to the shipment.
    pub rel_path: String,
    /// What the object is, as [`facts_line`] renders it.
    pub facts: String,
}

/// One row of the table [`NativeError::StaticRuntime`] renders.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StaticRow {
    /// The application the shared object belongs to.
    pub package: String,
    /// The shared object, relative to the shipment.
    pub rel_path: String,
}

/// What a build would do with one artifact for one target.
///
/// The `doctor` column, and the same decision [`reconcile`] makes, so that the
/// table a user reads before a build says what the build will say.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Verdict {
    /// The artifact is already for this target.
    Ok,
    /// The target's `native` table names a replacement for it.
    Override,
    /// Its package has a build hook.
    Hook,
    /// It is for another target and nothing was configured.
    Mismatch,
    /// It is a shared object and this target's runtime cannot load one.
    StaticRuntime,
}

impl Verdict {
    /// The word this verdict prints as in `doctor`'s native table.
    ///
    /// `MISMATCH` is upper case on purpose: it is the one value that stops a
    /// build, and a table where the failure looks like the other columns is a
    /// table nobody reads twice.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Override => "override",
            Self::Hook => "hook",
            Self::Mismatch => "MISMATCH",
            Self::StaticRuntime => "static-runtime",
        }
    }
}

/// What one build hook is told.
#[derive(Clone, Copy, Debug)]
pub struct HookCtx<'a> {
    /// The target being built for.
    pub target: &'a Target,
    /// The directory the hook writes its output into.
    pub out_dir: &'a Path,
    /// The directory the hook runs in.
    pub project_root: &'a Path,
    /// The resolved runtime root, which the include directories come from.
    pub erts_root: &'a Path,
    /// That runtime's ERTS version, which names the `erts-<vsn>` directory.
    pub erts_version: &'a str,
    /// The OTP version, as `erlang:system_info(version)` reports it.
    pub otp_version: &'a str,
}

/// How an object is described in a mismatch row.
///
/// `ELF x86_64 glibc (linux-x86_64-gnu)` — the format, the machine, the C
/// library and the target it adds up to, and `unreadable` for an object whose
/// header would not parse.
pub fn facts_line(facts: Option<&ObjectFacts>) -> String {
    let Some(facts) = facts else {
        return "unreadable".to_owned();
    };
    let libc = match (facts.linkage, facts.target.map(|target| target.libc)) {
        (Linkage::Static, _) => "static".to_owned(),
        (Linkage::Dynamic, Some(crate::target::Libc::Gnu)) => "glibc".to_owned(),
        (Linkage::Dynamic, Some(crate::target::Libc::Musl)) => "musl".to_owned(),
        (Linkage::Dynamic, _) => "dynamic".to_owned(),
    };
    let target = facts
        .target
        .map_or_else(|| "no target".to_owned(), |target| target.name());
    format!(
        "{} {} {libc} ({target})",
        facts.format.as_str().to_uppercase(),
        facts.machine,
    )
}

/// Renders the table and the fix lines [`NativeError::Mismatch`] prints.
///
/// The house table renderer, so that this message lines its columns up the way
/// every other table in the tool does, and then one `fix:` line per row naming
/// the two `gleam.toml` keys and the flag. Each row gets its own line because
/// the keys hold the row's own path and package: a single sentence would have
/// to be edited by hand for every file it named.
fn render_mismatch(target: Target, rows: &[MismatchRow]) -> String {
    let name = target.name();
    let mut text = format!("native code in the shipment does not match target {name}\n");
    let cells: Vec<[String; 3]> = rows
        .iter()
        .map(|row| [row.package.clone(), row.rel_path.clone(), row.facts.clone()])
        .collect();
    text.push_str(
        crate::closure::render_table(["package", "artifact", "object"], &cells).trim_end(),
    );
    for row in rows {
        let _ = write!(
            text,
            "\nfix: [tools.ginary.target.\"{name}\".native] \"{}\" = \"<a file for {name}>\", or \
             [tools.ginary.native.{}] build = \"<a command>\", or --allow-native-mismatch",
            row.rel_path, row.package,
        );
    }
    text
}

/// The sentence and table an `--allow-native-mismatch` build prints instead.
///
/// The same rows as [`NativeError::Mismatch`], because the facts have not
/// changed: the only difference is who decided, and the first line says so.
pub fn mismatch_warning(target: Target, rows: &[MismatchRow]) -> String {
    let name = target.name();
    let mut text = format!(
        "native code in the shipment does not match target {name}, and \
         --allow-native-mismatch was given: the artifact carries it as it is\n"
    );
    let cells: Vec<[String; 3]> = rows
        .iter()
        .map(|row| [row.package.clone(), row.rel_path.clone(), row.facts.clone()])
        .collect();
    text.push_str(
        crate::closure::render_table(["package", "artifact", "object"], &cells).trim_end(),
    );
    text
}

/// Renders the table [`NativeError::StaticRuntime`] prints.
fn render_static_runtime(target: Target, rows: &[StaticRow]) -> String {
    let name = target.name();
    let mut text = format!(
        "the runtime bundled for {name} is statically linked and cannot load a shared object\n"
    );
    let cells: Vec<[String; 2]> = rows
        .iter()
        .map(|row| [row.package.clone(), row.rel_path.clone()])
        .collect();
    text.push_str(crate::closure::render_table(["package", "artifact"], &cells).trim_end());
    let _ = write!(
        text,
        "\nfix: [tools.ginary.target.\"{name}\"] otp_variant = \"dynamic\", or build for a gnu \
         target; --allow-native-mismatch does not lift this one, because a static emulator has no \
         dynamic loader to call"
    );
    text
}

/// Why native code could not be reconciled with the target.
#[derive(Debug, thiserror::Error)]
pub enum NativeError {
    /// A file could not be read.
    #[error("cannot read `{path}`: {source}")]
    Io {
        /// The file that could not be read.
        path: PathBuf,
        /// The underlying operating system error.
        #[source]
        source: std::io::Error,
    },
    /// The shipment's native code is not for the target being built.
    #[error("{}", render_mismatch(*target, rows))]
    Mismatch {
        /// The target that was being built for.
        target: Target,
        /// One row per artifact nothing accounted for.
        rows: Vec<MismatchRow>,
    },
    /// The target's runtime cannot load the shared objects the shipment holds.
    #[error("{}", render_static_runtime(*target, rows))]
    StaticRuntime {
        /// The target that was being built for.
        target: Target,
        /// One row per shared object that would never load.
        rows: Vec<StaticRow>,
    },
    /// A configured override names a file that is not there.
    #[error("the native override for `{rel_path}` names `{path}`, which is not a file")]
    OverrideMissing {
        /// The artifact the override was configured for.
        rel_path: String,
        /// The file the override named.
        path: PathBuf,
    },
    /// A configured override is not for the target either.
    #[error("the native override `{path}` for `{rel_path}` is {found}, and {target} was asked for")]
    OverrideMismatch {
        /// The artifact the override was configured for.
        rel_path: String,
        /// The file the override named.
        path: PathBuf,
        /// What that file turned out to be.
        found: String,
        /// The target that was being built for.
        target: Target,
    },
    /// A build hook could not be started, or outlived [`HOOK_TIMEOUT`].
    #[error("the build hook for `{package}` could not be run: {source}")]
    HookProcess {
        /// The package whose hook it is.
        package: String,
        /// What stopped it.
        #[source]
        source: crate::process::ProcessError,
    },
    /// A build hook ran and failed.
    #[error("the build hook for `{package}` failed ({command}): {stderr}")]
    HookFailed {
        /// The package whose hook it is.
        package: String,
        /// The command that was run, after substitution.
        command: String,
        /// Everything it wrote to standard error.
        stderr: String,
    },
    /// A build hook succeeded and wrote nothing where the artifact belongs.
    #[error("the build hook for `{package}` wrote no `{expected}`")]
    HookOutputMissing {
        /// The package whose hook it is.
        package: String,
        /// The file the hook was expected to produce.
        expected: PathBuf,
    },
    /// A build hook produced an object for the wrong target.
    #[error(
        "the build hook for `{package}` wrote `{path}`, which is {found}, and {target} was \
             asked for"
    )]
    HookMismatch {
        /// The package whose hook it is.
        package: String,
        /// The file it wrote.
        path: PathBuf,
        /// What that file turned out to be.
        found: String,
        /// The target that was being built for.
        target: Target,
    },
    /// A replacement names a staged file the tree does not hold.
    #[error(
        "the staged tree holds no `{path}` to replace: the artifact does not carry application \
         `{package}`, so nothing in the shipment's copy of it travels"
    )]
    StagedMissing {
        /// The staged path, relative to the staging root.
        path: String,
        /// The application the missing path belongs to.
        package: String,
    },
}

/// The largest file the scan reads whole in order to read its header.
///
/// The same bound `ginary verify` applies, for the same reason: an object
/// header says nothing about how large the file behind it is, and a packaging
/// tool may not be the program that runs a machine out of memory. A `priv`
/// file larger than this is listed with [`NativeKind::Unknown`] and a warning
/// rather than dropped, because an artifact carrying an object nobody read is
/// exactly what the manifest has to say out loud.
pub const MAX_OBJECT_BYTES: u64 = crate::verify::MAX_OBJECT_BYTES;

/// How deep under one application's `priv` the scan walks.
///
/// A directory symlink is never descended into, so a real tree cannot cycle
/// and this bound is not against a loop: it is against unbounded recursion on
/// a tree nobody expected, and it is deliberately far below any `priv` a
/// project produces — a NIF under `priv/lib/<arch>/<abi>/` is four, and a
/// `priv/static` holding somebody's JavaScript build is the only thing that
/// gets near it.
///
/// Reaching it is reported rather than obeyed in silence: the walk lists the
/// directory it stopped at as an artifact with [`NativeKind::Unknown`] and a
/// [`NativeArtifact::warning`] naming the depth, so a tree with an object
/// buried below it is a build that says so instead of a build that shipped it
/// unread.
pub const MAX_PRIV_DEPTH: usize = 32;

/// How many bytes of a file are read to decide whether it is an object at all.
///
/// Enough for every magic this module knows and for a PE's `e_lfanew`, which
/// sits at `0x3c`. A `priv` directory holds assets as well as objects, and a
/// ninety-megabyte one that is not an object must not be read into memory to
/// learn that it is not.
const MAGIC_BYTES: usize = 64;

/// The signature at a PE file's `e_lfanew` offset.
const PE_MAGIC: &[u8; 4] = b"PE\0\0";

/// Where a DOS header records the offset of [`PE_MAGIC`].
const E_LFANEW_OFFSET: usize = 0x3c;

/// The shell a build hook is run through, on every host.
///
/// A hook is a command line rather than a program and an argument vector, so
/// that `make -C c_src && cp …` is one setting, and a shell is what reads it.
/// It is a POSIX shell wherever ginary runs, and deliberately not "whatever
/// this platform's shell is": [`run_hook`] substitutes its two tokens through
/// [`crate::process::shell_quote`], which renders for `/bin/sh` and for
/// nothing else, and a line quoted for one shell and read by another is a line
/// whose quote characters become part of a path. A host with no `/bin/sh` gets
/// [`NativeError::HookProcess`] naming it, which is the honest answer: a hook
/// compiles native code for a Linux target and wants a POSIX toolchain anyway.
pub const HOOK_SHELL: &str = "/bin/sh";

/// The flag [`HOOK_SHELL`] takes a whole command line after.
pub const HOOK_SHELL_FLAG: &str = "-c";

/// The directory of a runtime root that holds the `erl_interface` headers.
const ERL_INTERFACE_PREFIX: &str = "erl_interface-";

/// What one file on disk turned out to be.
///
/// [`scan_shipment`] turns each of these into a [`NativeArtifact`], and the
/// build reads one back off the *staged* tree after the replacements have been
/// applied, which is where the manifest's facts come from: what an artifact
/// records has to be what it carries, not what the shipment held.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectDescription {
    /// The container format its magic named.
    pub format: ObjectFormat,
    /// What kind of artifact it is.
    pub kind: NativeKind,
    /// What its header said, or [`None`] when it could not be read.
    pub facts: Option<ObjectFacts>,
    /// Why the header could not be read, when it could not.
    ///
    /// The reason alone, with no path in it: the caller knows which file it
    /// asked about and spells the sentence in the terms its own reader uses.
    pub unreadable: Option<String>,
}

/// Reads the object at `path`, or [`None`] when the file is not one.
///
/// The magic decides and never the extension: a `priv/lib/x.so` that is really
/// a shell wrapper is not native code, and a NIF may be called anything at
/// all. A file that begins like an object and will not parse is described with
/// [`NativeKind::Unknown`] and an [`ObjectDescription::unreadable`] reason,
/// because a build has to be able to say what it is shipping even when the
/// answer is "nobody could read this".
///
/// # Errors
///
/// [`NativeError::Io`] when the file cannot be stat'd or read.
pub fn describe_object(path: &Path) -> Result<Option<ObjectDescription>, NativeError> {
    let metadata = std::fs::metadata(path).map_err(io_at(path))?;
    if !metadata.is_file() {
        return Ok(None);
    }
    let head = read_head(path)?;
    let Some(format) = magic_of(&head) else {
        return Ok(None);
    };
    if metadata.len() > MAX_OBJECT_BYTES {
        return Ok(Some(unreadable(
            format,
            format!(
                "it is {} bytes, and no more than {MAX_OBJECT_BYTES} are read to inspect a header",
                metadata.len()
            ),
        )));
    }
    let bytes = std::fs::read(path).map_err(io_at(path))?;
    if format == ObjectFormat::Pe && !has_pe_signature(&bytes) {
        // `MZ` and no `PE\0\0` behind it: a DOS program, or a `.dll`
        // truncated before its PE header begins. Neither is an object any
        // target of this tool loads, and both are files that begin like one —
        // so this is the same answer a damaged ELF gets, a listed row saying
        // nobody could read it, rather than a file that quietly leaves the
        // scan.
        return Ok(Some(unreadable(format, NOT_A_PE_OBJECT.to_owned())));
    }
    Ok(Some(match format {
        ObjectFormat::Elf => describe_elf(&bytes),
        ObjectFormat::Pe | ObjectFormat::MachO => describe_with_object_crate(format, &bytes),
    }))
}

/// [`describe_object`] over an ELF, through [`crate::elf`].
///
/// The ELF reader is the crate's own rather than the `object` crate's generic
/// one, because a Linux object's *target* is its `PT_INTERP` and only
/// [`crate::elf::ElfInfo`] carries that.
fn describe_elf(bytes: &[u8]) -> ObjectDescription {
    let info = match crate::elf::inspect_bytes(bytes) {
        Ok(info) => info,
        Err(error) => return unreadable(ObjectFormat::Elf, error.to_string()),
    };
    let named = Target::from_elf(&info.machine, info.interp.as_deref());
    let facts = ObjectFacts {
        format: ObjectFormat::Elf,
        machine: info.machine.clone(),
        target: named.and_then(crate::target::ElfTarget::target),
        // A machine ginary has no target for still has an interpreter or no
        // interpreter, and that is what the linkage is: falling back to the
        // header rather than to `Dynamic` keeps a static object for an
        // unsupported machine from being reported as a dynamic one.
        linkage: named.map_or_else(
            || {
                if info.interp.is_some() {
                    Linkage::Dynamic
                } else {
                    Linkage::Static
                }
            },
            crate::target::ElfTarget::linkage,
        ),
    };
    ObjectDescription {
        format: ObjectFormat::Elf,
        // `ET_DYN` is both shapes, so `e_type` alone is not the answer:
        // every program a modern toolchain links is one, and reading a port
        // program under `priv/bin` as a library the emulator would `dlopen`
        // is what makes [`NativeError::StaticRuntime`] refuse a build nothing
        // was wrong with. [`kind_of_elf`] holds the rule, and `doctor` reads
        // the same one.
        kind: kind_of_elf(info.kind, info.is_pie),
        facts: Some(facts),
        unreadable: match info.kind {
            crate::elf::ElfKind::SharedObject | crate::elf::ElfKind::Executable => None,
            other => Some(format!(
                "it is an ELF of type {other:?}, which is neither a program nor a shared object"
            )),
        },
    }
}

/// [`describe_object`] over a PE or a Mach-O, through the `object` crate.
///
/// Neither format's C library is written down in a header — a PE names its
/// imports and a Mach-O its dylibs, and both platforms have one system
/// library — so the target either format names is whole as it stands and the
/// linkage is always [`Linkage::Dynamic`].
fn describe_with_object_crate(format: ObjectFormat, bytes: &[u8]) -> ObjectDescription {
    use object::Object as _;

    let file = match object::read::File::parse(bytes) {
        Ok(file) => file,
        Err(error) => return unreadable(format, error.to_string()),
    };
    let arch = arch_of(file.architecture());
    let machine = arch.map_or_else(
        || format!("{:?}", file.architecture()).to_lowercase(),
        |arch| arch.as_str().to_owned(),
    );
    let os = os_of(format);
    ObjectDescription {
        format,
        kind: match file.kind() {
            object::ObjectKind::Dynamic => NativeKind::SharedObject,
            object::ObjectKind::Executable => NativeKind::Executable,
            object::ObjectKind::Relocatable => NativeKind::Relocatable,
            object::ObjectKind::Core => NativeKind::Core,
            _ => NativeKind::Unknown,
        },
        facts: Some(ObjectFacts {
            format,
            machine,
            target: arch.map(|arch| Target::new(os, arch, crate::target::Libc::None)),
            linkage: Linkage::Dynamic,
        }),
        unreadable: None,
    }
}

/// What one object needs from the machine that runs it, whatever container
/// format it is in.
///
/// The shape [`crate::elf::ElfInfo`] already had, widened to the two formats
/// this crate reads through the `object` crate. It exists because three
/// readers of an artifact's native code — [`crate::verify`]'s deep check,
/// [`crate::report`]'s `needs:` line and `doctor`'s crypto report — each
/// asked "what does this file load" and each asked it of an ELF only, so on a
/// platform whose objects are PE all three reported the *absence* of native
/// code rather than the absence of a reader for it.
///
/// [`Self::interp`] and [`Self::glibc_max`] are ELF facts and are [`None`]
/// for the other two formats: neither a PE nor a Mach-O names a program
/// interpreter, and neither carries a glibc symbol-version table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectNeeds {
    /// The container format the file's magic named.
    pub format: ObjectFormat,
    /// The word size, `32` or `64`.
    pub class: u8,
    /// What kind of object the file is.
    ///
    /// [`crate::elf::ElfKind`] is the shared vocabulary rather than an ELF
    /// detail leaking out: its four named variants are exactly the four kinds
    /// the `object` crate reports for every format it reads, so a PE shared
    /// library and an ELF one describe as the same kind and a caller that
    /// prints the word needs no second table.
    pub kind: crate::elf::ElfKind,
    /// The machine, spelled the way [`crate::target::Arch`] spells it.
    pub machine: String,
    /// The program interpreter, for a format that names one.
    pub interp: Option<String>,
    /// The shared libraries the file loads at start-up, in header order and
    /// without repeats.
    ///
    /// An ELF's `DT_NEEDED` table, a PE's import directory, a Mach-O's
    /// `LC_LOAD_DYLIB` commands. The three are the same question asked of
    /// three headers.
    pub needed: Vec<String>,
    /// The highest `GLIBC_x.y` the file requires, without the prefix.
    pub glibc_max: Option<String>,
}

/// Why an object's header could not be read.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ObjectError {
    /// The bytes begin like no container format this build reads.
    #[error("the bytes begin like no object this build reads")]
    NotAnObject,
    /// The bytes begin like an object and are not a whole one.
    #[error("{message}")]
    Unreadable {
        /// What the reader said, as one sentence and with no path in it.
        message: String,
    },
}

/// Reads what the object in `bytes` loads, whatever container format it is in.
///
/// The format is [`crate::platform::object_format_of`]'s answer and never the
/// file name. An ELF goes through [`crate::elf`], because a Linux object's
/// target is its `PT_INTERP` and only that reader carries it; a PE and a
/// Mach-O go through the `object` crate, which names the same four kinds and
/// the same import lists.
///
/// # Errors
///
/// [`ObjectError::NotAnObject`] when the bytes begin like none of the three,
/// and [`ObjectError::Unreadable`] when they begin like one and will not
/// parse — including a file that begins `MZ` and carries no `PE\0\0`
/// signature behind it, which is a DOS program rather than a PE.
pub fn inspect_object_bytes(bytes: &[u8]) -> Result<ObjectNeeds, ObjectError> {
    let Some(format) = crate::platform::object_format_of(bytes) else {
        return Err(ObjectError::NotAnObject);
    };
    if format == ObjectFormat::Elf {
        let info = crate::elf::inspect_bytes(bytes).map_err(|source| ObjectError::Unreadable {
            message: source.to_string(),
        })?;
        return Ok(ObjectNeeds {
            format,
            class: info.class,
            kind: info.kind,
            machine: info.machine,
            interp: info.interp,
            needed: info.needed,
            glibc_max: info.glibc_max,
        });
    }
    if format == ObjectFormat::Pe && !has_pe_signature(bytes) {
        return Err(ObjectError::Unreadable {
            message: NOT_A_PE_OBJECT.to_owned(),
        });
    }
    read_with_object_crate(format, bytes)
}

/// What a file that begins `MZ` and carries no PE signature is told it is.
///
/// One sentence for one condition, because both readers reach it:
/// [`describe_object`] over a path and [`inspect_object_bytes`] over bytes.
/// Written once because it was written twice and the two spellings drifted —
/// one escaped the backslashes of the signature and the other put two literal
/// NUL bytes into a sentence that reaches a terminal and a JSON document. See
/// `tests/regressions/e11_a_pe_sentence_carried_two_raw_nul_bytes.rs`.
const NOT_A_PE_OBJECT: &str = "it begins with the DOS magic `MZ` and carries no `PE\\0\\0` \
                               signature behind it, so it is not a PE object";

/// [`inspect_object_bytes`] for the two formats the `object` crate reads.
fn read_with_object_crate(format: ObjectFormat, bytes: &[u8]) -> Result<ObjectNeeds, ObjectError> {
    use object::Object as _;

    let file = object::read::File::parse(bytes).map_err(|source| ObjectError::Unreadable {
        message: source.to_string(),
    })?;
    let machine = arch_of(file.architecture()).map_or_else(
        || format!("{:?}", file.architecture()).to_lowercase(),
        |arch| arch.as_str().to_owned(),
    );
    let mut needed: Vec<String> = Vec::new();
    // An import list names its library once per imported symbol, and the
    // question is which libraries the machine has to have rather than how
    // many symbols were asked of each.
    // A reader that could not follow the import directory has not learned
    // that the object imports nothing: it has learned nothing. Reported as
    // unreadable, the way a header that will not parse is, because the
    // alternative is a `needs: (none)` line about a file nobody read.
    let imports = file.imports().map_err(|source| ObjectError::Unreadable {
        message: source.to_string(),
    })?;
    for import in imports.into_iter().flatten() {
        let name = String::from_utf8_lossy(import.library()).into_owned();
        if !name.is_empty() && !needed.contains(&name) {
            needed.push(name);
        }
    }
    Ok(ObjectNeeds {
        format,
        class: if file.is_64() { 64 } else { 32 },
        kind: kind_of_object(file.kind()),
        machine,
        interp: None,
        needed,
        glibc_max: None,
    })
}

/// The [`crate::elf::ElfKind`] one `object` crate value names.
///
/// [`crate::elf::ElfKind::Other`] carries the number an ELF header held, and
/// a format with no such number is described with `0`: it is the value no
/// `e_type` has, so a reader cannot mistake it for one a header stated.
const fn kind_of_object(kind: object::ObjectKind) -> crate::elf::ElfKind {
    match kind {
        object::ObjectKind::Dynamic => crate::elf::ElfKind::SharedObject,
        object::ObjectKind::Executable => crate::elf::ElfKind::Executable,
        object::ObjectKind::Relocatable => crate::elf::ElfKind::Relocatable,
        object::ObjectKind::Core => crate::elf::ElfKind::Core,
        _ => crate::elf::ElfKind::Other(0),
    }
}

/// One [`ObjectDescription`] for a file that begins like an object and is not
/// a whole one.
fn unreadable(format: ObjectFormat, reason: String) -> ObjectDescription {
    ObjectDescription {
        format,
        kind: NativeKind::Unknown,
        facts: None,
        unreadable: Some(reason),
    }
}

/// The architecture one `object` crate value names, when ginary has a name for
/// it.
fn arch_of(architecture: object::Architecture) -> Option<crate::target::Arch> {
    match architecture {
        object::Architecture::X86_64 => Some(crate::target::Arch::X86_64),
        object::Architecture::Aarch64 => Some(crate::target::Arch::Aarch64),
        _ => None,
    }
}

/// The operating system a container format belongs to.
const fn os_of(format: ObjectFormat) -> crate::target::Os {
    match format {
        ObjectFormat::Elf => crate::target::Os::Linux,
        ObjectFormat::Pe => crate::target::Os::Windows,
        ObjectFormat::MachO => crate::target::Os::Macos,
    }
}

/// The format `head` begins like, or [`None`] when it begins like no object.
///
/// [`crate::platform::object_format_of`] holds the rule, because three other
/// call sites — [`crate::verify`], [`crate::strip`] and [`crate::report`] —
/// used to spell it for themselves and each of them spelled only its ELF
/// half. This one was the only complete spelling; it is now the only
/// spelling.
///
/// The PE answer is provisional: a DOS header is two bytes, and only the
/// `PE\0\0` signature it points at makes the file a PE. [`describe_object`]
/// confirms it once the bytes are in hand.
fn magic_of(head: &[u8]) -> Option<ObjectFormat> {
    crate::platform::object_format_of(head)
}

/// Whether `bytes` carries [`PE_MAGIC`] where its DOS header points.
fn has_pe_signature(bytes: &[u8]) -> bool {
    let Some(field) = bytes.get(E_LFANEW_OFFSET..E_LFANEW_OFFSET + 4) else {
        return false;
    };
    let Ok(field) = <[u8; 4]>::try_from(field) else {
        return false;
    };
    let Ok(at) = usize::try_from(u32::from_le_bytes(field)) else {
        return false;
    };
    bytes
        .get(at..at.saturating_add(PE_MAGIC.len()))
        .is_some_and(|signature| signature == PE_MAGIC)
}

/// The first [`MAGIC_BYTES`] of a file, or fewer when it is shorter.
///
/// # Errors
///
/// [`NativeError::Io`] when the file cannot be opened or read.
fn read_head(path: &Path) -> Result<Vec<u8>, NativeError> {
    use std::io::Read as _;

    let file = std::fs::File::open(path).map_err(io_at(path))?;
    let mut head = Vec::with_capacity(MAGIC_BYTES);
    file.take(MAGIC_BYTES as u64)
        .read_to_end(&mut head)
        .map_err(io_at(path))?;
    Ok(head)
}

/// [`NativeError::Io`] about one path, as a closure a `map_err` takes.
fn io_at(path: &Path) -> impl Fn(std::io::Error) -> NativeError + use<'_> {
    move |source| NativeError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// Whether an object's facts answer for one target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fit {
    /// The object names this target.
    Match,
    /// The machine agrees and the file says nothing about its C library.
    ///
    /// What a statically linked Linux object is, and what every musl NIF built
    /// `-static` looks like: refusing it would refuse the ordinary case, and
    /// recording a libc for it would be a guess written into a manifest. It is
    /// accepted, and the caller says so.
    StaticMatch,
    /// It is for something else.
    Wrong,
}

/// Whether one object can travel to `target`.
///
/// The machine and the container format first, because neither is negotiable:
/// an ELF is not a Windows object however much its architecture agrees. The C
/// library is compared only when the file names one.
fn fit(facts: &ObjectFacts, target: &Target) -> Fit {
    if facts.machine != target.arch.as_str() || os_of(facts.format) != target.os {
        return Fit::Wrong;
    }
    match facts.target {
        Some(named) if named == *target => Fit::Match,
        Some(_) => Fit::Wrong,
        None => Fit::StaticMatch,
    }
}

/// `base` with a `/`-separated relative path joined onto it.
///
/// [`None`] for anything that is not a relative path made of ordinary
/// components, so a `..` in an artifact path — which the scan cannot produce
/// and a caller could hand in — cannot make a copy land outside the tree it
/// was given.
fn under(base: &Path, relative: &str) -> Option<PathBuf> {
    let mut path = base.to_path_buf();
    let mut components = 0usize;
    for component in relative.split('/') {
        if component.is_empty() || component == "." || component == ".." {
            return None;
        }
        path.push(component);
        components = components.saturating_add(1);
    }
    (components > 0).then_some(path)
}

/// Every native file under `shipment`'s application `priv` directories.
///
/// Walked recursively, classified by magic rather than by extension, and
/// returned in path order. A file whose magic says it is an object and which
/// will not parse is listed with [`NativeKind::Unknown`] and a
/// [`NativeArtifact::warning`], never raised: the scan describes the shipment,
/// and deciding what to do about it is [`reconcile`]'s job.
///
/// Only `<app>/priv` is walked. An object under `ebin` is whatever a build
/// system left beside the compiler's output, and nothing in the artifact loads
/// it as native code.
///
/// # Errors
///
/// [`NativeError::Io`] when the shipment cannot be walked.
pub fn scan_shipment(shipment: &Path) -> Result<Vec<NativeArtifact>, NativeError> {
    let mut found = Vec::new();
    for app in read_dir_sorted(shipment)? {
        let Some(package) = app.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let package = package.to_owned();
        if !app.is_dir() {
            continue;
        }
        let priv_dir = app.join(PRIV_DIR);
        if !priv_dir.is_dir() {
            continue;
        }
        scan_dir(shipment, &priv_dir, &package, 0, &mut found)?;
    }
    found.sort_by(|left, right| left.rel_path.cmp(&right.rel_path));
    Ok(found)
}

/// One directory of the walk [`scan_shipment`] performs.
///
/// A directory symlink is never descended into and a symlink is followed only
/// when it resolves to a regular file, which is the rule `doctor`'s own walk
/// follows: a NIF installed as a link is native code, and a link to a
/// directory is a loop waiting to happen.
fn scan_dir(
    shipment: &Path,
    dir: &Path,
    package: &str,
    depth: usize,
    found: &mut Vec<NativeArtifact>,
) -> Result<(), NativeError> {
    if depth >= MAX_PRIV_DEPTH {
        let rel_path = dir
            .strip_prefix(shipment)
            .unwrap_or(dir)
            .to_string_lossy()
            .replace('\\', "/");
        found.push(NativeArtifact {
            package: package.to_owned(),
            warning: Some(format!(
                "{rel_path}: the walk stopped at depth {MAX_PRIV_DEPTH}, so whatever is \
                 below this directory was not read"
            )),
            rel_path,
            kind: NativeKind::Unknown,
            object: None,
            size: 0,
        });
        return Ok(());
    }
    for path in read_dir_sorted(dir)? {
        let kind = std::fs::symlink_metadata(&path).map_err(io_at(&path))?;
        if kind.is_dir() {
            scan_dir(shipment, &path, package, depth.saturating_add(1), found)?;
            continue;
        }
        // `metadata` rather than `symlink_metadata`: this is where a symlink
        // is followed, and only one whose target is a regular file gets past
        // it. A dangling link, a device and a socket are not native code.
        if !std::fs::metadata(&path).is_ok_and(|target| target.is_file()) {
            continue;
        }
        let Ok(relative) = path.strip_prefix(shipment) else {
            continue;
        };
        let rel_path = relative.to_string_lossy().replace('\\', "/");
        let Some(description) = describe_object(&path)? else {
            continue;
        };
        let size = std::fs::metadata(&path).map_err(io_at(&path))?.len();
        found.push(NativeArtifact {
            package: package.to_owned(),
            warning: description
                .unreadable
                .map(|reason| format!("{rel_path}: {reason}")),
            rel_path,
            kind: description.kind,
            object: description.facts,
            size,
        });
    }
    Ok(())
}

/// The entries of one directory, in path order.
///
/// # Errors
///
/// [`NativeError::Io`] when the directory cannot be read. A shipment ginary
/// cannot walk is a build that stops: an object nobody looked at is exactly
/// what this module exists to refuse.
fn read_dir_sorted(dir: &Path) -> Result<Vec<PathBuf>, NativeError> {
    let mut paths = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(io_at(dir))? {
        paths.push(entry.map_err(io_at(dir))?.path());
    }
    paths.sort();
    Ok(paths)
}

/// The artifacts a staged tree actually carries.
///
/// The shipment is every application `gleam` exported; an artifact carries the
/// dependency *closure* of the one application being packaged, which is a
/// subset — and an application the closure shadowed with the OTP copy is
/// another way the two differ. An object in an application the artifact does
/// not carry is not the build's business: refusing a cross build over a file
/// that would never have travelled would be a refusal with no remedy that
/// makes sense, and an override written for one would name a staged path the
/// tree does not hold.
///
/// This is not a silent skip. Every artifact the scan found is reported by the
/// scan, warnings included, before this filter is applied; what the filter
/// decides is only which of them the *target* has to answer for.
pub fn staged_only(artifacts: &[NativeArtifact], staged_root: &Path) -> Vec<NativeArtifact> {
    artifacts
        .iter()
        .filter(|artifact| {
            under(staged_root, &artifact.staged_path()).is_some_and(|path| path.is_file())
        })
        .cloned()
        .collect()
}

/// Matches every artifact against the target, replacing what is configured.
///
/// Per artifact, in this order: an override for its path, a hook for its
/// package, the artifact's own target, and last a mismatch. The mismatches are
/// collected rather than raised one at a time, so a shipment with four foreign
/// objects produces one table rather than four builds.
///
/// One package's hook runs once however many artifacts it accounts for, and it
/// does not run at all for an artifact an override already answered: a hook is
/// a compiler, and running one to throw its output away would be minutes spent
/// on a decision that was already made. For the same reason the static-runtime
/// refusal is reached *before* the loop rather than after it: it is decided by
/// the scan and the runtime alone, and no replacement can turn a shared object
/// into something a static emulator could open.
///
/// # Errors
///
/// [`NativeError::Mismatch`] unless `allow_mismatch`,
/// [`NativeError::StaticRuntime`] whether or not it is set, and the override
/// and hook failures.
pub fn reconcile(
    artifacts: &[NativeArtifact],
    ctx: &ReconcileCtx<'_>,
) -> Result<Reconciliation, NativeError> {
    // Before the loop, because nothing in the loop can change the answer: a
    // NIF an override or a hook answered for is still a NIF, and a runtime
    // with no dynamic loader in it can no more open the replacement than the
    // original. Deciding it afterwards would spend one `HOOK_TIMEOUT` per
    // configured package — ten minutes each, of the project's own compiler —
    // on output thrown away one line later, to reach an error
    // `--allow-native-mismatch` does not lift either.
    if !ctx.erts_nif_loading {
        let mut rows: Vec<StaticRow> = artifacts
            .iter()
            .filter(|artifact| artifact.kind == NativeKind::SharedObject)
            .map(|artifact| StaticRow {
                package: artifact.package.clone(),
                rel_path: artifact.rel_path.clone(),
            })
            .collect();
        rows.sort_by(|left, right| left.rel_path.cmp(&right.rel_path));
        if !rows.is_empty() {
            return Err(NativeError::StaticRuntime {
                target: *ctx.target,
                rows,
            });
        }
    }

    let mut replacements = Vec::new();
    let mut warnings = Vec::new();
    let mut mismatches: Vec<MismatchRow> = Vec::new();
    let mut hooked: BTreeMap<String, PathBuf> = BTreeMap::new();

    for artifact in artifacts {
        if let Some(named) = ctx.cfg.overrides.get(&artifact.rel_path) {
            replacements.push(from_override(artifact, named, ctx, &mut warnings)?);
            continue;
        }
        if let Some(command) = ctx.cfg.hooks.get(&artifact.package) {
            replacements.push(from_hook(
                artifact,
                command,
                ctx,
                &mut hooked,
                &mut warnings,
            )?);
            continue;
        }
        match artifact.object.as_ref().map(|facts| fit(facts, ctx.target)) {
            Some(Fit::Match) => {}
            Some(Fit::StaticMatch) => warnings.push(format!(
                "{}: no interpreter, so which C library it wants is not written down in the \
                 file; it is kept for {}",
                artifact.rel_path,
                ctx.target.name()
            )),
            Some(Fit::Wrong) | None => mismatches.push(MismatchRow {
                package: artifact.package.clone(),
                rel_path: artifact.rel_path.clone(),
                facts: facts_line(artifact.object.as_ref()),
            }),
        }
    }

    // One table rather than one build per file, and in path order rather than
    // in whichever order the artifacts arrived: the list is what a user reads
    // and then goes and edits `gleam.toml` from.
    mismatches.sort_by(|left, right| left.rel_path.cmp(&right.rel_path));
    if !mismatches.is_empty() {
        if !ctx.allow_mismatch {
            return Err(NativeError::Mismatch {
                target: *ctx.target,
                rows: mismatches,
            });
        }
        warnings.push(mismatch_warning(*ctx.target, &mismatches));
    }

    Ok(Reconciliation {
        replacements,
        warnings,
    })
}

/// One artifact answered by the override its path names.
///
/// # Errors
///
/// [`NativeError::OverrideMissing`] when the file is not there and
/// [`NativeError::OverrideMismatch`] when it is not for the target either.
fn from_override(
    artifact: &NativeArtifact,
    named: &str,
    ctx: &ReconcileCtx<'_>,
    warnings: &mut Vec<String>,
) -> Result<Replacement, NativeError> {
    let path = ctx.project_root.join(named);
    if !path.is_file() {
        return Err(NativeError::OverrideMissing {
            rel_path: artifact.rel_path.clone(),
            path,
        });
    }
    match verify_replacement(&path, ctx.target)? {
        Fit::Match => {}
        Fit::StaticMatch => warnings.push(format!(
            "the native override {} for {} has no interpreter, so which C library it wants is \
             not written down in the file; it is accepted for {}",
            path.display(),
            artifact.rel_path,
            ctx.target.name()
        )),
        Fit::Wrong => {
            return Err(NativeError::OverrideMismatch {
                rel_path: artifact.rel_path.clone(),
                found: describe_replacement(&path)?,
                path,
                target: *ctx.target,
            });
        }
    }
    Ok(Replacement {
        artifact_rel_path: artifact.rel_path.clone(),
        source: ReplacementSource::Override(path),
    })
}

/// One artifact answered by its package's build hook.
///
/// `hooked` is what keeps a hook to one run: the package's output directory
/// once the hook has written it, so a second artifact of the same package
/// reads the same tree rather than compiling it again.
///
/// # Errors
///
/// The hook failures, [`NativeError::HookOutputMissing`] when it wrote nothing
/// where the artifact belongs, and [`NativeError::HookMismatch`] when what it
/// wrote is not for the target.
fn from_hook(
    artifact: &NativeArtifact,
    command: &str,
    ctx: &ReconcileCtx<'_>,
    hooked: &mut BTreeMap<String, PathBuf>,
    warnings: &mut Vec<String>,
) -> Result<Replacement, NativeError> {
    let out_dir = match hooked.get(&artifact.package) {
        Some(dir) => dir.clone(),
        None => {
            let dir = hook_out_dir(ctx.work_dir, ctx.target, &artifact.package);
            run_hook(
                &artifact.package,
                command,
                &HookCtx {
                    target: ctx.target,
                    out_dir: &dir,
                    project_root: ctx.project_root,
                    erts_root: ctx.erts_root,
                    erts_version: ctx.erts_version,
                    otp_version: ctx.otp_version,
                },
            )?;
            hooked.insert(artifact.package.clone(), dir.clone());
            dir
        }
    };
    let expected = under(&out_dir, &artifact.rel_path).unwrap_or_else(|| out_dir.clone());
    if !expected.is_file() {
        return Err(NativeError::HookOutputMissing {
            package: artifact.package.clone(),
            expected,
        });
    }
    match verify_replacement(&expected, ctx.target)? {
        Fit::Match => {}
        Fit::StaticMatch => warnings.push(format!(
            "the build hook for {} wrote {}, which has no interpreter, so which C library it \
             wants is not written down in the file; it is accepted for {}",
            artifact.package,
            expected.display(),
            ctx.target.name()
        )),
        Fit::Wrong => {
            return Err(NativeError::HookMismatch {
                package: artifact.package.clone(),
                found: describe_replacement(&expected)?,
                path: expected,
                target: *ctx.target,
            });
        }
    }
    Ok(Replacement {
        artifact_rel_path: artifact.rel_path.clone(),
        source: ReplacementSource::Hook {
            package: artifact.package.clone(),
            out_path: expected,
        },
    })
}

/// Where one package's hook writes: `<work>/native/<target>/<package>/`.
///
/// See [`HOOK_OUT_COMPONENT`] for why the target is a component of it.
pub fn hook_out_dir(work_dir: &Path, target: &Target, package: &str) -> PathBuf {
    work_dir
        .join(HOOK_OUT_COMPONENT)
        .join(target.name())
        .join(package)
}

/// Whether the replacement at `path` may stand in for the target's object.
///
/// # Errors
///
/// [`NativeError::Io`] when the file cannot be read.
fn verify_replacement(path: &Path, target: &Target) -> Result<Fit, NativeError> {
    Ok(match describe_object(path)? {
        Some(description) => match description.facts.as_ref() {
            Some(facts) => fit(facts, target),
            None => Fit::Wrong,
        },
        // A replacement that is not an object at all is refused with the same
        // sentence a foreign one is: `unreadable` is what the row says, and
        // "this file is not native code" is the fault either way.
        None => Fit::Wrong,
    })
}

/// How a refused replacement is described in the error that names it.
///
/// # Errors
///
/// [`NativeError::Io`] when the file cannot be read.
fn describe_replacement(path: &Path) -> Result<String, NativeError> {
    Ok(facts_line(
        describe_object(path)?
            .as_ref()
            .and_then(|description| description.facts.as_ref()),
    ))
}

/// Runs one package's build hook and returns the directory it wrote into.
///
/// `command` is substituted — [`HOOK_TARGET_TOKEN`] and
/// [`HOOK_OUT_DIR_TOKEN`] — and run through [`HOOK_SHELL`] in the project
/// root, with [`HOOK_ENV`] set and under [`HOOK_TIMEOUT`].
///
/// Each token is substituted as one shell word, through
/// [`crate::process::shell_quote`], so a project directory whose name holds a
/// space reaches the hook whole. A command that writes `"{out_dir}"` would
/// therefore be quoting an already-quoted word; `$OUT_DIR` is there for the
/// hook that wants the value inside a string.
///
/// The output directory is created before the hook runs, because a hook whose
/// first act had to be `mkdir -p "$OUT_DIR"` would be a contract every project
/// implements slightly differently.
///
/// # Errors
///
/// [`NativeError::HookProcess`] when it cannot be run or outlives the budget,
/// and [`NativeError::HookFailed`] when it exits non-zero.
pub fn run_hook(package: &str, command: &str, ctx: &HookCtx<'_>) -> Result<PathBuf, NativeError> {
    std::fs::create_dir_all(ctx.out_dir).map_err(io_at(ctx.out_dir))?;
    let target = ctx.target.name();
    // Quoted, because the line goes to a shell and a project under `My
    // Documents` is the ordinary case rather than the exotic one: an
    // unquoted `{out_dir}` stops being one argument at the first space, and a
    // `$(...)` in a path would be worse than a broken command. A hook author
    // therefore writes `{out_dir}` and never `"{out_dir}"`.
    let line = command
        .replace(HOOK_TARGET_TOKEN, &crate::process::shell_quote(&target))
        .replace(
            HOOK_OUT_DIR_TOKEN,
            &crate::process::shell_quote_path(ctx.out_dir),
        );

    let mut env: Vec<(&str, std::ffi::OsString)> = vec![
        ("GINARY_TARGET", target.clone().into()),
        ("GINARY_TARGET_TRIPLE", ctx.target.rust_triple().into()),
        ("OUT_DIR", ctx.out_dir.as_os_str().to_os_string()),
        (
            "ERTS_INCLUDE_DIR",
            ctx.erts_root
                .join(format!("erts-{}", ctx.erts_version))
                .join("include")
                .into_os_string(),
        ),
        ("OTP_VERSION", ctx.otp_version.into()),
    ];
    // Absent rather than empty when the runtime ships no `erl_interface`:
    // naming a directory that is not there would send a compiler looking for
    // headers nobody installed, and `-I` with an empty argument is worse.
    if let Some(include) = erl_interface_include(ctx.erts_root) {
        env.push(("ERL_INTERFACE_INCLUDE_DIR", include.into_os_string()));
    }

    let output = crate::process::run_env_in_dir_with_timeout(
        Path::new(HOOK_SHELL),
        &[HOOK_SHELL_FLAG, &line],
        &env,
        Some(ctx.project_root),
        HOOK_TIMEOUT,
    )
    .map_err(|source| NativeError::HookProcess {
        package: package.to_owned(),
        source,
    })?;
    if !output.success {
        return Err(NativeError::HookFailed {
            package: package.to_owned(),
            command: line,
            stderr: output.stderr,
        });
    }
    Ok(ctx.out_dir.to_path_buf())
}

/// The `erl_interface` include directory of a runtime root, when it has one.
///
/// `<root>/lib/erl_interface-<vsn>/include`, and the last version in name
/// order when a root holds two, which is the only rule that is not a guess: an
/// installation with two of them has two, and the newer one is the one every
/// other tool would find through `code:lib_dir/1`.
fn erl_interface_include(erts_root: &Path) -> Option<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(erts_root.join("lib"))
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(ERL_INTERFACE_PREFIX))
        })
        .map(|path| path.join("include"))
        .filter(|include| include.is_dir())
        .collect();
    found.sort();
    found.pop()
}

/// Copies every replacement over its staged path.
///
/// The staged file's own mode is kept: a NIF that arrives executable stays
/// executable, and the sizes the listing carries are re-stated by the refresh
/// the build performs afterwards.
///
/// # Errors
///
/// [`NativeError::StagedMissing`] when the tree holds no such file, and
/// [`NativeError::Io`] when the copy fails.
pub fn apply(replacements: &[Replacement], staged_root: &Path) -> Result<(), NativeError> {
    for replacement in replacements {
        let relative = format!("{STAGED_LIB_DIR}/{}", replacement.artifact_rel_path);
        let staged = under(staged_root, &relative);
        let missing = || NativeError::StagedMissing {
            path: relative.clone(),
            package: replacement
                .artifact_rel_path
                .split('/')
                .next()
                .unwrap_or(&replacement.artifact_rel_path)
                .to_owned(),
        };
        let staged = staged.ok_or_else(missing)?;
        if !staged.is_file() {
            return Err(missing());
        }
        let source = replacement.source.path();
        let bytes = std::fs::read(source).map_err(io_at(source))?;
        // Written into the existing file rather than copied over it, so the
        // mode assembly gave it survives: `fs::copy` takes the source's
        // permissions with it, and a NIF that arrived executable would stop
        // being one because the vendored replacement was `chmod 644`.
        std::fs::write(&staged, &bytes).map_err(io_at(&staged))?;
    }
    Ok(())
}

/// What a build for `target` would decide about each artifact, in order.
///
/// The same decision [`reconcile`] makes, without running a hook or reading an
/// override's bytes, so that `ginary doctor` can print a column per configured
/// target on a machine that has built nothing.
///
/// [`Verdict::StaticRuntime`] outranks the rest, which is the one place this
/// differs from the order [`reconcile`] applies: a shared object is the
/// finding whatever else was configured for it, because no override and no
/// hook can give a static emulator a dynamic loader.
pub fn verdicts_for_target(
    artifacts: &[NativeArtifact],
    target: &Target,
    erts_nif_loading: bool,
    cfg: &TargetNativeCfg<'_>,
) -> Vec<Verdict> {
    artifacts
        .iter()
        .map(|artifact| {
            if !erts_nif_loading && artifact.kind == NativeKind::SharedObject {
                return Verdict::StaticRuntime;
            }
            if cfg.overrides.contains_key(&artifact.rel_path) {
                return Verdict::Override;
            }
            if cfg.hooks.contains_key(&artifact.package) {
                return Verdict::Hook;
            }
            match artifact.object.as_ref().map(|facts| fit(facts, target)) {
                Some(Fit::Match | Fit::StaticMatch) => Verdict::Ok,
                Some(Fit::Wrong) | None => Verdict::Mismatch,
            }
        })
        .collect()
}
