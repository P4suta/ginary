// SPDX-License-Identifier: MIT OR Apache-2.0
//! Finding the Gleam project and exporting it.
//!
//! Everything but the last two tests runs against directories the test wrote,
//! so no Gleam installation is needed to pin where the project is, what
//! `--skip-export` looks for, or how a version line is read. The two that do
//! run `gleam` are gated on it, because they are the only ones that can say
//! whether the export ginary asks for is the one Gleam performs.

mod common;

use ginary::diag::Diag;
use ginary::gleam::{self, GleamError, MANIFEST_NAME, ProjectDir, SHIPMENT_DIR};

use crate::common::fixture::FixtureProject;
use crate::common::project::TempProject;
use crate::common::tools::require_tools;

// -------------------------------------------------------- finding it --

#[test]
fn a_project_is_the_directory_holding_gleam_toml() {
    let project = TempProject::named("here");

    let found = gleam::find_project(project.root()).expect("the project is found");

    assert_eq!(found.root(), project.root());
    assert_eq!(found.manifest(), project.root().join(MANIFEST_NAME));
    assert_eq!(found.shipment(), project.root().join(SHIPMENT_DIR));
}

#[test]
fn the_search_walks_up_from_a_subdirectory_of_the_project() {
    let project = TempProject::named("deep");
    let inner = project.subdir("src/nested/further");

    let found = gleam::find_project(&inner).expect("the project is found from below");

    assert_eq!(found.root(), project.root());
}

#[test]
fn the_nearest_project_wins_over_the_one_above_it() {
    let outer = TempProject::named("outer");
    let inner_root = outer.subdir("vendor/inner");
    std::fs::write(
        inner_root.join(MANIFEST_NAME),
        "name = \"inner\"\nversion = \"0.1.0\"\n",
    )
    .expect("the inner manifest");

    let found = gleam::find_project(&inner_root).expect("the inner project is found");

    assert_eq!(
        found.root(),
        inner_root,
        "a nested project belongs to itself, not to the tree it sits in"
    );
}

#[test]
fn a_directory_with_no_project_above_it_names_where_the_search_began() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let start = dir.path().join("a/b");
    std::fs::create_dir_all(&start).expect("the starting directory");

    let error = gleam::find_project(&start).expect_err("there is no project here");

    let GleamError::NoProject { start: named } = &error else {
        panic!("expected GleamError::NoProject, got {error:?}");
    };
    assert_eq!(named, &start);
    let message = error.to_string();
    assert!(
        message.contains(MANIFEST_NAME) && message.contains("Gleam project"),
        "the message must say what was looked for and where to run the command: {message}"
    );
}

// --------------------------------------------------------- skip-export --

#[test]
fn skip_export_reuses_the_shipment_that_is_already_there() {
    let project = TempProject::named("reuse");
    let shipment = project.empty_shipment();
    let found = ProjectDir::new(project.root().to_path_buf());

    let used = gleam::existing_shipment(&found).expect("the existing export is used");

    assert_eq!(used, shipment);
}

#[test]
fn skip_export_without_an_export_says_how_to_make_one() {
    let project = TempProject::named("empty");
    let found = ProjectDir::new(project.root().to_path_buf());

    let error = gleam::existing_shipment(&found).expect_err("there is no export to reuse");

    let GleamError::MissingShipment { path } = &error else {
        panic!("expected GleamError::MissingShipment, got {error:?}");
    };
    assert_eq!(path, &project.root().join(SHIPMENT_DIR));
    let message = error.to_string();
    assert!(
        message.contains("gleam export erlang-shipment") && message.contains("--skip-export"),
        "the message must say both ways out of it: {message}"
    );
}

// ------------------------------------------------------------ version --

#[test]
fn the_version_gleam_prints_is_read_and_nothing_else_is_a_version() {
    // Both halves in one test on purpose: a parser that answers `None` to
    // everything satisfies every negative case on its own, and a claim about
    // what is *not* a version is only worth something beside the claim about
    // what is.
    assert_eq!(
        gleam::parse_version("gleam 1.18.1\n").as_deref(),
        Some("1.18.1")
    );
    assert_eq!(
        gleam::parse_version("gleam 1.2.3").as_deref(),
        Some("1.2.3")
    );
    assert_eq!(
        gleam::parse_version("gleam 1.19.0-rc1\n").as_deref(),
        Some("1.19.0-rc1"),
        "a pre-release is a version too; the field is recorded, never compared"
    );

    for output in ["", "\n", "gleam\n", "1.18.1\n", "erl 29\n", "   \n"] {
        assert_eq!(
            gleam::parse_version(output),
            None,
            "`{output:?}` is not a version line"
        );
    }
}

// ---------------------------------------------------- the real export --

#[test]
fn exporting_the_fixture_writes_the_shipment_directory() {
    let Some(tools) = require_tools(&["gleam"]) else {
        return;
    };
    let _ = tools;
    let dir = tempfile::tempdir().expect("a temporary directory");
    let fixture = FixtureProject::copy("hello_ffi", dir.path());
    let project = ProjectDir::new(fixture.dir().to_path_buf());

    let shipment =
        gleam::export_shipment(&project, &Diag::disabled()).expect("the fixture exports");

    assert_eq!(shipment, project.shipment());
    assert!(
        shipment.join("hello_ffi/ebin/hello_ffi.app").is_file(),
        "the export must hold the application it was run for: {}",
        shipment.display()
    );
}

#[test]
fn a_gleam_that_fails_hands_its_own_diagnosis_through_verbatim() {
    let Some(tools) = require_tools(&["gleam"]) else {
        return;
    };
    let _ = tools;
    let dir = tempfile::tempdir().expect("a temporary directory");
    let fixture = FixtureProject::copy("hello_ffi", dir.path());
    // A Gleam type error. What matters is not which error it is but that the
    // compiler's own words survive: a summarised Gleam diagnostic is worse
    // than none.
    std::fs::write(
        fixture.dir().join("src/hello_ffi.gleam"),
        "pub fn main() -> Nil {\n  this_function_does_not_exist()\n}\n",
    )
    .expect("break the fixture");
    let project = ProjectDir::new(fixture.dir().to_path_buf());

    let error = gleam::export_shipment(&project, &Diag::disabled())
        .expect_err("a project that does not compile does not export");

    let GleamError::Export { dir: named, stderr } = &error else {
        panic!("expected GleamError::Export, got {error:?}");
    };
    assert_eq!(named, fixture.dir());
    assert!(
        stderr.contains("this_function_does_not_exist"),
        "gleam's own diagnosis must travel verbatim:\n{stderr}"
    );
    assert!(
        !project.shipment().exists(),
        "a failed export must not leave a shipment behind at {}",
        project.shipment().display()
    );
}

#[test]
fn the_version_of_the_gleam_on_path_is_read() {
    let Some(tools) = require_tools(&["gleam"]) else {
        return;
    };
    let _ = tools;

    let version = gleam::gleam_version().expect("a `gleam` on PATH reports its version");

    assert!(
        version.chars().next().is_some_and(|c| c.is_ascii_digit()),
        "a version starts with a number, and this is `{version}`"
    );
    assert!(
        !version.starts_with("gleam"),
        "the program name must be stripped, and this is `{version}`"
    );
}
