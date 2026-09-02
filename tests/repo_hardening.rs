// SPDX-License-Identifier: MIT OR Apache-2.0
//! The records a public repository is run from, held against the tree.
//!
//! E3 adds the half of a public repository that is not code: the branch and
//! tag rulesets, the owner of every path, the forms a stranger files an issue
//! or a pull request on, and the policy that tells them where a vulnerability
//! goes. None of it executes here — a ruleset is applied through the GitHub
//! API, a form is rendered by GitHub — and all of it rots in the same silent
//! way `tests/ci_matrix.rs` was written for. The dangerous one is the ruleset:
//! its required status check is a *string*, matched against a check name, so
//! renaming the CI fan-in job leaves `main` protected by a check that will
//! never report and a merge queue that waits forever, or — worse, depending on
//! the setting — by nothing at all. That cross-check is the reason this file
//! exists; the rest is the same discipline applied to the documents beside it.
//!
//! The two ruleset snapshots are the committed record the orchestrator applies
//! by API. They are canonicalised through `serde_json` first, so what they pin
//! is the ruleset rather than its indentation.
//!
//! Ungated: a repository policy is neither half of the crate.

mod common;

use saphyr::YamlOwned;
use serde_json::Value;

use crate::common::repo::{read, read_or_missing, yaml};

/// A committed ruleset, parsed.
fn ruleset(relative: &str) -> Value {
    let text = read(relative);
    serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("{relative} is not valid JSON: {error}"))
}

/// A ruleset re-serialised with sorted keys, or the `(missing …)` marker.
///
/// The GitHub API takes this document verbatim, so the snapshot is of the
/// document and not of the file's whitespace: two spellings of the same
/// ruleset are the same record.
fn canonical(relative: &str) -> String {
    let text = read_or_missing(relative);
    if text.starts_with("(missing") {
        return text;
    }
    let parsed: Value = serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("{relative} is not valid JSON: {error}"));
    serde_json::to_string_pretty(&parsed).expect("re-serialise a parsed document")
}

/// Prose with every run of whitespace collapsed to one space.
///
/// The documents in this repository hard-wrap at about 100 columns, so a
/// sentence a test looks for is usually split across two lines. Collapsing
/// first means an assertion is about the sentence rather than about where the
/// wrap happened to fall.
fn flowed(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The `rules` array of a ruleset, by `type`.
fn rule<'a>(ruleset: &'a Value, kind: &str) -> Option<&'a Value> {
    ruleset
        .get("rules")?
        .as_array()?
        .iter()
        .find(|rule| rule.get("type").and_then(Value::as_str) == Some(kind))
}

// ------------------------------------------------------- the rulesets --

#[test]
fn the_main_ruleset_protects_the_default_branch_from_every_way_around_ci() {
    let main = ruleset(".github/rulesets/main.json");
    assert_eq!(main["name"], Value::from("Protect main"));
    assert_eq!(main["target"], Value::from("branch"));
    assert_eq!(main["enforcement"], Value::from("active"));
    assert_eq!(
        main["bypass_actors"],
        Value::Array(Vec::new()),
        "nobody bypasses the rule, the author included: a bypass actor is the hole every one of \
         these rules is meant to close"
    );
    assert_eq!(
        main["conditions"]["ref_name"]["include"],
        Value::from(vec!["~DEFAULT_BRANCH"]),
        "the ruleset applies to the default branch"
    );
    for kind in [
        "deletion",
        "non_fast_forward",
        "required_linear_history",
        "pull_request",
        "required_status_checks",
    ] {
        assert!(
            rule(&main, kind).is_some(),
            "the main ruleset carries no `{kind}` rule; deleting the branch, force-pushing over \
             it or merging without CI would all still be possible"
        );
    }
    let pull_request = rule(&main, "pull_request").expect("a pull_request rule");
    assert_eq!(
        pull_request["parameters"]["allowed_merge_methods"],
        Value::from(vec!["squash"]),
        "one commit per pull request, which is what `required_linear_history` and the \
         Conventional Commits history assume"
    );
    assert_eq!(
        pull_request["parameters"]["required_review_thread_resolution"],
        Value::Bool(true),
        "an unresolved review thread blocks the merge"
    );
    let checks = rule(&main, "required_status_checks").expect("a required_status_checks rule");
    assert_eq!(
        checks["parameters"]["strict_required_status_checks_policy"],
        Value::Bool(true),
        "a branch behind main re-runs CI before it merges"
    );
}

