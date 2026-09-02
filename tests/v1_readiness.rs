// SPDX-License-Identifier: MIT OR Apache-2.0
//! The v1 readiness sweep and the documents around it, held against the tree.
//!
//! E1 is the milestone that decides whether the project is v1, and the record
//! of that decision is `docs/dev/v1-readiness.md`: a fail-closed checklist that
//! enumerates every phase with its acceptance evidence, an honest `## Known
//! limitations` section, and a one-paragraph `## What v1 delivers`. Around it
//! sit the documents a first release needs — `docs/RELEASE.md`, the README's
//! status matrix and badges, the CHANGELOG's 0.1.0 entry — and the ADR index,
//! which has to name every decision including the last one. This file pins that
//! each of those exists and says what the milestone promised, so "v1 is ready"
//! is a claim backed by a document rather than a feeling.
//!
//! Most of these files or sections do not exist yet; each test fails at the
//! assertion that looks for its subject.
//!
//! Ungated: documentation belongs to the whole project.

mod common;

use crate::common::repo::{read, root};

// -------------------------------------------- docs/dev/v1-readiness.md --

#[test]
fn the_readiness_sweep_enumerates_every_phase() {
    let sweep = read("docs/dev/v1-readiness.md");
    for phase in ["Phase A", "Phase B", "Phase C", "Phase D", "Phase E"] {
        assert!(
            sweep.contains(phase),
            "the readiness sweep does not account for `{phase}`"
        );
    }
    for evidence in ["e2e_hello", "e2e_cross", "smoke", "smoke-matrix", "TLA+"] {
        assert!(
            sweep.contains(evidence),
            "the sweep does not cite the `{evidence}` acceptance evidence"
        );
    }
}

#[test]
fn the_readiness_sweep_records_the_artifact_sizes() {
    let sweep = read("docs/dev/v1-readiness.md");
    // The numbers the plan fixed as acceptance evidence, each beside its target.
    for size in ["5.4", "6.3", "6.6", "4.9", "12.2"] {
        assert!(
            sweep.contains(size),
            "the sweep is missing the `{size} MB` artifact-size evidence"
        );
    }
    assert!(
        sweep.contains("MB"),
        "the sizes are stated in megabytes a reader can compare"
    );
}

#[test]
fn deferred_items_are_honest_about_being_ci_gated_rather_than_hand_waved() {
    let sweep = read("docs/dev/v1-readiness.md");
    assert!(
        sweep.contains("CI-gated") || sweep.contains("runs when the repo has a remote"),
        "a deferred item has to say it is workflow-authored and runs on a remote, not claim it \
         is done:\n{sweep}"
    );
    // The runner-only work is named as deferred rather than omitted.
    for deferred in ["macOS", "Windows", "provenance"] {
        assert!(
            sweep.contains(deferred),
            "the sweep does not list the runner-gated item `{deferred}`"
        );
    }
}

#[test]
fn the_sweep_has_a_known_limitations_section_with_the_real_caveats() {
    let sweep = read("docs/dev/v1-readiness.md");
    assert!(
        sweep.contains("## Known limitations"),
        "the sweep consolidates the honest caveats under `## Known limitations`"
    );
    let limits = sweep
        .split("## Known limitations")
        .nth(1)
        .expect("the section body");
    for caveat in [
        "NIF",        // static-musl cannot dlopen a NIF
        "2.36",       // the glibc floor of the gnu variant
        "hot-code",   // no hot code upgrade
        "-name",      // distribution requires -name in config
        "Gatekeeper", // ad-hoc signing satisfies the kernel, not Gatekeeper
        "major",      // host OTP major must match
    ] {
        assert!(
            limits.contains(caveat),
            "the known-limitations section does not mention `{caveat}`:\n{limits}"
        );
    }
}

#[test]
fn the_sweep_says_what_v1_delivers_in_one_paragraph() {
    let sweep = read("docs/dev/v1-readiness.md");
    assert!(
        sweep.contains("## What v1 delivers"),
        "the sweep carries a `## What v1 delivers` summary suitable for the README top"
    );
}

// ------------------------------------------------------ docs/RELEASE.md --

#[test]
fn the_release_document_says_what_a_maintainer_runs_to_cut_v0_1_0() {
    let release = read("docs/RELEASE.md");
    assert!(
        release.contains("0.1.0"),
        "docs/RELEASE.md walks through cutting v0.1.0 specifically"
    );
    assert!(
        release.contains("release-please") && release.contains("distribute"),
        "it names the two workflows a release goes through"
    );
    assert!(
        release.contains("version-locked") || release.contains("version lock"),
        "it states that ginary is version-locked to its stubs, so one release shares one version"
    );
}

// ------------------------------------------------------------- README --

