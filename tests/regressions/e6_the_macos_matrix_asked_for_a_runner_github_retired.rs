// SPDX-License-Identifier: MIT OR Apache-2.0
//! The macOS matrix asked for `macos-13`, a runner image GitHub no longer
//! schedules, so the run never finished and the pull request could never go
//! green.
//!
//! **What went wrong.** The `macos` job of `ci.yml` is a two-row matrix:
//! `macos-13` for x86_64 and `macos-14` for arm64. The arm64 row is picked up
//! in seconds; the x86_64 row is never picked up at all. Two consecutive runs,
//! twenty-four hours apart, agree:
//!
//! ```text
//! {"labels":["macos-13"],"name":"macOS build, launch and signature (macos-13, macos-x86_64)",
//!  "runner_name":"","started_at":"2026-09-02T17:03:57Z","status":"queued","conclusion":null}
//! {"labels":["macos-14"],"name":"macOS build, launch and signature (macos-14, macos-aarch64)",
//!  "runner_name":"GitHub Actions 1000139302","started_at":"2026-09-02T17:04:00Z",
//!  "status":"completed","conclusion":"failure"}
//! ```
//!
//! (runs <https://github.com/P4suta/ginary/actions/runs/33658759531> and
//! <https://github.com/P4suta/ginary/actions/runs/33681144884>; the second
//! still reports `"status":"queued"` for the whole run a day later, with
//! `job/100417746021` never assigned a runner.)
//!
//! An unschedulable label is worse than a failing job. GitHub reports the
//! *run* as queued for as long as one job is waiting, so the run has no
//! conclusion, the `Required CI` fan-in never resolves, and a branch
//! protection rule that waits for it waits forever. Every other job in the run
//! can be green and the pull request is still not mergeable.
//!
//! **The input.** Any `runs-on:` — or any `strategy.matrix` value feeding one
//! — naming a runner image GitHub has withdrawn. Nothing in the repository
//! notices: the workflow is valid YAML, `actionlint` is happy, and the failure
//! mode is silence.
//!
//! **The correct behaviour.** No committed workflow asks for a retired image.
//! The list below is the check, and it is deliberately a list rather than a
//! network call: a test that asked GitHub which images exist would be a test
//! that fails when the network does, and the answer changes about twice a year.
//! Each entry carries the image that replaces it, so the failure is also the
//! fix. The macOS matrix keeps both architectures — the x86_64 row is what
//! proves the darwin stub and its ad-hoc signature on Intel, and dropping it
//! would trade a blocked run for a coverage hole nobody wrote down.

use saphyr::YamlOwned;

use crate::common::repo::{yaml, yaml_files_under};

/// Runner images GitHub has withdrawn, and what each is replaced by.
///
/// A withdrawn label is still accepted by the workflow parser and still shows
/// up as a job; it simply never runs.
const RETIRED: [(&str, &str); 4] = [
    (
        "macos-12",
        "macos-15-intel, the x86_64 macOS image GitHub still schedules",
    ),
    (
        "macos-13",
        "macos-15-intel for x86_64; macos-14 or macos-15 for arm64",
    ),
    ("ubuntu-20.04", "ubuntu-22.04 or ubuntu-24.04"),
    (
        "windows-2019",
        "windows-2022, which this repository already pins",
    ),
];

/// Images that are current, so a failure proves the scan reads real values
/// rather than an empty document.
const CURRENT: [&str; 3] = ["macos-14", "windows-2022", "ubuntu-24.04"];

#[test]
fn no_workflow_asks_for_a_runner_image_github_no_longer_schedules() {
    let mut labels: Vec<(String, String)> = Vec::new();
    for relative in yaml_files_under(".github/workflows") {
        for label in runner_labels(&yaml(&relative)) {
            labels.push((relative.clone(), label));
        }
    }

    for current in CURRENT {
        assert!(
            labels.iter().any(|(_, label)| label == current),
            "the scan did not find `{current}` in any workflow, so it is not reading the values \
             a job actually runs on. Found: {labels:?}"
        );
    }

    let mut offenders = Vec::new();
    for (workflow, label) in &labels {
        for (retired, replacement) in RETIRED {
            if label == retired {
                offenders.push(format!(
                    "{workflow} asks for `{retired}`; use {replacement}"
                ));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "a workflow asks for a runner image GitHub has withdrawn. Such a job is never assigned a \
         runner and never fails either: it stays queued, the whole run keeps the `queued` status, \
         and a required check computed from that run never resolves:\n{}",
        offenders.join("\n")
    );
}

/// Every scalar a job's `runs-on` or `strategy` could put in front of the
/// scheduler.
///
/// Both halves are needed and neither is enough on its own. `runs-on:
/// windows-2022` names the image directly; `runs-on: ${{ matrix.runner }}`
/// names nothing at all, and the images are down in `strategy.matrix.include`
/// under a key whose name — `runner`, `os`, `image` — is the workflow author's
/// choice. So the whole `strategy` subtree is read, and an expression is
/// skipped because it is not a label.
fn runner_labels(document: &YamlOwned) -> Vec<String> {
    let mut out = Vec::new();
    let Some(jobs) = document
        .as_mapping_get("jobs")
        .and_then(YamlOwned::as_mapping)
    else {
        return out;
    };
    for (_, job) in jobs {
        collect_scalars(job.as_mapping_get("runs-on"), &mut out);
        collect_scalars(job.as_mapping_get("strategy"), &mut out);
    }
    out.sort();
    out.dedup();
    out
}

/// Appends every string scalar under one node, skipping `${{ .. }}`.
fn collect_scalars(node: Option<&YamlOwned>, out: &mut Vec<String>) {
    let Some(node) = node else {
        return;
    };
    if let Some(text) = node.as_str() {
        if !text.contains("${{") {
            out.push(text.to_owned());
        }
        return;
    }
    if let Some(items) = node.as_vec() {
        for item in items {
            collect_scalars(Some(item), out);
        }
        return;
    }
    if let Some(mapping) = node.as_mapping() {
        for (_, value) in mapping {
            collect_scalars(Some(value), out);
        }
    }
}
