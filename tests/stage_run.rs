// SPDX-License-Identifier: MIT OR Apache-2.0
//! Booting a staged root.
//!
//! Everything else in the suite asserts on trees and values. This file is the
//! only place that asks the question the whole project is about: does the
//! directory `assemble::stage` writes actually run? It stages the zero-hex
//! fixture `hello_ffi` against the host OTP installation and launches it
//! through [`crate::common::erl::run_staged`], which is a hermetic subset of
//! the launch contract ADR 0003 records — it clears the environment where the
//! launcher will scrub a denylist from an inherited one.
//!
//! Every test is gated on `gleam` and `erl`, and a machine without them reports
//! a skip. `GINARY_REQUIRE_TOOLCHAIN=1` turns that skip into a failure, which
//! is what CI sets: this is the coverage that would be worth the most and cost
//! the least to lose silently.
// The command line half of the suite: every claim in this file is about a
// module the `cli` feature carries, so a `--no-default-features` build has
// nothing here to run. See `docs/dev/log/C2.md`.
#![cfg(feature = "cli")]

mod common;

use std::path::Path;

use ginary::assemble::{Category, StageOptions, StagedRoot};
use ginary::closure::app_dependency_closure;
use ginary::report::SizeReport;
use ginary::strip::{StripOptions, StripReport};
use tempfile::TempDir;

use crate::common::erl::{crash_dump_path, run_cwd, run_staged};
use crate::common::fixture::FixtureProject;
use crate::common::tools::{Toolchain, require_tools};

/// The application the fixture ships, and the `-root` the closure starts from.
const APP: &str = "hello_ffi";

/// A staged `hello_ffi`, and the temporary directory holding everything.
struct Staged {
    dir: TempDir,
    root: StagedRoot,
}

impl Staged {
    /// The staged tree.
    fn root(&self) -> &Path {
        self.root.root()
    }

    /// A fresh `HOME` for one run, so two runs cannot see each other.
    fn home(&self, name: &str) -> std::path::PathBuf {
        let home = self.dir.path().join(name);
        std::fs::create_dir_all(&home).expect("a home directory");
        home
    }
}

/// Exports the fixture, resolves the closure and stages it.
///
/// The three steps `ginary build` will run in this order, with nothing faked:
/// a real `gleam export erlang-shipment`, the real host OTP root, and the real
/// assembly. A failure in any of them is a failure of this test.
fn stage_hello_ffi(tools: &Toolchain) -> Staged {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let project = FixtureProject::copy(APP, dir.path());
    let shipment = project.export_shipment_with(tools.path("gleam"));

    let otp = ginary::otp::discover(None).expect("the host OTP installation");
    let set = app_dependency_closure(&shipment, &otp.lib, &[APP.to_owned()], &[])
        .expect("the fixture's closure resolves");

    let root = ginary::assemble::stage(
        &set,
        &otp,
        &StageOptions::default(),
        &dir.path().join("staged"),
    )
    .expect("the fixture stages");

    Staged { dir, root }
}

#[test]
fn a_staged_hello_ffi_prints_its_arguments_and_its_priv_file() {
    let Some(tools) = require_tools(&["gleam", "erl"]) else {
        return;
    };
    let staged = stage_hello_ffi(&tools);
    let home = staged.home("run");

    let output = run_staged(staged.root(), APP, &["3", "a", "b"], &home);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        stdout.contains("args=3 a b"),
        "`-extra` did not reach init:get_plain_arguments/0:\n{stdout}\n--- stderr ---\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains("hello from priv"),
        "code:priv_dir/1 did not find the staged priv:\n{stdout}"
    );
    assert!(
        stdout.contains(&format!("cwd={}", run_cwd(&home).display())),
        "the application did not start in the directory it was given:\n{stdout}"
    );
    assert_eq!(
        output.status.code(),
        Some(3),
        "the first argument is the exit code"
    );
}

