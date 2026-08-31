// SPDX-License-Identifier: MIT OR Apache-2.0
//! `[tools.ginary]` in `gleam.toml`, and the CLI flags merged over it.
//!
//! A build takes its settings from three places, and the precedence between
//! them is fixed:
//!
//! 1. a flag on the command line, which wins whenever it is present;
//! 2. the `[tools.ginary]` table of the project's `gleam.toml`;
//! 3. the defaults in this module.
//!
//! Nothing here reads the environment and nothing here touches the network.
//! [`ProjectConfig::read`] opens one file; every other function is pure, so
//! the rules are unit-testable against text rather than against a tree.
//!
//! The table is read with `deny_unknown_fields`, which is the whole point of
//! reading it with serde at all: a key ginary does not know is a setting the
//! user believes is in force, and accepting it silently would be worse than
//! refusing the build.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::strip::StripOptions;

/// Where an artifact is written when `[tools.ginary] output` says nothing.
///
/// A directory relative to the project root; the artifact lands at
/// `<project>/build/ginary/<app>`.
pub const DEFAULT_OUTPUT: &str = "build/ginary";

/// The zstd level the payload is packed at by default.
pub const DEFAULT_COMPRESSION_LEVEL: i32 = 19;

/// The lowest zstd level `compression_level` may name.
pub const MIN_COMPRESSION_LEVEL: i32 = 1;

/// The highest zstd level `compression_level` may name.
pub const MAX_COMPRESSION_LEVEL: i32 = 22;

/// The emulator flags `[tools.ginary] erl_flags` may not repeat.
///
/// Every one of them is an argument the launcher builds itself, from the
/// manifest, at run time. A second copy in `erl_flags` either contradicts the
/// one ginary passes or silently changes what the artifact does, so the build
/// refuses it and says which one the launcher owns. See [`erl_flag_reason`].
pub const REJECTED_ERL_FLAGS: [&str; 5] = ["-boot", "-extra", "-noshell", "-pa", "-pz"];

/// Why a name in `erts_extra_bins` or `--extra-bin` may be refused.
///
/// The name is joined onto the runtime's `erts-<vsn>/bin` to find the program
/// and onto the staged one to write it, so a name that is a path reads and
/// writes outside both trees. It is the same rule the closure applies to an
/// application name, for the same reason: the value is interpolated into a
/// path.
pub const EXTRA_BIN_REASON: &str =
    "a program name is a file name in the runtime's bin directory, not a path";

/// The `[tools.ginary]` table of a `gleam.toml`.
///
/// Every field is optional and a missing table is [`ToolsConfig::default`],
/// so a project that has never heard of ginary builds with the defaults. The
/// accessors below are what a caller should read: they apply the defaults and
/// the `strip` / `strip_elf` / `strip_beams` precedence in one place.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ToolsConfig {
    /// The directory the artifact is written into, relative to the project.
    pub output: Option<String>,
    /// Whether to strip at all. Defaults to `true`.
    pub strip: Option<bool>,
    /// Whether to strip the native binaries, overriding [`ToolsConfig::strip`].
    pub strip_elf: Option<bool>,
    /// Whether to strip the `.beam` modules, overriding [`ToolsConfig::strip`].
    pub strip_beams: Option<bool>,
    /// The zstd level, between [`MIN_COMPRESSION_LEVEL`] and
    /// [`MAX_COMPRESSION_LEVEL`].
    pub compression_level: Option<i32>,
    /// Extra closure seeds: applications that are bundled and not started.
    pub otp_applications: Vec<String>,
    /// Programs to stage from the runtime's `bin` beyond the required four.
    pub erts_extra_bins: Vec<String>,
    /// Emulator flags the launcher passes before `-eval`.
    pub erl_flags: Vec<String>,
}

impl ToolsConfig {
    /// The output directory, or [`DEFAULT_OUTPUT`].
    pub fn output(&self) -> &str {
        match &self.output {
            Some(output) => output,
            None => DEFAULT_OUTPUT,
        }
    }

