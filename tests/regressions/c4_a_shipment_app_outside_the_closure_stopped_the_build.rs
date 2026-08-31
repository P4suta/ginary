// SPDX-License-Identifier: MIT OR Apache-2.0
//! Native code in a shipment application the artifact never carries refused
//! the build.
//!
//! `bundle::build` scans the whole shipment once and handed every object it
//! found to `native::reconcile`, but what an artifact carries is the
//! *dependency closure* of the application being packaged — a subset.
//! `native_manifest_rows` was the one consumer that knew the two differ; the
//! other two did not.
//!
//! So a shipment application nothing depends on — a dependency of a dependency
//! that was dropped, a package left behind by an earlier export — carrying an
//! object for another machine stopped a build that would never have shipped it,
//! and an override written for one produced `the staged tree holds no ... to
//! replace`, which names a path the user never wrote and says nothing about the
//! real cause.
//!
//! The right behaviour: reconciliation is about what the artifact will carry.
//! An object in a shipment application the closure did not stage is not this
//! build's business.
//!
//! Gated on `gleam` and `erl`, because only a whole build has both a shipment
//! and a closure to tell apart.
#![cfg(feature = "cli")]

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use crate::common::bounded::run_bounded;
use crate::common::built::ginary_bin;
use crate::common::fixture::FixtureProject;
use crate::common::native::{gnu_interp, plant, shared_object};
use crate::common::repack::EM_AARCH64;
use crate::common::tools::require_tools;

/// How long the build gets.
const BUILD_BUDGET: Duration = Duration::from_secs(900);

/// The application nothing in the project depends on.
const GHOST: &str = "ghost/priv/lib/ghost.so";

#[test]
fn an_object_in_an_application_the_artifact_never_carries_is_not_the_builds_business() {
    let Some(tools) = require_tools(&["gleam", "erl"]) else {
        return;
    };
    let dir = tempfile::tempdir().expect("a temporary directory");
    let project = FixtureProject::copy("hello_ffi", dir.path());
    let shipment = project.export_shipment_with(tools.path("gleam"));
    // An aarch64 object, which this host is not, in an application no `.app`
    // file names: the export left it behind and the closure will not stage it.
    plant(
        &shipment,
        GHOST,
        &shared_object(EM_AARCH64, Some(&gnu_interp(EM_AARCH64))),
    );

    let output = build(project.dir(), dir.path());

    let said = format!(
        "--- stdout ---\n{}\n--- stderr ---\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert!(
        output.status.success(),
        "a host build is not refused over an object it does not ship:\n{said}"
    );
    assert!(
        !said.contains("ghost"),
        "and does not mention it either:\n{said}"
    );
}

/// Runs `ginary build` over the exported shipment, with a private cache.
fn build(project: &Path, dir: &Path) -> std::process::Output {
    let mut command = Command::new(ginary_bin());
    command
        .arg("build")
        .arg("--skip-export")
        .current_dir(project)
        .env("GINARY_CACHE_DIR", dir.join("cache"));
    run_bounded(&mut command, BUILD_BUDGET, "`ginary build`")
}
