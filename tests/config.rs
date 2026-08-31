// SPDX-License-Identifier: MIT OR Apache-2.0
//! `[tools.ginary]` in `gleam.toml`, and the CLI flags merged over it.
//!
//! Nothing here needs a toolchain, a network or a Gleam project: every
//! manifest is text, read through `ProjectConfig::from_toml`, and the only
//! tests that touch the filesystem are the four about `--out` and the one
//! about a manifest that is not there.
//!
//! The file is in three parts. The first reads manifests and asserts what each
//! key means, including the five rules serde cannot state. The second is the
//! merge, where the precedence — flags, then the table, then the defaults — is
//! pinned one setting at a time. The third is `resolve_output`, which decides
//! whether a `--out` is a directory or a file name.

mod common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use ginary::config::{
    self, ArgsToken, BuildFlags, BuildOptions, ConfigError, DEFAULT_COMPRESSION_LEVEL,
    DEFAULT_FILENAME_ENCODING, DEFAULT_OUTPUT, FILENAME_ENCODINGS, ProjectConfig,
    REJECTED_ARGS_FILE_FLAGS, REJECTED_ENV_NAMES, ToolsConfig,
};
use ginary::erts_source::ErtsSourceSpec;
use ginary::strip::StripOptions;
use ginary::target::{Target, TargetError};

use crate::common::project::{TempProject, config_fixture};

/// The path every `from_toml` in this file names, so a message is comparable.
const MANIFEST: &str = "/w/gleam.toml";

/// Parses one fixture, or fails with what the parser said.
fn parse(fixture: &str) -> ProjectConfig {
    ProjectConfig::from_toml(&config_fixture(fixture), Path::new(MANIFEST))
        .unwrap_or_else(|error| panic!("{fixture} must parse, and it did not: {error}"))
}

/// Parses one fixture, expecting it to be refused.
fn refuse(fixture: &str) -> ConfigError {
    match ProjectConfig::from_toml(&config_fixture(fixture), Path::new(MANIFEST)) {
        Ok(config) => panic!("{fixture} must be refused, and it parsed as {config:?}"),
        Err(error) => error,
    }
}

/// Parses a manifest written inline, for the one-key cases no fixture earns.
fn parse_text(text: &str) -> ProjectConfig {
    ProjectConfig::from_toml(text, Path::new(MANIFEST))
        .unwrap_or_else(|error| panic!("the manifest must parse, and it did not: {error}"))
}

// ------------------------------------------------------- the manifest --

#[test]
fn a_manifest_with_no_tools_table_takes_every_default() {
    let config = parse("defaults.toml");

    assert_eq!(config.name, "plain_app");
    assert_eq!(config.version.as_deref(), Some("0.3.1"));
    assert_eq!(config.tools, ToolsConfig::default());
    assert_eq!(config.tools.output(), DEFAULT_OUTPUT);
    assert_eq!(config.tools.compression_level(), DEFAULT_COMPRESSION_LEVEL);
    assert!(config.tools.strip_elf(), "stripping is on by default");
    assert!(config.tools.strip_beams(), "stripping is on by default");
    assert_eq!(config.tools.otp_applications, Vec::<String>::new());
    assert_eq!(config.tools.erts_extra_bins, Vec::<String>::new());
    assert_eq!(config.tools.erl_flags, Vec::<String>::new());
}

#[test]
fn every_key_of_the_tools_table_is_read() {
    let config = parse("full.toml");

    assert_eq!(config.name, "full_app");
    assert_eq!(config.version.as_deref(), Some("2.5.0"));
    assert_eq!(config.tools.output(), "dist");
    assert_eq!(config.tools.compression_level(), 7);
    assert_eq!(config.tools.otp_applications, ["sasl", "runtime_tools"]);
    assert_eq!(config.tools.erts_extra_bins, ["heart", "epmd"]);
    assert_eq!(config.tools.erl_flags, ["+fnu", "+SDio", "4"]);
}

#[test]
fn strip_elf_and_strip_beams_each_override_the_combined_strip() {
    // `full.toml` has `strip = false`, `strip_elf = true`, `strip_beams =
    // false`: the individual keys are the answer wherever they are set.
    let config = parse("full.toml");
    assert!(config.tools.strip_elf(), "strip_elf = true overrides strip");
    assert!(!config.tools.strip_beams());
    assert_eq!(
        config.tools.strip_options(),
        StripOptions {
            elf: true,
            beams: false
        }
    );

    // `strip = false` alone turns both halves off.
    let both_off = parse_text("name = \"a\"\n\n[tools.ginary]\nstrip = false\n");
    assert_eq!(
        both_off.tools.strip_options(),
        StripOptions {
            elf: false,
            beams: false
        }
    );

    // A lone `strip_beams = false` leaves the ELF half at its default.
    let beams_off = parse_text("name = \"a\"\n\n[tools.ginary]\nstrip_beams = false\n");
    assert_eq!(
        beams_off.tools.strip_options(),
        StripOptions {
            elf: true,
            beams: false
        }
    );
}

#[test]
fn an_unknown_key_in_the_tools_table_names_the_key_and_the_file() {
    let error = refuse("unknown_key.toml");

    let ConfigError::Parse { path, message } = &error else {
        panic!("expected ConfigError::Parse, got {error:?}");
    };
    assert_eq!(path, Path::new(MANIFEST));
    assert!(
        message.contains("outpu"),
        "the message must name the key that was not understood: {message}"
    );
    let rendered = error.to_string();
    assert!(
        rendered.contains(MANIFEST),
        "the message must name the file: {rendered}"
    );
}

#[test]
fn a_manifest_that_is_not_toml_names_the_file() {
    let error = refuse("malformed.toml");

    assert!(
        matches!(error, ConfigError::Parse { .. }),
        "expected ConfigError::Parse, got {error:?}"
    );
    assert!(
        error.to_string().contains(MANIFEST),
        "the message must name the file: {error}"
    );
}

#[test]
fn a_project_that_declares_no_name_is_refused() {
    let error = refuse("no_name.toml");

    let ConfigError::MissingName { path } = &error else {
        panic!("expected ConfigError::MissingName, got {error:?}");
    };
    assert_eq!(path, Path::new(MANIFEST));
}

#[test]
fn a_project_name_that_is_not_a_gleam_name_is_refused() {
    let error = refuse("bad_name.toml");

    let ConfigError::InvalidName { path, name } = &error else {
        panic!("expected ConfigError::InvalidName, got {error:?}");
    };
    assert_eq!(path, Path::new(MANIFEST));
    assert_eq!(name, "Hello-World");
}