    /// Whether the native binaries are stripped.
    ///
    /// `strip_elf` wins over `strip`, which defaults to `true`.
    pub fn strip_elf(&self) -> bool {
        self.strip_elf.or(self.strip).unwrap_or(true)
    }

    /// Whether the `.beam` modules are stripped.
    ///
    /// `strip_beams` wins over `strip`, which defaults to `true`.
    pub fn strip_beams(&self) -> bool {
        self.strip_beams.or(self.strip).unwrap_or(true)
    }

    /// The two booleans [`crate::strip::strip`] takes.
    pub fn strip_options(&self) -> StripOptions {
        StripOptions {
            elf: self.strip_elf(),
            beams: self.strip_beams(),
        }
    }

    /// The zstd level, or [`DEFAULT_COMPRESSION_LEVEL`].
    pub fn compression_level(&self) -> i32 {
        self.compression_level.unwrap_or(DEFAULT_COMPRESSION_LEVEL)
    }

    /// Checks the rules serde cannot express.
    ///
    /// `path` names the file in every message, because a lint a user cannot
    /// locate is a lint they cannot act on.
    ///
    /// # Errors
    ///
    /// [`ConfigError::CompressionLevel`] when the level is outside
    /// [`MIN_COMPRESSION_LEVEL`]`..=`[`MAX_COMPRESSION_LEVEL`],
    /// [`ConfigError::ExtraBin`] for the first `erts_extra_bins` entry that is
    /// not a program name, and [`ConfigError::ErlFlag`] for the first flag of
    /// [`REJECTED_ERL_FLAGS`] that `erl_flags` holds.
    pub fn validate(&self, path: &Path) -> Result<(), ConfigError> {
        let level = self.compression_level();
        if !(MIN_COMPRESSION_LEVEL..=MAX_COMPRESSION_LEVEL).contains(&level) {
            return Err(ConfigError::CompressionLevel {
                path: path.to_path_buf(),
                level,
            });
        }
        for name in &self.erts_extra_bins {
            if !crate::assemble::is_erts_bin_name(name) {
                return Err(ConfigError::ExtraBin {
                    path: path.to_path_buf(),
                    name: name.clone(),
                });
            }
        }
        for flag in &self.erl_flags {
            if let Some(reason) = erl_flag_reason(flag) {
                return Err(ConfigError::ErlFlag {
                    path: path.to_path_buf(),
                    flag: flag.clone(),
                    reason,
                });
            }
        }
        Ok(())
    }
}

/// What a project's `gleam.toml` says, as far as ginary reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectConfig {
    /// The project name, which is also the application name and the artifact's
    /// file name.
    pub name: String,
    /// The project version, when the manifest declares one.
    pub version: Option<String>,
    /// The `[tools.ginary]` table, or its defaults when there is none.
    pub tools: ToolsConfig,
}

impl ProjectConfig {
    /// Reads and validates the `gleam.toml` at `path`.
    ///
    /// # Errors
    ///
    /// [`ConfigError::Read`] when the file cannot be opened, and whatever
    /// [`ProjectConfig::from_toml`] reports about its contents.
    pub fn read(path: &Path) -> Result<Self, ConfigError> {
        let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        Self::from_toml(&text, path)
    }

    /// Parses and validates one `gleam.toml`, named by `path` in its errors.
    ///
    /// # Errors
    ///
    /// [`ConfigError::Parse`] for a syntax error or a key `[tools.ginary]`
    /// does not have — the message names the key and the file — and
    /// [`ConfigError::MissingName`], [`ConfigError::InvalidName`],
    /// [`ConfigError::CompressionLevel`], [`ConfigError::ExtraBin`] or
    /// [`ConfigError::ErlFlag`] for the five rules serde cannot state.
    pub fn from_toml(text: &str, path: &Path) -> Result<Self, ConfigError> {
        let raw: RawManifest = toml::from_str(text).map_err(|error| ConfigError::Parse {
            path: path.to_path_buf(),
            message: error.to_string().trim_end().to_owned(),
        })?;

        let name = raw.name.ok_or_else(|| ConfigError::MissingName {
            path: path.to_path_buf(),
        })?;
        if !is_gleam_name(&name) {
            return Err(ConfigError::InvalidName {
                path: path.to_path_buf(),
                name,
            });
        }

        let tools = raw.tools.ginary.unwrap_or_default();
        tools.validate(path)?;

        Ok(Self {
            name,
            version: raw.version,
            tools,
        })
    }
}

