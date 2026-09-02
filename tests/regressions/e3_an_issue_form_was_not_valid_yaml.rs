// SPDX-License-Identifier: MIT OR Apache-2.0
//! `.github/ISSUE_TEMPLATE/feature_request.yml` was not valid YAML, and every
//! test over it was green.
//!
//! **What went wrong.** The form's `description:` was written as a plain
//! scalar carrying a colon-space:
//!
//! ```yaml
//! description: Propose a capability that fits what ginary is: one file, no runtime on the target machine.
//! ```
//!
//! A YAML plain scalar may not contain `": "`, so the document fails to load
//! ("mapping values are not allowed here"). GitHub cannot parse the form, so
//! it is never offered in the issue chooser — the file exists and does
//! nothing. Nothing in the tree noticed: `actionlint` does not look under
//! `.github/ISSUE_TEMPLATE/`, and every assertion over the forms,
//! `dependabot.yml` and the workflows was a `str::contains` substring check,
//! which is just as happy with a file no parser will accept.
//!
//! **The input.** The committed `.github` records GitHub itself loads as
//! YAML: the issue forms and their config, `dependabot.yml`, every workflow,
//! and any composite action.
//!
//! **The correct behaviour.** Every one of them parses as a YAML document,
//! and a file that does not fails here naming the path and the parser's own
//! message. The E3 records are held to the same standard as the two rulesets,
//! which have gone through `serde_json` from the start.

use crate::common::repo::{parse_yaml, read, yaml_files_under};

/// The records GitHub loads as YAML, beyond the `.yml` files under `.github/`.
///
/// `yaml_files_under(".github")` already reaches the workflows, the issue
/// forms and any composite action; `dependabot.yml` sits at the top of that
/// directory and is picked up by the same walk, so this list exists only to
/// state the two records that must be present rather than merely valid.
const REQUIRED: &[&str] = &[
    ".github/dependabot.yml",
    ".github/ISSUE_TEMPLATE/feature_request.yml",
];

#[test]
fn every_github_record_parses_as_yaml() {
    let files = yaml_files_under(".github");
    assert!(
        !files.is_empty(),
        "there are no `.github` YAML records to parse"
    );
    for required in REQUIRED {
        assert!(
            files.iter().any(|found| found == required),
            "{required} is one of the records this test exists for, and the walk did not find \
             it: {files:?}"
        );
    }
    let mut offenders: Vec<String> = Vec::new();
    for file in &files {
        if let Err(error) = parse_yaml(&read(file)) {
            offenders.push(format!("{file}: {error}"));
        }
    }
    assert!(
        offenders.is_empty(),
        "GitHub loads each of these as YAML, and it cannot load these:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn the_feature_request_description_is_a_scalar_yaml_can_read() {
    let form = crate::common::repo::yaml(".github/ISSUE_TEMPLATE/feature_request.yml");
    let description = form
        .as_mapping_get("description")
        .and_then(saphyr::YamlOwned::as_str)
        .expect("the feature-request form carries a top-level `description:` string");
    assert!(
        description.contains("one file"),
        "the description survived the fix intact: {description}"
    );
}