#[test]
fn the_required_status_check_is_the_fan_in_job_ci_actually_declares() {
    let main = ruleset(".github/rulesets/main.json");
    let checks = rule(&main, "required_status_checks").expect("a required_status_checks rule");
    let required = checks["parameters"]["required_status_checks"]
        .as_array()
        .expect("a list of required checks");
    assert_eq!(
        required.len(),
        1,
        "one required check, the CI fan-in, which already gates on every other job: {required:?}"
    );
    let context = required[0]["context"]
        .as_str()
        .expect("the check names a context");
    // The name is matched as a string against what the run reports. A rename
    // of the fan-in job on one side and not the other leaves `main` guarded by
    // a check that never arrives, so the two are compared here rather than
    // trusted to stay in step.
    // Read out of the parsed workflow, not out of its text: the job's own
    // `name:` key and nothing else. Scanning forward from a `required:` line
    // finds the first `name:` after it, which is a *step* name as soon as the
    // job's display name is gone — so the guard would report a disagreement
    // over a name neither side ever set, and would miss the rename it exists
    // to catch.
    let ci = yaml(".github/workflows/ci.yml");
    let fan_in = ci
        .as_mapping_get("jobs")
        .and_then(|jobs| jobs.as_mapping_get("required"))
        .expect("ci.yml declares the `required:` fan-in job");
    let display = fan_in
        .as_mapping_get("name")
        .and_then(YamlOwned::as_str)
        .expect("the fan-in job carries a display name of its own");
    assert_eq!(
        context, display,
        "the ruleset requires the check `{context}` and ci.yml's fan-in job reports as \
         `{display}`; a required check that never reports blocks or opens `main` by accident"
    );
    assert_eq!(
        required[0]["integration_id"],
        Value::from(15368),
        "the check has to come from the GitHub Actions app (integration 15368), or any app that \
         posts a status of that name would satisfy it"
    );
}

#[test]
fn the_main_ruleset_requires_no_check_this_repository_does_not_run() {
    let text = read(".github/rulesets/main.json");
    // release-glz requires a CodeRabbit review; that app is not installed here,
    // and a required check nothing posts is a branch nobody can merge to.
    assert!(
        !text.contains("CodeRabbit"),
        "the ruleset requires `CodeRabbit`, which is not installed on this repository"
    );
}

#[test]
fn the_release_tag_ruleset_makes_every_version_tag_immutable() {
    let tags = ruleset(".github/rulesets/release-tags.json");
    assert_eq!(tags["name"], Value::from("Protect release tags"));
    assert_eq!(tags["target"], Value::from("tag"));
    assert_eq!(tags["enforcement"], Value::from("active"));
    assert_eq!(tags["bypass_actors"], Value::Array(Vec::new()));
    assert_eq!(
        tags["conditions"]["ref_name"]["include"],
        Value::from(vec!["refs/tags/v*"]),
        "the release tags are `v*`, the ones release-please pushes and distribute.yml builds from"
    );
    for kind in ["deletion", "non_fast_forward"] {
        assert!(
            rule(&tags, kind).is_some(),
            "a released tag cannot be deleted or moved, so an artifact keeps naming the source it \
             was built from; the `{kind}` rule is missing"
        );
    }
}

#[test]
fn the_main_ruleset_is_the_committed_record() {
    insta::assert_snapshot!("main_ruleset", canonical(".github/rulesets/main.json"));
}

#[test]
fn the_release_tag_ruleset_is_the_committed_record() {
    insta::assert_snapshot!(
        "release_tags_ruleset",
        canonical(".github/rulesets/release-tags.json")
    );
}

// ------------------------------------------------------- the templates --

#[test]
fn codeowners_puts_every_path_under_one_owner() {
    let owners = read(".github/CODEOWNERS");
    assert!(
        owners.starts_with("# SPDX-License-Identifier: MIT OR Apache-2.0"),
        "every text file in the tree carries the SPDX header:\n{owners}"
    );
    for line in ["* @P4suta", "/.github/ @P4suta", "/SECURITY.md @P4suta"] {
        assert!(
            owners.lines().any(|l| l.trim() == line),
            "CODEOWNERS is missing `{line}`; the catch-all comes first and the paths that decide \
             who can change the gates come after it"
        );
    }
}

