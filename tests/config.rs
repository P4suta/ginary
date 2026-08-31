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

use std::path::{Path, PathBuf};

use ginary::config::{
    self, BuildFlags, BuildOptions, ConfigError, DEFAULT_COMPRESSION_LEVEL, DEFAULT_OUTPUT,
    ProjectConfig, ToolsConfig,
};
use ginary::strip::StripOptions;

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