#[test]
fn a_gleam_name_is_a_lower_case_letter_then_letters_digits_and_underscores() {
    for name in ["a", "hello", "hello_ffi", "gleam_stdlib", "h2o", "a_1_b"] {
        assert!(config::is_gleam_name(name), "`{name}` is a Gleam name");
    }
    // The reasons each of these must be refused are the three places the name
    // is interpolated: a path, the manifest's `app`, and the `-eval` atom.
    for name in [
        "",
        "Hello",
        "hello-ffi",
        "1hello",
        "_hello",
        "hello.ffi",
        "hello/ffi",
        "../escape",
        "hello ffi",
        "héllo",
    ] {
        assert!(!config::is_gleam_name(name), "`{name}` is not a Gleam name");
    }
}

#[test]
fn a_compression_level_outside_one_to_twenty_two_is_refused() {
    for level in [0, -1, 23, 100] {
        let text = format!("name = \"a\"\n\n[tools.ginary]\ncompression_level = {level}\n");
        let error = ProjectConfig::from_toml(&text, Path::new(MANIFEST))
            .expect_err("the level must be refused");
        let ConfigError::CompressionLevel {
            path,
            level: reported,
        } = &error
        else {
            panic!("expected ConfigError::CompressionLevel for {level}, got {error:?}");
        };
        assert_eq!(path, Path::new(MANIFEST));
        assert_eq!(*reported, level);
    }
}

#[test]
fn the_ends_of_the_compression_range_are_accepted() {
    for level in [1, 19, 22] {
        let text = format!("name = \"a\"\n\n[tools.ginary]\ncompression_level = {level}\n");
        let config = parse_text(&text);
        assert_eq!(config.tools.compression_level(), level);
    }
}

#[test]
fn the_bad_level_fixture_is_the_one_past_the_top_of_the_range() {
    let error = refuse("bad_level.toml");
    let ConfigError::CompressionLevel { level, .. } = &error else {
        panic!("expected ConfigError::CompressionLevel, got {error:?}");
    };
    assert_eq!(*level, 23);
}

#[test]
fn a_flag_the_launcher_owns_is_refused_in_erl_flags() {
    let error = refuse("bad_erl_flags.toml");

    let ConfigError::ErlFlag { path, flag, reason } = &error else {
        panic!("expected ConfigError::ErlFlag, got {error:?}");
    };
    assert_eq!(path, Path::new(MANIFEST));
    assert_eq!(flag, "-pa");
    assert!(
        !reason.is_empty(),
        "the message must say who sets the flag instead"
    );
}

#[test]
fn a_program_name_that_is_a_path_is_refused_in_erts_extra_bins() {
    let error = refuse("bad_extra_bin.toml");

    let ConfigError::ExtraBin { path, name } = &error else {
        panic!("expected ConfigError::ExtraBin, got {error:?}");
    };
    assert_eq!(path, Path::new(MANIFEST));
    assert_eq!(
        name, "../../../etc/passwd",
        "the first name that is not a program name is the one reported"
    );
    let message = error.to_string();
    assert!(
        message.contains("erts_extra_bins") && message.contains(config::EXTRA_BIN_REASON),
        "the message must name the key and say what a program name is: {message}"
    );
}

#[test]
fn a_program_name_is_a_file_name_and_nothing_else() {
    for name in ["heart", "epmd", "erl_call", "a.out"] {
        assert!(
            ginary::assemble::is_erts_bin_name(name),
            "`{name}` is an ordinary program name"
        );
    }
    for name in ["", ".", "..", "a/b", "../x", "/bin/sh", "a\0b"] {
        assert!(
            !ginary::assemble::is_erts_bin_name(name),
            "`{name}` is not a file name in the runtime's bin directory"
        );
    }
}

#[test]
fn each_flag_the_launcher_owns_has_its_own_reason() {
    let mut lines = String::new();
    for flag in config::REJECTED_ERL_FLAGS {
        let reason = config::erl_flag_reason(flag)
            .unwrap_or_else(|| panic!("`{flag}` is rejected and must carry a reason"));
        lines.push_str(&format!("{flag}: {reason}\n"));
    }
    insta::assert_snapshot!("erl_flag_reasons", lines);
}

#[test]
fn every_rejected_flag_is_refused_wherever_it_appears_in_the_list() {
    for flag in config::REJECTED_ERL_FLAGS {
        let text = format!("name = \"a\"\n\n[tools.ginary]\nerl_flags = [\"+fnu\", \"{flag}\"]\n");
        let error = ProjectConfig::from_toml(&text, Path::new(MANIFEST))
            .expect_err("the flag must be refused");
        let ConfigError::ErlFlag { flag: reported, .. } = &error else {
            panic!("expected ConfigError::ErlFlag for `{flag}`, got {error:?}");
        };
        assert_eq!(reported, flag);
    }
}

#[test]
fn a_flag_the_launcher_does_not_own_is_kept() {
    for flag in ["+fnu", "-mode", "+SDio", "-pav", "-boots"] {
        assert!(
            config::erl_flag_reason(flag).is_none(),
            "`{flag}` is not one the launcher sets and must be kept"
        );
    }
    let config = parse_text("name = \"a\"\n\n[tools.ginary]\nerl_flags = [\"+fnu\", \"-mode\"]\n");
    assert_eq!(config.tools.erl_flags, ["+fnu", "-mode"]);
}

#[test]
fn reading_a_manifest_that_is_not_there_names_the_file() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("gleam.toml");

    let error = ProjectConfig::read(&path).expect_err("a missing manifest is an error");

    let ConfigError::Read { path: named, .. } = &error else {
        panic!("expected ConfigError::Read, got {error:?}");
    };
    assert_eq!(named, &path);
}

#[test]
fn reading_a_manifest_from_disk_gives_what_parsing_its_text_gives() {
    let project = TempProject::new(&config_fixture("full.toml"));

    let from_disk = ProjectConfig::read(&project.manifest()).expect("the manifest reads");
    let from_text = ProjectConfig::from_toml(&config_fixture("full.toml"), &project.manifest())
        .expect("the manifest parses");

    assert_eq!(from_disk, from_text);
}

// ------------------------------------------ the runtime settings (B1) --

#[test]
fn a_manifest_with_no_tools_table_takes_every_runtime_default() {
    let config = parse("defaults.toml");

    assert_eq!(config.tools.vm_args(), None);
    assert_eq!(config.tools.sys_config(), None);
    assert!(!config.tools.distribution, "distribution is opt in");
    assert!(!config.tools.heart, "heart is opt in");
    assert_eq!(config.tools.env, BTreeMap::new());
    assert_eq!(
        config.tools.filename_encoding(),
        DEFAULT_FILENAME_ENCODING,
        "a Gleam shipment's file names are UTF-8, so `utf8` is the only safe default"
    );
    assert_eq!(config.tools.encoding_flag(), Some("+fnu"));
}

