// SPDX-License-Identifier: MIT OR Apache-2.0
//! `ginary build` over a shipment that carries native code.
//!
//! The claims here are the ones only a whole build can make: that the scan
//! reaches the shipment the export produced, that the refusals arrive before
//! an artifact is written, that `--allow-native-mismatch` and a `native`
//! override change the outcome, and that the manifest of the artifact says
//! what was really shipped.
//!
//! No real cross-built NIF is needed and none exists. The fixture's `priv`
//! gets a *copy of this test run's own binary*, which is a genuine ELF for
//! this machine; a cross-built stub from `target/stubs` stands in for an
//! object built for another target, a real file a real linker wrote; and where
//! the claim is about a *library* rather than a program the object is
//! fabricated by `tests/common/native.rs`, because every binary a modern
//! toolchain links is a position-independent `ET_DYN` and none of them is
//! something the emulator would ever `dlopen`.
//!
//! Gated four ways, each absence a printed skip naming the task that fixes it:
//! `gleam` and `erl` on `PATH`, the committed catalogue, and a cross-built
//! stub for the target under test.
// The command line half of the suite: `build` is a `cli` command.
#![cfg(feature = "cli")]

mod common;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::Duration;

use ginary::target::{Arch, Libc, Os, Target};
use serde_json::Value;

use crate::common::bounded::run_bounded;
use crate::common::built::ginary_bin;
use crate::common::fixture::FixtureProject;
use crate::common::native::{host_native_object, plant, shared_object};
use crate::common::repack::{EM_AARCH64, EM_X86_64, patch_elf_machine, test_binary};
use crate::common::stubfile::cross_stub;
use crate::common::tools::{Toolchain, require_tools};

/// How long one build gets.
const BUILD_BUDGET: Duration = Duration::from_secs(900);

/// The catalogue the cross builds read, relative to the crate root.
const CATALOG: &str = "dist/otp/catalog.json";

/// Where the planted object goes inside the shipment.
const PLANTED: &str = "hello_ffi/priv/lib/nif.so";

/// The committed catalogue, or a printed skip naming the task that writes it.
fn catalog() -> Option<PathBuf> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(CATALOG);
    if path.is_file() {
        return Some(path);
    }
    let required = std::env::var_os("GINARY_REQUIRE_TOOLCHAIN").is_some_and(|value| value == "1");
    assert!(
        !required,
        "{CATALOG} is not there and GINARY_REQUIRE_TOOLCHAIN=1 forbids skipping"
    );
    eprintln!("skipping: no {CATALOG}; run `mise run otp:repack`");
    None
}

/// A copy of the fixture whose shipment has been exported and planted in.
///
/// The export happens once, here, and every build below runs with
/// `--skip-export`: a build that re-exported would overwrite the object the
/// test just planted, and the scan would have nothing to find.
fn prepared(dir: &Path, tools: &Toolchain, object: &[u8]) -> FixtureProject {
    let project = FixtureProject::copy("hello_ffi", dir);
    let shipment = project.export_shipment_with(tools.path("gleam"));
    plant(&shipment, PLANTED, object);
    project
}

/// Runs `ginary build` in `project` with the catalogue and a private cache.
fn build(project: &FixtureProject, dir: &Path, args: &[&str]) -> Output {
    let mut command = Command::new(ginary_bin());
    command
        .arg("build")
        .arg("--skip-export")
        .args(args)
        .current_dir(project.dir())
        .env("GINARY_CACHE_DIR", dir.join("cache"));
    if let Some(catalog) = catalog() {
        command.env("GINARY_CATALOG", catalog);
    }
    run_bounded(&mut command, BUILD_BUDGET, "`ginary build`")
}