#[test]
fn the_pull_request_template_asks_for_the_house_gate_and_a_regression_test() {
    let template = read(".github/pull_request_template.md");
    assert!(
        template.starts_with("<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->"),
        "the template carries the SPDX header:\n{template}"
    );
    let boxes: Vec<&str> = template
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("- [ ]"))
        .collect();
    assert!(
        boxes.iter().any(|line| line.contains("mise run check")),
        "the checklist has a row for `mise run check`, the one gate this project has: {boxes:?}"
    );
    assert!(
        boxes
            .iter()
            .any(|line| line.contains("regression test") && line.contains("tests/regressions/")),
        "the house rule is that a bug fix without a regression test under `tests/regressions/` is \
         rejected; the checklist has to ask for it: {boxes:?}"
    );
    assert!(
        boxes
            .iter()
            .any(|line| line.contains("fail") && line.contains("intended reason")),
        "TDD evidence: the test was watched failing for the intended reason before the fix: \
         {boxes:?}"
    );
    assert!(
        template.contains("type(scope): subject") || template.contains("Conventional Commits"),
        "the title is a Conventional Commit with a module scope; the template says so"
    );
}

/// The `body:` items of a parsed issue form.
fn body(form: &YamlOwned) -> &[YamlOwned] {
    form.as_mapping_get("body")
        .and_then(YamlOwned::as_vec)
        .map_or(&[][..], Vec::as_slice)
}

/// One `body:` item of a form, by its `id:`.
fn field<'a>(form: &'a YamlOwned, id: &str) -> &'a YamlOwned {
    body(form)
        .iter()
        .find(|item| item.as_mapping_get("id").and_then(YamlOwned::as_str) == Some(id))
        .unwrap_or_else(|| panic!("the form declares no field with id `{id}`"))
}

/// Whether a `body:` item is a field the reporter has to fill in.
fn is_required(item: &YamlOwned) -> bool {
    item.as_mapping_get("validations")
        .and_then(|validations| validations.as_mapping_get("required"))
        .and_then(YamlOwned::as_bool)
        == Some(true)
}

/// The `labels:` a form applies.
fn form_labels(form: &YamlOwned) -> Vec<&str> {
    form.as_mapping_get("labels")
        .and_then(YamlOwned::as_vec)
        .map(|labels| labels.iter().filter_map(YamlOwned::as_str).collect())
        .unwrap_or_default()
}

/// Every scalar string of a parsed document, in document order.
///
/// A form is a tree of `body:` items, and what these tests want to know is
/// whether the form *asks* something — which target, which revision, what to
/// strip — rather than where in the tree the sentence sits. Flattening keeps
/// the assertion about the question; unlike a substring over the file it
/// cannot be satisfied by a comment, which is exactly the gap that let an
/// unparseable form through once already.
fn scalars(node: &YamlOwned, out: &mut Vec<String>) {
    match node {
        YamlOwned::Sequence(items) => {
            for item in items {
                scalars(item, out);
            }
        }
        YamlOwned::Mapping(entries) => {
            for (key, value) in entries {
                scalars(key, out);
                scalars(value, out);
            }
        }
        other => {
            if let Some(text) = other.as_str() {
                out.push(text.to_owned());
            }
        }
    }
}

/// The flattened prose of a parsed form, one scalar per line.
fn asked(form: &YamlOwned) -> String {
    let mut out = Vec::new();
    scalars(form, &mut out);
    out.join("\n")
}

