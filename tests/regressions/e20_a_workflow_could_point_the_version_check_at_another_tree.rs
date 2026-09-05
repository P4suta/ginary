// SPDX-License-Identifier: MIT OR Apache-2.0
//! The release gate grew an environment variable that redirects which tree it
//! validates, and nothing said that a workflow must never set it.
//!
//! **What went wrong.** E20 gave `scripts/ci/version-consistency.sh` a seam so
//! the suite could drive it over fixture trees in the release states this
//! checkout is not in:
//!
//! ```sh
//! root="${GINARY_VERSION_ROOT:-$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)}"
//! ```
//!
//! That is the right seam and the wrong silence. `GINARY_VERSION_ROOT` reaching
//! `distribute.yml` — as a job `env:`, a step `env:`, or an exported variable
//! in a composite action — would make the gate compare a tag against some other
//! directory's `Cargo.toml` and pass, which is precisely the check not running.
//! `a_workflow_runs_the_version_consistency_check` asserted only that
//! `distribute.yml` contains the string `version-consistency.sh`.
//!
//! **The input.** Any workflow that sets the variable, for any reason. Nothing
//! about the name suggests it is dangerous, and a test seam borrowed into a
//! workflow reads as configuration.
//!
//! **The correct behaviour.** The variable is a test seam and nothing else:
//! no workflow mentions it, the suite asserts that, and the script's own header
//! says that a workflow setting it is a defect rather than a knob.

use crate::common::repo::{read, root};
use crate::common::version::ROOT_VAR;

/// The script the seam lives in.
const SCRIPT: &str = "scripts/ci/version-consistency.sh";

#[test]
fn no_workflow_mentions_the_version_check_seam() {
    let mut offenders: Vec<String> = Vec::new();
    let directory = root().join(".github/workflows");
    for entry in std::fs::read_dir(&directory).expect("read .github/workflows") {
        let path = entry.expect("a workflow directory entry").path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("yml") {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("read a workflow");
        for (number, line) in text.lines().enumerate() {
            if line.contains(ROOT_VAR) {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                offenders.push(format!("{name}:{}: {}", number + 1, line.trim()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "{ROOT_VAR} points {SCRIPT} at a tree other than the repository it lives in. It exists \
         for `tests/version_consistency.rs` and for nothing else: a workflow that sets it makes \
         the release gate prove a tag matches some other directory's Cargo.toml, which is the \
         check passing while not running.\n{}",
        offenders.join("\n")
    );
}

#[test]
fn the_script_says_a_workflow_setting_the_seam_is_a_defect() {
    let script = read(SCRIPT);
    let header: String = script
        .lines()
        .take_while(|line| line.starts_with('#') || line.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        header.contains(ROOT_VAR),
        "{SCRIPT} reads {ROOT_VAR} and its header has to say so: a reader who cannot see the seam \
         cannot know the gate has one"
    );
    assert!(
        header.contains("defect"),
        "the header has to say that a workflow setting {ROOT_VAR} is a defect, not a knob. \
         Documenting what a variable does without saying who may set it is how a test seam \
         becomes configuration"
    );
}