/// Everything the run said, for an assertion message.
fn said(output: &Output) -> String {
    format!(
        "--- stdout ---\n{}\n--- stderr ---\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    )
}

/// The `native` list of the artifact's own manifest.
fn manifest_native(artifact: &Path) -> Vec<Value> {
    let mut command = Command::new(ginary_bin());
    command.args(["inspect", "--json"]).arg(artifact);
    let output = run_bounded(&mut command, BUILD_BUDGET, "`ginary inspect --json`");
    assert!(output.status.success(), "{}", said(&output));
    let value: Value = serde_json::from_slice(&output.stdout).expect("inspect --json is JSON");
    value["manifest"]["native"]
        .as_array()
        .expect("the manifest carries a native list")
        .clone()
}

#[test]
fn a_host_build_records_the_native_code_it_shipped() {
    let Some(tools) = require_tools(&["gleam", "erl"]) else {
        return;
    };
    let dir = tempfile::tempdir().expect("a temporary directory");
    // The host's own native code, in the host's own container format, and not
    // the committed ELF fixture: this build targets the host, and a build that
    // is handed an ELF for a Windows target refuses it — correctly. See
    // `common::native::host_native_object`.
    let project = prepared(dir.path(), &tools, &host_native_object());

    let output = build(&project, dir.path(), &[]);

    assert!(output.status.success(), "{}", said(&output));
    let native = manifest_native(&project.dir().join("build/ginary/hello_ffi"));
    assert_eq!(native.len(), 1, "{native:?}");
    assert_eq!(
        native[0]["path"].as_str(),
        Some("lib/hello_ffi/priv/lib/nif.so"),
        "the manifest names the file as the extracted tree holds it"
    );
    assert_eq!(
        native[0]["machine"].as_str(),
        Some(Target::host().arch.as_str())
    );
    assert_eq!(native[0]["replaced"].as_bool(), Some(false));
}

#[test]
fn a_cross_build_refuses_native_code_for_another_machine() {
    let Some(tools) = require_tools(&["gleam", "erl"]) else {
        return;
    };
    let target = Target::new(Os::Linux, Arch::Aarch64, Libc::Musl);
    let Some(stub) = cross_stub(&target) else {
        return;
    };
    if catalog().is_none() {
        return;
    }
    let dir = tempfile::tempdir().expect("a temporary directory");
    let project = prepared(dir.path(), &tools, &test_binary());
    std::fs::write(
        project.dir().join("gleam.toml"),
        with_target(project.dir(), &target, ""),
    )
    .expect("the target section");

    let output = build(
        &project,
        dir.path(),
        &[
            "--target",
            &target.name(),
            "--stub",
            &stub.to_string_lossy(),
        ],
    );

    assert!(!output.status.success(), "{}", said(&output));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("does not match target linux-aarch64-musl")
            && stderr.contains("hello_ffi/priv/lib/nif.so")
            && stderr.contains("--allow-native-mismatch"),
        "the refusal names the object and the three ways out:\n{stderr}"
    );
    assert!(
        !project
            .dir()
            .join(format!("build/ginary/hello_ffi-{}", target.name()))
            .exists(),
        "no artifact is written for a build that was refused"
    );
}

#[test]
fn allowing_the_mismatch_builds_the_artifact_and_says_what_it_did() {
    let Some(tools) = require_tools(&["gleam", "erl"]) else {
        return;
    };
    // A *gnu* target, and the object planted for the machine it is not. The
    // flag waives a mismatch and nothing else: a musl target's runtime is the
    // catalog's static variant, and a static emulator carrying a shared object
    // is the refusal no flag lifts — which is the test below this one. So the
    // build that shows what `--allow-native-mismatch` does is one whose only
    // fault is the mismatch.
    let target = Target::new(Os::Linux, Arch::X86_64, Libc::Gnu);
    let Some(stub) = cross_stub(&target) else {
        return;
    };
    if catalog().is_none() {
        return;
    }
    let dir = tempfile::tempdir().expect("a temporary directory");
    let project = prepared(
        dir.path(),
        &tools,
        &patch_elf_machine(&test_binary(), EM_AARCH64),
    );
    std::fs::write(
        project.dir().join("gleam.toml"),
        with_target(project.dir(), &target, ""),
    )
    .expect("the target section");

    let output = build(
        &project,
        dir.path(),
        &[
            "--target",
            &target.name(),
            "--stub",
            &stub.to_string_lossy(),
            "--allow-native-mismatch",
        ],
    );

    assert!(output.status.success(), "{}", said(&output));
    let printed = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        printed.contains("does not match target linux-x86_64-gnu")
            && printed.contains("--allow-native-mismatch was given")
            && printed.contains("hello_ffi/priv/lib/nif.so"),
        "a build that shipped native code for another machine says which target it \
         does not match, that the flag is why it shipped anyway, and which file it \
         was:\n{printed}"
    );
}

