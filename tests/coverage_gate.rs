// SPDX-License-Identifier: MIT OR Apache-2.0
//! The coverage gate, held to its threshold against fixture lcov reports.
//!
//! `cargo llvm-cov` writes an lcov report; the CI `coverage` job has to fail
//! when line coverage falls below 90%, and — because a build that cannot read
//! the report is not a build with high coverage — it has to fail loudly on a
//! report it cannot parse rather than wave it through. `scripts/ci/
//! coverage-gate.sh` is that gate, and this file runs it against four
//! committed lcov fixtures: one above the line, one below it, one that rounds
//! up to the floor but is truly below it, and one holding no line records at
//! all. The percentages are arithmetic a person can check — 95/100, 80/100,
//! and 89999/100000 — so the fixtures pin the exact number the gate prints and
//! the exact boundary it enforces.
//!
//! The script does not exist yet; every test here fails at the assertion that
//! looks for it.

mod common;

use std::path::PathBuf;
use std::process::Command;

use crate::common::repo::root;

/// The coverage-gate script, asserted present and executable.
fn script() -> PathBuf {
    let path = root().join("scripts/ci/coverage-gate.sh");
    assert!(
        path.is_file(),
        "scripts/ci/coverage-gate.sh is the gate the CI coverage job runs; it is not committed"
    );
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(&path)
        .expect("script metadata")
        .permissions()
        .mode();
    assert_eq!(mode & 0o111, 0o111, "the gate has to be executable");
    path
}

/// Runs the gate over a fixture at `min` percent, returning (code, stdout+stderr).
fn run(fixture: &str, min: &str) -> (i32, String) {
    let lcov = root().join("tests/fixtures/coverage").join(fixture);
    let output = Command::new(script())
        .arg(&lcov)
        .arg(min)
        .current_dir(root())
        .output()
        .expect("spawn coverage-gate.sh");
    let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
    combined.push_str(&String::from_utf8_lossy(&output.stderr));
    (output.status.code().unwrap_or(-1), combined)
}

#[test]
fn coverage_above_the_threshold_passes_and_reports_the_percentage() {
    // pass.info: LH 57+38 = 95, LF 60+40 = 100 -> 95.00%.
    let (code, out) = run("pass.info", "90");
    assert_eq!(code, 0, "95.00% clears a 90% gate: {out}");
    assert!(
        out.contains("95.00") && out.contains("95/100"),
        "the gate prints the exact ratio it computed: {out}"
    );
}

#[test]
fn coverage_below_the_threshold_fails_and_names_the_shortfall() {
    // below.info: LH 42+38 = 80, LF 60+40 = 100 -> 80.00%.
    let (code, out) = run("below.info", "90");
    assert_eq!(code, 1, "80.00% is below a 90% gate and must fail: {out}");
    assert!(
        out.contains("80.00") && out.contains("90"),
        "the failure names both the coverage and the floor it missed: {out}"
    );
}

#[test]
fn a_ratio_that_rounds_up_to_the_floor_still_fails() {
    // boundary.info: LH 89999, LF 100000 -> a true 89.999%, which printf
    // "%.2f" renders as "90.00". A gate that compared the rounded display
    // value would read "90.00 < 90" as false and wave it through; the gate
    // must compare the raw ratio, so this is below the 90% floor and fails.
    let (code, out) = run("boundary.info", "90");
    assert_eq!(
        code, 1,
        "89.999% is below a 90% floor even though it displays as 90.00%: {out}"
    );
    assert!(
        out.contains("90.00"),
        "the human-readable line still rounds to 90.00%, but the gate failed on the raw ratio: {out}"
    );
}

#[test]
fn a_report_with_no_line_records_is_an_error_not_a_pass() {
    // malformed.info holds no LF/LH lines. A gate that divided by zero and
    // called the result 100% would be worse than no gate; it must refuse.
    let (code, out) = run("malformed.info", "90");
    assert_eq!(
        code, 2,
        "an unparseable report is a distinct error, not a silent pass: {out}"
    );
    assert!(
        out.to_lowercase().contains("no line") || out.to_lowercase().contains("record"),
        "the error says what it could not find rather than a bare non-zero exit: {out}"
    );
}

#[test]
fn the_ci_coverage_job_runs_the_gate_at_ninety_percent() {
    // The gate is only a gate if CI runs it. The threshold lives in the
    // workflow, not only in the script's default, so it cannot be lowered by
    // editing the script alone without the diff showing it.
    let ci = std::fs::read_to_string(root().join(".github/workflows/ci.yml")).expect("ci.yml");
    assert!(
        ci.contains("coverage-gate.sh"),
        "ci.yml has to run scripts/ci/coverage-gate.sh in its coverage job:\n{ci}"
    );
    assert!(
        ci.contains("llvm-cov") && ci.contains("90"),
        "the coverage job produces an lcov report and gates it at 90% lines"
    );
}