#[test]
fn every_runtime_key_of_the_tools_table_is_read() {
    let config = parse("runtime.toml");

    assert_eq!(config.name, "runtime_app");
    assert_eq!(config.tools.vm_args(), Some("config/vm.args"));
    assert_eq!(config.tools.sys_config(), Some("config/sys.config"));
    assert!(config.tools.distribution);
    assert!(config.tools.heart);
    assert_eq!(config.tools.filename_encoding(), "latin1");
    assert_eq!(config.tools.encoding_flag(), Some("+fnl"));
    assert_eq!(
        config.tools.env,
        BTreeMap::from([
            ("LOG_LEVEL".to_owned(), "info".to_owned()),
            ("RELEASE_NAME".to_owned(), "runtime_app".to_owned()),
        ])
    );
}

#[test]
fn each_filename_encoding_maps_to_its_emulator_flag() {
    assert_eq!(config::filename_encoding_flag("utf8"), Some("+fnu"));
    assert_eq!(config::filename_encoding_flag("latin1"), Some("+fnl"));
    assert_eq!(config::filename_encoding_flag("auto"), Some("+fna"));
    for name in ["utf-8", "UTF8", "", "koi8-r", "+fnu"] {
        assert_eq!(
            config::filename_encoding_flag(name),
            None,
            "`{name}` is not one of the three the emulator has a flag for"
        );
    }
}

#[test]
fn the_three_encodings_are_the_ones_the_error_lists() {
    assert_eq!(FILENAME_ENCODINGS, ["utf8", "latin1", "auto"]);
    for name in FILENAME_ENCODINGS {
        assert!(
            config::filename_encoding_flag(name).is_some(),
            "`{name}` is offered to the user and must map to a flag"
        );
    }
}

#[test]
fn a_filename_encoding_the_emulator_has_no_flag_for_is_refused() {
    let error = refuse("bad_encoding.toml");

    assert!(
        matches!(
            &error,
            ConfigError::FilenameEncoding { path, value }
                if path == Path::new(MANIFEST) && value == "koi8-r"
        ),
        "expected ConfigError::FilenameEncoding, got {error:?}"
    );
    let message = error.to_string();
    for name in FILENAME_ENCODINGS {
        assert!(
            message.contains(name),
            "the message must list `{name}`, and it is `{message}`"
        );
    }
}

// ------------------------------------------------- the args file lint --

#[test]
fn an_args_file_splits_on_whitespace_and_carries_its_line_numbers() {
    let tokens = config::tokenize_args_file("-sname node\n+S 2:2\n");

    assert_eq!(
        tokens,
        vec![
            ArgsToken {
                line: 1,
                text: "-sname".to_owned()
            },
            ArgsToken {
                line: 1,
                text: "node".to_owned()
            },
            ArgsToken {
                line: 2,
                text: "+S".to_owned()
            },
            ArgsToken {
                line: 2,
                text: "2:2".to_owned()
            },
        ]
    );
}

#[test]
fn a_quoted_token_keeps_its_spaces_and_loses_its_quotes() {
    let tokens = config::tokenize_args_file("-setcookie \"a b\" -name 'x@y'\n");

    assert_eq!(
        tokens
            .into_iter()
            .map(|token| token.text)
            .collect::<Vec<String>>(),
        ["-setcookie", "a b", "-name", "x@y"],
        "both quote characters group a run of characters and are not part of it"
    );
}

#[test]
fn a_comment_runs_to_the_end_of_its_line_and_no_further() {
    let tokens = config::tokenize_args_file("# a whole line\n+S 2:2 # trailing\n-sname node\n");

    assert_eq!(
        tokens,
        vec![
            ArgsToken {
                line: 2,
                text: "+S".to_owned()
            },
            ArgsToken {
                line: 2,
                text: "2:2".to_owned()
            },
            ArgsToken {
                line: 3,
                text: "-sname".to_owned()
            },
            ArgsToken {
                line: 3,
                text: "node".to_owned()
            },
        ]
    );
}

#[test]
fn a_hash_inside_quotes_is_not_a_comment() {
    let tokens = config::tokenize_args_file("-setcookie \"a#b\"\n");

    assert_eq!(
        tokens
            .into_iter()
            .map(|token| token.text)
            .collect::<Vec<String>>(),
        ["-setcookie", "a#b"]
    );
}

#[test]
fn an_args_file_that_names_no_flag_the_launcher_owns_passes_the_lint() {
    let text = "# the node\n-sname worker\n-setcookie \"a b\"\n+S 2:2\n";

    assert!(
        config::lint_args_file(text, Path::new("/w/vm.args")).is_ok(),
        "an args file may hold every flag but the seven ginary passes itself"
    );

    let refused = config::lint_args_file("-sname a\n-boot start_clean\n", Path::new("/w/vm.args"))
        .expect_err("`-boot` is the launcher's");
    assert!(
        matches!(
            &refused,
            ConfigError::ArgsFileFlag { path, line, flag, .. }
                if path == Path::new("/w/vm.args") && *line == 2 && flag == "-boot"
        ),
        "expected ConfigError::ArgsFileFlag on line 2, got {refused:?}"
    );
}

#[test]
fn every_flag_the_launcher_owns_is_refused_in_an_args_file_with_its_line() {
    for flag in REJECTED_ARGS_FILE_FLAGS {
        let text = format!("+S 2:2\n# a comment\n{flag} value\n");
        let error = config::lint_args_file(&text, Path::new("/w/vm.args"))
            .expect_err("a flag the launcher owns must be refused in an args file");
        assert!(
            matches!(
                &error,
                ConfigError::ArgsFileFlag { line, flag: found, .. }
                    if *line == 3 && found == flag
            ),
            "`{flag}` must be refused on line 3, and it said {error:?}"
        );
    }
}

#[test]
fn each_rejected_args_file_flag_has_its_own_reason() {
    for flag in REJECTED_ARGS_FILE_FLAGS {
        let reason = config::args_file_flag_reason(flag)
            .unwrap_or_else(|| panic!("`{flag}` is refused and must say why"));
        assert!(
            !reason.is_empty(),
            "`{flag}` must carry an actionable reason"
        );
    }
    for flag in ["-sname", "-name", "-setcookie", "+S", "-config"] {
        assert_eq!(
            config::args_file_flag_reason(flag),
            None,
            "`{flag}` is the user's to set in an args file"
        );
    }
}

