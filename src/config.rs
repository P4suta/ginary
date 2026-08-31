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

use std::collections::BTreeMap;
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

/// The filename encoding an artifact uses when the project says nothing.
///
/// `+fnu` is what a Gleam application wants on every machine ginary targets:
/// the shipment's file names are UTF-8 and a runtime that read them as
/// Latin-1 would not find its own modules.
pub const DEFAULT_FILENAME_ENCODING: &str = "utf8";

/// The three values `filename_encoding` may take, in the order an error lists
/// them.
pub const FILENAME_ENCODINGS: [&str; 3] = ["utf8", "latin1", "auto"];

/// The flags an `-args_file` may not hold.
///
/// The first five are [`REJECTED_ERL_FLAGS`] plus `-noinput`, for the same
/// reason: the launcher passes them itself and a second copy either
/// contradicts it or silently changes what the artifact does. `-args_file`
/// itself is refused because `erl` follows the nesting and a file that
/// includes another is a file whose content ginary has not linted.
pub const REJECTED_ARGS_FILE_FLAGS: [&str; 7] = [
    "-args_file",
    "-boot",
    "-extra",
    "-noinput",
    "-noshell",
    "-pa",
    "-pz",
];

/// The prefix `[tools.ginary] env` may not name.
///
/// Every `ERL_*` variable is either one the launcher scrubs — see
/// [`crate::launch::REMOVED_VARS`] — or one it sets, and the launcher applies
/// `env` *after* the scrub. A default that could be reintroduced there would
/// be the artifact putting back the very variable the launcher removed to make
/// the run reproducible, so the name is refused at build time instead.
pub const REJECTED_ENV_PREFIX: &str = "ERL_";

/// The variables `[tools.ginary] env` may not name, sorted.
///
/// The four the launcher derives from the extracted root, plus `HOME`, which
/// it defaults only when the caller has not set one.
pub const REJECTED_ENV_NAMES: [&str; 5] = ["BINDIR", "EMU", "HOME", "PROGNAME", "ROOTDIR"];

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
    /// A project-relative `-args_file`, copied into the artifact.
    pub vm_args: Option<String>,
    /// A project-relative `sys.config`, copied into the artifact.
    pub sys_config: Option<String>,
    /// Whether the artifact ships `epmd` and starts the runtime distributed.
    pub distribution: bool,
    /// `utf8`, `latin1` or `auto`; see [`FILENAME_ENCODINGS`].
    pub filename_encoding: Option<String>,
    /// Variables the launcher sets, each only when the caller has not.
    pub env: BTreeMap<String, String>,
    /// Whether the artifact ships `heart` and starts the runtime under it.
    pub heart: bool,
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

    /// The configured `vm_args`, or [`None`].
    pub fn vm_args(&self) -> Option<&str> {
        self.vm_args.as_deref()
    }

    /// The configured `sys_config`, or [`None`].
    pub fn sys_config(&self) -> Option<&str> {
        self.sys_config.as_deref()
    }

    /// The filename encoding, or [`DEFAULT_FILENAME_ENCODING`].
    pub fn filename_encoding(&self) -> &str {
        match &self.filename_encoding {
            Some(encoding) => encoding,
            None => DEFAULT_FILENAME_ENCODING,
        }
    }

    /// The emulator flag [`ToolsConfig::filename_encoding`] maps to.
    ///
    /// [`None`] when the value is not one of [`FILENAME_ENCODINGS`], which
    /// [`ToolsConfig::validate`] refuses before a build can reach this.
    pub fn encoding_flag(&self) -> Option<&'static str> {
        filename_encoding_flag(self.filename_encoding())
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
    /// not a program name, [`ConfigError::ErlFlag`] for the first flag of
    /// [`REJECTED_ERL_FLAGS`] that `erl_flags` holds,
    /// [`ConfigError::FilenameEncoding`] when `filename_encoding` is not one of
    /// [`FILENAME_ENCODINGS`], and [`ConfigError::EnvName`] for the first `env`
    /// key the launcher owns.
    ///
    /// The two paths — `vm_args` and `sys_config` — are *not* checked here.
    /// Whether a file exists and what is in it is a question about a tree, and
    /// this function is pure; [`crate::bundle`] asks it, at the moment it
    /// copies the file into the artifact.
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
        if self.encoding_flag().is_none() {
            return Err(ConfigError::FilenameEncoding {
                path: path.to_path_buf(),
                value: self.filename_encoding().to_owned(),
            });
        }
        for name in self.env.keys() {
            if let Some(reason) = env_name_reason(name) {
                return Err(ConfigError::EnvName {
                    path: path.to_path_buf(),
                    name: name.clone(),
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

/// The emulator flag one of [`FILENAME_ENCODINGS`] maps to.
///
/// `utf8` is `+fnu`, `latin1` is `+fnl` and `auto` is `+fna`. [`None`] for
/// anything else, which is what [`ToolsConfig::validate`] turns into
/// [`ConfigError::FilenameEncoding`].
pub fn filename_encoding_flag(name: &str) -> Option<&'static str> {
    match name {
        "utf8" => Some("+fnu"),
        "latin1" => Some("+fnl"),
        "auto" => Some("+fna"),
        _ => None,
    }
}

/// Why a flag may not appear in an args file, or [`None`] if it may.
///
/// The reason is the actionable half: a user told that `-pa` is refused still
/// has to be told who sets it instead. See [`REJECTED_ARGS_FILE_FLAGS`].
pub fn args_file_flag_reason(flag: &str) -> Option<&'static str> {
    match flag {
        "-args_file" => Some(
            "erl follows the nesting, and the file it would include is one ginary has not read",
        ),
        "-noinput" => Some("ginary always starts the runtime with -noshell, which implies it"),
        // The five the launcher passes itself, with the same explanation the
        // `erl_flags` lint gives: one rule, stated once.
        _ => erl_flag_reason(flag),
    }
}

/// Why a variable may not appear in `[tools.ginary] env`, or [`None`].
///
/// See [`REJECTED_ENV_PREFIX`] and [`REJECTED_ENV_NAMES`].
pub fn env_name_reason(name: &str) -> Option<&'static str> {
    if name.starts_with(REJECTED_ENV_PREFIX) {
        return Some(
            "every ERL_ variable is one the launcher sets or scrubs, and `env` is applied after \
             the scrub, so a default here could only put back what the scrub removed",
        );
    }
    if REJECTED_ENV_NAMES.contains(&name) {
        return Some(
            "the launcher derives this from the extracted root, and an artifact that overrode it \
             would be pointing the runtime at a tree that is not its own",
        );
    }
    None
}

/// One word of an args file, with the line it started on.
///
/// `erl -args_file` splits on whitespace, honours `'` and `"` quoting and
/// treats `#` as a comment to the end of the line. The line number is carried
/// because it is the only thing that makes a refusal actionable: an args file
/// is a list of flags with no other structure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArgsToken {
    /// The 1-based line the token started on.
    pub line: u32,
    /// The token with its quotes removed.
    pub text: String,
}

