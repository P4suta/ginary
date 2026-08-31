// SPDX-License-Identifier: MIT OR Apache-2.0
//! The payoff: a Linux artifact built for a machine this one is not, and run
//! inside a container that has no Erlang on it.
//!
//! Everything below C3 could be asserted on this machine. This cannot: a
//! `linux-aarch64-musl` artifact has to be *executed* somewhere, and the only
//! honest somewhere is a container. So every test here is gated on four things
//! at once, and each absence prints the task that produces it rather than
//! quietly passing:
//!
//! - `gleam`, `erl` and `docker` on `PATH` — `require_tools`;
//! - `dist/otp/catalog.json` — `mise run otp:repack`;
//! - a cross-built stub in `target/stubs` — `mise run stubs:build`;
//! - for the aarch64 rows, a registered `binfmt` handler —
//!   `mise run smoke:matrix` installs one.
//!
//! `scripts/smoke-matrix.sh` is the same matrix outside cargo, with a PASS/FAIL
//! table; this file is the half that can assert on the *build* as well as on
//! the run.
// The command line half of the suite: the build is a `cli` command.
#![cfg(feature = "cli")]

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use ginary::target::Target;

use crate::common::bounded::run_bounded;
use crate::common::fixture::FixtureProject;
use crate::common::stubfile::cross_stub;
use crate::common::tools::require_tools;

/// How long one cross build gets.
pub const BUILD_BUDGET: Duration = Duration::from_secs(900);

/// How long one container run gets.
pub const RUN_BUDGET: Duration = Duration::from_secs(180);

/// The catalogue the cross builds read, relative to the crate root.
pub const CATALOG: &str = "dist/otp/catalog.json";