/// Whether `name` is a Gleam project name.
///
/// Gleam names are a lower-case letter followed by lower-case letters, digits
/// and underscores. The rule matters here rather than in Gleam because the
/// name is interpolated into a path, into the manifest's `app`, and into the
/// `-eval` expression the launcher passes to the emulator.
pub fn is_gleam_name(name: &str) -> bool {
    let mut characters = name.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    if !first.is_ascii_lowercase() {
        return false;
    }
    characters.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// Why a flag may not appear in `erl_flags`, or [`None`] if it may.
///
/// The reason is the actionable half of the message: a user who is told that
/// `-pa` is refused still has to be told who sets it instead.
pub fn erl_flag_reason(flag: &str) -> Option<&'static str> {
    match flag {
        "-boot" => Some("ginary boots the runtime from the bundled bin/no_dot_erlang"),
        "-extra" => Some("everything after -extra is the packaged application's own arguments"),
        "-noshell" => Some("ginary always starts the runtime with -noshell"),
        "-pa" | "-pz" => Some("the code path is built from the applications ginary bundles"),
        _ => None,
    }
}

/// Where the artifact is written.
///
/// `flag` is `--out` and wins when it is present. A `--out` that names an
/// existing directory, or that ends in a path separator, is a directory and
/// the application name is appended to it; anything else is the artifact's own
/// path. Without `--out` the answer is `<root>/<configured>/<app>`, because
/// `[tools.ginary] output` is always a directory.
pub fn resolve_output(root: &Path, configured: &str, flag: Option<&Path>, app: &str) -> PathBuf {
    match flag {
        Some(out) if names_a_directory(out) => out.join(app),
        Some(out) => out.to_path_buf(),
        None => root.join(configured).join(app),
    }
}

/// Whether a `--out` value names a directory rather than the artifact itself.
///
/// A directory that is already there answers for itself. One that is not can
/// only say so with a trailing separator, and it has to be able to:
/// `ginary build --out out/` must write `out/<app>` rather than a *file*
/// called `out`.
fn names_a_directory(path: &Path) -> bool {
    if path.is_dir() {
        return true;
    }
    path.as_os_str()
        .as_encoded_bytes()
        .last()
        .copied()
        .is_some_and(|byte| byte == b'/' || (cfg!(windows) && byte == b'\\'))
}

/// The `ginary build` flags, before the project configuration is read.
///
/// One field per flag, in the order `ginary build --help` lists them. The
/// values are what clap parsed and nothing more: the merge, the defaults and
/// the validation all happen in [`BuildOptions::merge`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BuildFlags {
    /// Where to start looking for `gleam.toml`. The working directory when the
    /// caller does not say otherwise.
    pub start: PathBuf,
    /// `--out`: a directory to write the artifact into, or its whole path.
    pub out: Option<PathBuf>,
    /// `--no-strip`.
    pub no_strip: bool,
    /// `--strip-elf-only`.
    pub strip_elf_only: bool,
    /// `--strip-beams-only`.
    pub strip_beams_only: bool,
    /// `--otp-root`.
    pub otp_root: Option<PathBuf>,
    /// `--skip-export`.
    pub skip_export: bool,
    /// `--keep-staging`.
    pub keep_staging: bool,
    /// `--compression-level`.
    pub compression_level: Option<i32>,
    /// `--extra-otp-app`, repeatable.
    pub extra_otp_apps: Vec<String>,
    /// `--extra-bin`, repeatable.
    pub extra_bins: Vec<String>,
    /// `--explain`.
    pub explain: bool,
    /// `-v`, counted, so that `-vv` stays open.
    pub verbose: u8,
}

