// SPDX-License-Identifier: MIT OR Apache-2.0
//! `ginary.json` and `ginary.index.json`.
//!
//! The manifest is the one thing the launcher reads before it does anything,
//! so what is asserted here is its *compatibility*: the field order it is
//! written in, that an unknown key survives a round trip, and that a version
//! this build cannot act on is a typed error rather than a missing field. The
//! index is asserted against a staging root, because its categories come from
//! the listing rather than from a second guess at what a file is.

mod common;

use std::collections::BTreeMap;
use std::ffi::OsString;

use common::payload::{sample_launch, sample_manifest, sha256_hex, staging_tree};
use ginary::assemble::Category;
use ginary::manifest::{
    EnvSnapshot, FORMAT_VERSION, INDEX_NAME, Index, IndexError, MANIFEST_NAME, Manifest,
    ManifestError, OtpProvenance, created_at,
};

#[test]
fn a_manifest_round_trips_through_its_json() {
    let manifest = sample_manifest();

    let json = serde_json::to_string(&manifest).expect("serialise");
    let parsed: Manifest = serde_json::from_str(&json).expect("parse");

    assert_eq!(parsed, manifest);
}

#[test]
fn the_manifest_serialises_its_fields_in_the_order_the_format_document_prints() {
    let json = serde_json::to_string_pretty(&sample_manifest()).expect("serialise");

    insta::assert_snapshot!("canonical_manifest_json", json);
}

#[test]
fn a_manifest_written_before_the_otp_block_still_parses() {
    // The compatibility promise `docs/format.md` makes for the C1 addition:
    // `otp` is additive and `format_version` stayed at 1, so an artifact built
    // before the block reads back as the default rather than failing to parse.
    // The document states the four values, so they are asserted rather than
    // compared to `OtpProvenance::default()` alone.
    let json = serde_json::to_value(sample_manifest()).expect("serialise");
    let mut object = json.as_object().expect("a JSON object").clone();
    assert!(
        object.remove("otp").is_some(),
        "the key this test removes has to be there to begin with"
    );

    let parsed: Manifest = serde_json::from_value(serde_json::Value::Object(object))
        .expect("an artifact built before C1 is one this ginary can still read");

    assert_eq!(parsed.otp, OtpProvenance::default());
    assert_eq!(parsed.otp.linkage, "unknown");
    assert_eq!(parsed.otp.libc, None);
    assert!(
        parsed.otp.nif_loading,
        "every artifact that predates the block bundled a dynamically linked host runtime"
    );
    assert_eq!(parsed.otp.source, "unknown");
}

#[test]
fn the_two_payload_entries_are_named_by_the_format() {
    assert_eq!(MANIFEST_NAME, "ginary.json");
    assert_eq!(INDEX_NAME, "ginary.index.json");
    assert_eq!(FORMAT_VERSION, 1);
}

#[test]
fn a_key_this_build_does_not_know_survives_a_round_trip() {
    let mut object = serde_json::to_value(sample_manifest()).expect("to value");
    object["signature"] = serde_json::json!({ "alg": "ed25519", "sig": "AAAA" });
    let json = serde_json::to_string(&object).expect("serialise");

    let parsed: Manifest = serde_json::from_str(&json).expect("a newer key is not an error");

    assert_eq!(
        parsed.extra.get("signature"),
        Some(&serde_json::json!({ "alg": "ed25519", "sig": "AAAA" })),
        "the unknown key landed in `extra`"
    );
    let written = serde_json::to_value(&parsed).expect("re-serialise");
    assert_eq!(
        written["signature"],
        serde_json::json!({ "alg": "ed25519", "sig": "AAAA" }),
        "and is written back out at the top level"
    );
}

#[test]
fn check_version_accepts_the_version_this_build_writes() {
    sample_manifest()
        .check_version()
        .expect("the version this build writes is readable");
}

#[test]
fn a_newer_format_version_parses_and_is_then_refused_by_check_version() {
    let mut object = serde_json::to_value(sample_manifest()).expect("to value");
    object["format_version"] = serde_json::json!(2);
    let json = serde_json::to_string(&object).expect("serialise");

    let parsed: Manifest = serde_json::from_str(&json).expect("parsing stays permissive");
    assert_eq!(parsed.format_version, 2);

    let error = parsed
        .check_version()
        .expect_err("a version this build cannot act on is refused");

    assert_eq!(
        error,
        ManifestError::UnsupportedVersion {
            found: 2,
            supported: FORMAT_VERSION,
        }
    );
}

