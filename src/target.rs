// SPDX-License-Identifier: MIT OR Apache-2.0
//! Build and run targets that `ginary` knows how to package for.
//!
//! A [`Target`] is the triple `(os, arch, libc)` that identifies both the BEAM
//! runtime that gets bundled and the stub executable that carries it. The
//! canonical spelling is [`Target::name`] (for example `linux-x86_64-gnu`); it
//! is the string that appears in manifests, artifact file names and CLI flags.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Operating system half of a [`Target`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Os {
    /// Linux, packaged against either glibc or musl.
    Linux,
    /// Apple macOS.
    Macos,
    /// Microsoft Windows.
    Windows,
}

impl Os {
    /// Returns the canonical lowercase spelling used in target names.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Linux => "linux",
            Self::Macos => "macos",
            Self::Windows => "windows",
        }
    }
}

impl fmt::Display for Os {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// CPU architecture half of a [`Target`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Arch {
    /// 64-bit x86.
    X86_64,
    /// 64-bit ARM.
    Aarch64,
}

impl Arch {
    /// Returns the canonical lowercase spelling used in target names.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::X86_64 => "x86_64",
            Self::Aarch64 => "aarch64",
        }
    }
}

impl fmt::Display for Arch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// C runtime a Linux target links against.
///
/// macOS and Windows have exactly one system C runtime, so their targets carry
/// [`Libc::None`] and their names have no libc suffix.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Libc {
    /// GNU libc.
    Gnu,
    /// musl libc.
    Musl,
    /// The platform has a single system C runtime; no suffix is used.
    None,
}

impl Libc {
    /// Returns the name suffix for this C runtime, including the leading `-`.
    pub const fn suffix(self) -> &'static str {
        match self {
            Self::Gnu => "-gnu",
            Self::Musl => "-musl",
            Self::None => "",
        }
    }
}

/// Whether a runtime resolves its C library at load time or carries it.
///
/// The distinction is not cosmetic: a fully static runtime cannot `dlopen`
/// anything, so a NIF under `priv/lib` will never load into one. It is derived
/// from the ELF itself — a `PT_INTERP` and a non-empty `DT_NEEDED` set — and
/// never from what a tarball or a catalogue entry claimed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Linkage {
    /// The runtime is linked against a C library it loads at start-up.
    Dynamic,
    /// The runtime carries its C library and loads nothing.
    Static,
}

impl Linkage {
    /// Returns the canonical lowercase spelling used in manifests and reports.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Dynamic => "dynamic",
            Self::Static => "static",
        }
    }

    /// Whether a runtime with this linkage can load a NIF.
    ///
    /// A statically linked emulator has no dynamic loader to call, which is
    /// what makes `nif_loading` a property of the runtime rather than of the
    /// application.
    pub const fn loads_nifs(self) -> bool {
        matches!(self, Self::Dynamic)
    }
}

impl fmt::Display for Linkage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The substring of a `PT_INTERP` that names musl's dynamic loader.
pub const MUSL_INTERPRETER: &str = "ld-musl";

/// The substring of a `PT_INTERP` that names glibc's dynamic loader.
pub const GNU_INTERPRETER: &str = "ld-linux";

/// What an ELF header and its interpreter say the binary is for.
///
/// Two answers rather than one, because a static Linux binary does not name a
/// C library at all: the machine is known and the libc is not, so the caller —
/// [`crate::erts_source`] and `ginary verify` — decides what to report it as.
/// A static runtime built for musl and one built against glibc are the same
/// bytes as far as the header is concerned, so pretending to know would be a
/// guess written into a manifest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ElfTarget {
    /// The interpreter named the C library, so the target is complete.
    Dynamic(Target),
    /// A Linux binary with no interpreter: the architecture, and no libc.
    StaticLinux(Arch),
}