/// Splits an args file the way `erl -args_file` does.
///
/// Whitespace separates tokens; `'` and `"` quote a run of characters,
/// including whitespace, and are not part of the token; `#` starts a comment
/// that runs to the end of the line, unless it is inside quotes.
pub fn tokenize_args_file(text: &str) -> Vec<ArgsToken> {
    scan_args_file(text).0
}

/// The tokens of an args file, and the line a quote is left open on.
///
/// The second half is what makes the first half trustworthy: a quote that is
/// never closed swallows the rest of the file into one token, so every token
/// after it is a guess, and a caller that lints tokens has to know. See
/// [`ConfigError::ArgsFileQuote`].
fn scan_args_file(text: &str) -> (Vec<ArgsToken>, Option<u32>) {
    let mut tokens = Vec::new();
    // The token being built, with the line it started on. A quote opens one
    // even when it encloses nothing, so that `-setcookie ''` is two tokens
    // rather than one.
    let mut pending: Option<(u32, String)> = None;
    // The quote character and the line it was opened on, which is the line a
    // refusal names.
    let mut quote: Option<(char, u32)> = None;
    let mut comment = false;
    let mut line: u32 = 1;

    for character in text.chars() {
        if character == '\n' {
            match quote {
                // A newline inside quotes is a character of the token, and the
                // token keeps the line it started on.
                Some(_) => push(&mut pending, line, character),
                None => flush(&mut pending, &mut tokens),
            }
            comment = false;
            line = line.saturating_add(1);
            continue;
        }
        if comment {
            continue;
        }
        match quote {
            Some((open, _)) if character == open => quote = None,
            Some(_) => push(&mut pending, line, character),
            None if character == '\'' || character == '"' => {
                quote = Some((character, line));
                if pending.is_none() {
                    pending = Some((line, String::new()));
                }
            }
            None if character == '#' => {
                flush(&mut pending, &mut tokens);
                comment = true;
            }
            None if character.is_whitespace() => flush(&mut pending, &mut tokens),
            None => push(&mut pending, line, character),
        }
    }
    flush(&mut pending, &mut tokens);
    (tokens, quote.map(|(_, line)| line))
}

