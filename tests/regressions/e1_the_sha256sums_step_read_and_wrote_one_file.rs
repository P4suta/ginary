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

use crate::common::repo::{root, yaml_files_under};
use crate::common::tools::require_actionlint;
use std::process::Command;

/// The fewest workflows this repository has ever had, and the floor a scan
/// that found none has to fail against.
///
/// The list used to be four names written out here, which went stale the
/// moment E3 added three workflows: the lint the milestone claims to run over
/// every workflow ran over four of seven. It is read out of the directory
/// now, and a directory read that answers with nothing is a green run that
/// linted nothing.
const FEWEST_WORKFLOWS: usize = 4;

#[test]
fn actionlint_accepts_every_workflow() {
    // Gated on `GINARY_REQUIRE_ACTIONLINT` and not on the toolchain flag: the
    // tool belongs to the job that lints, and demanding it of every job that
    // has `erl` installed is what failed three CI jobs at once. See
    // `tests/regressions/e7_actionlint_was_required_of_every_toolchain_job.rs`.
    let Some(actionlint) = require_actionlint() else {
        return;
    };

    let workflows = yaml_files_under(".github/workflows");
    assert!(
        workflows.len() >= FEWEST_WORKFLOWS,
        "`.github/workflows` holds {} files, which is not this repository: a lint over nothing \
         passes for the wrong reason",
        workflows.len()
    );

    for workflow in &workflows {
        let output = Command::new(&actionlint)
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
