// SPDX-License-Identifier: MIT OR Apache-2.0
//! The SPDX 2.3 bill of materials.
//!
//! Two halves, split the way `tests/inspect.rs` splits. The document is a pure
//! function of a manifest, a payload digest and a list of packages, so the
//! shape is pinned by a snapshot over values a test wrote by hand and nothing
//! is read from disk. The rest runs against a
//! [`crate::common::artifact::SyntheticArtifact`], which is the
//! `ginary sbom <exe>` case: an artifact and no project around it, where every
//! download location the shipment cannot vouch for is `NOASSERTION`.
//!
//! Two gated tests at the end run the real `ginary build --sbom` and
//! `--sbom-out` over the `hello_ffi` fixture, because everything above them
//! reaches `sbom::for_artifact` directly and none of it touches the flag merge,
//! the project the build passes in, or the `sbom:` line the report ends with.
//!
//! The one thing every test here is really about is that the document is a
//! function of the artifact. An SPDX document needs a unique namespace, the
//! usual answer is a random UUID and a clock, and either would make the bill
//! of materials the one part of a reproducible build that is not reproducible.

mod common;

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use ginary::sbom::{
    self, DATA_LICENSE, DOCUMENT_SPDX_ID, HEX_PACKAGE_PREFIX, HexPackage, NAMESPACE_PREFIX,
    NOASSERTION, OTP_DOWNLOAD_LOCATION, OTP_LICENCE, OTP_PACKAGE_NAME, SPDX_VERSION,
};
use serde_json::Value;

use crate::common::artifact::{APP, OTP_VERSION, SyntheticArtifact, canonical_manifest};
use crate::common::built::BuiltProject;
use crate::common::tools::require_tools;

/// A `Command` for the `ginary` binary.
fn ginary() -> Command {
    Command::cargo_bin("ginary").expect("the `ginary` binary is built for tests")
}

/// A temporary directory the test owns.
fn tempdir() -> tempfile::TempDir {
    tempfile::tempdir().expect("a temporary directory")
}

/// A payload digest that is not all one byte, so a slice of it is visible.
fn digest() -> [u8; 32] {
    std::array::from_fn(|index| index as u8)
}

/// The two packages the fixture `manifest.toml` locks.
fn packages() -> Vec<HexPackage> {
    vec![
        HexPackage {
            name: "gleam_stdlib".to_owned(),
            version: "0.65.0".to_owned(),
            source: Some("hex".to_owned()),
        },
        HexPackage {
            name: "internal_thing".to_owned(),
            version: "1.0.0".to_owned(),
            source: None,
        },
    ]
}

/// A `manifest.toml` locking exactly [`packages`].
const MANIFEST_TOML: &str = r#"# Do not manually edit this file, it is managed by Gleam.
packages = [
  { name = "gleam_stdlib", version = "0.65.0", build_tools = ["gleam"], requirements = [], otp_app = "gleam_stdlib", source = "hex", outer_checksum = "AAAA" },
  { name = "internal_thing", version = "1.0.0", build_tools = ["gleam"], requirements = [] },
]

[requirements]
gleam_stdlib = { version = ">= 0.44.0 and < 2.0.0" }
"#;

// ------------------------------------------------------ the namespace --

#[test]
fn the_uuid_is_the_first_sixteen_bytes_of_the_payload_digest() {
    // Not a random version 4 UUID: the digest is already unique per artifact,
    // and a random one would make two runs over one file disagree.
    assert_eq!(
        sbom::uuid_from_sha256(&digest()),
        "00010203-0405-4607-8809-0a0b0c0d0e0f"
    );
}

#[test]
fn the_document_namespace_is_the_prefix_the_app_and_that_uuid() {
    assert_eq!(
        sbom::namespace("hello", "1.2.3", &digest()),
        format!("{NAMESPACE_PREFIX}/hello-1.2.3-00010203-0405-4607-8809-0a0b0c0d0e0f")
    );
}

#[test]
fn one_artifact_always_produces_one_namespace() {
    assert_eq!(
        sbom::namespace("hello", "1.2.3", &digest()),
        sbom::namespace("hello", "1.2.3", &digest())
    );
    assert_ne!(
        sbom::namespace("hello", "1.2.3", &digest()),
        sbom::namespace("hello", "1.2.3", &[0xff; 32])
    );
}

// ------------------------------------------------------- the document --

#[test]
fn the_document_is_this() {
    let document = sbom::build(&canonical_manifest(), &digest(), &packages());
    insta::assert_snapshot!(
        "sbom_document",
        sbom::to_json(&document).expect("the document serialises")
    );
}