#[test]
fn the_bug_report_form_asks_for_the_artifact_the_target_and_no_secrets() {
    let text = read(".github/ISSUE_TEMPLATE/bug_report.yml");
    assert!(
        text.starts_with("# SPDX-License-Identifier: MIT OR Apache-2.0"),
        "the form carries the SPDX header:\n{text}"
    );
    let form = yaml(".github/ISSUE_TEMPLATE/bug_report.yml");
    assert_eq!(
        form.as_mapping_get("name").and_then(YamlOwned::as_str),
        Some("Bug report"),
        "an issue form needs a name; GitHub lists it in the chooser by that name"
    );
    assert!(
        form.as_mapping_get("description")
            .and_then(YamlOwned::as_str)
            .is_some_and(|description| !description.trim().is_empty()),
        "the chooser shows the description under the name; an empty one tells a reporter nothing"
    );
    assert!(
        form_labels(&form).contains(&"type: bug"),
        "the form applies `type: bug`, the vocabulary the sibling repositories use: {:?}",
        form_labels(&form)
    );
    // A ginary bug is a bug in one of three places, and which one it is cannot
    // be told without the target the artifact was built for and the ginary
    // revision that built it. The targets are held against the dropdown's own
    // `options:`, not against the file: a target named only in a comment is
    // not a target a reporter can pick.
    let options: Vec<&str> = field(&form, "target")
        .as_mapping_get("attributes")
        .and_then(|attributes| attributes.as_mapping_get("options"))
        .and_then(YamlOwned::as_vec)
        .map(|options| options.iter().filter_map(YamlOwned::as_str).collect())
        .expect("the `target` field is a dropdown with options");
    for target in [
        "linux-x86_64-gnu",
        "linux-x86_64-musl",
        "linux-aarch64-gnu",
        "linux-aarch64-musl",
        "macos-x86_64",
        "macos-aarch64",
        "windows-x86_64",
    ] {
        assert!(
            options.contains(&target),
            "the target dropdown does not offer `{target}`: {options:?}"
        );
    }
    let prose = asked(&form);
    for word in ["ginary version", "artifact", "launcher"] {
        assert!(
            prose.contains(word),
            "the form does not ask about the `{word}`, which is half of what reproduces a bug here"
        );
    }
    let lowered = prose.to_lowercase();
    assert!(
        lowered.contains("secret") || lowered.contains("credential"),
        "the form has to tell a reporter to strip secrets before pasting a log or a manifest"
    );
    // The fields a reproduction needs are required rather than optional, and
    // `required:` is read off the field rather than found anywhere in the file.
    for id in ["behavior", "reproduction", "version", "target"] {
        assert!(
            is_required(field(&form, id)),
            "the `{id}` field is optional; a report without it cannot be reproduced"
        );
    }
}

#[test]
fn the_feature_request_form_asks_for_the_failing_example_first() {
    let text = read(".github/ISSUE_TEMPLATE/feature_request.yml");
    assert!(
        text.starts_with("# SPDX-License-Identifier: MIT OR Apache-2.0"),
        "the form carries the SPDX header:\n{text}"
    );
    let form = yaml(".github/ISSUE_TEMPLATE/feature_request.yml");
    assert_eq!(
        form.as_mapping_get("name").and_then(YamlOwned::as_str),
        Some("Feature request"),
        "the form is named, so the chooser can offer it"
    );
    assert!(
        form_labels(&form).contains(&"type: feature"),
        "the form applies `type: feature`: {:?}",
        form_labels(&form)
    );
    let lowered = asked(&form).to_lowercase();
    assert!(
        lowered.contains("test") && (lowered.contains("example") || lowered.contains("acceptance")),
        "this project starts every change from a failing test, so the form asks the proposer for \
         the example that would become one"
    );
    for id in ["problem", "acceptance"] {
        assert!(
            is_required(field(&form, id)),
            "the `{id}` field is optional; the problem and the example it would be tested by are \
             what makes a proposal answerable"
        );
    }
}

#[test]
fn the_issue_config_routes_a_vulnerability_to_a_private_advisory() {
    let text = read(".github/ISSUE_TEMPLATE/config.yml");
    assert!(
        text.starts_with("# SPDX-License-Identifier: MIT OR Apache-2.0"),
        "the config carries the SPDX header:\n{text}"
    );
    let config = yaml(".github/ISSUE_TEMPLATE/config.yml");
    assert_eq!(
        config
            .as_mapping_get("blank_issues_enabled")
            .and_then(YamlOwned::as_bool),
        Some(false),
        "a blank issue skips both forms; the forms are what make a report reproducible"
    );
    let first = contact_links(&config)
        .into_iter()
        .next()
        .expect("the config offers at least one contact link");
    assert_eq!(
        first.1, "https://github.com/P4suta/ginary/security/advisories/new",
        "the first contact link is the private advisory form, so a vulnerability never becomes a \
         public issue by default"
    );
}

