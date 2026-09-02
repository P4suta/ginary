// SPDX-License-Identifier: MIT OR Apache-2.0
//! `distribute.yml`'s `Compute SHA256SUMS` step read and wrote the same file
//! in one pipeline, so `actionlint` (via shellcheck SC2094) rejected the
//! workflow.
//!
//! **What went wrong.** The step globbed every asset with
//! `find . -maxdepth 1 -type f ! -name SHA256SUMS ... | xargs sha256sum >
//! SHA256SUMS`. shellcheck SC2094 ("Make sure not to read and write the same
//! file in the same pipeline") fires because `SHA256SUMS` is named both in the
//! `find` predicate and in the redirect target of the same pipeline, so
//! `actionlint .github/workflows/distribute.yml` exits 1. The milestone
//! requires every workflow to be actionlint-clean, and the E1 log claims all
//! four are.
//!
//! **The input.** The committed workflow YAML.
//!
//! **The correct behaviour.** `actionlint` exits 0 on every workflow, so the
//! manifest is written without reading and writing one file in a single
//! pipeline (write a `.tmp` and rename, or otherwise keep the read and the
//! write off the same name).

use crate::common::repo::root;
use crate::common::tools::require_tools;
use std::process::Command;

/// Every workflow actionlint must accept.
const WORKFLOWS: &[&str] = &[
    ".github/workflows/ci.yml",
    ".github/workflows/nightly.yml",
    ".github/workflows/release.yml",
    ".github/workflows/distribute.yml",
];

#[test]
fn actionlint_accepts_every_workflow() {
    let Some(tools) = require_tools(&["actionlint"]) else {
        return;
    };
    let actionlint = tools.path("actionlint");

    for workflow in WORKFLOWS {
        let output = Command::new(actionlint)
            .current_dir(root())
            .arg(workflow)
            .output()
            .unwrap_or_else(|error| panic!("cannot run actionlint on {workflow}: {error}"));
        assert!(
            output.status.success(),
            "actionlint rejected {workflow} (exit {:?}); every workflow must be \
             actionlint-clean:\n{}{}",
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }
}