#[test]
fn every_field_spdx_requires_is_present() {
    let document = sbom::build(&canonical_manifest(), &digest(), &packages());
    let value: Value = serde_json::from_str(&sbom::to_json(&document).expect("serialises"))
        .expect("the document is JSON");

    assert_eq!(value["spdxVersion"], SPDX_VERSION);
    assert_eq!(value["dataLicense"], DATA_LICENSE);
    assert_eq!(value["SPDXID"], DOCUMENT_SPDX_ID);
    assert_eq!(value["name"], format!("{APP}-1.2.3"));
    assert!(
        value["documentNamespace"]
            .as_str()
            .is_some_and(|namespace| namespace.starts_with(NAMESPACE_PREFIX)),
        "{}",
        value["documentNamespace"]
    );
    assert_eq!(
        value["creationInfo"]["created"], "2026-08-31T00:00:00Z",
        "the manifest's own timestamp, not a clock"
    );
    assert_eq!(
        value["creationInfo"]["creators"][0],
        format!("Tool: ginary-{}", canonical_manifest().ginary_version),
        "the ginary that built the artifact, not the one reading it"
    );
    for package in value["packages"].as_array().expect("packages") {
        for key in [
            "SPDXID",
            "name",
            "versionInfo",
            "downloadLocation",
            "filesAnalyzed",
            "licenseConcluded",
            "licenseDeclared",
        ] {
            assert!(!package[key].is_null(), "{key} is missing from {package}");
        }
    }
}

#[test]
fn the_document_describes_the_application_and_depends_on_everything_else() {
    let document = sbom::build(&canonical_manifest(), &digest(), &packages());

    let describes: Vec<&sbom::Relationship> = document
        .relationships
        .iter()
        .filter(|relationship| relationship.relationship_type == "DESCRIBES")
        .collect();
    assert_eq!(describes.len(), 1, "{:?}", document.relationships);
    assert_eq!(describes[0].spdx_element_id, DOCUMENT_SPDX_ID);

    let application = &describes[0].related_spdx_element;
    let depends: Vec<&str> = document
        .relationships
        .iter()
        .filter(|relationship| relationship.relationship_type == "DEPENDS_ON")
        .map(|relationship| {
            assert_eq!(&relationship.spdx_element_id, application);
            relationship.related_spdx_element.as_str()
        })
        .collect();
    assert_eq!(
        depends.len(),
        document.packages.len() - 1,
        "every package but the application itself: {:?}",
        document.packages
    );
}

#[test]
fn a_hex_package_points_at_hex_and_one_with_no_source_does_not() {
    let document = sbom::build(&canonical_manifest(), &digest(), &packages());

    let hex = document
        .packages
        .iter()
        .find(|package| package.name == "gleam_stdlib")
        .expect("the hex package is in the document");
    assert_eq!(hex.version_info, "0.65.0");
    assert_eq!(
        hex.download_location,
        format!("{HEX_PACKAGE_PREFIX}gleam_stdlib")
    );

    let unknown = document
        .packages
        .iter()
        .find(|package| package.name == "internal_thing")
        .expect("the package with no source is in the document");
    assert_eq!(
        unknown.download_location, NOASSERTION,
        "the shipment records no origin, so nothing is guessed"
    );
}

#[test]
fn the_runtime_is_a_package_of_its_own() {
    let document = sbom::build(&canonical_manifest(), &digest(), &packages());

    let otp = document
        .packages
        .iter()
        .find(|package| package.name == OTP_PACKAGE_NAME)
        .expect("the bundled runtime is a package");
    assert_eq!(otp.version_info, OTP_VERSION);
    assert_eq!(otp.license_concluded, OTP_LICENCE);
    assert_eq!(otp.download_location, OTP_DOWNLOAD_LOCATION);
}

// ------------------------------------------------- the Gleam manifest --

#[test]
fn the_packages_come_from_the_projects_manifest_toml() {
    let dir = tempdir();
    let path = dir.path().join("manifest.toml");
    std::fs::write(&path, MANIFEST_TOML).expect("the manifest is written");

    assert_eq!(
        sbom::read_manifest_toml(&path).expect("the manifest parses"),
        packages()
    );
}

#[test]
fn a_manifest_that_is_not_toml_names_the_file_and_what_the_parser_said() {
    let dir = tempdir();
    let path = dir.path().join("manifest.toml");
    std::fs::write(&path, "packages = [ oops\n").expect("the manifest is written");

    let error = sbom::read_manifest_toml(&path).expect_err("a broken manifest is refused");
    let message = error.to_string();
    assert!(message.contains("manifest.toml"), "{message}");
}

// ------------------------------------------------------- an artifact --