#[test]
fn the_args_file_lint_reports_the_first_offending_line() {
    let text = "+S 2:2\n-pa /opt/lib\n-pz /opt/other\n";
    let error = config::lint_args_file(text, Path::new("/w/vm.args"))
        .expect_err("an args file that sets the code path must be refused");

    assert!(
        matches!(
            &error,
            ConfigError::ArgsFileFlag { line, flag, .. } if *line == 2 && flag == "-pa"
        ),
        "the first offence is the one reported, and it said {error:?}"
    );
    let message = error.to_string();
    assert!(
        message.contains("/w/vm.args:2"),
        "the message must locate the flag, and it is `{message}`"
    );
}

// ------------------------------------------------- the sys.config lint --

#[test]
fn a_sys_config_that_is_one_list_of_terms_is_accepted() {
    let text = "[{kernel, [{logger_level, info}]}, {my_app, [{port, 8080}]}].\n";

    assert!(
        config::validate_sys_config(text, Path::new("/w/sys.config")).is_ok(),
        "this is exactly the shape `file:consult/1` reads"
    );

    let error = config::validate_sys_config("{kernel, []}.\n", Path::new("/w/sys.config"))
        .expect_err("a sys.config holds a list, not a tuple");
    assert!(
        matches!(&error, ConfigError::SysConfigShape { .. }),
        "expected ConfigError::SysConfigShape, got {error:?}"
    );
}

#[test]
fn a_sys_config_that_does_not_parse_names_the_line_and_the_column() {
    // `%` is Erlang's comment, so the offending `#` is on line 2, column 12.
    let text = "% the config\n[{kernel, #{}}].\n";
    let error = config::validate_sys_config(text, Path::new("/w/sys.config"))
        .expect_err("a map is not in the subset a sys.config may use");

    assert!(
        matches!(
            &error,
            ConfigError::SysConfigSyntax { path, line, col, .. }
                if path == Path::new("/w/sys.config") && *line == 2 && *col == 11
        ),
        "expected a syntax error at 2:11, got {error:?}"
    );
    assert!(
        error.to_string().starts_with("/w/sys.config:2:11: "),
        "the message must lead with file:line:col, and it is `{error}`"
    );
}

#[test]
fn a_sys_config_holding_two_terms_is_refused() {
    let error = config::validate_sys_config("[].\n[].\n", Path::new("/w/sys.config"))
        .expect_err("a sys.config holds exactly one term");

    assert!(
        matches!(
            &error,
            ConfigError::SysConfigShape { found, .. } if found.contains('2')
        ),
        "the message must say how many terms it found, and it said {error:?}"
    );
}

#[test]
fn an_empty_sys_config_is_refused_rather_than_read_as_an_empty_list() {
    let error = config::validate_sys_config("\n% nothing\n", Path::new("/w/sys.config"))
        .expect_err("a file with no term in it is not a sys.config");

    assert!(
        matches!(&error, ConfigError::SysConfigShape { .. }),
        "expected ConfigError::SysConfigShape, got {error:?}"
    );
}

// ------------------------------------------------- the env key refusals --

#[test]
fn an_env_key_the_launcher_scrubs_is_refused() {
    let error = refuse("bad_env_key.toml");

    assert!(
        matches!(
            &error,
            ConfigError::EnvName { path, name, .. }
                if path == Path::new(MANIFEST) && name == "ERL_AFLAGS"
        ),
        "expected ConfigError::EnvName, got {error:?}"
    );
}

#[test]
fn an_env_key_the_launcher_derives_is_refused() {
    let error = refuse("bad_env_name.toml");

    assert!(
        matches!(
            &error,
            ConfigError::EnvName { name, .. } if name == "ROOTDIR"
        ),
        "expected ConfigError::EnvName for ROOTDIR, got {error:?}"
    );
}

#[test]
fn every_erl_prefixed_name_and_every_derived_name_is_refused() {
    for name in [
        "ERL_",
        "ERL_LIBS",
        "ERL_AFLAGS",
        "ERL_CRASH_DUMP",
        "ERL_OTP29_FLAGS",
    ] {
        assert!(
            config::env_name_reason(name).is_some(),
            "`{name}` is the launcher's and must be refused"
        );
    }
    assert_eq!(
        REJECTED_ENV_NAMES,
        ["BINDIR", "EMU", "HOME", "PROGNAME", "ROOTDIR"]
    );
    for name in REJECTED_ENV_NAMES {
        assert!(
            config::env_name_reason(name).is_some(),
            "`{name}` is derived from the extracted root and must be refused"
        );
    }
}

#[test]
fn a_variable_the_launcher_does_not_own_is_kept() {
    for name in ["LOG_LEVEL", "RELEASE_NAME", "PATH", "ERLANG_HOME", "TERM"] {
        assert_eq!(
            config::env_name_reason(name),
            None,
            "`{name}` is the project's to default, and `ERLANG_HOME` only looks like ours"
        );
    }
    // The other half of the same rule, so that a `None` for everything cannot
    // read as this test passing.
    assert!(
        config::env_name_reason("ERL_LIBS").is_some(),
        "`ERL_LIBS` is the launcher's and must not be kept"
    );
    let config = parse("runtime.toml");
    assert_eq!(config.tools.env.len(), 2);
}

// ----------------------------------------------------------- the merge --

/// The flags a build starts from: everything off, nothing named.
fn no_flags(start: &Path) -> BuildFlags {
    BuildFlags {
        start: start.to_path_buf(),
        ..BuildFlags::default()
    }
}

/// Merges `full.toml` and the given flags against a project root.
fn merge(root: &Path, flags: &BuildFlags) -> BuildOptions {
    let config = parse("full.toml");
    BuildOptions::merge(root, &config, flags).expect("the merge succeeds")
}

#[test]
fn the_merge_falls_back_to_the_defaults_when_neither_flags_nor_table_speak() {
    let root = Path::new("/w/plain_app");
    let config = parse("defaults.toml");
    let options = BuildOptions::merge(root, &config, &no_flags(root)).expect("the merge succeeds");

    assert_eq!(options.root, root);
    assert_eq!(options.app, "plain_app");
    assert_eq!(options.app_version.as_deref(), Some("0.3.1"));
    assert_eq!(options.out, root.join(DEFAULT_OUTPUT).join("plain_app"));
    assert_eq!(options.compression_level, DEFAULT_COMPRESSION_LEVEL);
    assert_eq!(
        options.strip,
        StripOptions {
            elf: true,
            beams: true
        }
    );
    assert_eq!(options.otp_root, None);
    assert!(!options.skip_export);
    assert!(!options.keep_staging);
    assert!(!options.explain);
    assert_eq!(options.verbose, 0);
    assert_eq!(options.otp_applications, Vec::<String>::new());
    assert_eq!(options.erts_extra_bins, Vec::<String>::new());
    assert_eq!(options.erl_flags, Vec::<String>::new());
    assert_eq!(options.vm_args, None);
    assert_eq!(options.sys_config, None);
    assert!(!options.distribution);
    assert!(!options.heart);
    assert_eq!(options.env, BTreeMap::new());
    assert_eq!(options.filename_encoding, DEFAULT_FILENAME_ENCODING);
}

