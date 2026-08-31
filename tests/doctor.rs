// SPDX-License-Identifier: MIT OR Apache-2.0
//! What `ginary doctor` learned in B2: whether the cache directory can be
//! written to *and* executed out of, what project it is standing in, and what
//! the host OTP's `crypto` NIF needs.
//!
//! Every one of those is environment-shaped, and two of them cannot be
//! produced on demand: a `noexec` mount is not something a test may create,
//! and neither is a read-only home. So the probe is a value. [`CacheProbe`] is
//! what a test builds by hand to pin the rendering, and `probe_cache_dir` is
//! run once, honestly, against a directory the test owns — which is the half
//! that would catch a probe that reports success without trying anything.
//!
//! The project half needs no such seam: a Gleam project is a directory with a
//! `gleam.toml` in it, and `tests/common/project.rs` writes one.

mod common;

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use assert_cmd::Command;
use ginary::doctor::{
    self, CACHE_DIR_HINT, CacheProbe, ConfigStatus, CryptoReport, NativeObject, ProjectReport,
};
use ginary::elf::ElfKind;
use serde_json::Value;

use crate::common::fake_otp::FakeOtp;
use crate::common::project::TempProject;
use crate::common::repack::{foreign_machine, patch_elf_machine, test_binary};
use crate::common::tools::require_tools;

/// A `Command` for the `ginary` binary, run in `dir`.
fn ginary_in(dir: &Path) -> Command {
    let mut command = Command::cargo_bin("ginary").expect("the `ginary` binary is built for tests");
    command.current_dir(dir);
    command
}

/// A temporary directory the test owns.
fn tempdir() -> tempfile::TempDir {
    tempfile::tempdir().expect("a temporary directory")
}

/// Writes `bytes` at `<root>/<relative>`, creating the parents.
fn write(root: &Path, relative: &str, bytes: &[u8]) -> PathBuf {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("the parent directory");
    std::fs::write(&path, bytes).expect("the file is written");
    path
}

// --------------------------------------------------- the cache probe --

#[test]
fn a_directory_this_test_owns_passes_both_halves_of_the_probe() {
    // The honest run: a probe that reported success without trying anything
    // would pass every rendering test above and fail here only if it lied in
    // the other direction, so this is paired with the two below.
    let dir = tempdir();

    let probe = doctor::probe_cache_dir(dir.path());

    assert!(probe.writable, "{probe:?}");
    assert!(probe.executable, "{probe:?}");
    assert_eq!(probe.detail, None);
}

#[test]
fn the_probe_leaves_nothing_behind() {
    let dir = tempdir();

    let probe = doctor::probe_cache_dir(dir.path());

    // Asserted first, so that a probe which creates nothing cannot satisfy
    // "it cleaned up" by never having tried.
    assert!(probe.writable && probe.executable, "{probe:?}");
    let left: Vec<String> = std::fs::read_dir(dir.path())
        .expect("the directory lists")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        left,
        Vec::<String>::new(),
        "the probe cleans up after itself"
    );
}

#[test]
fn a_directory_that_cannot_be_written_to_renders_the_hint() {
    let probe = CacheProbe {
        writable: false,
        executable: false,
        detail: Some("Read-only file system (os error 30)".to_owned()),
    };

    let text = probe.render();

    assert!(text.contains("cache writable: no"), "{text}");
    assert!(
        text.contains("Read-only file system (os error 30)"),
        "what the operating system said travels verbatim:\n{text}"
    );
    assert!(text.contains(CACHE_DIR_HINT), "{text}");
}

#[test]
fn a_directory_that_cannot_be_executed_out_of_says_noexec() {
    let probe = CacheProbe {
        writable: true,
        executable: false,
        detail: Some("Permission denied (os error 13)".to_owned()),
    };

    let text = probe.render();

    assert!(text.contains("cache writable: yes"), "{text}");
    assert!(
        text.contains("cache executable: no (mounted noexec?)"),
        "a cache on a noexec mount is the failure users actually hit:\n{text}"
    );
    assert!(text.contains(CACHE_DIR_HINT), "{text}");
}