impl ElfTarget {
    /// How the binary is linked.
    pub const fn linkage(self) -> Linkage {
        match self {
            Self::Dynamic(_) => Linkage::Dynamic,
            Self::StaticLinux(_) => Linkage::Static,
        }
    }

    /// The architecture, which both answers carry.
    pub const fn arch(self) -> Arch {
        match self {
            Self::Dynamic(target) => target.arch,
            Self::StaticLinux(arch) => arch,
        }
    }

    /// The target when the binary named one, and [`None`] when it did not.
    pub const fn target(self) -> Option<Target> {
        match self {
            Self::Dynamic(target) => Some(target),
            Self::StaticLinux(_) => None,
        }
    }

    /// The target, reporting a static binary as `assumed`.
    ///
    /// `assumed` is the libc of the build being made: a static runtime is
    /// packaged for whichever Linux target asked for it, and the linkage
    /// beside it is what says the answer was not read off the file.
    pub const fn resolve(self, assumed: Libc) -> Target {
        match self {
            Self::Dynamic(target) => target,
            Self::StaticLinux(arch) => Target::new(Os::Linux, arch, assumed),
        }
    }
}

/// A packaging target: operating system, architecture and C runtime.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub struct Target {
    /// Operating system.
    pub os: Os,
    /// CPU architecture.
    pub arch: Arch,
    /// C runtime, or [`Libc::None`] on platforms that have only one.
    pub libc: Libc,
}

/// Every target `ginary` intends to support in v1.
pub const ALL: [Target; 7] = [
    Target::new(Os::Linux, Arch::X86_64, Libc::Gnu),
    Target::new(Os::Linux, Arch::X86_64, Libc::Musl),
    Target::new(Os::Linux, Arch::Aarch64, Libc::Gnu),
    Target::new(Os::Linux, Arch::Aarch64, Libc::Musl),
    Target::new(Os::Macos, Arch::X86_64, Libc::None),
    Target::new(Os::Macos, Arch::Aarch64, Libc::None),
    Target::new(Os::Windows, Arch::X86_64, Libc::None),
];

impl Target {
    /// Builds a target without checking that the combination is supported.
    ///
    /// Prefer [`Target::from_str`] for anything that comes from a user.
    pub const fn new(os: Os, arch: Arch, libc: Libc) -> Self {
        Self { os, arch, libc }
    }

    /// Returns the target `ginary` itself was compiled for.
    ///
    /// ```
    /// # use ginary::target::Target;
    /// assert!(ginary::target::ALL.contains(&Target::host()));
    /// ```
    pub const fn host() -> Self {
        Self::new(HOST_OS, HOST_ARCH, HOST_LIBC)
    }

    /// Returns the canonical name, for example `linux-x86_64-gnu` or
    /// `macos-aarch64`.
    pub fn name(self) -> String {
        format!("{}-{}{}", self.os, self.arch, self.libc.suffix())
    }