#[test]
fn a_staged_hello_ffi_exits_zero_when_the_first_argument_is_zero() {
    let Some(tools) = require_tools(&["gleam", "erl"]) else {
        return;
    };
    let staged = stage_hello_ffi(&tools);

    let output = run_staged(staged.root(), APP, &["0"], &staged.home("run"));

    assert_eq!(output.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&output.stdout).contains("args=0"));
}

#[test]
fn a_crash_exits_one_and_leaves_no_dump_in_the_working_directory() {
    let Some(tools) = require_tools(&["gleam", "erl"]) else {
        return;
    };
    let staged = stage_hello_ffi(&tools);
    let home = staged.home("run");

    let output = run_staged(staged.root(), APP, &["--crash"], &home);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(output.status.code(), Some(1), "stderr:\n{stderr}");
    assert!(
        stderr.contains("runtime error"),
        "Gleam's own error report did not reach standard error:\n{stderr}"
    );
    assert!(
        !run_cwd(&home).join("erl_crash.dump").exists(),
        "a crash dump in the user's working directory is litter; ERL_CRASH_DUMP points into HOME"
    );
    if crash_dump_path(&home).exists() {
        // Not required — the runtime writes one only for some failures — but
        // if there is one it belongs where the launch contract put it.
        assert!(crash_dump_path(&home).is_file());
    }
}

#[test]
fn the_staged_root_holds_no_sources_and_the_kernel_the_boot_file_names() {
    let Some(tools) = require_tools(&["gleam", "erl"]) else {
        return;
    };
    let staged = stage_hello_ffi(&tools);

    // Scoped to the top level of an application, which is the rule assembly
    // actually has: `ebin` and `priv` are copied and the rest of the
    // application directory is not. A name matched at any depth would forbid
    // `priv/mibs/*.bin`, which a real `snmp` needs at run time.
    for app in staged.root.apps() {
        for excluded in ginary::assemble::EXCLUDED_APP_DIRS {
            let prefix = format!("{}/{excluded}/", app.dir);
            assert!(
                !staged
                    .root
                    .files()
                    .iter()
                    .any(|file| file.path.starts_with(&prefix)),
                "`{prefix}` was staged out of a real OTP tree"
            );
        }
    }

    let kernel = staged
        .root
        .boot_refs()
        .iter()
        .find(|dir| dir.starts_with("kernel-"))
        .expect("the boot file names a kernel version")
        .clone();
    assert!(
        staged
            .root()
            .join("lib")
            .join(&kernel)
            .join("ebin")
            .is_dir(),
        "the boot file requires lib/{kernel}/ebin and the staged tree must hold exactly it"
    );
}

#[test]
fn running_the_staged_root_does_not_change_a_byte_of_it() {
    let Some(tools) = require_tools(&["gleam", "erl"]) else {
        return;
    };
    let staged = stage_hello_ffi(&tools);

    let before: Vec<Vec<u8>> = staged
        .root
        .files()
        .iter()
        .map(|file| std::fs::read(staged.root().join(&file.path)).expect("a staged file"))
        .collect();

    let first = run_staged(staged.root(), APP, &["0"], &staged.home("run-one"));
    let second = run_staged(staged.root(), APP, &["0"], &staged.home("run-two"));

    assert_eq!(first.status.code(), Some(0));
    assert_eq!(second.status.code(), Some(0));

    let after: Vec<Vec<u8>> = staged
        .root
        .files()
        .iter()
        .map(|file| std::fs::read(staged.root().join(&file.path)).expect("a staged file"))
        .collect();
    assert_eq!(
        before, after,
        "a cache entry is immutable; running out of it must not write to it"
    );
}

#[test]
fn the_staged_hello_ffi_has_bytes_in_every_category_an_artifact_needs() {
    let Some(tools) = require_tools(&["gleam", "erl"]) else {
        return;
    };
    let staged = stage_hello_ffi(&tools);
    let totals = staged.root.bytes_by_category();

    for category in [
        Category::ErtsBinary,
        Category::Boot,
        Category::OtpBeam,
        Category::GleamBeam,
        Category::AppResource,
        Category::Priv,
    ] {
        let (bytes, files) = totals
            .get(&category)
            .copied()
            .unwrap_or_else(|| panic!("nothing was staged as {category}"));
        assert!(files > 0 && bytes > 0, "{category} is empty");
    }

    // The first real size number the project has. `docs/dev/log/A1c.md`
    // records it; printing it here is how it is kept honest.
    eprintln!("{}", staged.root.explain());
}