#[test]
fn a_working_cache_directory_renders_no_hint() {
    let probe = CacheProbe {
        writable: true,
        executable: true,
        detail: None,
    };

    let text = probe.render();

    assert_eq!(text, "cache writable: yes\ncache executable: yes\n");
}

// ------------------------------------------------- the project context --

#[test]
fn a_project_is_reported_exactly_where_there_is_a_gleam_toml() {
    let project = TempProject::named("notify");

    // Both halves in one test: a `project_context` that always answered
    // `None` would satisfy the negative on its own.
    assert!(
        doctor::project_context(project.root(), SystemTime::now()).is_some(),
        "the project directory is a project"
    );
    assert_eq!(
        doctor::project_context(project.outside(), SystemTime::now()),
        None,
        "its parent holds no gleam.toml"
    );
}

#[test]
fn a_project_is_reported_by_name_and_version() {
    let project = TempProject::new("name = \"notify\"\nversion = \"3.1.4\"\n");

    let report = doctor::project_context(project.root(), SystemTime::now())
        .expect("a directory with a gleam.toml is a project");

    assert_eq!(report.root, project.root());
    assert_eq!(report.name, "notify");
    assert_eq!(report.version.as_deref(), Some("3.1.4"));
    assert_eq!(report.shipment, None);
    assert_eq!(report.config, ConfigStatus::Absent);
    assert_eq!(report.native, Vec::new());
}

#[test]
fn the_project_is_found_from_a_directory_below_it() {
    let project = TempProject::named("notify");
    let deep = project.subdir("src/nested");

    let report = doctor::project_context(&deep, SystemTime::now())
        .expect("the search walks up, as `ginary build` does");

    assert_eq!(report.root, project.root());
}

#[test]
fn an_exported_shipment_is_reported_with_its_age() {
    let project = TempProject::named("notify");
    let shipment = project.empty_shipment();
    let now = SystemTime::now() + Duration::from_secs(3_600);

    let report = doctor::project_context(project.root(), now).expect("a project");

    let found = report.shipment.expect("the shipment is reported");
    assert_eq!(found.path, shipment);
    assert!(
        (3_590..=3_610).contains(&found.age_secs),
        "the age is measured against the `now` it was given: {}",
        found.age_secs
    );
}

#[test]
fn a_tools_ginary_table_that_parses_is_reported_as_such() {
    let project = TempProject::new(
        "name = \"notify\"\nversion = \"0.1.0\"\n\n[tools.ginary]\ncompression_level = 19\n",
    );

    let report = doctor::project_context(project.root(), SystemTime::now()).expect("a project");

    assert_eq!(report.config, ConfigStatus::Ok);
}

#[test]
fn a_tools_ginary_table_that_does_not_parse_is_shown_verbatim() {
    let project = TempProject::new(
        "name = \"notify\"\nversion = \"0.1.0\"\n\n[tools.ginary]\nnot_a_key = true\n",
    );

    let report = doctor::project_context(project.root(), SystemTime::now()).expect("a project");

    match &report.config {
        ConfigStatus::Error { message } => assert!(
            message.contains("not_a_key"),
            "serde names the key and a paraphrase would lose it: {message}"
        ),
        other => panic!("expected an error status, got {other:?}"),
    }
}

// ----------------------------------------------------- native code --

#[test]
fn a_real_elf_under_priv_is_listed_with_what_it_is() {
    let project = TempProject::named("notify");
    let shipment = project.empty_shipment();
    write(&shipment, "notify/priv/lib/nif.so", &test_binary());
    let host = ginary::elf::inspect_bytes(&test_binary()).expect("the test binary is ELF");

    let report = doctor::project_context(project.root(), SystemTime::now()).expect("a project");

    assert_eq!(
        report.native,
        vec![NativeObject {
            path: "notify/priv/lib/nif.so".to_owned(),
            machine: host.machine.clone(),
            kind: host.kind,
            needed: host.needed.clone(),
            matches_host: true,
        }]
    );
}

