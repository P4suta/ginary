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