/// Adds one character to the token being built, starting one if there is none.
fn push(pending: &mut Option<(u32, String)>, line: u32, character: char) {
    pending
        .get_or_insert_with(|| (line, String::new()))
        .1
        .push(character);
}

/// Ends the token being built, if there is one.
fn flush(pending: &mut Option<(u32, String)>, tokens: &mut Vec<ArgsToken>) {
    if let Some((line, text)) = pending.take() {
        tokens.push(ArgsToken { line, text });
    }
}

/// Refuses an args file that holds a flag the launcher owns.
///
/// `path` names the file in the message, and the message names the line, so
/// that a refusal points at a place in a file rather than at a setting.
///
/// # Errors
///
/// [`ConfigError::ArgsFileQuote`] when a quote is left open, which is checked
/// first because it decides what the tokens are, and then
/// [`ConfigError::ArgsFileFlag`] for the first token of
/// [`REJECTED_ARGS_FILE_FLAGS`] the file holds.
pub fn lint_args_file(text: &str, path: &Path) -> Result<(), ConfigError> {
    let (tokens, unterminated) = scan_args_file(text);
    // Before the flags, because it decides what the flags even are: with a
    // quote left open the tail of the file is one token, so a lint that
    // reported what it found there would be reporting a reading of the file
    // `erl` does not share.
    if let Some(line) = unterminated {
        return Err(ConfigError::ArgsFileQuote {
            path: path.to_path_buf(),
            line,
        });
    }
    for token in tokens {
        if let Some(reason) = args_file_flag_reason(&token.text) {
            return Err(ConfigError::ArgsFileFlag {
                path: path.to_path_buf(),
                line: token.line,
                flag: token.text,
                reason,
            });
        }
    }
    Ok(())
}

/// Checks that `text` is a `sys.config`: exactly one term, and a list.
///
/// # Errors
///
/// [`ConfigError::SysConfigSyntax`] carrying the line and column
/// [`crate::appfile::parse_terms`] reported, and
/// [`ConfigError::SysConfigShape`] when the file parses and is not one list.
pub fn validate_sys_config(text: &str, path: &Path) -> Result<(), ConfigError> {
    use crate::appfile::Term;

    let terms =
        crate::appfile::parse_terms(text).map_err(|error| ConfigError::SysConfigSyntax {
            path: path.to_path_buf(),
            line: error.line,
            col: error.col,
            message: format!("expected {}, found {}", error.expected, error.found),
        })?;

    let found = match terms.as_slice() {
        [Term::List(_)] => return Ok(()),
        // A file with nothing in it is not an empty list: `file:consult/1`
        // answers `[]` for both, and an artifact whose configuration silently
        // vanished is exactly the failure this lint exists to catch.
        [] => "nothing at all".to_owned(),
        [term] => describe_term(term).to_owned(),
        terms => format!("{} terms", terms.len()),
    };
    Err(ConfigError::SysConfigShape {
        path: path.to_path_buf(),
        found,
    })
}

/// What one term is, as the phrase [`ConfigError::SysConfigShape`] reads with.
fn describe_term(term: &crate::appfile::Term) -> &'static str {
    use crate::appfile::Term;

    match term {
        Term::Atom(_) => "an atom",
        Term::Str(_) => "a string",
        Term::Bin(_) => "a binary",
        Term::Int(_) => "an integer",
        Term::Float(_) => "a float",
        Term::Tuple(_) => "a tuple",
        Term::List(_) => "a list",
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
    /// `--distribution`, which can only turn the table's setting on.
    pub distribution: bool,
    /// `--vm-args`, a path as the user typed it.
    pub vm_args: Option<PathBuf>,
    /// `--sys-config`, a path as the user typed it.
    pub sys_config: Option<PathBuf>,
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
    /// The args file to copy into the artifact, resolved against the project.
    pub vm_args: Option<PathBuf>,
    /// The `sys.config` to copy into the artifact, resolved against it too.
    pub sys_config: Option<PathBuf>,
    /// Whether to ship `epmd` and start the runtime distributed.
    pub distribution: bool,
    /// The filename encoding, always one of [`FILENAME_ENCODINGS`].
    pub filename_encoding: String,
    /// Variables the launcher sets, each only when the caller has not.
    pub env: BTreeMap<String, String>,
    /// Whether to ship `heart` and start the runtime under it.
    pub heart: bool,
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
            // A flag is a path the user typed on a command line and is used as
            // typed; a table value is the project's and is relative to it, as
            // `output` is.
            vm_args: resolve_file(root, flags.vm_args.as_deref(), config.tools.vm_args()),
            sys_config: resolve_file(root, flags.sys_config.as_deref(), config.tools.sys_config()),
            // A boolean flag has one direction: its absence is not a request to
            // turn the table's setting off.
            distribution: flags.distribution || config.tools.distribution,
            filename_encoding: config.tools.filename_encoding().to_owned(),
            env: config.tools.env.clone(),
            heart: config.tools.heart,
            explain: flags.explain,
            verbose: flags.verbose,
        })
    }
}