#[test]
fn the_readme_carries_a_target_status_matrix() {
    let readme = read("README.md");
    for target in [
        "linux-x86_64-gnu",
        "linux-aarch64-musl",
        "macos-aarch64",
        "windows-x86_64",
    ] {
        assert!(
            readme.contains(target),
            "the README status matrix has no row for `{target}`"
        );
    }
    // The three axes the spec names, distinct from the prose that already
    // mentions the targets: a matrix says, per target, whether it builds here,
    // runs here, and runs on CI.
    assert!(
        readme.contains("runs on CI"),
        "the status matrix has to distinguish `runs here` from `runs on CI`, which the current \
         prose does not"
    );
    assert!(
        readme.contains("builds") && readme.contains("runs here"),
        "the matrix columns name where each target builds and runs"
    );
}

/// Markdown with every HTML comment removed.
///
/// A badge inside `<!-- ... -->` renders nothing, so a test that only looks
/// for the text cannot tell a live badge from a commented placeholder.
fn uncommented(markdown: &str) -> String {
    let mut out = String::new();
    let mut rest = markdown;
    while let Some(start) = rest.find("<!--") {
        out.push_str(&rest[..start]);
        rest = match rest[start..].find("-->") {
            Some(end) => &rest[start + end + 3..],
            None => "",
        };
    }
    out.push_str(rest);
    out
}

#[test]
fn the_readme_badges_point_at_the_published_repository() {
    let readme = read("README.md");
    // E1 left the badges commented out because the repository did not exist and
    // a live badge would have 404ed. E3 decides the slug, so they go live: a
    // commented badge is a status nobody sees.
    assert!(
        !readme.contains("<owner>"),
        "the `<owner>` placeholder outlived E1; the repository is `P4suta/ginary`"
    );
    let live = uncommented(&readme);
    let badges: Vec<&str> = live
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("![") || line.starts_with("[!["))
        .collect();
    assert!(
        !badges.is_empty(),
        "every badge in the README is still inside an HTML comment, so none of them renders"
    );
    for workflow in ["ci.yml", "codeql.yml"] {
        let url =
            format!("https://github.com/P4suta/ginary/actions/workflows/{workflow}/badge.svg");
        assert!(
            badges.iter().any(|badge| badge.contains(&url)),
            "no live badge reports `{workflow}`: {badges:?}"
        );
    }
    assert!(
        badges
            .iter()
            .any(|badge| badge.to_lowercase().contains("scorecard")),
        "the OpenSSF Scorecard result is a public number; the README shows it: {badges:?}"
    );
    assert!(
        badges
            .iter()
            .any(|badge| badge.to_lowercase().contains("licen") && badge.contains("MIT")),
        "a badge names the licence, which is `MIT OR Apache-2.0`: {badges:?}"
    );
    // The crate is `publish = false` and has no rustdoc on docs.rs. A badge for
    // either would be a claim the tree cannot back.
    for absent in ["crates.io", "docs.rs"] {
        assert!(
            badges.iter().all(|badge| !badge.contains(absent)),
            "a `{absent}` badge claims a publication that has not happened: {badges:?}"
        );
    }
}

#[test]
fn the_readme_carries_the_one_paragraph_v1_summary() {
    let readme = read("README.md");
    // The README top no longer calls the project Alpha; it states what v1
    // delivers, mirroring the readiness sweep's `## What v1 delivers`.
    assert!(
        !readme.contains("**Alpha.**"),
        "the README still calls the project Alpha; v1 replaces that with the delivery summary"
    );
    assert!(
        readme.contains("v1"),
        "the README top carries the one-paragraph v1 summary"
    );
}

// ---------------------------------------------------------- CHANGELOG --

#[test]
fn the_changelog_summarizes_phase_a_through_e_for_the_first_release() {
    let changelog = read("CHANGELOG.md");
    assert!(
        changelog.contains("0.1.0"),
        "the CHANGELOG has a 0.1.0 entry for the first release, not only `[Unreleased]`"
    );
    // The release note names each phase's headline capability rather than a
    // bare version bump.
    for capability in ["launcher", "cache", "catalog", "cross", "verify"] {
        assert!(
            changelog.to_lowercase().contains(capability),
            "the 0.1.0 note does not mention the `{capability}` work"
        );
    }
}

// --------------------------------------------------------- the ADR index --

#[test]
fn every_adr_is_listed_in_the_index() {
    let index = read("docs/adr/README.md");
    let mut missing: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(root().join("docs/adr")).expect("read docs/adr") {
        let name = entry
            .expect("dir entry")
            .file_name()
            .to_string_lossy()
            .into_owned();
        // Every numbered ADR file, but not the index itself.
        if !name.ends_with(".md") || name == "README.md" {
            continue;
        }
        let stem = name.trim_end_matches(".md");
        if !index.contains(stem) {
            missing.push(stem.to_owned());
        }
    }
    missing.sort();
    assert!(
        missing.is_empty(),
        "docs/adr/README.md does not index these ADRs: {missing:?}"
    );
}