#[test]
fn an_object_for_another_machine_is_flagged() {
    let project = TempProject::named("notify");
    let shipment = project.empty_shipment();
    write(
        &shipment,
        "notify/priv/lib/nif.so",
        &patch_elf_machine(&test_binary(), foreign_machine()),
    );

    let report = doctor::project_context(project.root(), SystemTime::now()).expect("a project");

    assert_eq!(report.native.len(), 1, "{:?}", report.native);
    assert!(
        !report.native[0].matches_host,
        "an object for {} on a {} host: {:?}",
        report.native[0].machine,
        ginary::target::Target::host().arch.as_str(),
        report.native[0]
    );
}

#[test]
fn a_priv_file_that_is_not_elf_is_not_native_code() {
    let project = TempProject::named("notify");
    let shipment = project.empty_shipment();
    write(&shipment, "notify/priv/greeting.txt", b"hello\n");
    write(
        &shipment,
        "notify/priv/lib/wrapper.so",
        b"#!/bin/sh\nexit 0\n",
    );

    let report = doctor::project_context(project.root(), SystemTime::now()).expect("a project");

    assert_eq!(
        report.native,
        Vec::new(),
        "the magic decides, never the extension"
    );
}

#[test]
fn a_nif_installed_as_a_symlink_is_still_native_code() {
    // The walk takes `symlink_metadata` so that a directory link cannot make
    // it loop, which used to drop a symlinked *file* with no report at all.
    let project = TempProject::named("notify");
    let shipment = project.empty_shipment();
    write(&shipment, "notify/priv/lib/nif.so.1", &test_binary());
    std::os::unix::fs::symlink("nif.so.1", shipment.join("notify/priv/lib/nif.so"))
        .expect("the symlink");

    let report = doctor::project_context(project.root(), SystemTime::now()).expect("a project");

    assert_eq!(
        report
            .native
            .iter()
            .map(|object| object.path.as_str())
            .collect::<Vec<_>>(),
        ["notify/priv/lib/nif.so", "notify/priv/lib/nif.so.1"],
        "a NIF installed as a link is the file it points at"
    );
}

#[test]
fn a_symlink_that_points_at_a_directory_is_never_descended_into() {
    let project = TempProject::named("notify");
    let shipment = project.empty_shipment();
    write(&shipment, "notify/priv/lib/nif.so", &test_binary());
    std::os::unix::fs::symlink("..", shipment.join("notify/priv/loop")).expect("the symlink");

    let report = doctor::project_context(project.root(), SystemTime::now()).expect("a project");

    assert_eq!(report.native.len(), 1, "{:?}", report.native);
}

#[test]
fn the_project_block_names_every_subject() {
    let report = ProjectReport {
        root: PathBuf::from("/w/notify"),
        name: "notify".to_owned(),
        version: Some("3.1.4".to_owned()),
        shipment: Some(doctor::ShipmentReport {
            path: PathBuf::from("/w/notify/build/erlang-shipment"),
            age_secs: 3_600,
        }),
        config: ConfigStatus::Error {
            message: "unknown field `not_a_key`".to_owned(),
        },
        native: vec![NativeObject {
            path: "notify/priv/lib/nif.so".to_owned(),
            machine: "aarch64".to_owned(),
            kind: ElfKind::SharedObject,
            needed: vec!["libc.so.6".to_owned()],
            matches_host: false,
        }],
    };

    let text = report.render();

    assert!(text.contains("project: notify 3.1.4"), "{text}");
    assert!(text.contains("/w/notify"), "{text}");
    assert!(text.contains("shipment:"), "{text}");
    assert!(text.contains("unknown field `not_a_key`"), "{text}");
    assert!(text.contains("notify/priv/lib/nif.so"), "{text}");
    assert!(
        text.contains("aarch64"),
        "the machine an object was built for is the point of the table:\n{text}"
    );
}

// ---------------------------------------------------------- crypto --

