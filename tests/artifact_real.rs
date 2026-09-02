// SPDX-License-Identifier: MIT OR Apache-2.0
//! One real artifact, assembled by hand and run.
//!
//! Every other launcher test replaces the BEAM with a shell script, because
//! the launcher's contract is about paths, permissions, argv and the
//! environment and none of that needs an emulator. This file asks the only
//! question those tests cannot: does a real ERTS, extracted out of a real
//! payload by the real launcher, actually boot and run Gleam code?
//!
//! `ginary build` is milestone A4, so the artifact is assembled here out of
//! the pieces that already exist — export, closure, stage, strip, pack,
//! append — which is exactly the sequence `bundle.rs` will run. When that
//! command arrives this file becomes its first customer rather than being
//! replaced.
//!
//! Gated on `gleam`, `erl` and `strip`; a machine without them reports a skip,
//! and `GINARY_REQUIRE_TOOLCHAIN=1` turns the skip into a failure.
// The command line half of the suite: every claim in this file is about a
// module the `cli` feature carries, so a `--no-default-features` build has
// nothing here to run. See `docs/dev/log/C2.md`.
#![cfg(feature = "cli")]

mod common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use common::bounded::run_bounded;
use common::fixture::FixtureProject;
use common::tools::{Toolchain, require_tools};

use ginary::assemble::{StageOptions, StagedRoot, StagedSource};
use ginary::closure::app_dependency_closure;
use ginary::manifest::{AppRef, LaunchSpec, Manifest};
use ginary::strip::StripOptions;
use ginary::trailer::Trailer;

/// The fixture, the `-root` of the closure, and the `<app>` of every cache
/// path the artifact writes.
const APP: &str = "hello_ffi";

/// The zstd level a shipped artifact is packed at.
///
/// The size this file records in `docs/dev/log/A3b.md` is only meaningful at
/// the level a release uses, so the one real artifact the suite builds is
/// built the way a user's would be.
const LEVEL: i32 = 19;

/// How long one run of the real artifact gets, cold cache included.
const RUN_BUDGET: std::time::Duration = std::time::Duration::from_secs(120);

/// A built artifact and the temporary tree holding it.
struct RealArtifact {
    dir: tempfile::TempDir,
    path: PathBuf,
    stub_len: u64,
    payload_len: u64,
    staged: StagedRoot,
}

impl RealArtifact {
    /// The artifact's size in bytes: the number `docs/dev/log/A3b.md` records.
    fn size(&self) -> u64 {
        std::fs::metadata(&self.path)
            .expect("stat the artifact")
            .len()
    }

    /// Runs it with a cleared environment and a working directory of its own.
    fn run(&self, name: &str, args: &[&str]) -> (std::process::Output, PathBuf) {
        let home = self.dir.path().join(format!("{name}-home"));
        let cwd = self.dir.path().join(format!("{name}-cwd"));
        let empty = self.dir.path().join(format!("{name}-path"));
        for directory in [&home, &cwd, &empty] {
            std::fs::create_dir_all(directory).expect("a run directory");
        }

        let mut command = Command::new(&self.path);
        command
            .env_clear()
            .env("HOME", &home)
            .env("PATH", &empty)
            .env("XDG_CACHE_HOME", &home)
            .current_dir(&cwd)
            .args(args);
        common::coverage::preserve_coverage_env(&mut command);
        let output = run_bounded(&mut command, RUN_BUDGET, &format!("the {APP} artifact"));
        (output, cwd)
    }

    /// `<cache>/<app>`, where a crash dump lands.
    fn app_dir(&self, name: &str) -> PathBuf {
        self.dir
            .path()
            .join(format!("{name}-home"))
            .join("ginary")
            .join(APP)
    }
}

/// Exports, resolves, stages, strips, packs and appends.
fn build(tools: &Toolchain) -> RealArtifact {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let project = FixtureProject::copy(APP, dir.path());
    let shipment = project.export_shipment_with(tools.path("gleam"));

    let otp = ginary::otp::discover(None).expect("the host OTP installation");
    let set = app_dependency_closure(&shipment, &otp.lib, &[APP.to_owned()], &[])
        .expect("the fixture's closure resolves");
    let staged = ginary::assemble::stage(
        &set,
        &otp,
        &StageOptions::default(),
        &dir.path().join("staged"),
    )
    .expect("the fixture stages");

    ginary::strip::strip(staged.root(), &otp, &StripOptions::default()).expect("the tree strips");
    let staged = staged
        .refresh()
        .expect("the listing is refreshed after strip");

    let manifest = manifest_for(&staged);
    let mut payload = Vec::new();
    let packed =
        ginary::payload::pack(staged.root(), &manifest, LEVEL, &mut payload).expect("the pack");

    let stub = std::fs::read(env!("CARGO_BIN_EXE_ginary")).expect("read the ginary binary");
    let trailer = Trailer {
        payload_offset: stub.len() as u64,
        payload_len: packed.len,
        payload_sha256: packed.sha256,
    };

    // Not `<dir>/<APP>`: the fixture copy already owns that name.
    let bin = dir.path().join("bin");
    std::fs::create_dir_all(&bin).expect("a directory for the artifact");
    let path = bin.join(APP);
    let mut bytes = stub.clone();
    bytes.extend_from_slice(&payload);
    bytes.extend_from_slice(&trailer.to_bytes());
    std::fs::write(&path, &bytes).expect("write the artifact");
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("make the artifact executable");
    }

    RealArtifact {
        dir,
        path,
        stub_len: stub.len() as u64,
        payload_len: packed.len,
        staged,
    }
}