#[test]
fn the_table_wins_over_the_defaults() {
    let root = Path::new("/w/full_app");
    let options = merge(root, &no_flags(root));

    assert_eq!(options.out, root.join("dist").join("full_app"));
    assert_eq!(options.compression_level, 7);
    assert_eq!(
        options.strip,
        StripOptions {
            elf: true,
            beams: false
        }
    );
    assert_eq!(options.otp_applications, ["sasl", "runtime_tools"]);
    assert_eq!(options.erts_extra_bins, ["heart", "epmd"]);
    assert_eq!(options.erl_flags, ["+fnu", "+SDio", "4"]);
}

#[test]
fn a_flag_wins_over_the_table() {
    let root = Path::new("/w/full_app");
    let options = merge(
        root,
        &BuildFlags {
            compression_level: Some(3),
            otp_root: Some(PathBuf::from("/opt/otp")),
            skip_export: true,
            keep_staging: true,
            explain: true,
            verbose: 1,
            ..no_flags(root)
        },
    );

    assert_eq!(options.compression_level, 3, "--compression-level wins");
    assert_eq!(options.otp_root.as_deref(), Some(Path::new("/opt/otp")));
    assert!(options.skip_export);
    assert!(options.keep_staging);
    assert!(options.explain);
    assert_eq!(options.verbose, 1);
}

#[test]
fn no_strip_turns_both_halves_off_whatever_the_table_says() {
    let root = Path::new("/w/full_app");
    let options = merge(
        root,
        &BuildFlags {
            no_strip: true,
            ..no_flags(root)
        },
    );

    assert_eq!(
        options.strip,
        StripOptions {
            elf: false,
            beams: false
        }
    );
}

#[test]
fn each_strip_only_flag_turns_the_other_half_off_whatever_the_table_says() {
    let root = Path::new("/w/full_app");

    let elf_only = merge(
        root,
        &BuildFlags {
            strip_elf_only: true,
            ..no_flags(root)
        },
    );
    assert_eq!(
        elf_only.strip,
        StripOptions {
            elf: true,
            beams: false
        }
    );

    // The table says `strip_beams = false`; `--strip-beams-only` still turns
    // the beam half on, because a flag is what the user typed just now.
    let beams_only = merge(
        root,
        &BuildFlags {
            strip_beams_only: true,
            ..no_flags(root)
        },
    );
    assert_eq!(
        beams_only.strip,
        StripOptions {
            elf: false,
            beams: true
        }
    );
}

#[test]
fn extra_applications_and_extra_binaries_are_added_to_the_tables_lists() {
    let root = Path::new("/w/full_app");
    let options = merge(
        root,
        &BuildFlags {
            // `sasl` is already in the table: a name asked for twice is
            // bundled once.
            extra_otp_apps: vec!["observer".to_owned(), "sasl".to_owned()],
            extra_bins: vec!["epmd".to_owned(), "erl_call".to_owned()],
            ..no_flags(root)
        },
    );

    assert_eq!(
        options.otp_applications,
        ["sasl", "runtime_tools", "observer"],
        "the table's order comes first and the flags are appended, deduplicated"
    );
    assert_eq!(options.erts_extra_bins, ["heart", "epmd", "erl_call"]);
}

#[test]
fn the_merge_refuses_a_table_the_project_could_not_have_built_with() {
    // The lint is part of the merge as well as of parsing, because
    // `BuildOptions` is what the bundler reads and a caller that built one by
    // hand must not get past the rule.
    let root = Path::new("/w/a");
    let config = ProjectConfig {
        name: "a".to_owned(),
        version: None,
        tools: ToolsConfig {
            erl_flags: vec!["-boot".to_owned(), "start_clean".to_owned()],
            ..ToolsConfig::default()
        },
    };

    let error = BuildOptions::merge(root, &config, &no_flags(root))
        .expect_err("the merge must apply the erl_flags lint");
    assert!(
        matches!(error, ConfigError::ErlFlag { ref flag, .. } if flag == "-boot"),
        "expected ConfigError::ErlFlag for -boot, got {error:?}"
    );
}

// ------------------------------------------------------- the output path --

#[test]
fn without_out_the_artifact_goes_under_the_configured_output_directory() {
    let root = Path::new("/w/app");
    assert_eq!(
        config::resolve_output(root, DEFAULT_OUTPUT, None, "app"),
        root.join("build/ginary/app")
    );
    assert_eq!(
        config::resolve_output(root, "dist", None, "app"),
        root.join("dist/app")
    );
}

#[test]
fn an_absolute_output_directory_is_not_joined_onto_the_project() {
    let root = Path::new("/w/app");
    assert_eq!(
        config::resolve_output(root, "/srv/artifacts", None, "app"),
        PathBuf::from("/srv/artifacts/app")
    );
}

#[test]
fn an_out_that_is_an_existing_directory_takes_the_application_name() {
    let dir = tempfile::tempdir().expect("a temporary directory");

    assert_eq!(
        config::resolve_output(Path::new("/w/app"), DEFAULT_OUTPUT, Some(dir.path()), "app"),
        dir.path().join("app")
    );
}

#[test]
fn an_out_that_ends_in_a_separator_takes_the_application_name() {
    // The directory does not exist, so only the trailing separator can say
    // that the user meant a directory. `ginary build --out out/` must not
    // write a *file* called `out`.
    assert_eq!(
        config::resolve_output(
            Path::new("/w/app"),
            DEFAULT_OUTPUT,
            Some(Path::new("/tmp/nowhere/")),
            "app"
        ),
        PathBuf::from("/tmp/nowhere/app")
    );
}

#[test]
fn an_out_that_names_a_file_is_the_artifacts_own_path() {
    assert_eq!(
        config::resolve_output(
            Path::new("/w/app"),
            DEFAULT_OUTPUT,
            Some(Path::new("/usr/local/bin/myapp")),
            "app"
        ),
        PathBuf::from("/usr/local/bin/myapp")
    );
}

#[test]
fn a_relative_out_is_relative_to_where_the_user_is_and_not_to_the_project() {
    // `--out` is typed on a command line, and every other path on a command
    // line is relative to the working directory. Only `[tools.ginary] output`
    // is relative to the project.
    assert_eq!(
        config::resolve_output(
            Path::new("/w/app"),
            DEFAULT_OUTPUT,
            Some(Path::new("here/myapp")),
            "app"
        ),
        PathBuf::from("here/myapp")
    );
}