#[test]
fn crypto_is_reported_exactly_when_the_installation_carries_it() {
    let bare = tempdir();
    let with_crypto = tempdir();
    let plain = FakeOtp::new().build_in(bare.path());
    let full = FakeOtp::new()
        .app_with("crypto", "5.9.2", |app| {
            app.priv_file("lib/crypto.so", &test_binary())
        })
        .build_in(with_crypto.path());

    // Both halves in one test: a `crypto_report` that always answered `None`
    // would satisfy the negative on its own.
    assert!(
        doctor::crypto_report(&full.root).is_some(),
        "the installation carries a crypto NIF"
    );
    assert_eq!(
        doctor::crypto_report(&plain.root),
        None,
        "a runtime assembled from ERTS binaries alone has no crypto"
    );
}

#[test]
fn the_crypto_nif_is_found_and_read() {
    let dir = tempdir();
    let otp = FakeOtp::new()
        .app_with("crypto", "5.9.2", |app| {
            app.priv_file("lib/crypto.so", &test_binary())
        })
        .build_in(dir.path());
    let host = ginary::elf::inspect_bytes(&test_binary()).expect("the test binary is ELF");

    let report = doctor::crypto_report(&otp.root).expect("the installation has a crypto NIF");

    assert_eq!(
        report.path,
        otp.app_dir("crypto").join("priv/lib/crypto.so")
    );
    assert_eq!(report.needed, host.needed);
}

#[test]
fn a_crypto_that_needs_only_a_c_runtime_is_the_portability_guarantee() {
    let report = CryptoReport {
        path: PathBuf::from("/opt/otp/lib/crypto-5.9.2/priv/lib/crypto.so"),
        needed: vec!["libc.so.6".to_owned()],
        statically_linked_openssl: true,
    };

    let text = report.render();

    assert!(text.contains("libc.so.6"), "{text}");
    assert!(
        text.contains("statically"),
        "an OTP built against a static OpenSSL is what makes an artifact portable:\n{text}"
    );
}

#[test]
fn a_crypto_that_needs_openssl_says_what_the_target_must_have() {
    let report = CryptoReport {
        path: PathBuf::from("/opt/otp/lib/crypto-5.9.2/priv/lib/crypto.so"),
        needed: vec!["libc.so.6".to_owned(), "libcrypto.so.3".to_owned()],
        statically_linked_openssl: false,
    };

    let text = report.render();

    assert!(text.contains("libcrypto.so.3"), "{text}");
    assert!(!text.contains("statically"), "{text}");
}

#[test]
fn the_host_installation_answers_the_crypto_question() {
    let Some(_tools) = require_tools(&["erl"]) else {
        return;
    };
    let info = ginary::otp::discover(None).expect("a usable OTP installation");

    let report = doctor::crypto_report(&info.root).expect("a real OTP carries crypto");

    assert!(report.path.starts_with(&info.root), "{:?}", report.path);
    assert!(!report.needed.is_empty(), "{report:?}");
}

// -------------------------------------------------------- the command --

#[test]
fn doctor_reports_the_cache_probe_and_the_project_it_stands_in() {
    let project = TempProject::named("notify");

    let assert = ginary_in(project.root())
        .arg("doctor")
        .env("GINARY_CACHE_DIR", project.outside().join("cache"))
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8");

    assert!(stdout.contains("cache writable: yes"), "{stdout}");
    assert!(stdout.contains("cache executable: yes"), "{stdout}");
    assert!(stdout.contains("project: notify"), "{stdout}");
}

#[test]
fn doctor_json_carries_the_new_members() {
    let project = TempProject::named("notify");

    let assert = ginary_in(project.root())
        .args(["doctor", "--json"])
        .env("GINARY_CACHE_DIR", project.outside().join("cache"))
        .assert()
        .success();
    let value: Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("the output is JSON");

    assert_eq!(value["cache_probe"]["writable"], true);
    assert_eq!(value["cache_probe"]["executable"], true);
    assert_eq!(value["project"]["name"], "notify");
    assert!(
        value["project"]["native"].is_array(),
        "{}",
        value["project"]
    );
}