/// Everything one build needs, with the flags merged over the configuration.
///
/// Produced by [`BuildOptions::merge`] and consumed by
/// [`crate::bundle::build`]. Every field is resolved: there is no `Option`
/// left for a later stage to default, because a default applied twice is a
/// default that can disagree with itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildOptions {
    /// The project root, the directory holding `gleam.toml`.
    pub root: PathBuf,
    /// The application name, which is the project name.
    pub app: String,
    /// The project version, when the manifest declares one.
    pub app_version: Option<String>,
    /// The artifact's path, directory and file name together.
    pub out: PathBuf,
    /// What to strip.
    pub strip: StripOptions,
    /// `--otp-root`, or [`None`] for the installation `erl` reports.
    pub otp_root: Option<PathBuf>,
    /// Whether to reuse the existing export instead of running `gleam`.
    pub skip_export: bool,
    /// Whether to keep the staging work directory and print its path.
    pub keep_staging: bool,
    /// The zstd level.
    pub compression_level: i32,
    /// Extra closure seeds: bundled, not started.
    pub otp_applications: Vec<String>,
    /// Extra programs to stage from the runtime's `bin`.
    pub erts_extra_bins: Vec<String>,
    /// Emulator flags the launcher passes before `-eval`.
    pub erl_flags: Vec<String>,
    /// Whether to print the closure and staging accounts before the report.
    pub explain: bool,
    /// How much the build says about itself on standard error.
    pub verbose: u8,
}

impl BuildOptions {
    /// Merges `flags` over `config`, resolving every default.
    ///
    /// The precedence is flags, then `[tools.ginary]`, then the constants in
    /// this module. Two settings merge rather than replace, because both are
    /// lists a project and a command line add to independently:
    /// `--extra-otp-app` is appended to `otp_applications` and `--extra-bin`
    /// to `erts_extra_bins`, each deduplicated and left in the order the
    /// configuration named them, with the flags after.
    ///
    /// # Errors
    ///
    /// Whatever [`ToolsConfig::validate`] reports about the table, and
    /// [`ConfigError::ExtraBinFlag`] when a `--extra-bin` is not a program
    /// name. The flag half of the range check belongs to clap, which refuses
    /// `--compression-level 99` as a usage error before anything is read.
    pub fn merge(
        root: &Path,
        config: &ProjectConfig,
        flags: &BuildFlags,
    ) -> Result<Self, ConfigError> {
        let manifest = root.join(crate::gleam::MANIFEST_NAME);
        config.tools.validate(&manifest)?;

        // The table's half is `validate`'s; this is the flag's, which no file
        // names and which would otherwise reach `assemble::stage` unchecked.
        for name in &flags.extra_bins {
            if !crate::assemble::is_erts_bin_name(name) {
                return Err(ConfigError::ExtraBinFlag { name: name.clone() });
            }
        }

        Ok(Self {
            root: root.to_path_buf(),
            app: config.name.clone(),
            app_version: config.version.clone(),
            out: resolve_output(
                root,
                config.tools.output(),
                flags.out.as_deref(),
                &config.name,
            ),
            strip: merge_strip(&config.tools, flags),
            otp_root: flags.otp_root.clone(),
            skip_export: flags.skip_export,
            keep_staging: flags.keep_staging,
            compression_level: flags
                .compression_level
                .unwrap_or_else(|| config.tools.compression_level()),
            otp_applications: append_unique(&config.tools.otp_applications, &flags.extra_otp_apps),
            erts_extra_bins: append_unique(&config.tools.erts_extra_bins, &flags.extra_bins),
            erl_flags: config.tools.erl_flags.clone(),
            explain: flags.explain,
            verbose: flags.verbose,
        })
    }
}

/// What to strip, with the four flags applied over the table.
///
/// A flag is what the user typed just now, so it decides both halves: even
/// `--strip-beams-only` against a table that says `strip_beams = false` turns
/// the beam half on. Without a flag the table answers, through
/// [`ToolsConfig::strip_options`].
fn merge_strip(tools: &ToolsConfig, flags: &BuildFlags) -> StripOptions {
    if flags.no_strip {
        return StripOptions {
            elf: false,
            beams: false,
        };
    }
    if flags.strip_elf_only {
        return StripOptions {
            elf: true,
            beams: false,
        };
    }
    if flags.strip_beams_only {
        return StripOptions {
            elf: false,
            beams: true,
        };
    }
    tools.strip_options()
}