#[test]
fn the_launch_spec_of_a_manifest_this_build_writes_validates() {
    sample_launch().validate().expect("the sample spec is safe");
}

#[test]
fn an_absolute_launch_path_is_refused() {
    let spec = ginary::manifest::LaunchSpec {
        bindir: "/opt/erts-17.0.5/bin".to_owned(),
        ..sample_launch()
    };

    let error = spec.validate().expect_err("an absolute path is refused");

    assert_eq!(
        error,
        ManifestError::UnsafePath {
            field: "launch.bindir".to_owned(),
            value: "/opt/erts-17.0.5/bin".to_owned(),
        }
    );
}

#[test]
fn a_launch_path_that_climbs_out_of_the_root_is_refused() {
    let spec = ginary::manifest::LaunchSpec {
        pa: vec!["lib/hello/ebin".to_owned(), "../../etc".to_owned()],
        ..sample_launch()
    };

    let error = spec.validate().expect_err("a `..` component is refused");

    assert_eq!(
        error,
        ManifestError::UnsafePath {
            field: "launch.pa[1]".to_owned(),
            value: "../../etc".to_owned(),
        },
        "the field name says which element of `pa` it was"
    );
}

#[test]
fn a_launch_path_separated_by_backslashes_is_refused() {
    let spec = ginary::manifest::LaunchSpec {
        boot: "bin\\no_dot_erlang".to_owned(),
        ..sample_launch()
    };

    let error = spec.validate().expect_err("a backslash is not a separator");

    assert_eq!(
        error,
        ManifestError::UnsafePath {
            field: "launch.boot".to_owned(),
            value: "bin\\no_dot_erlang".to_owned(),
        }
    );
}

#[test]
fn an_empty_launch_path_is_refused() {
    let spec = ginary::manifest::LaunchSpec {
        boot: String::new(),
        ..sample_launch()
    };

    let error = spec.validate().expect_err("an empty path is refused");

    assert_eq!(
        error,
        ManifestError::UnsafePath {
            field: "launch.boot".to_owned(),
            value: String::new(),
        }
    );
}

#[test]
fn a_launch_program_that_is_a_path_rather_than_a_name_is_refused() {
    let spec = ginary::manifest::LaunchSpec {
        program: "erts-17.0.5/bin/erlexec".to_owned(),
        ..sample_launch()
    };

    let error = spec
        .validate()
        .expect_err("`program` is a name inside `bindir`, not a path");

    assert_eq!(
        error,
        ManifestError::UnsafePath {
            field: "launch.program".to_owned(),
            value: "erts-17.0.5/bin/erlexec".to_owned(),
        },
        "this is the shape the format decision moved away from: `bindir` and `program` are \
         assembled, not stored twice"
    );
}

#[test]
fn a_launch_path_holding_a_dot_component_is_refused() {
    let spec = ginary::manifest::LaunchSpec {
        pa: vec!["lib/./hello/ebin".to_owned()],
        ..sample_launch()
    };

    let error = spec.validate().expect_err("a `.` component is refused");

    assert_eq!(
        error,
        ManifestError::UnsafePath {
            field: "launch.pa[0]".to_owned(),
            value: "lib/./hello/ebin".to_owned(),
        }
    );
}

#[test]
fn a_launch_path_with_an_empty_component_is_refused() {
    let spec = ginary::manifest::LaunchSpec {
        bindir: "erts-17.0.5//bin".to_owned(),
        ..sample_launch()
    };

    let error = spec
        .validate()
        .expect_err("a doubled separator is an empty component");

    assert_eq!(
        error,
        ManifestError::UnsafePath {
            field: "launch.bindir".to_owned(),
            value: "erts-17.0.5//bin".to_owned(),
        }
    );
}

#[test]
fn created_at_formats_the_seconds_it_is_given_as_rfc_3339_in_utc() {
    let stamp = created_at(&EnvSnapshot::default(), 1_788_179_696).expect("a plain second count");

    assert_eq!(stamp, "2026-08-31T12:34:56Z");
}

#[test]
fn created_at_gets_a_leap_day_right() {
    let stamp = created_at(&EnvSnapshot::default(), 951_782_400).expect("a plain second count");

    assert_eq!(
        stamp, "2000-02-29T00:00:00Z",
        "2000 is a leap year and 1900 was not"
    );
}