// ---------------------------------------------------------------------------
// A2: the same tree, stripped. This is the project's first size measurement,
// and the only place a real `beam.smp` and a real `beam_lib:strip_files/1`
// meet each other. `docs/dev/log/A2.md` records the numbers it prints.
// ---------------------------------------------------------------------------

/// The ceiling the whole staged tree has to come under once stripped.
///
/// A real OTP 29 runtime plus `hello_ffi` is around 200 MB unstripped, and the
/// milestone's target is a fifth of that before compression. The number is a
/// budget rather than a measurement: it fails when a change puts the artifact
/// back in the class of thing nobody wants to download.
const TOTAL_BUDGET: u64 = 25_000_000;

/// The ceiling `beam.smp` alone has to come under once stripped.
const BEAM_SMP_BUDGET: u64 = 15_000_000;

/// A stripped `hello_ffi`, with the account of what stripping cost.
struct Stripped {
    staged: Staged,
    before: StagedRoot,
    after: StagedRoot,
    strip: StripReport,
    report: SizeReport,
}

/// Stages the fixture and strips it, exactly as `ginary stage` does.
fn stage_and_strip(tools: &Toolchain) -> Stripped {
    let staged = stage_hello_ffi(tools);
    let otp = ginary::otp::discover(None).expect("the host OTP installation");
    let before = staged.root.clone();

    let strip = ginary::strip::strip(staged.root(), &otp, &StripOptions::default())
        .expect("a real runtime strips");
    let after = before.refresh().expect("the stripped tree can be re-read");
    let report =
        ginary::report::measure(&before, &strip, staged.root()).expect("the stripped tree reads");

    Stripped {
        staged,
        before,
        after,
        strip,
        report,
    }
}

/// The staged `beam.smp` of a tree.
fn beam_smp(staged: &Staged) -> std::path::PathBuf {
    staged
        .root()
        .join(format!("erts-{}", staged.root.erts_vsn()))
        .join("bin/beam.smp")
}

#[test]
fn a_stripped_hello_ffi_fits_in_the_size_budget() {
    let Some(tools) = require_tools(&["gleam", "erl", "strip"]) else {
        return;
    };
    let stripped = stage_and_strip(&tools);

    // The first real before-and-after the project has. Printed as well as
    // asserted, because `docs/dev/log/A2.md` records the table.
    eprintln!("{}", stripped.strip);
    eprintln!("{}", stripped.report.render_text());

    assert!(
        stripped.after.total_bytes() < TOTAL_BUDGET,
        "the stripped tree is {} bytes, over the {TOTAL_BUDGET} budget",
        stripped.after.total_bytes()
    );
    assert!(
        stripped.after.total_bytes() < stripped.before.total_bytes(),
        "stripping a real runtime has to remove something"
    );

    let smp = std::fs::metadata(beam_smp(&stripped.staged))
        .expect("the staged beam.smp")
        .len();
    assert!(
        smp < BEAM_SMP_BUDGET,
        "beam.smp is {smp} bytes, over the {BEAM_SMP_BUDGET} budget"
    );
}

