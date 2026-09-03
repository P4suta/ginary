// SPDX-License-Identifier: MIT OR Apache-2.0
//! The C3 scaffolding, held against the repository.
//!
//! `scripts/smoke-matrix.sh` runs the cross-Linux matrix outside cargo, and the
//! documents around it are what make a local-first catalogue usable by somebody
//! who did not write it. None of that is code a test can execute — the script
//! needs a docker daemon and three images — but all of it can rot silently, and
//! a claim nobody checks reads as evidence. This file is the same shape as
//! `tests/formal.rs`: it pins that the script is there, that it probes before
//! it installs, that the tasks which produce its inputs exist, that the
//! catalogue is committed and the tarballs are not, and that the four documents
//! say what the milestone promised.
// The script and the tasks are the command line half's; a launcher-only build
// ships none of it.
#![cfg(feature = "cli")]

mod common;

use std::path::PathBuf;
use std::process::Command;

use crate::common::tools::require_tools;

/// The repository root.
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Reads a repository file as text.
///
/// # Panics
///
/// If the file is not there, which is what these tests are about.
fn read(relative: &str) -> String {
    let path = root().join(relative);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()))
}

// -------------------------------------------------------- the script --

#[test]
fn the_smoke_matrix_script_is_committed_and_executable() {
    let path = root().join("scripts/smoke-matrix.sh");
    assert!(
        path.is_file(),
        "`mise run smoke:matrix` runs {}",
        path.display()
    );

    // As `tests/ci_matrix.rs`: a Windows checkout carries no mode bits, so the
    // claim is made where it can be observed.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path)
            .expect("the script")
            .permissions()
            .mode();
        assert_eq!(
            mode & 0o111,
            0o111,
            "a script mise runs directly has to be executable, not sourced by luck"
        );
    }
}

#[test]
fn the_script_probes_for_binfmt_before_it_installs_anything() {
    let script = read("scripts/smoke-matrix.sh");

    let probe = script
        .find("--platform linux/arm64")
        .expect("the script asks whether arm64 already runs");
    let install = script
        .find("tonistiigi/binfmt")
        .expect("and installs a handler when it does not");
    assert!(
        probe < install,
        "the probe comes first: a --privileged container is not run on a machine that does not \
         need one"
    );
    assert!(
        script.contains("--privileged"),
        "and when it does install one, it says so with the flag it needs"
    );
}

#[test]
fn the_script_runs_every_row_of_the_matrix_and_prints_a_table() {
    let script = read("scripts/smoke-matrix.sh");

    for needle in [
        "alpine:3.20",
        "linux/amd64",
        "linux/arm64",
        "debian:1",
        "--network none",
    ] {
        assert!(
            script.contains(needle),
            "the matrix covers {needle}: {script}"
        );
    }
    assert!(
        script.contains("PASS") && script.contains("FAIL"),
        "the script's answer is a table a person reads, not an exit code alone"
    );
}

#[test]
fn an_unreachable_docker_daemon_is_a_reported_skip_the_way_smoke_sh_treats_one() {
    let script = read("scripts/smoke-matrix.sh");

    assert!(
        script.contains("skipping:"),
        "the same rule `require_tools` follows: a skip says so"
    );
    assert!(
        script.contains("GINARY_REQUIRE_TOOLCHAIN"),
        "and CI can turn that skip into a failure"
    );
}

// --------------------------------------------------------- the tasks --

#[test]
fn the_two_tasks_that_produce_the_matrixs_inputs_exist() {
    let mise = read("mise.toml");

    for task in ["[tasks.\"smoke:matrix\"]", "[tasks.\"otp:repack\"]"] {
        assert!(mise.contains(task), "{task} is what a developer runs");
    }
    assert!(
        mise.contains("scripts/smoke-matrix.sh"),
        "the task runs the committed script rather than an inline copy of it"
    );
    assert!(
        mise.contains("otp repack"),
        "and the repack task drives the command, so the pipeline has one implementation"
    );
}

// ------------------------------------------------------ the catalogue --

#[test]
fn the_catalog_is_committed_and_the_tarballs_beside_it_are_not() {
    let Some(tools) = require_tools(&["git"]) else {
        return;
    };

    let ignored = |relative: &str| -> bool {
        Command::new(tools.path("git"))
            .current_dir(root())
            .args(["check-ignore", "-q", relative])
            .status()
            .expect("run git check-ignore")
            .success()
    };

    assert!(
        !ignored("dist/otp/catalog.json"),
        "the catalog is the reproducible record of what the pipeline produced, so it is committed"
    );
    assert!(
        ignored("dist/otp/otp-29.0.5-linux-x86_64-musl-static.tar.zst"),
        "and the 40 MB tarballs beside it are not"
    );
    assert!(ignored("dist/anything-else"));
}

// ------------------------------------------------------ the documents --

#[test]
fn the_adr_for_a_local_first_catalog_is_written_and_says_what_flips() {
    let adr = read("docs/adr/0013-local-first-otp-catalog.md");

    assert!(
        adr.contains("ginary-otp"),
        "the ADR names the hosted repository it is deferring"
    );
    for needle in ["local", "catalog.json", "sha256"] {
        assert!(adr.contains(needle), "the ADR covers {needle}");
    }
    assert!(
        read("docs/adr/README.md").contains("0013-local-first-otp-catalog"),
        "and the index lists it"
    );
}

#[test]
fn the_format_document_specifies_the_catalog_schema() {
    let format = read("docs/format.md");

    assert!(
        format.contains("schema_version"),
        "the catalog is a format ginary reads, so it belongs in docs/format.md"
    );
    for field in [
        "nif_loading",
        "excluded_apps",
        "upstream",
        "built_at",
        "linkage",
    ] {
        assert!(format.contains(field), "the schema section covers {field}");
    }
}

#[test]
fn the_readme_carries_the_cross_build_quickstart_and_the_two_caveats() {
    let readme = read("README.md");

    for needle in ["stubs:build", "otp repack", "--target", "--catalog"] {
        assert!(readme.contains(needle), "the quickstart names {needle}");
    }
    assert!(
        readme.contains("NIF") && readme.contains("static"),
        "the static musl build cannot dlopen a NIF, and a reader has to be told before they ship"
    );
    assert!(
        readme.contains("sha256"),
        "the trust model is that every claim is checked; say so where people read it"
    );
}

#[test]
fn the_testing_document_lists_the_targets_this_milestone_added() {
    let testing = read("docs/dev/testing.md");

    for target in [
        "tests/download.rs",
        "tests/catalog.rs",
        "tests/otp_repack.rs",
        "tests/otp_cli.rs",
        "tests/erts_source_catalog.rs",
        "tests/e2e_cross.rs",
        "tests/smoke_matrix.rs",
    ] {
        assert!(
            testing.contains(target),
            "a test target nothing documents is one nobody runs on purpose: {target}"
        );
    }

    // The helpers as well as the targets. A fixture builder with no paragraph
    // is one the next milestone rebuilds from scratch because nobody knew it
    // was there, which is what every other entry of `tests/common/` has a
    // paragraph to prevent.
    for helper in ["tests/common/http.rs", "tests/common/catalog.rs"] {
        assert!(
            testing.contains(helper),
            "a test helper nothing documents is one the next milestone writes again: {helper}"
        );
    }
    for name in [
        "TestServer",
        "CatalogBuilder",
        "FakeUpstream",
        "plant_cached_otp",
        "runtime_tarball",
    ] {
        assert!(
            testing.contains(name),
            "and the document says what each one builds: {name}"
        );
    }
}