#[test]
fn an_artifact_with_no_project_around_it_asserts_nothing_about_origins() {
    let dir = tempdir();
    let artifact = SyntheticArtifact::build(dir.path());

    let document =
        sbom::for_artifact(artifact.path(), None).expect("an artifact alone is enough for a bill");

    assert_eq!(
        document.document_namespace,
        sbom::namespace(APP, "1.2.3", &artifact.packed().sha256)
    );
    let application = document
        .packages
        .iter()
        .find(|package| package.name == APP)
        .expect("the application is a package");
    assert_eq!(application.download_location, NOASSERTION);
    assert_eq!(application.license_concluded, NOASSERTION);
}

#[test]
fn two_runs_over_one_artifact_produce_the_same_bytes() {
    let dir = tempdir();
    let artifact = SyntheticArtifact::build(dir.path());

    let first = sbom::for_artifact(artifact.path(), None).expect("the first run");
    let second = sbom::for_artifact(artifact.path(), None).expect("the second run");

    assert_eq!(
        sbom::to_json(&first).expect("serialises"),
        sbom::to_json(&second).expect("serialises")
    );
}

#[test]
fn the_document_goes_beside_the_artifact_under_the_application_name() {
    let dir = tempdir();
    let artifact = dir.path().join("build/ginary/hello");

    assert_eq!(
        sbom::out_path(&artifact, "hello"),
        dir.path().join("build/ginary/hello.spdx.json")
    );
}

#[test]
fn a_project_beside_the_artifact_supplies_the_download_locations() {
    let dir = tempdir();
    let artifact = SyntheticArtifact::build(dir.path());
    let project = dir.path().join("project");
    std::fs::create_dir_all(&project).expect("the project directory");
    std::fs::write(project.join("manifest.toml"), MANIFEST_TOML).expect("the manifest");

    let document = sbom::for_artifact(artifact.path(), Some(&project))
        .expect("an artifact and a project are enough for a bill");

    let hex = document
        .packages
        .iter()
        .find(|package| package.name == "gleam_stdlib")
        .expect("the locked package reached the document");
    assert_eq!(hex.version_info, "0.65.0");
    assert_eq!(
        hex.download_location,
        format!("{HEX_PACKAGE_PREFIX}gleam_stdlib"),
        "the `manifest.toml` is the one place the origin is written down"
    );
}

#[test]
fn a_project_that_has_never_resolved_its_dependencies_is_not_a_failure() {
    // No `manifest.toml` at all is the NOASSERTION case, the same one
    // `ginary sbom <exe>` is in; every other unreadable manifest is an error.
    let dir = tempdir();
    let artifact = SyntheticArtifact::build(dir.path());
    let project = dir.path().join("unresolved");
    std::fs::create_dir_all(&project).expect("the project directory");

    let document = sbom::for_artifact(artifact.path(), Some(&project))
        .expect("a project with no lock file still has a bill of materials");

    assert_eq!(
        sbom::to_json(&document).expect("serialises"),
        sbom::to_json(&sbom::for_artifact(artifact.path(), None).expect("no project"))
            .expect("serialises"),
        "a project with nothing locked says exactly what an artifact alone says"
    );
}

#[test]
fn a_project_whose_manifest_is_unreadable_is_a_failure() {
    let dir = tempdir();
    let artifact = SyntheticArtifact::build(dir.path());
    let project = dir.path().join("broken");
    std::fs::create_dir_all(&project).expect("the project directory");
    std::fs::write(project.join("manifest.toml"), "packages = [ oops\n").expect("the manifest");

    let error = sbom::for_artifact(artifact.path(), Some(&project))
        .expect_err("a manifest that is there and unreadable is one this run was to use");

    assert!(error.to_string().contains("manifest.toml"), "{error}");
}

// -------------------------------------------------------- the command --

#[test]
fn sbom_writes_the_document_beside_the_artifact_and_says_where() {
    let dir = tempdir();
    let artifact = SyntheticArtifact::build(dir.path());
    let expected: PathBuf = dir.path().join(format!("{APP}.spdx.json"));

    let assert = ginary().arg("sbom").arg(artifact.path()).assert().success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8");

    assert_eq!(stdout, format!("sbom: {}\n", expected.display()));
    let written = std::fs::read_to_string(&expected).expect("the document was written");
    let value: Value = serde_json::from_str(&written).expect("the document is JSON");
    assert_eq!(value["spdxVersion"], SPDX_VERSION);
    assert!(written.ends_with('\n'), "the file ends in a newline");
}

