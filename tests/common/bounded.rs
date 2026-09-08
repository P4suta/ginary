// SPDX-License-Identifier: MIT OR Apache-2.0
//! The configured subprocess executor used by the integration-test harness.
//!
//! Tests and product probes share bounded capture and child cleanup. A harness
//! failure prints the retained evidence and, when `GINARY_TEST_EVIDENCE_DIR` is
//! configured, saves exact stream bytes and a machine-readable report there.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output};
use std::time::Duration;

use ginary::process::{CAPTURE_LIMIT, ProcessReport, run_command, wait_child};

/// Runs a configured command within `budget`, capturing both output streams.
///
/// # Panics
///
/// If spawning, waiting, cleanup, or complete capture fails. A nonzero exit is
/// returned to the caller, with evidence saved when its directory is configured.
pub fn run_bounded(command: &mut Command, budget: Duration, what: &str) -> Output {
    collect(run_command(command, budget, CAPTURE_LIMIT), what)
}

/// Observes an already-spawned child, retaining its output even on timeout.
///
/// The budget starts now. Both output streams should have been piped when the
/// child was spawned. Callers running several children concurrently account for
/// the time each has already been running before they start waiting for it.
///
/// # Panics
///
/// As [`run_bounded`].
pub fn wait_bounded(child: Child, budget: Duration, what: &str) -> Output {
    collect(wait_child(child, budget, CAPTURE_LIMIT), what)
}

/// Saves a failure before turning the detailed product report into `Output`.
fn collect(report: ProcessReport, what: &str) -> Output {
    let complete = report.stdout.is_complete() && report.stderr.is_complete();
    let cleanup_ok = report
        .cleanup
        .as_ref()
        .is_none_or(|cleanup| cleanup.reaped && cleanup.error.is_none());
    let healthy = report.error.is_none() && report.status.is_some() && complete && cleanup_ok;
    let saved = if !healthy || !report.success() {
        std::env::var_os("GINARY_TEST_EVIDENCE_DIR").map(|directory| {
            match persist_evidence(&report, what, Path::new(&directory)) {
                Ok(path) => format!("evidence saved in {}", path.display()),
                Err(error) => format!(
                    "cannot save evidence in {}: {error}",
                    Path::new(&directory).display()
                ),
            }
        })
    } else {
        None
    };
    if let Some(saved) = &saved {
        eprintln!("{what}: {saved}");
    }
    assert!(
        healthy,
        "{what}: {}; elapsed={}ms; status={:?}; cleanup={:?}; stdout omitted={} EOF={} error={:?}: {}; stderr omitted={} EOF={} error={:?}: {}; {}",
        report
            .error
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_else(|| "incomplete process observation".to_owned()),
        report.elapsed.as_millis(),
        report.status,
        report.cleanup,
        report.stdout.omitted_bytes,
        report.stdout.complete,
        report.stdout.error,
        report.stdout.text(),
        report.stderr.omitted_bytes,
        report.stderr.complete,
        report.stderr.error,
        report.stderr.text(),
        saved
            .as_deref()
            .unwrap_or("set GINARY_TEST_EVIDENCE_DIR to retain evidence files")
    );
    Output {
        status: report.status.expect("healthy report has an exit status"),
        stdout: report.stdout.bytes,
        stderr: report.stderr.bytes,
    }
}

/// Stores bounded, exact output and metadata in a unique directory for this call.
fn persist_evidence(
    report: &ProcessReport,
    what: &str,
    directory: &Path,
) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(directory)?;
    let evidence = tempfile::Builder::new()
        .prefix(&format!("process-{}-", std::process::id()))
        .tempdir_in(directory)?;
    std::fs::write(evidence.path().join("stdout.bin"), &report.stdout.bytes)?;
    std::fs::write(evidence.path().join("stderr.bin"), &report.stderr.bytes)?;
    let stream = |output: &ginary::process::CapturedOutput| {
        serde_json::json!({
            "retained_bytes": output.bytes.len(), "omitted_bytes": output.omitted_bytes,
            "eof": output.complete, "read_error": output.error,
        })
    };
    let cleanup = report.cleanup.as_ref().map(|cleanup| serde_json::json!({
        "pid": cleanup.pid, "kill_requested": cleanup.kill_requested,
        "reaped": cleanup.reaped, "background_reaper": cleanup.background_reaper, "error": cleanup.error,
    }));
    let metadata = serde_json::json!({
        "schema_version": 1, "what": what, "elapsed_ms": report.elapsed.as_millis(),
        "success": report.success(), "status": report.status.map(|status| status.to_string()),
        "exit_code": report.status.and_then(|status| status.code()),
        "cause": report.error.as_ref().map(ToString::to_string), "cleanup": cleanup,
        "stdout": stream(&report.stdout), "stderr": stream(&report.stderr),
    });
    std::fs::write(
        evidence.path().join("report.json"),
        serde_json::to_vec_pretty(&metadata)?,
    )?;
    Ok(evidence.keep())
}