#[test]
fn the_out_flag_reaches_the_merged_options() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let root = Path::new("/w/full_app");
    let options = merge(
        root,
        &BuildFlags {
            out: Some(dir.path().to_path_buf()),
            ..no_flags(root)
        },
    );

    assert_eq!(options.out, dir.path().join("full_app"));
}

// --------------------------------- the runtime settings through the merge --

/// Merges `runtime.toml` and the given flags against a project root.
fn merge_runtime(root: &Path, flags: &BuildFlags) -> BuildOptions {
    let config = parse("runtime.toml");
    BuildOptions::merge(root, &config, flags).expect("the merge succeeds")
}

#[test]
fn the_table_decides_every_runtime_setting_when_no_flag_speaks() {
    let root = Path::new("/w/runtime_app");
    let options = merge_runtime(root, &no_flags(root));

    assert_eq!(
        options.vm_args.as_deref(),
        Some(root.join("config/vm.args").as_path()),
        "a table path is relative to the project, as `output` is"
    );
    assert_eq!(
        options.sys_config.as_deref(),
        Some(root.join("config/sys.config").as_path())
    );
    assert!(options.distribution);
    assert!(options.heart);
    assert_eq!(options.filename_encoding, "latin1");
    assert_eq!(
        options.env,
        BTreeMap::from([
            ("LOG_LEVEL".to_owned(), "info".to_owned()),
            ("RELEASE_NAME".to_owned(), "runtime_app".to_owned()),
        ])
    );
}

#[test]
fn the_vm_args_flag_wins_over_the_table_and_is_relative_to_the_user() {
    // `--vm-args` is typed on a command line, and every other path on a
    // command line is relative to the working directory.
    let root = Path::new("/w/runtime_app");
    let options = merge_runtime(
        root,
        &BuildFlags {
            vm_args: Some(PathBuf::from("other/vm.args")),
            ..no_flags(root)
        },
    );

    assert_eq!(
        options.vm_args,
        Some(PathBuf::from("other/vm.args")),
        "the flag is used as typed and is not joined onto the project"
    );
}

#[test]
fn the_sys_config_flag_wins_over_the_table() {
    let root = Path::new("/w/runtime_app");
    let options = merge_runtime(
        root,
        &BuildFlags {
            sys_config: Some(PathBuf::from("/etc/app/sys.config")),
            ..no_flags(root)
        },
    );

    assert_eq!(
        options.sys_config,
        Some(PathBuf::from("/etc/app/sys.config"))
    );
}

#[test]
fn the_distribution_flag_turns_the_setting_on_and_never_off() {
    // A boolean flag has one direction: `--distribution` on a project that
    // already asked for it changes nothing, and its absence is not a request
    // to turn the table's setting off.
    let root = Path::new("/w/full_app");
    let plain = merge(root, &no_flags(root));
    assert!(!plain.distribution, "`full.toml` does not ask for it");

    let flagged = merge(
        root,
        &BuildFlags {
            distribution: true,
            ..no_flags(root)
        },
    );
    assert!(flagged.distribution);

    let table = merge_runtime(Path::new("/w/runtime_app"), &no_flags(root));
    assert!(
        table.distribution,
        "the absence of the flag must not override a table that asked for it"
    );
}

#[test]
fn a_flag_that_names_a_file_reaches_the_merged_options_from_either_side() {
    let root = Path::new("/w/full_app");
    let options = merge(
        root,
        &BuildFlags {
            vm_args: Some(PathBuf::from("vm.args")),
            sys_config: Some(PathBuf::from("sys.config")),
            ..no_flags(root)
        },
    );

    assert_eq!(options.vm_args, Some(PathBuf::from("vm.args")));
    assert_eq!(options.sys_config, Some(PathBuf::from("sys.config")));
    assert_eq!(
        options.filename_encoding, DEFAULT_FILENAME_ENCODING,
        "a table that names no encoding still gets the default through the merge"
    );
}

#[test]
fn the_merge_refuses_an_env_name_the_launcher_owns() {
    let root = Path::new("/w/app");
    let config = ProjectConfig {
        name: "app".to_owned(),
        version: None,
        tools: ToolsConfig {
            env: BTreeMap::from([("ERL_LIBS".to_owned(), "/opt/lib".to_owned())]),
            ..ToolsConfig::default()
        },
    };

    let error = BuildOptions::merge(root, &config, &no_flags(root))
        .expect_err("the merge must apply the env lint");

    assert!(
        matches!(&error, ConfigError::EnvName { name, .. } if name == "ERL_LIBS"),
        "expected ConfigError::EnvName, got {error:?}"
    );
}

#[test]
fn the_merge_refuses_an_encoding_the_emulator_has_no_flag_for() {
    let root = Path::new("/w/app");
    let config = ProjectConfig {
        name: "app".to_owned(),
        version: None,
        tools: ToolsConfig {
            filename_encoding: Some("ebcdic".to_owned()),
            ..ToolsConfig::default()
        },
    };

    let error = BuildOptions::merge(root, &config, &no_flags(root))
        .expect_err("the merge must apply the encoding lint");

    assert!(
        matches!(&error, ConfigError::FilenameEncoding { value, .. } if value == "ebcdic"),
        "expected ConfigError::FilenameEncoding, got {error:?}"
    );
}

// ------------------------------------------------- the target settings --

/// Merges `targets.toml` and the given flags against a project root.
fn merge_targets(root: &Path, flags: &BuildFlags) -> BuildOptions {
    let config = parse("targets.toml");
    BuildOptions::merge(root, &config, flags).expect("the merge succeeds")
}

/// `flags` as a `--target` list.
fn target_flags(root: &Path, targets: &[&str]) -> BuildFlags {
    BuildFlags {
        targets: targets.iter().map(|name| (*name).to_owned()).collect(),
        ..no_flags(root)
    }
}

/// The named target.
fn target(name: &str) -> Target {
    name.parse()
        .unwrap_or_else(|error| panic!("`{name}` must be a target: {error}"))
}

#[test]
fn a_sub_table_is_read_key_by_key() {
    let config = parse("targets.toml");
    let macos = config
        .tools
        .target
        .get("macos-aarch64")
        .expect("the macOS sub-table is read");

    assert_eq!(
        macos.erts_spec(),
        Ok(ErtsSourceSpec::Tarball(PathBuf::from(
            "/srv/otp-29.0.5-macos-aarch64.tar.zst"
        )))
    );
    assert_eq!(macos.otp_variant.as_deref(), Some("dynamic"));
    assert_eq!(
        macos
            .native
            .get("esqlite/priv/esqlite3_nif.so")
            .map(String::as_str),
        Some("vendor/esqlite3_nif-macos-aarch64.so"),
        "the native map is recorded now and read by the native milestone"
    );
    assert!(
        macos
            .codesign
            .as_ref()
            .and_then(|value| value.get("identity"))
            .and_then(toml::Value::as_str)
            .is_some_and(|identity| identity.starts_with("Developer ID Application")),
        "codesign is parsed loosely and kept whole: {:?}",
        macos.codesign
    );
}