#[test]
fn sbom_out_puts_the_document_where_it_was_asked_to() {
    let dir = tempdir();
    let artifact = SyntheticArtifact::build(dir.path());
    let destination = dir.path().join("elsewhere/bill.json");
    std::fs::create_dir_all(destination.parent().expect("a parent")).expect("the directory");

    ginary()
        .arg("sbom")
        .arg(artifact.path())
        .arg("--out")
        .arg(&destination)
        .assert()
        .success();

    assert!(destination.is_file(), "{}", destination.display());
}

#[test]
fn sbom_refuses_a_file_that_is_not_a_packaged_application() {
    let dir = tempdir();
    let plain = dir.path().join("notes.txt");
    std::fs::write(&plain, b"not an artifact\n").expect("the file is written");

    let assert = ginary().arg("sbom").arg(&plain).assert().failure();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf-8");

    assert!(stderr.contains("no ginary trailer"), "{stderr}");
}

// ------------------------------------------------------ a real build --

/// The programs a build of the fixture needs.
const TOOLS: [&str; 3] = ["gleam", "erl", "strip"];

/// The fixture these two tests build.
const FIXTURE: &str = "hello_ffi";

/// The version its `gleam.toml` declares.
const FIXTURE_VERSION: &str = "0.1.0";

#[test]
fn build_sbom_writes_the_document_beside_the_artifact_and_names_it_last() {
    let Some(_tools) = require_tools(&TOOLS) else {
        return;
    };
    let project = BuiltProject::copy(FIXTURE);

    let output = project.build_with(&["--sbom"], &[]);
    let stdout = String::from_utf8(output.stdout.clone()).expect("utf-8");
    let expected = project
        .artifact()
        .parent()
        .expect("the artifact has a directory")
        .join(format!("{FIXTURE}.spdx.json"));

    assert!(
        output.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        stdout.lines().last(),
        Some(format!("sbom: {}", expected.display()).as_str()),
        "the path a caller reads is the last line of the report:\n{stdout}"
    );
    let value: Value = serde_json::from_str(
        &std::fs::read_to_string(&expected).expect("the document was written"),
    )
    .expect("the document is JSON");
    assert_eq!(value["spdxVersion"], SPDX_VERSION);
    assert_eq!(
        value["name"],
        format!("{FIXTURE}-{FIXTURE_VERSION}"),
        "the document is named after the application the artifact carries"
    );
    assert!(
        value["packages"]
            .as_array()
            .expect("packages")
            .iter()
            .any(|package| package["name"] == OTP_PACKAGE_NAME),
        "the bundled runtime is a package of the real document too: {value}"
    );
}

#[test]
fn build_sbom_out_puts_the_document_where_it_was_asked_to() {
    let Some(_tools) = require_tools(&TOOLS) else {
        return;
    };
    let project = BuiltProject::copy(FIXTURE);
    let destination = project.root().join("elsewhere/bill.json");
    std::fs::create_dir_all(destination.parent().expect("a parent")).expect("the directory");

    let output = project.build_with(&["--sbom-out", &destination.display().to_string()], &[]);
    let stdout = String::from_utf8(output.stdout.clone()).expect("utf-8");

    assert!(
        output.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(destination.is_file(), "{}", destination.display());
    assert!(
        !project.artifact().with_extension("spdx.json").exists(),
        "`--sbom-out` is where the document goes, and the default is not also written"
    );
    assert_eq!(
        stdout.lines().last(),
        Some(format!("sbom: {}", destination.display()).as_str()),
        "{stdout}"
    );
}

#[test]
fn build_report_json_names_the_document_it_wrote() {
    let Some(_tools) = require_tools(&TOOLS) else {
        return;
    };
    let project = BuiltProject::copy(FIXTURE);
    let destination = project.root().join("machine-readable.spdx.json");

    let output = project.build_with(
        &[
            "--report",
            "json",
            "--sbom-out",
            &destination.display().to_string(),
        ],
        &[],
    );
    let stdout = String::from_utf8(output.stdout.clone()).expect("utf-8");

    assert!(
        output.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|error| panic!("`--report json` emits one JSON object: {error}\n{stdout}"));
    assert_eq!(
        report.get("sbom").and_then(Value::as_str),
        Some(destination.display().to_string().as_str()),
        "a machine consumer reads the path out of the report rather than re-deriving it: {report}"
    );
    assert!(
        destination.is_file(),
        "the path the report names is the document on disk: {}",
        destination.display()
    );
}

/// The absolute path of the `hello_ffi` fixture's own `manifest.toml`.
#[test]
fn the_zero_dependency_fixture_locks_no_packages() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hello_ffi/manifest.toml");

    assert_eq!(
        sbom::read_manifest_toml(&path).expect("the fixture manifest parses"),
        Vec::new(),
        "hello_ffi has no hex dependencies at all, which is the point of it"
    );
}