#[test]
fn a_static_runtime_refuses_a_nif_it_could_never_load() {
    let Some(tools) = require_tools(&["gleam", "erl"]) else {
        return;
    };
    let target = Target::new(Os::Linux, Arch::X86_64, Libc::Musl);
    let Some(stub) = cross_stub(&target) else {
        return;
    };
    if catalog().is_none() {
        return;
    }
    let dir = tempfile::tempdir().expect("a temporary directory");
    // A *shared object* for the target's own machine, with no interpreter,
    // which is what a NIF built against musl looks like: the machine agrees
    // with the target and the C library is not written down anywhere in the
    // file. So nothing here is a mismatch, and the only thing wrong is the
    // runtime. It is fabricated rather than borrowed from `target/stubs`,
    // because a stub is a *program* — every one a modern toolchain links is a
    // position-independent `ET_DYN` — and a static runtime never has to open
    // one of those.
    let project = prepared(dir.path(), &tools, &shared_object(EM_X86_64, None));
    std::fs::write(
        project.dir().join("gleam.toml"),
        with_target(project.dir(), &target, "otp_variant = \"static\"\n"),
    )
    .expect("the target section");

    let output = build(
        &project,
        dir.path(),
        &[
            "--target",
            &target.name(),
            "--stub",
            &stub.to_string_lossy(),
            "--allow-native-mismatch",
        ],
    );

    assert!(!output.status.success(), "{}", said(&output));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("otp_variant") && stderr.contains("hello_ffi/priv/lib/nif.so"),
        "the refusal names the setting that fixes it, and --allow-native-mismatch \
         did not lift it:\n{stderr}"
    );
}

#[test]
fn an_override_replaces_the_object_and_the_manifest_says_so() {
    let Some(tools) = require_tools(&["gleam", "erl"]) else {
        return;
    };
    let target = Target::new(Os::Linux, Arch::X86_64, Libc::Gnu);
    let Some(stub) = cross_stub(&target) else {
        return;
    };
    if catalog().is_none() {
        return;
    }
    let dir = tempfile::tempdir().expect("a temporary directory");
    // Planted for the machine the target is not, and overridden by one that
    // is: the gnu stub is an x86_64 ELF with a glibc interpreter.
    let project = prepared(
        dir.path(),
        &tools,
        &patch_elf_machine(&test_binary(), EM_AARCH64),
    );
    let replacement = project.dir().join("vendor/nif.so");
    plant(
        project.dir(),
        "vendor/nif.so",
        &std::fs::read(&stub).expect("the cross-built stub"),
    );
    std::fs::write(
        project.dir().join("gleam.toml"),
        with_target(
            project.dir(),
            &target,
            "\n[tools.ginary.target.\"linux-x86_64-gnu\".native]\n\"hello_ffi/priv/lib/nif.so\" = \
             \"vendor/nif.so\"\n",
        ),
    )
    .expect("the target section");
    assert!(replacement.is_file(), "the override file is in the project");

    let output = build(
        &project,
        dir.path(),
        &[
            "--target",
            &target.name(),
            "--stub",
            &stub.to_string_lossy(),
        ],
    );

    assert!(output.status.success(), "{}", said(&output));
    let native = manifest_native(
        &project
            .dir()
            .join(format!("build/ginary/hello_ffi-{}", target.name())),
    );
    assert_eq!(native.len(), 1, "{native:?}");
    assert_eq!(native[0]["replaced"].as_bool(), Some(true));
    assert_eq!(native[0]["source"].as_str(), Some("override"));
    assert_eq!(native[0]["machine"].as_str(), Some("x86_64"));
}

/// The fixture's `gleam.toml` with a target sub-table appended.
///
/// `target.<name>`, not `targets.<name>`: `targets` is the array of what to
/// build and `[tools.ginary.target.<name>]` is how to build one.
fn with_target(project: &Path, target: &Target, extra: &str) -> String {
    let mut text = std::fs::read_to_string(project.join("gleam.toml"))
        .expect("the fixture's gleam.toml is readable");
    text.push_str(&format!(
        "\n[tools.ginary.target.\"{}\"]\nerts = \"catalog\"\n{extra}",
        target.name()
    ));
    text
}