    /// Returns the Rust target triple used to cross-compile a stub.
    ///
    /// The function is total because the match must be exhaustive, but only the
    /// members of [`ALL`] are reachable through [`Target::from_str`]; the arms
    /// marked below exist for the compiler, not for a caller.
    pub const fn rust_triple(self) -> &'static str {
        match (self.os, self.arch, self.libc) {
            (Os::Linux, Arch::X86_64, Libc::Musl) => "x86_64-unknown-linux-musl",
            (Os::Linux, Arch::X86_64, _) => "x86_64-unknown-linux-gnu",
            (Os::Linux, Arch::Aarch64, Libc::Musl) => "aarch64-unknown-linux-musl",
            (Os::Linux, Arch::Aarch64, _) => "aarch64-unknown-linux-gnu",
            (Os::Macos, Arch::X86_64, _) => "x86_64-apple-darwin",
            (Os::Macos, Arch::Aarch64, _) => "aarch64-apple-darwin",
            (Os::Windows, Arch::X86_64, _) => "x86_64-pc-windows-gnu",
            // Not in `ALL`, rejected by `FromStr`, and no runtime is published
            // for it. Kept only so the match is exhaustive.
            (Os::Windows, Arch::Aarch64, _) => "aarch64-pc-windows-gnu",
        }
    }

    /// Returns the executable file-name suffix for this target.
    pub const fn exe_suffix(self) -> &'static str {
        match self.os {
            Os::Windows => ".exe",
            Os::Linux | Os::Macos => "",
        }
    }

    /// Returns the `docker --platform` value for this target.
    ///
    /// [`None`] for macOS and Windows: no Linux container runs either, so a
    /// caller that has to say "there is no image for this target" gets an
    /// answer rather than a string that would fail at `docker create`.
    pub const fn docker_platform(self) -> Option<&'static str> {
        match (self.os, self.arch) {
            (Os::Linux, Arch::X86_64) => Some("linux/amd64"),
            (Os::Linux, Arch::Aarch64) => Some("linux/arm64"),
            (Os::Macos | Os::Windows, _) => None,
        }
    }

    /// Reads a target out of an ELF machine name and its `PT_INTERP`.
    ///
    /// `machine` is spelled the way [`crate::elf::ElfInfo::machine`] spells it
    /// and `interp` is the program interpreter, absent for a static binary.
    /// The interpreter is what names the C library — `ld-musl-*.so.1` against
    /// `ld-linux-*.so.2` — and its absence is [`ElfTarget::StaticLinux`]
    /// rather than a guess.
    ///
    /// Returns [`None`] for a machine ginary has no target for and for an
    /// interpreter that names neither loader, because an answer that made one
    /// up would be recorded in a manifest as though it had been read.
    pub fn from_elf(machine: &str, interp: Option<&str>) -> Option<ElfTarget> {
        let arch = match machine {
            "x86_64" => Arch::X86_64,
            "aarch64" => Arch::Aarch64,
            _ => return None,
        };
        // No `PT_INTERP` at all is the one honest answer that is not a whole
        // target: the machine was read and the C library was not written down
        // anywhere in the file.
        let Some(interp) = interp else {
            return Some(ElfTarget::StaticLinux(arch));
        };
        // musl first: its loader is `ld-musl-<arch>.so.1` and glibc's is
        // `ld-linux-<arch>.so.2`, so neither substring is inside the other and
        // the order is documentation rather than precedence.
        let libc = if interp.contains(MUSL_INTERPRETER) {
            Libc::Musl
        } else if interp.contains(GNU_INTERPRETER) {
            Libc::Gnu
        } else {
            return None;
        };
        Some(ElfTarget::Dynamic(Self::new(Os::Linux, arch, libc)))
    }
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}-{}{}", self.os, self.arch, self.libc.suffix())
    }
}

impl From<Target> for String {
    fn from(target: Target) -> Self {
        target.name()
    }
}

/// Failure to parse a [`Target`] from its canonical name.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ParseTargetError {
    /// The name did not have the `<os>-<arch>[-<libc>]` shape.
    #[error("`{0}` is not a target name; expected `<os>-<arch>[-<libc>]`")]
    Shape(String),
    /// The name was well shaped but names a combination ginary does not support.
    #[error("`{0}` is not a supported target; run `ginary version --json` to see the host target")]
    Unsupported(String),
}

impl FromStr for Target {
    type Err = ParseTargetError;

    /// Parses a canonical target name.
    ///
    /// The pseudo-names `host` and `all` are resolved by the caller, not here,
    /// because they expand to a different number of targets.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parts = s.split('-');
        let (Some(os), Some(arch)) = (parts.next(), parts.next()) else {
            return Err(ParseTargetError::Shape(s.to_owned()));
        };
        let libc = parts.next();
        if parts.next().is_some() {
            return Err(ParseTargetError::Shape(s.to_owned()));
        }