/// The manifest for a staged root, derived from what was actually staged.
fn manifest_for(staged: &StagedRoot) -> Manifest {
    let mut pa = Vec::new();
    let mut gleam_applications = Vec::new();
    let mut otp_applications = Vec::new();
    for app in staged.apps() {
        match app.source {
            StagedSource::Shipment => {
                pa.push(format!("{}/ebin", app.dir));
                gleam_applications.push(app.name.clone());
            }
            StagedSource::Otp => otp_applications.push(AppRef {
                name: app.name.clone(),
                vsn: app.vsn.clone(),
            }),
        }
    }
    // The root application first: it is the one whose modules resolve last on
    // a code path and the one whose `priv` the test reads.
    pa.sort_by_key(|entry| entry != &format!("lib/{APP}/ebin"));

    Manifest {
        format_version: ginary::manifest::FORMAT_VERSION,
        app: APP.to_owned(),
        app_version: "1.0.0".to_owned(),
        gleam_version: None,
        otp_release: staged.otp_release(),
        otp_version: staged.otp_version().to_owned(),
        erts_version: staged.erts_vsn().to_owned(),
        // The tree is assembled by hand rather than resolved through
        // `erts_source`, so nothing here read the emulator; the default is the
        // honest answer and is what an artifact built before C1 carries.
        otp: ginary::manifest::OtpProvenance::default(),
        target: ginary::target::Target::host(),
        otp_applications,
        gleam_applications,
        launch: LaunchSpec {
            program: "erlexec".to_owned(),
            bindir: format!("erts-{}/bin", staged.erts_vsn()),
            boot: "bin/no_dot_erlang".to_owned(),
            pa,
            eval: format!("'{APP}@@main':run('{APP}')"),
            erl_flags: Vec::new(),
            args_file: None,
            config: None,
            distribution: false,
            filename_encoding: ginary::config::DEFAULT_FILENAME_ENCODING.to_owned(),
            heart: false,
            env: BTreeMap::new(),
        },
        native: Vec::new(),
        created_at: "2026-08-31T00:00:00Z".to_owned(),
        ginary_version: env!("CARGO_PKG_VERSION").to_owned(),
        extra: BTreeMap::new(),
    }
}

fn names_in(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn a_real_artifact_runs_a_gleam_program_with_no_erlang_on_the_machine() {
    let Some(tools) = require_tools(&["gleam", "erl", "strip"]) else {
        return;
    };
    let artifact = build(&tools);

    let (output, cwd) = artifact.run("first", &["3", "a", "b"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        stdout.contains("args=3 a b"),
        "`-extra` did not reach init:get_plain_arguments/0:\n{stdout}\n--- stderr ---\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains("hello from priv"),
        "code:priv_dir/1 did not find the extracted priv:\n{stdout}"
    );
    assert!(
        stdout.contains(&format!(
            "cwd={}",
            std::fs::canonicalize(&cwd).expect("canonicalise").display()
        )),
        "the application did not start in the caller's working directory:\n{stdout}"
    );
    assert_eq!(
        output.status.code(),
        Some(3),
        "the application's own exit code must survive execve"
    );

    // A cold run and then a warm one: the second must be a hit, and it must
    // produce the same answer.
    let (again, _) = artifact.run("second", &["0"]);
    assert_eq!(again.status.code(), Some(0));
    assert_eq!(
        names_in(&artifact.app_dir("second")).len(),
        1,
        "the warm run must reuse the entry the cold one wrote"
    );
}

#[test]
fn a_real_artifact_reports_a_runtime_error_as_exit_one_and_leaves_the_cwd_clean() {
    let Some(tools) = require_tools(&["gleam", "erl", "strip"]) else {
        return;
    };
    let artifact = build(&tools);

    let (output, cwd) = artifact.run("crash", &["--crash"]);

    assert_eq!(
        output.status.code(),
        Some(1),
        "an uncaught error is Gleam's exit 1, not one of ginary's codes"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("runtime error"),
        "the crash was not reported as a Gleam runtime error:\n{stderr}"
    );
    assert_eq!(
        names_in(&cwd),
        Vec::<String>::new(),
        "the runtime must not write erl_crash.dump into the user's working directory"
    );
    for name in names_in(&artifact.app_dir("crash")) {
        assert!(
            name == "erl_crash.dump" || !name.starts_with('.'),
            "the application directory holds residue: {name}"
        );
    }
}

#[test]
fn the_real_artifact_is_one_file_and_its_size_is_recorded() {
    let Some(tools) = require_tools(&["gleam", "erl", "strip"]) else {
        return;
    };
    let artifact = build(&tools);
    let size = artifact.size();

    assert_eq!(
        size,
        artifact.stub_len + artifact.payload_len + 64,
        "the artifact is exactly the stub, the payload and the 64-byte trailer"
    );
    // The number the milestone log records. Printed rather than asserted
    // against a constant: a size gate that fails on a new ERTS release is a
    // gate that gets deleted, and the budget belongs in the release checks.
    println!(
        "artifact: {size} bytes = stub {} + payload {} + trailer 64 (zstd -{LEVEL}, {} staged files)",
        artifact.stub_len,
        artifact.payload_len,
        artifact.staged.files().len()
    );
    assert!(
        size < 64 * 1024 * 1024,
        "a single-file Gleam application must not be {size} bytes"
    );
}