#[test]
fn created_at_honours_source_date_epoch_over_the_clock_it_is_given() {
    let env = EnvSnapshot {
        source_date_epoch: Some(OsString::from("0")),
    };

    let stamp = created_at(&env, 1_788_179_696).expect("the override is a second count");

    assert_eq!(stamp, "1970-01-01T00:00:00Z");
}

#[test]
fn an_empty_source_date_epoch_is_an_unset_one() {
    let env = EnvSnapshot {
        source_date_epoch: Some(OsString::new()),
    };

    let stamp = created_at(&env, 1_788_179_696)
        .expect("an exported-but-empty variable did not ask for a fixed timestamp");

    assert_eq!(
        stamp, "2026-08-31T12:34:56Z",
        "the clock the caller passed is used, which is the rule `cache_dir::resolve` follows"
    );
}

#[test]
fn a_source_date_epoch_that_is_not_a_second_count_is_an_error_rather_than_ignored() {
    let env = EnvSnapshot {
        source_date_epoch: Some(OsString::from("yesterday")),
    };

    let error = created_at(&env, 1_788_179_696)
        .expect_err("a build asked to be reproducible does not quietly stop being one");

    assert_eq!(
        error,
        ManifestError::InvalidSourceDateEpoch {
            value: "yesterday".to_owned(),
        }
    );
}

#[test]
fn the_index_hashes_every_file_the_listing_names_and_keeps_its_category() {
    let dir = tempfile::tempdir().expect("tempdir");
    let tree = staging_tree(dir.path());

    let index = Index::from_staged(&tree.root, tree.files()).expect("the tree is readable");

    let paths: Vec<&str> = index.files.iter().map(|file| file.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "bin/no_dot_erlang.boot",
            "erts-17.0.5/bin/erlexec",
            "lib/hello/ebin/hello.app",
            "lib/hello/ebin/hello.beam",
            "lib/hello/priv/greeting.txt",
        ],
        "sorted by path, and `ginary.stage.json` is not among them"
    );

    let boot = &index.files[0];
    assert_eq!(boot.size, 17);
    assert_eq!(boot.mode, 0o644);
    assert_eq!(boot.category, Category::Boot);
    assert_eq!(boot.sha256, sha256_hex(b"boot script bytes"));

    let erlexec = &index.files[1];
    assert_eq!(erlexec.mode, 0o755, "the execute bit is part of the index");
    assert_eq!(erlexec.category, Category::ErtsBinary);
    assert_eq!(erlexec.sha256, sha256_hex(b"#!/bin/sh\nexit 0\n"));

    assert_eq!(index.files[3].category, Category::GleamBeam);
    assert_eq!(index.files[4].category, Category::Priv);
}

#[test]
fn a_file_the_listing_names_and_the_tree_no_longer_holds_is_an_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    let tree = staging_tree(dir.path());
    let missing = tree.root.join("lib/hello/ebin/hello.beam");
    std::fs::remove_file(&missing).expect("remove");

    let error =
        Index::from_staged(&tree.root, tree.files()).expect_err("a missing file is not skipped");

    match error {
        IndexError::Io { path, .. } => assert_eq!(path, missing),
        other => panic!("expected IndexError::Io, got {other:?}"),
    }
}

#[test]
fn an_index_round_trips_through_its_json() {
    let dir = tempfile::tempdir().expect("tempdir");
    let tree = staging_tree(dir.path());
    let index = Index::from_staged(&tree.root, tree.files()).expect("the tree is readable");

    let json = serde_json::to_string(&index).expect("serialise");
    let parsed: Index = serde_json::from_str(&json).expect("parse");

    assert_eq!(parsed, index);
}

#[test]
fn a_manifest_carrying_no_unknown_keys_writes_none() {
    let manifest = Manifest {
        extra: BTreeMap::new(),
        ..sample_manifest()
    };

    let object = serde_json::to_value(&manifest).expect("to value");
    let keys: Vec<&str> = object
        .as_object()
        .expect("an object")
        .keys()
        .map(String::as_str)
        .collect();

    assert_eq!(
        keys,
        [
            "app",
            "app_version",
            "created_at",
            "erts_version",
            "format_version",
            "ginary_version",
            "gleam_applications",
            "gleam_version",
            "launch",
            "native",
            "otp",
            "otp_applications",
            "otp_release",
            "otp_version",
            "target",
        ],
        "`serde_json::Value` sorts them; the wire order is the snapshot's"
    );
}