#[test]
fn a_target_with_no_sub_table_takes_the_host_runtime() {
    let config = parse("targets.toml");

    assert_eq!(
        config.tools.target.get("linux-x86_64-musl"),
        None,
        "the fixture names three targets and not this one"
    );
    assert_eq!(
        ginary::config::TargetConfig::default().erts_spec(),
        Ok(ErtsSourceSpec::Host),
        "a target that says nothing bundles the runtime the machine already has"
    );
}

#[test]
fn a_sub_table_named_after_a_selection_rather_than_a_target_is_refused() {
    let error = refuse("bad_target_table.toml");

    assert!(
        matches!(&error, ConfigError::TargetTable { name, .. } if name == "host"),
        "expected ConfigError::TargetTable, got {error:?}"
    );
    assert!(
        error.to_string().contains("name a set"),
        "the message says why a selection cannot be configured: {error}"
    );
}

#[test]
fn a_sub_table_for_a_target_ginary_does_not_package_for_is_refused() {
    let error = refuse("bad_target_name.toml");

    assert!(
        matches!(
            &error,
            ConfigError::TargetTable { name, .. } if name == "linux-riscv64-gnu"
        ),
        "expected ConfigError::TargetTable, got {error:?}"
    );
}

#[test]
fn an_unknown_key_in_a_sub_table_names_the_table_and_the_key() {
    let error = refuse("bad_target_key.toml");

    assert!(
        matches!(
            &error,
            ConfigError::TargetKey { target, message, .. }
                if target == "macos-aarch64" && message.contains("codesing")
        ),
        "expected ConfigError::TargetKey naming both, got {error:?}"
    );
    let sentence = error.to_string();
    assert!(
        sentence.contains("[tools.ginary.target.macos-aarch64]"),
        "a manifest can hold seven of these tables, so the sentence names one: {sentence}"
    );
}

#[test]
fn an_erts_source_that_is_none_of_the_five_is_refused_by_the_manifest() {
    let error = refuse("bad_erts.toml");

    assert!(
        matches!(
            &error,
            ConfigError::ErtsSource { target, value, .. }
                if target == "linux-x86_64-musl" && value == "system"
        ),
        "expected ConfigError::ErtsSource, got {error:?}"
    );
    assert!(
        error.to_string().contains("dir:PATH"),
        "the message lists the spellings that would work: {error}"
    );
}

#[test]
fn an_otp_variant_that_is_neither_static_nor_dynamic_is_refused() {
    let error = refuse("bad_otp_variant.toml");

    assert!(
        matches!(
            &error,
            ConfigError::OtpVariant { target, value, .. }
                if target == "linux-x86_64-musl" && value == "hybrid"
        ),
        "expected ConfigError::OtpVariant, got {error:?}"
    );
}

#[test]
fn a_targets_list_that_names_no_target_is_refused_where_it_is_read() {
    // The list is refused by the manifest reader and not only by the build, so
    // that `ginary doctor` — which reads the table and never resolves it —
    // reports the entry nothing can build instead of printing a host row as
    // though the project had asked for one.
    let error = refuse("bad_targets_list.toml");

    assert!(
        matches!(
            &error,
            ConfigError::Target(TargetError::Unknown { name }) if name == "linux-riscv64-gnu"
        ),
        "expected ConfigError::Target, got {error:?}"
    );
}

#[test]
fn the_two_selections_that_are_not_targets_are_still_a_targets_list() {
    let config = parse_text("name = \"a\"\n\n[tools.ginary]\ntargets = [\"host\", \"all\"]\n");

    assert_eq!(config.tools.targets, ["host", "all"]);
}

#[test]
fn a_bare_build_targets_the_host_and_keeps_the_plain_artifact_name() {
    let root = Path::new("/w/plain_app");
    let config = parse("defaults.toml");
    let options = BuildOptions::merge(root, &config, &no_flags(root)).expect("the merge succeeds");

    assert_eq!(options.targets, [Target::host()]);
    assert!(
        !options.suffixed(),
        "the name a script already quotes does not move because a milestone landed"
    );
    assert_eq!(
        options.artifact_path(Target::host()),
        root.join(DEFAULT_OUTPUT).join("plain_app")
    );
    assert_eq!(options.manifest_copy_path(Target::host()), None);
}

#[test]
fn the_configured_targets_decide_when_no_flag_names_one() {
    let root = Path::new("/w/cross_app");
    let options = merge_targets(root, &no_flags(root));

    assert_eq!(
        options.targets,
        [Target::host(), target("linux-aarch64-musl")]
    );
    assert!(
        options.named_targets,
        "the table names `linux-aarch64-musl`, which is a target named"
    );
    assert!(
        options.suffixed(),
        "a project that builds for two targets cannot write both to one name"
    );
}

#[test]
fn a_target_flag_replaces_the_configured_list() {
    let root = Path::new("/w/cross_app");
    let options = merge_targets(root, &target_flags(root, &["macos-x86_64"]));

    assert_eq!(options.targets, [target("macos-x86_64")]);
    assert!(options.named_targets);
}

#[test]
fn an_explicit_host_target_still_gets_the_suffix_and_a_manifest_copy() {
    // The host's own canonical name is the case the rule has to be stated
    // for: it resolves to exactly the target a bare build produces, and the
    // file name still says so, because the user asked in as many words.
    let root = Path::new("/w/plain_app");
    let config = parse("defaults.toml");
    let host = Target::host();
    let options = BuildOptions::merge(root, &config, &target_flags(root, &[&host.name()]))
        .expect("the merge succeeds");

    assert_eq!(options.targets, [host]);
    assert!(options.suffixed());
    assert_eq!(
        options.artifact_path(host),
        root.join(DEFAULT_OUTPUT)
            .join(format!("plain_app-{}", host.name()))
    );
    assert_eq!(
        options.manifest_copy_path(host),
        Some(
            root.join(DEFAULT_OUTPUT)
                .join(format!("plain_app-{}.json", host.name()))
        )
    );
}

/// A manifest whose `[tools.ginary] targets` is exactly `entries`.
fn targets_text(entries: &[&str]) -> String {
    let list = entries
        .iter()
        .map(|entry| format!("\"{entry}\""))
        .collect::<Vec<_>>()
        .join(", ");
    format!("name = \"plain_app\"\n\n[tools.ginary]\ntargets = [{list}]\n")
}