/// The `contact_links:` of a parsed issue config, as (name, url).
fn contact_links(config: &YamlOwned) -> Vec<(String, String)> {
    config
        .as_mapping_get("contact_links")
        .and_then(YamlOwned::as_vec)
        .map(|links| {
            links
                .iter()
                .map(|link| {
                    let at = |key: &str| {
                        link.as_mapping_get(key)
                            .and_then(YamlOwned::as_str)
                            .unwrap_or_default()
                            .to_owned()
                    };
                    (at("name"), at("url"))
                })
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn a_contact_link_that_needs_a_repository_setting_is_a_required_handover_step() {
    // A contact link is rendered by GitHub whether or not the destination
    // exists. `/discussions` 404s until Discussions is turned on, and a
    // first-time reporter sent to a 404 is the failure this file exists to
    // avoid — so the link and the setting are tied here rather than left to an
    // orchestrator's judgement about an optional step.
    let config = yaml(".github/ISSUE_TEMPLATE/config.yml");
    let handover = flowed(&read("docs/dev/log/E3.md"));
    for (name, url) in contact_links(&config) {
        if url.ends_with("/discussions") {
            assert!(
                handover.contains("**Enable Discussions** — required"),
                "`{name}` links to `{url}`, which 404s until Discussions is on, and the E3 \
                 handover does not make enabling it a required step"
            );
            assert!(
                handover.contains(&url),
                "the E3 handover has to name `{url}`, the link the setting is for"
            );
        }
    }
}

// ------------------------------------------------- the test catalogue --

#[test]
fn the_testing_document_lists_the_repository_targets_and_their_shared_reader() {
    // `docs/dev/testing.md` is the file CLAUDE.md sends a contributor to before
    // a behaviour change, and its table is titled "What exists now". A target
    // missing from it is a target nobody knows to extend; E1 left two out and
    // E3 would have made it three.
    let testing = read("docs/dev/testing.md");
    for target in [
        "tests/ci_matrix.rs",
        "tests/repo_hardening.rs",
        "tests/v1_readiness.rs",
        // E4's two: the dependency record the pre-push freshness gate reads,
        // and the digests that are on-disk format.
        "tests/deps.rs",
        "tests/digest.rs",
    ] {
        assert!(
            testing.contains(target),
            "`docs/dev/testing.md` does not list `{target}`, and its table claims to be what \
             exists now"
        );
    }
    let prose = flowed(&testing);
    for helper in [
        "tests/common/repo.rs",
        // E4's fixture half: the published SHA-256 vectors, and the recording
        // order that makes them evidence rather than a recording.
        "tests/common/digest.rs",
    ] {
        assert!(
            prose.contains(helper),
            "the repository-as-fixture targets share their readers; the document that catalogues \
             them has to say where `{helper}` is"
        );
    }
    for helper in [
        "read_or_missing",
        "parse_yaml",
        "yaml_files_under",
        // E4 added the fourth: which Rust every CI job installs, parsed rather
        // than grepped, so the MSRV cannot creep back into every job.
        "rust_toolchain_sites",
    ] {
        assert!(
            prose.contains(helper),
            "`{helper}` is a helper a test author has to know exists — the alternative is a \
             sixth hand-rolled copy — and the testing document does not name it"
        );
    }
}

// -------------------------------------------------------- the policy --

#[test]
fn the_security_policy_is_ready_for_a_public_repository() {
    let policy = read("SECURITY.md");
    let prose = flowed(&policy);
    assert!(
        prose.contains("https://github.com/P4suta/ginary/security/advisories/new"),
        "the policy names GitHub's private vulnerability reporting as the channel"
    );
    assert!(
        prose.contains("## Supported versions"),
        "the policy states which versions get a fix"
    );
    assert!(
        prose.contains("do not open a public issue"),
        "the policy says plainly that a vulnerability is not a public issue"
    );
    assert!(
        prose.contains("seven days") || prose.contains("72 hours"),
        "the policy commits to a response window, so a reporter knows when silence is a problem"
    );
    // A reproduction for ginary is a manifest, a log or an artifact, and any of
    // the three can carry a signing identity, a token from the reporter's
    // environment or a path that names them. The policy has to say what to
    // strip before that arrives in an advisory.
    let lowered = prose.to_lowercase();
    assert!(
        lowered.contains("secret") && lowered.contains("credential"),
        "the policy has to tell a reporter which parts of a reproduction to strip: no secrets, no \
         credentials, no signing identities"
    );
}