#[test]
fn the_manifest_this_build_writes_validates() {
    sample_manifest()
        .validate()
        .expect("the sample manifest is safe");
}

/// Asserts that `app` is refused as an application name.
///
/// The application name is the `<app>` component of `<cache>/<app>/<key>`, so
/// a value that is not a single path component is a manifest that chose a
/// directory outside the cache for the launcher to create, chmod 0700 and
/// extract into.
fn an_app_is_refused(app: &str) {
    let manifest = Manifest {
        app: app.to_owned(),
        ..sample_manifest()
    };

    let error = manifest
        .validate()
        .expect_err("an application name that is not one path component is refused");

    assert_eq!(
        error,
        ManifestError::UnsafePath {
            field: "app".to_owned(),
            value: app.to_owned(),
        }
    );
}

#[test]
fn an_application_name_that_walks_out_of_the_cache_is_refused() {
    an_app_is_refused("..");
    an_app_is_refused("../../etc");
}

#[test]
fn an_application_name_with_a_separator_is_refused() {
    an_app_is_refused("a/b");
    an_app_is_refused("hello/../..");
}

#[test]
fn an_absolute_application_name_is_refused() {
    an_app_is_refused("/etc");
}

#[test]
fn an_empty_application_name_is_refused() {
    an_app_is_refused("");
}

#[test]
fn a_current_directory_application_name_is_refused() {
    an_app_is_refused(".");
}

// ------------------------------------ the additive launch fields (B1) --

#[test]
fn a_manifest_written_before_the_runtime_fields_still_reads() {
    // The whole claim behind keeping `format_version` at 1: every field B1
    // adds is optional on the way in, so an artifact built by an older ginary
    // parses and takes the documented defaults.
    let json = serde_json::to_string(&sample_manifest()).expect("the sample manifest serialises");
    let value: serde_json::Value = serde_json::from_str(&json).expect("it is JSON");
    let mut object = value.as_object().cloned().expect("a JSON object");
    let launch = object
        .get_mut("launch")
        .and_then(serde_json::Value::as_object_mut)
        .expect("the launch object");
    for key in [
        "args_file",
        "config",
        "distribution",
        "filename_encoding",
        "heart",
        "env",
    ] {
        launch.remove(key);
    }
    let older = serde_json::Value::Object(object).to_string();

    let parsed: Manifest = serde_json::from_str(&older).expect("an older manifest still parses");

    assert_eq!(parsed.launch.args_file, None);
    assert_eq!(parsed.launch.config, None);
    assert!(!parsed.launch.distribution);
    assert!(!parsed.launch.heart);
    assert_eq!(parsed.launch.env, std::collections::BTreeMap::new());
    assert_eq!(
        parsed.launch.filename_encoding, "utf8",
        "the default has to be the encoding every artifact so far was built with"
    );
    assert_eq!(parsed.format_version, FORMAT_VERSION);
    assert_eq!(
        parsed.launch.validate(),
        Ok(()),
        "a manifest with no optional launch paths in it validates"
    );

    // And the fields, once they are there, are checked like every other path:
    // an older manifest is not a way past `LaunchSpec::validate`.
    let escaping = ginary::manifest::LaunchSpec {
        args_file: Some("../vm.args".to_owned()),
        ..parsed.launch.clone()
    };
    assert_eq!(
        escaping.validate(),
        Err(ManifestError::UnsafePath {
            field: "launch.args_file".to_owned(),
            value: "../vm.args".to_owned(),
        })
    );
}

#[test]
fn an_args_file_that_is_not_root_relative_is_refused() {
    let spec = ginary::manifest::LaunchSpec {
        args_file: Some("/etc/vm.args".to_owned()),
        ..sample_launch()
    };

    let error = spec
        .validate()
        .expect_err("an absolute args file is refused");

    assert_eq!(
        error,
        ManifestError::UnsafePath {
            field: "launch.args_file".to_owned(),
            value: "/etc/vm.args".to_owned(),
        }
    );
}

#[test]
fn a_config_that_climbs_out_of_the_root_is_refused() {
    let spec = ginary::manifest::LaunchSpec {
        config: Some("../../etc/sys".to_owned()),
        ..sample_launch()
    };

    let error = spec.validate().expect_err("a `..` component is refused");

    assert_eq!(
        error,
        ManifestError::UnsafePath {
            field: "launch.config".to_owned(),
            value: "../../etc/sys".to_owned(),
        }
    );
}