#[test]
fn no_staged_module_holds_debug_information_after_stripping() {
    let Some(tools) = require_tools(&["gleam", "erl", "strip"]) else {
        return;
    };
    let stripped = stage_and_strip(&tools);

    let mut checked = 0usize;
    for file in stripped.after.files() {
        if !file.path.ends_with(".beam") {
            continue;
        }
        let bytes = std::fs::read(stripped.staged.root().join(&file.path)).expect("a module");
        assert!(
            !ginary::beam::has_chunk(&bytes, &ginary::beam::DEBUG_INFO_CHUNK),
            "{} still holds Dbgi",
            file.path
        );
        assert!(
            !ginary::beam::has_chunk(&bytes, &ginary::beam::DOCS_CHUNK),
            "{} still holds Docs",
            file.path
        );
        assert!(
            ginary::beam::has_chunk(&bytes, &ginary::beam::CODE_CHUNK),
            "{} lost its Code chunk",
            file.path
        );
        checked += 1;
    }
    assert!(
        checked > 100,
        "a real OTP closure holds hundreds of modules; only {checked} were checked"
    );
}

#[test]
fn a_stripped_runtime_still_runs_the_application() {
    // The whole point. Every other assertion in this file is about bytes; this
    // one is about whether the thing still works after they were removed.
    let Some(tools) = require_tools(&["gleam", "erl", "strip"]) else {
        return;
    };
    let stripped = stage_and_strip(&tools);
    let home = stripped.staged.home("run");

    let output = run_staged(stripped.staged.root(), APP, &["3", "a", "b"], &home);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        stdout.contains("args=3 a b"),
        "the stripped runtime lost the arguments:\n{stdout}\n--- stderr ---\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains("hello from priv"),
        "the stripped runtime lost code:priv_dir/1:\n{stdout}"
    );
    assert_eq!(
        output.status.code(),
        Some(3),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn stripping_a_real_tree_twice_changes_not_one_byte() {
    let Some(tools) = require_tools(&["gleam", "erl", "strip"]) else {
        return;
    };
    let stripped = stage_and_strip(&tools);
    let otp = ginary::otp::discover(None).expect("the host OTP installation");

    let once: Vec<Vec<u8>> = stripped
        .after
        .files()
        .iter()
        .map(|file| std::fs::read(stripped.staged.root().join(&file.path)).expect("a staged file"))
        .collect();

    ginary::strip::strip(stripped.staged.root(), &otp, &StripOptions::default())
        .expect("a second strip succeeds");

    let twice: Vec<Vec<u8>> = stripped
        .after
        .files()
        .iter()
        .map(|file| std::fs::read(stripped.staged.root().join(&file.path)).expect("a staged file"))
        .collect();
    assert_eq!(
        once, twice,
        "identical input produces identical artifact bytes, stripping included"
    );
}

#[test]
fn the_needs_line_lists_the_libraries_the_runtime_loads() {
    let Some(tools) = require_tools(&["gleam", "erl", "strip"]) else {
        return;
    };
    let stripped = stage_and_strip(&tools);

    let needs = stripped.report.needs_line();
    for library in [
        "libc.so.6",
        "libtinfo.so.6",
        "libstdc++.so.6",
        "libgcc_s.so.1",
    ] {
        assert!(
            needs.contains(library),
            "`{library}` is what beam.smp loads, and an artifact that does not say so is a trap:\n{needs}"
        );
    }
    assert!(
        needs.contains("(GLIBC_"),
        "the glibc floor is the number a user needs most:\n{needs}"
    );
}

#[test]
fn the_report_accounts_for_every_byte_stripping_removed() {
    let Some(tools) = require_tools(&["gleam", "erl", "strip"]) else {
        return;
    };
    let stripped = stage_and_strip(&tools);

    assert_eq!(stripped.report.total_before, stripped.before.total_bytes());
    assert_eq!(stripped.report.total_after, stripped.after.total_bytes());
    let erts = stripped
        .report
        .categories
        .get(&Category::ErtsBinary)
        .expect("the tree holds ERTS binaries");
    assert!(
        erts.bytes_after < erts.bytes_before,
        "the ERTS binaries are where most of the saving is: {erts:?}"
    );
    let beams = stripped
        .report
        .categories
        .get(&Category::OtpBeam)
        .expect("the tree holds OTP modules");
    assert!(
        beams.bytes_after < beams.bytes_before,
        "beam_lib:strip_files/1 removed nothing from the OTP modules: {beams:?}"
    );
}