        let os = match os {
            "linux" => Os::Linux,
            "macos" => Os::Macos,
            "windows" => Os::Windows,
            _ => return Err(ParseTargetError::Unsupported(s.to_owned())),
        };
        let arch = match arch {
            "x86_64" => Arch::X86_64,
            "aarch64" => Arch::Aarch64,
            _ => return Err(ParseTargetError::Unsupported(s.to_owned())),
        };
        let libc = match libc {
            Some("gnu") => Libc::Gnu,
            Some("musl") => Libc::Musl,
            None => Libc::None,
            Some(_) => return Err(ParseTargetError::Unsupported(s.to_owned())),
        };

        let target = Self::new(os, arch, libc);
        if ALL.contains(&target) {
            Ok(target)
        } else {
            Err(ParseTargetError::Unsupported(s.to_owned()))
        }
    }
}

impl TryFrom<String> for Target {
    type Error = ParseTargetError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

/// The two spellings that name a set of targets rather than one target.
pub const PSEUDO_TARGETS: [&str; 2] = [HOST_SELECTION, "all"];

/// The selection that means "whatever machine this build runs on".
///
/// Not a target name: it is the selection a build that names nothing already
/// makes, which is why [`names_a_target`] does not count it.
pub const HOST_SELECTION: &str = "host";

/// Every spelling [`resolve_targets`] accepts, in the order it lists them.
///
/// The two pseudo-names first, then [`ALL`] in its own order, so the sentence
/// an error prints is the same on every machine.
pub fn target_spellings() -> Vec<String> {
    let mut names: Vec<String> = PSEUDO_TARGETS
        .iter()
        .map(|name| (*name).to_owned())
        .collect();
    names.extend(ALL.iter().map(|target| target.name()));
    names
}

/// The spellings as one comma-separated list, for an error message.
fn spelling_list() -> String {
    target_spellings()
        .iter()
        .map(|name| format!("`{name}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Why a list of target selections could not be resolved.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TargetError {
    /// A selection is neither a pseudo-name nor a target name.
    #[error("`{name}` is not a target; expected one of {list}", list = spelling_list())]
    Unknown {
        /// The selection that was refused.
        name: String,
    },
}

/// Resolves the targets one build produces.
///
/// The precedence is the flags, then the `[tools.ginary] targets` list, then
/// the host on its own — a project that has never named a target builds for
/// the machine it is on. Each entry is `host`, `all` or a canonical target
/// name; `all` expands to [`ALL`], and a target named twice is built once,
/// with the first mention deciding its place in the order.
///
/// # Errors
///
/// [`TargetError::Unknown`] naming the first selection that is neither a
/// pseudo-name nor a target, and listing every spelling that is.
pub fn resolve_targets(
    flags: &[String],
    config_targets: &[String],
) -> Result<Vec<Target>, TargetError> {
    let selections = selections(flags, config_targets);
    if selections.is_empty() {
        return Ok(vec![Target::host()]);
    }

    let mut resolved: Vec<Target> = Vec::with_capacity(selections.len());
    for name in selections {
        match name.as_str() {
            HOST_SELECTION => push_unique(&mut resolved, Target::host()),
            "all" => {
                for target in ALL {
                    push_unique(&mut resolved, target);
                }
            }
            _ => {
                let target = name
                    .parse::<Target>()
                    .map_err(|_| TargetError::Unknown { name: name.clone() })?;
                push_unique(&mut resolved, target);
            }
        }
    }
    Ok(resolved)
}

/// Whether these selections name a target, or only ask for the host.
///
/// The precedence is [`resolve_targets`]': the flags when there are any, and
/// the project's list otherwise. `host` on its own is the selection a build
/// that named nothing already makes, so it names no target however it is
/// spelled — in `[tools.ginary] targets`, on the command line, or not at all.
/// Everything else — `all`, or a canonical name, the host's own included —
/// names one, and that is what puts the name in the artifact's file name; see
/// [`crate::config::BuildOptions::suffixed`].
pub fn names_a_target(flags: &[String], config_targets: &[String]) -> bool {
    selections(flags, config_targets)
        .iter()
        .any(|name| name != HOST_SELECTION)
}

/// The selections one build resolves, flags first.
///
/// One function so that the precedence [`resolve_targets`] applies and the
/// one [`names_a_target`] reads are the same rule rather than two copies of
/// it.
fn selections<'a>(flags: &'a [String], config_targets: &'a [String]) -> &'a [String] {
    // The flags decide the list when there are any: `--target` is what the
    // user typed just now, so a project that names four targets does not build
    // five because one flag was passed.
    if flags.is_empty() {
        config_targets
    } else {
        flags
    }
}

/// Appends `target` unless it is already in `list`.
///
/// The first mention decides the place, so `all` beside a name it already
/// holds is still [`ALL`] in [`ALL`]'s order, and `host` beside the host's own
/// canonical name is one target.
fn push_unique(list: &mut Vec<Target>, target: Target) {
    if !list.contains(&target) {
        list.push(target);
    }
}

#[cfg(target_os = "linux")]
const HOST_OS: Os = Os::Linux;
#[cfg(target_os = "macos")]
const HOST_OS: Os = Os::Macos;
#[cfg(target_os = "windows")]
const HOST_OS: Os = Os::Windows;
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
compile_error!("ginary supports linux, macos and windows hosts only");

#[cfg(target_arch = "x86_64")]
const HOST_ARCH: Arch = Arch::X86_64;
#[cfg(target_arch = "aarch64")]
const HOST_ARCH: Arch = Arch::Aarch64;
#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
compile_error!("ginary supports x86_64 and aarch64 hosts only");

#[cfg(all(target_os = "linux", target_env = "musl"))]
const HOST_LIBC: Libc = Libc::Musl;
#[cfg(all(target_os = "linux", not(target_env = "musl")))]
const HOST_LIBC: Libc = Libc::Gnu;
#[cfg(not(target_os = "linux"))]
const HOST_LIBC: Libc = Libc::None;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_names_carry_a_libc_suffix() {
        assert_eq!(
            Target::new(Os::Linux, Arch::X86_64, Libc::Gnu).name(),
            "linux-x86_64-gnu"
        );
        assert_eq!(
            Target::new(Os::Linux, Arch::Aarch64, Libc::Musl).name(),
            "linux-aarch64-musl"
        );
    }

    #[test]
    fn single_libc_platforms_have_no_suffix() {
        assert_eq!(
            Target::new(Os::Macos, Arch::Aarch64, Libc::None).name(),
            "macos-aarch64"
        );
        assert_eq!(
            Target::new(Os::Windows, Arch::X86_64, Libc::None).name(),
            "windows-x86_64"
        );
    }

    #[test]
    fn display_matches_name() {
        for target in ALL {
            assert_eq!(target.to_string(), target.name());
        }
    }

    #[test]
    fn every_supported_target_round_trips_through_its_name() {
        for target in ALL {
            let name = target.name();
            assert_eq!(name.parse::<Target>(), Ok(target), "round trip of {name}");
        }
    }

    #[test]
    fn all_contains_exactly_the_planned_seven_targets() {
        let mut names: Vec<String> = ALL.iter().map(|target| target.name()).collect();
        names.sort();
        assert_eq!(
            names,
            [
                "linux-aarch64-gnu",
                "linux-aarch64-musl",
                "linux-x86_64-gnu",
                "linux-x86_64-musl",
                "macos-aarch64",
                "macos-x86_64",
                "windows-x86_64",
            ]
        );
    }

    #[test]
    fn malformed_names_are_shape_errors() {
        for name in ["", "linux", "linux-x86_64-gnu-extra"] {
            assert_eq!(
                name.parse::<Target>(),
                Err(ParseTargetError::Shape(name.to_owned())),
                "{name} should be a shape error"
            );
        }
    }

    #[test]
    fn well_shaped_but_unknown_names_are_unsupported() {
        for name in ["freebsd-x86_64", "linux-riscv64-gnu", "linux-x86_64-uclibc"] {
            assert_eq!(
                name.parse::<Target>(),
                Err(ParseTargetError::Unsupported(name.to_owned())),
                "{name} should be unsupported"
            );
        }
    }

    #[test]
    fn combinations_outside_all_are_rejected() {
        // Well spelled, but ginary does not ship an aarch64 Windows runtime.
        assert_eq!(
            "windows-aarch64".parse::<Target>(),
            Err(ParseTargetError::Unsupported("windows-aarch64".to_owned()))
        );
        // Linux without a libc suffix is ambiguous and therefore rejected.
        assert_eq!(
            "linux-x86_64".parse::<Target>(),
            Err(ParseTargetError::Unsupported("linux-x86_64".to_owned()))
        );
    }

    #[test]
    fn the_pseudo_names_are_not_parsed_here() {
        assert!("host".parse::<Target>().is_err());
        assert!("all".parse::<Target>().is_err());
    }

    #[test]
    fn host_is_a_supported_target() {
        assert!(ALL.contains(&Target::host()));
    }

    #[test]
    fn only_windows_targets_have_an_exe_suffix() {
        for target in ALL {
            let expected = if target.os == Os::Windows { ".exe" } else { "" };
            assert_eq!(target.exe_suffix(), expected, "{}", target.name());
        }
    }

    #[test]
    fn rust_triples_are_distinct_per_target() {
        let mut triples: Vec<&str> = ALL.iter().map(|target| target.rust_triple()).collect();
        triples.sort_unstable();
        let count = triples.len();
        triples.dedup();
        assert_eq!(triples.len(), count, "rust triples must be unique");
    }

    #[test]
    fn rust_triples_match_the_documented_mapping() {
        let mut mapping: Vec<String> = ALL
            .iter()
            .map(|target| format!("{} => {}", target.name(), target.rust_triple()))
            .collect();
        mapping.sort();
        assert_eq!(
            mapping,
            [
                "linux-aarch64-gnu => aarch64-unknown-linux-gnu",
                "linux-aarch64-musl => aarch64-unknown-linux-musl",
                "linux-x86_64-gnu => x86_64-unknown-linux-gnu",
                "linux-x86_64-musl => x86_64-unknown-linux-musl",
                "macos-aarch64 => aarch64-apple-darwin",
                "macos-x86_64 => x86_64-apple-darwin",
                // The `-gnu` ABI, not `-msvc`: the stub is cross-compiled from
                // Linux, so `deny.toml` must list the same triple.
                "windows-x86_64 => x86_64-pc-windows-gnu",
            ]
        );
    }

    #[test]
    fn cargo_deny_checks_every_target_ginary_models() {
        let deny = include_str!("../deny.toml");
        let list = deny
            .split_once("targets = [")
            .expect("a [graph] targets list")
            .1
            .split_once(']')
            .expect("a closed targets list")
            .0;
        let mut listed: Vec<&str> = list
            .lines()
            .filter_map(|line| line.split('"').nth(1))
            .collect();
        listed.sort_unstable();

        let mut expected: Vec<&str> = ALL.iter().map(|target| target.rust_triple()).collect();
        expected.sort_unstable();

        assert_eq!(
            listed, expected,
            "deny.toml must list exactly the triples of `target::ALL`"
        );
    }

    #[test]
    fn serde_uses_the_canonical_name() {
        let target = Target::new(Os::Linux, Arch::X86_64, Libc::Musl);
        let json = serde_json::to_string(&target).expect("serializes");
        assert_eq!(json, "\"linux-x86_64-musl\"");
        assert_eq!(
            serde_json::from_str::<Target>(&json).expect("deserializes"),
            target
        );
    }
}