/// The repository root.
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The committed catalogue, or a printed skip naming the task that writes it.
fn catalog() -> Option<PathBuf> {
    let path = root().join(CATALOG);
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

/// Whether `docker run --platform <platform>` can execute a foreign binary.
///
/// Answered by running `true` in a tiny image rather than by reading
/// `/proc/sys/fs/binfmt_misc`, because what matters is whether the daemon this
/// test will use can do it, not what this kernel has registered.
fn platform_runs(docker: &Path, platform: &str) -> bool {
    let mut command = Command::new(docker);
    command.args(["run", "--rm", "--platform", platform, "alpine:3.20", "true"]);
    run_bounded(&mut command, RUN_BUDGET, "a binfmt probe")
        .status
        .success()
}

/// Builds the `hello_ffi` fixture for `target` out of the committed catalogue.
///
/// Returns the artifact, which is `<project>/build/ginary/hello_ffi-<target>`.
fn build_for(target: &Target, catalog: &Path, stub: &Path, dir: &Path) -> PathBuf {
    let project = FixtureProject::copy("hello_ffi", dir);
    let config = project.dir().join("gleam.toml");
    let mut text = std::fs::read_to_string(&config).expect("the fixture's gleam.toml");
    // `target.<name>`, not `targets.<name>`: `targets` is the array of what to
    // build and `[tools.ginary.target.<name>]` is how to build one, which is
    // the split `src/config.rs` states and `tests/config.rs` pins.
    text.push_str(&format!(
        "\n[tools.ginary.target.\"{}\"]\nerts = \"catalog\"\n",
        target.name()
    ));
    std::fs::write(&config, text).expect("write the target section");

    let mut command = Command::new(assert_cmd::cargo::cargo_bin("ginary"));
    command
        .current_dir(project.dir())
        .env("GINARY_CATALOG", catalog)
        .env("GINARY_CACHE_DIR", dir.join("cache"))
        .args(["build", "--target", &target.name(), "--stub"])
        .arg(stub);
    let output = run_bounded(&mut command, BUILD_BUDGET, "`ginary build --target`");
    assert!(
        output.status.success(),
        "the cross build failed\n--- stdout ---\n{}\n--- stderr ---\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    let artifact = project
        .dir()
        .join(format!("build/ginary/hello_ffi-{}", target.name()));
    assert!(
        artifact.is_file(),
        "the artifact is named after the target: {}",
        artifact.display()
    );
    artifact
}

/// Runs `artifact` inside `image` with no network and no Erlang.
fn run_in_container(
    docker: &Path,
    image: &str,
    platform: Option<&str>,
    artifact: &Path,
) -> std::process::Output {
    let mut command = Command::new(docker);
    command.args(["run", "--rm", "--network", "none"]);
    if let Some(platform) = platform {
        command.args(["--platform", platform]);
    }
    command
        .arg("-v")
        .arg(format!("{}:/app:ro", artifact.display()))
        .args([image, "/app", "3", "a", "b"]);
    run_bounded(&mut command, RUN_BUDGET, "the artifact in a container")
}

/// Asserts the one thing every row of the matrix claims.
fn assert_hello(output: &std::process::Output) {
    let stdout = String::from_utf8_lossy(&output.stdout);
    // The same three claims `scripts/smoke-matrix.sh` and `tests/e2e_hello.rs`
    // make, spelled the same way: the argument vector arrived whole and in
    // order, and the application read its own `priv`.
    assert!(
        stdout.contains("args=3 a b"),
        "the packaged application saw its arguments, in order and all of them: {stdout}"
    );
    assert!(
        stdout.contains("hello from priv"),
        "and read its own priv out of the payload: {stdout}"
    );
    assert_eq!(
        output.status.code(),
        Some(3),
        "and its exit code crossed the container boundary\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn a_static_musl_artifact_runs_on_alpine_with_no_erlang_and_no_network() {
    let Some(tools) = require_tools(&["gleam", "erl", "docker"]) else {
        return;
    };
    let Some(catalog) = catalog() else {
        return;
    };
    let target: Target = "linux-x86_64-musl".parse().expect("a target");
    let Some(stub) = cross_stub(&target) else {
        return;
    };
    let dir = tempfile::tempdir().expect("a temporary directory");

    let artifact = build_for(&target, &catalog, &stub, dir.path());
    let verify = Command::new(assert_cmd::cargo::cargo_bin("ginary"))
        .args(["verify"])
        .arg(&artifact)
        .output()
        .expect("run `ginary verify`");
    assert!(
        verify.status.success(),
        "a freshly built artifact verifies clean: {}",
        String::from_utf8_lossy(&verify.stdout)
    );

    let output = run_in_container(tools.path("docker"), "alpine:3.20", None, &artifact);
    assert_hello(&output);
}

#[test]
fn an_aarch64_musl_artifact_runs_under_the_emulated_platform() {
    let Some(tools) = require_tools(&["gleam", "erl", "docker"]) else {
        return;
    };
    let Some(catalog) = catalog() else {
        return;
    };
    let target: Target = "linux-aarch64-musl".parse().expect("a target");
    let Some(stub) = cross_stub(&target) else {
        return;
    };
    if !platform_runs(tools.path("docker"), "linux/arm64") {
        eprintln!(
            "skipping: linux/arm64 is not registered with binfmt; run `mise run smoke:matrix`, \
             which installs it"
        );
        return;
    }
    let dir = tempfile::tempdir().expect("a temporary directory");

    let artifact = build_for(&target, &catalog, &stub, dir.path());
    let output = run_in_container(
        tools.path("docker"),
        "alpine:3.20",
        Some("linux/arm64"),
        &artifact,
    );
    assert_hello(&output);
}

#[test]
fn a_glibc_artifact_runs_on_the_oldest_debian_its_catalog_entry_allows() {
    let Some(tools) = require_tools(&["gleam", "erl", "docker"]) else {
        return;
    };
    let Some(catalog) = catalog() else {
        return;
    };
    let target: Target = "linux-x86_64-gnu".parse().expect("a target");
    let Some(stub) = cross_stub(&target) else {
        return;
    };
    let dir = tempfile::tempdir().expect("a temporary directory");

    let text = std::fs::read_to_string(&catalog).expect("the catalog");
    let parsed = ginary::catalog::Catalog::parse(&text, CATALOG).expect("the committed catalog");
    let entry = &parsed.otp["29.0.5"].targets["linux-x86_64-gnu"].variants["default"];
    let min = entry
        .libc
        .min
        .clone()
        .expect("a gnu entry has a libc floor");
    // debian:11 is glibc 2.31. A catalog entry that needs more than that has
    // to be run somewhere newer, and the image is chosen from the entry rather
    // than assumed, so a change upstream fails honestly instead of silently.
    // `max_glibc_version` reads the `GLIBC_x.y` spelling `.gnu.version_r`
    // holds and ignores anything else, so both sides are written that way.
    let needed = format!("{}{min}", ginary::elf::GLIBC_VERSION_PREFIX);
    let newest = ginary::elf::max_glibc_version([needed.as_str(), "GLIBC_2.31"]);
    let image = if min != "2.31" && newest.as_deref() == Some(min.as_str()) {
        "debian:12"
    } else {
        "debian:11"
    };

    let artifact = build_for(&target, &catalog, &stub, dir.path());
    let output = run_in_container(tools.path("docker"), image, None, &artifact);
    assert_hello(&output);
}