/// `base` followed by whatever of `extra` is not already in it.
///
/// The table's order comes first because it is the project's stated intent;
/// the flags are an addition to it, not a replacement, and a name asked for
/// twice is bundled once.
fn append_unique(base: &[String], extra: &[String]) -> Vec<String> {
    let mut merged: Vec<String> = Vec::with_capacity(base.len() + extra.len());
    for name in base.iter().chain(extra) {
        if !merged.iter().any(|existing| existing == name) {
            merged.push(name.clone());
        }
    }
    merged
}

/// The keys of a `gleam.toml` that ginary reads.
///
/// Deliberately *not* `deny_unknown_fields`: a `gleam.toml` holds `target`,
/// `dependencies`, `[erlang]` and whatever else Gleam or another tool put
/// there, and refusing those would make ginary refuse every real project. The
/// strictness belongs one level down, on [`ToolsConfig`], which is the table
/// ginary owns.
#[derive(Debug, Default, Deserialize)]
struct RawManifest {
    /// The project name; required, but reported by
    /// [`ConfigError::MissingName`] rather than by serde, so the message names
    /// the file.
    name: Option<String>,
    /// The project version.
    version: Option<String>,
    /// The `[tools]` table, whose other members belong to other tools.
    #[serde(default)]
    tools: RawTools,
}

/// The `[tools]` table, of which ginary reads exactly one member.
#[derive(Debug, Default, Deserialize)]
struct RawTools {
    /// `[tools.ginary]`.
    ginary: Option<ToolsConfig>,
}

/// Why a project's configuration is not usable.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// The manifest could not be opened.
    #[error("cannot read {path}")]
    Read {
        /// The file that could not be read.
        path: PathBuf,
        /// What the operating system said.
        #[source]
        source: std::io::Error,
    },
    /// The manifest is not the TOML ginary expects.
    ///
    /// This is where an unknown `[tools.ginary]` key arrives: serde names the
    /// key and this variant names the file.
    #[error("{path}: {message}")]
    Parse {
        /// The file the error is in.
        path: PathBuf,
        /// What serde said, without its trailing newline.
        message: String,
    },
    /// The manifest declares no project name.
    #[error("{path}: the project declares no `name`, which every Gleam project must")]
    MissingName {
        /// The file that has no name in it.
        path: PathBuf,
    },
    /// The project name is not a Gleam name.
    #[error(
        "{path}: `{name}` is not a Gleam project name: a name is a lower-case letter followed \
         by lower-case letters, digits and underscores"
    )]
    InvalidName {
        /// The file the name is in.
        path: PathBuf,
        /// The name that was refused.
        name: String,
    },
    /// The zstd level is outside the range zstd accepts.
    #[error(
        "{path}: [tools.ginary] compression_level must be between {MIN_COMPRESSION_LEVEL} and \
         {MAX_COMPRESSION_LEVEL}, not {level}"
    )]
    CompressionLevel {
        /// The file the level is in.
        path: PathBuf,
        /// The level that was refused.
        level: i32,
    },
    /// `erts_extra_bins` holds something that is not a program name.
    #[error("{path}: [tools.ginary] erts_extra_bins may not hold `{name}`: {EXTRA_BIN_REASON}")]
    ExtraBin {
        /// The file the name is in.
        path: PathBuf,
        /// The name that was refused.
        name: String,
    },
    /// `--extra-bin` named something that is not a program name.
    #[error("--extra-bin `{name}` is not usable: {EXTRA_BIN_REASON}")]
    ExtraBinFlag {
        /// The name that was refused.
        name: String,
    },
    /// `erl_flags` holds a flag the launcher passes itself.
    #[error("{path}: [tools.ginary] erl_flags may not hold `{flag}`: {reason}")]
    ErlFlag {
        /// The file the flag is in.
        path: PathBuf,
        /// The flag that was refused.
        flag: String,
        /// Who sets it instead; see [`erl_flag_reason`].
        reason: &'static str,
    },
}
