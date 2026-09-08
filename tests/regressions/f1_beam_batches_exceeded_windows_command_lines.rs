// SPDX-License-Identifier: MIT OR Apache-2.0
//! A 256 KiB module batch exceeded Windows' 32,767 UTF-16 command-line limit.
//!
//! The native OTP repack reached CreateProcess error 206 after preparing the
//! runtime. These smaller trees reproduce the actual spawn boundary while
//! proving that every module, including those in later batches, is rewritten.

use std::path::{Path, PathBuf};

use crate::common::fake_otp::{FakeOtp, beam_bytes};
use ginary::beam::{CODE_CHUNK, DEBUG_INFO_CHUNK, DOCS_CHUNK};
use ginary::strip::{self, BeamOutcome, StripOptions};

const MODULES: usize = 400;

fn tree(root: &Path, bytes: &[u8]) -> Vec<PathBuf> {
    let modules = root.join("modules with spaces [1]");
    std::fs::create_dir_all(&modules).expect("module directory");
    (0..MODULES)
        .map(|number| {
            let path = modules.join(format!("module_{number:04}_{}.beam", "x".repeat(50)));
            std::fs::write(&path, bytes).expect("module fixture");
            path
        })
        .collect()
}

fn verify_each(report: &strip::StripReport, paths: &[PathBuf]) {
    assert!(matches!(
        report.beams,
        BeamOutcome::Stripped { files: MODULES, .. }
    ));
    assert_eq!(report.per_file.len(), MODULES);
    for path in paths {
        let bytes = std::fs::read(path).expect("rewritten module");
        assert!(
            ginary::beam::has_chunk(&bytes, &CODE_CHUNK),
            "{}",
            path.display()
        );
        assert!(
            !ginary::beam::has_chunk(&bytes, &DEBUG_INFO_CHUNK),
            "{}",
            path.display()
        );
        assert!(
            !ginary::beam::has_chunk(&bytes, &DOCS_CHUNK),
            "{}",
            path.display()
        );
    }
}

#[test]
fn f1_many_module_paths_fit_the_native_process_and_every_module_is_rewritten() {
    let dir = tempfile::tempdir().expect("tempdir");
    let otp = FakeOtp::new()
        .with_shrinking_erl_script()
        .build_in(dir.path().join("otp"));
    let info = ginary::otp::inspect_root(&otp.root).expect("fixture OTP");
    let staged = dir.path().join("staged");
    let paths = tree(
        &staged,
        &beam_bytes(&[
            (CODE_CHUNK, b"code"),
            (DEBUG_INFO_CHUNK, b"must be removed"),
        ]),
    );
    assert!(
        paths
            .iter()
            .map(|path| path.to_string_lossy().encode_utf16().count() + 1)
            .sum::<usize>()
            > 32_767,
        "the fixture must cross the actual Windows process limit"
    );

    let outcome = strip::strip(
        &staged,
        &info,
        &StripOptions {
            elf: false,
            beams: true,
        },
    );
    assert!(
        outcome.is_ok(),
        "a large file set must be split into executable batches: {outcome:?}"
    );
    verify_each(&outcome.expect("successful stripping"), &paths);
}

#[test]
fn f1_real_erlang_strips_every_module_across_the_windows_argument_boundary() {
    let Some(_tools) = crate::common::tools::require_tools(&["erl"]) else {
        return;
    };
    let otp = ginary::otp::discover(None).expect("installed OTP");
    let dir = tempfile::tempdir().expect("tempdir");
    let staged = dir.path().join("staged");
    let fixture = crate::common::erl::compile_strip_fixture(&otp, &dir.path().join("compiler"));
    let source = std::fs::read(&fixture).expect("real unstripped BEAM");
    assert!(ginary::beam::has_chunk(&source, &DEBUG_INFO_CHUNK));
    let paths = tree(&staged, &source);
    let outcome = strip::strip(
        &staged,
        &otp,
        &StripOptions {
            elf: false,
            beams: true,
        },
    );
    assert!(
        outcome.is_ok(),
        "real Erlang must strip all batches: {outcome:?}"
    );
    let report = outcome.expect("real stripping");
    verify_each(&report, &paths);
    assert!(report.after_total < report.before_total, "{report}");
    assert_eq!(std::fs::read(fixture).expect("source fixture"), source);
}

#[cfg(windows)]
#[test]
fn f1_a_failed_erlang_batch_retains_its_file_range_and_tool_evidence() {
    let dir = tempfile::tempdir().expect("tempdir");
    let otp = FakeOtp::new()
        .with_failing_erl_script("{error,simulated_beam_rewrite_failure}")
        .build_in(dir.path().join("otp"));
    let info = ginary::otp::inspect_root(&otp.root).expect("fixture OTP");
    let staged = dir.path().join("staged");
    let paths = tree(&staged, &beam_bytes(&[(CODE_CHUNK, b"code")]));
    let error = strip::strip(
        &staged,
        &info,
        &StripOptions {
            elf: false,
            beams: true,
        },
    )
    .expect_err("runtime fails deliberately");
    let message = error.to_string();
    assert!(message.contains("batch 1 of "), "{message}");
    assert!(message.contains("module_0000_"), "{message}");
    assert!(
        message.contains("simulated_beam_rewrite_failure"),
        "{message}"
    );
    assert!(paths.iter().all(|path| path.is_file()));
}