#[test]
fn a_configured_target_name_puts_itself_in_the_file_name_the_flag_would() {
    // The same canonical name through the two spellings that reach a build.
    // `--target <host>` writes `<app>-<host>`, and a `targets` key holding
    // that one name is the same request made in the project rather than on
    // the command line: one target named, one name in the file.
    let root = Path::new("/w/plain_app");
    let host = Target::host();
    let config = parse_text(&targets_text(&[&host.name()]));
    let options = BuildOptions::merge(root, &config, &no_flags(root)).expect("the merge succeeds");

    assert_eq!(options.targets, [host]);
    assert!(
        options.suffixed(),
        "a target named in the table is named, and the artifact says which"
    );
    assert_eq!(
        options.artifact_path(host),
        root.join(DEFAULT_OUTPUT)
            .join(format!("plain_app-{}", host.name()))
    );
    assert_eq!(
        options.manifest_copy_path(host),
        Some(
            root.join(DEFAULT_OUTPUT)
                .join(format!("plain_app-{}.json", host.name()))
        )
    );
}

#[test]
fn selecting_the_host_names_no_target_and_keeps_the_plain_name() {
    // `host` is not a target name: it is the selection a build that names
    // nothing already makes, so spelling it out — in the table, which is what
    // the README's example does, or on the command line — changes nothing
    // about the build and must change nothing about the file name.
    let root = Path::new("/w/plain_app");
    let host = Target::host();
    let configured =
        BuildOptions::merge(root, &parse_text(&targets_text(&["host"])), &no_flags(root))
            .expect("the merge succeeds");
    let flagged = BuildOptions::merge(
        root,
        &parse("defaults.toml"),
        &target_flags(root, &["host"]),
    )
    .expect("the merge succeeds");

    for (spelling, options) in [("the table", configured), ("the flag", flagged)] {
        assert_eq!(options.targets, [host], "{spelling}");
        assert!(
            !options.suffixed(),
            "`host` through {spelling} selects the host and keeps `build/ginary/<app>`"
        );
        assert_eq!(
            options.artifact_path(host),
            root.join(DEFAULT_OUTPUT).join("plain_app"),
            "{spelling}"
        );
        assert_eq!(options.manifest_copy_path(host), None, "{spelling}");
    }
}

#[test]
fn a_windows_artifact_carries_its_suffix_and_its_manifest_copy_does_not() {
    let root = Path::new("/w/plain_app");
    let config = parse("defaults.toml");
    let windows = target("windows-x86_64");
    let options = BuildOptions::merge(root, &config, &target_flags(root, &["windows-x86_64"]))
        .expect("the merge succeeds");

    assert_eq!(
        options.artifact_path(windows),
        root.join(DEFAULT_OUTPUT)
            .join("plain_app-windows-x86_64.exe")
    );
    assert_eq!(
        options.manifest_copy_path(windows),
        Some(
            root.join(DEFAULT_OUTPUT)
                .join("plain_app-windows-x86_64.json")
        ),
        "the manifest copy is a JSON document whatever the artifact's suffix is"
    );
}

#[test]
fn the_erts_source_of_a_configured_target_is_the_one_it_names() {
    let root = Path::new("/w/cross_app");
    let options = merge_targets(root, &target_flags(root, &["linux-x86_64-gnu"]));

    assert_eq!(
        options.erts_spec(target("linux-x86_64-gnu")),
        Ok(ErtsSourceSpec::Dir(PathBuf::from("/opt/otp-29-gnu")))
    );
}

#[test]
fn a_relative_erts_directory_is_relative_to_the_project() {
    // `[tools.ginary.target.<name>] erts = "dir:vendor/otp"` describes the
    // project, as `output`, `vm_args` and `sys_config` do, so it is joined
    // onto the root rather than onto whatever directory `ginary build` was
    // typed in. A build from a subdirectory otherwise looked in a tree that
    // is not the one the manifest names.
    let root = Path::new("/w/cross_app");
    let host = Target::host();
    let config = parse_text(&format!(
        "name = \"cross_app\"\nversion = \"1.1.0\"\n\n[tools.ginary.target.{name}]\n\
         erts = \"dir:vendor/otp\"\n",
        name = host.name()
    ));
    let options = BuildOptions::merge(root, &config, &no_flags(root)).expect("the merge succeeds");

    assert_eq!(
        options.erts_spec(host),
        Ok(ErtsSourceSpec::Dir(root.join("vendor/otp")))
    );
}

#[test]
fn an_absolute_erts_path_is_left_exactly_as_it_was_written() {
    let root = Path::new("/w/cross_app");
    let options = merge_targets(root, &no_flags(root));

    assert_eq!(
        options.erts_spec(target("linux-x86_64-gnu")),
        Ok(ErtsSourceSpec::Dir(PathBuf::from("/opt/otp-29-gnu"))),
        "a path that is already absolute names one tree on this machine"
    );
    assert_eq!(
        options.erts_spec(target("macos-aarch64")),
        Ok(ErtsSourceSpec::Tarball(PathBuf::from(
            "/srv/otp-29.0.5-macos-aarch64.tar.zst"
        ))),
        "and the rule is the archive's as much as the directory's"
    );
}

#[test]
fn the_erts_source_of_an_unconfigured_target_is_the_host() {
    let root = Path::new("/w/cross_app");
    let options = merge_targets(root, &target_flags(root, &["macos-x86_64"]));

    assert_eq!(
        options.erts_spec(target("macos-x86_64")),
        Ok(ErtsSourceSpec::Host)
    );
}

#[test]
fn the_merge_refuses_a_target_flag_that_is_not_a_target() {
    let root = Path::new("/w/plain_app");
    let config = parse("defaults.toml");

    let error = BuildOptions::merge(root, &config, &target_flags(root, &["linux-riscv64-gnu"]))
        .expect_err("the merge resolves the targets and must refuse this one");

    assert!(
        matches!(
            &error,
            ConfigError::Target(TargetError::Unknown { name }) if name == "linux-riscv64-gnu"
        ),
        "expected ConfigError::Target, got {error:?}"
    );
}

#[test]
fn the_merge_carries_every_sub_table_through_to_the_build() {
    let root = Path::new("/w/cross_app");
    let options = merge_targets(root, &no_flags(root));

    let mut named: Vec<&str> = options.target_config.keys().map(String::as_str).collect();
    named.sort_unstable();
    assert_eq!(
        named,
        ["linux-aarch64-musl", "linux-x86_64-gnu", "macos-aarch64"],
        "a sub-table for a target this build does not produce is still carried, because the \
         list and the settings are two independent keys"
    );
}