/// The path a file-naming setting resolves to, flag first.
///
/// `flag` is used exactly as the user typed it, because every other path on a
/// command line is relative to the working directory. `configured` is the
/// project's own, and is joined onto `root` for the reason
/// `[tools.ginary] output` is: a value in `gleam.toml` describes the project
/// rather than the terminal it is built from.
fn resolve_file(root: &Path, flag: Option<&Path>, configured: Option<&str>) -> Option<PathBuf> {
    match (flag, configured) {
        (Some(path), _) => Some(path.to_path_buf()),
        (None, Some(value)) => Some(root.join(value)),
        (None, None) => None,
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
    /// `filename_encoding` is not one of [`FILENAME_ENCODINGS`].
    #[error(
        "{path}: [tools.ginary] filename_encoding must be `utf8`, `latin1` or `auto`, not \
         `{value}`"
    )]
    FilenameEncoding {
        /// The file the value is in.
        path: PathBuf,
        /// The value that was refused.
        value: String,
    },
    /// A file `[tools.ginary]` names is not in the project.
    #[error("{path}: [tools.ginary] {key} names `{value}`, and there is no file at {missing}")]
    MissingFile {
        /// The manifest that names the file.
        path: PathBuf,
        /// The key that names it, such as `vm_args`.
        key: &'static str,
        /// The value as the manifest wrote it.
        value: String,
        /// Where the value resolved to.
        missing: PathBuf,
    },
    /// An args file holds a flag the launcher passes itself.
    #[error("{path}:{line}: `{flag}` may not appear in an args file: {reason}")]
    ArgsFileFlag {
        /// The args file the flag is in.
        path: PathBuf,
        /// The 1-based line it is on.
        line: u32,
        /// The flag that was refused.
        flag: String,
        /// Who passes it instead; see [`args_file_flag_reason`].
        reason: &'static str,
    },
    /// A file `[tools.ginary]` names is there and cannot be read.
    ///
    /// Separate from [`ConfigError::MissingFile`] because the two send a user
    /// to different places: one to the name in the manifest, the other to the
    /// file itself.
    #[error("{path}: [tools.ginary] {key} names `{value}`, and {file} cannot be read: {reason}")]
    UnreadableFile {
        /// The manifest that names the file.
        path: PathBuf,
        /// The key that names it, such as `sys_config`.
        key: &'static str,
        /// The value as the manifest wrote it.
        value: String,
        /// Where the value resolved to.
        file: PathBuf,
        /// What went wrong, as a phrase.
        reason: String,
    },
    /// An args file ends with a quote nobody closed.
    #[error(
        "{path}:{line}: a quote is opened here and never closed, so every token after it is a \
         guess; `erl -args_file` would read the rest of the file as one argument"
    )]
    ArgsFileQuote {
        /// The args file the quote is in.
        path: PathBuf,
        /// The 1-based line the quote was opened on.
        line: u32,
    },
    /// A `sys.config` is not the Erlang term syntax `file:consult/1` reads.
    #[error("{path}:{line}:{col}: {message}")]
    SysConfigSyntax {
        /// The file the error is in.
        path: PathBuf,
        /// The 1-based line.
        line: u32,
        /// The 1-based column, counted in characters.
        col: u32,
        /// What the parser expected and what it found.
        message: String,
    },
    /// A `sys.config` parses and is not one list.
    #[error("{path}: a sys.config holds exactly one term, a list, and this holds {found}")]
    SysConfigShape {
        /// The file that is the wrong shape.
        path: PathBuf,
        /// What it holds instead, as a phrase.
        found: String,
    },
    /// `env` names a variable the launcher owns.
    #[error("{path}: [tools.ginary] env may not name `{name}`: {reason}")]
    EnvName {
        /// The file the name is in.
        path: PathBuf,
        /// The name that was refused.
        name: String,
        /// Why; see [`env_name_reason`].
        reason: &'static str,
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
