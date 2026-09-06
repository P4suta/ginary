// SPDX-License-Identifier: MIT OR Apache-2.0
//! The v1 readiness sweep and the documents around it, held against the tree.
//!
//! E1 is the milestone that decides whether the project is v1, and the record
//! of that decision is `docs/dev/v1-readiness.md`: a fail-closed checklist that
//! enumerates every phase with its acceptance evidence, an honest `## Known
//! limitations` section, and a one-paragraph `## What v1 delivers`. Around it
//! sit the documents a first release needs — `docs/RELEASE.md`, the README's
//! status matrix and badges, and the CHANGELOG, which documents the phases
//! under `[Unreleased]` until a release is actually cut — and the ADR index,
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
use crate::common::version::{
    MANIFEST_FILE, last_released_version, released_section,
    sections_claiming_a_release_not_recorded, tag_references_not_recorded, unreleased_section,
};

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
        sweep.contains("CI-gated"),
        "a deferred item has to say it is workflow-authored and has never executed, not claim it \
         is done:\n{sweep}"
    );
    // The runner-only work is named rather than omitted. All three were
    // deferred once; two are closed by jobs that now run them on every push,
    // and the sweep still has to account for each by name — an item that
    // vanishes from the checklist is worse than one that stays deferred.
    for item in ["macOS", "Windows", "provenance"] {
        assert!(
            sweep.contains(item),
            "the sweep does not account for the runner-only item `{item}`"
        );
    }
}

#[test]
fn the_only_deferred_item_left_is_the_one_nothing_has_run() {
    // The fail-closed rule cuts both ways, and this is the half E23 added. A
    // checklist that leaves an item deferred once its evidence exists
    // understates the project in the document that exists to prevent
    // hand-waving, so the deferred list is held to naming exactly what has
    // never executed. Today that is the release path and nothing else: no tag
    // has been cut, so `distribute.yml` has produced no asset and no
    // attestation, while the macOS and Windows launches run on every push.
    let sweep = read("docs/dev/v1-readiness.md");
    let deferred = sweep
        .split("## The deferred items, restated plainly")
        .nth(1)
        .expect("the sweep restates its deferred items in a section of their own");
    assert!(
        deferred.contains("provenance"),
        "the release provenance has never run — no release has been cut — so it is the one item \
         the deferred section has to carry:\n{deferred}"
    );
    // The *entries*, not the prose. The section explains which items moved out
    // of it and why, and a rule that could not tell a bullet from a sentence
    // would forbid the explanation — leaving a reader who remembers the old
    // list with no account of where those two items went.
    let entries: Vec<&str> = deferred
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("- "))
        .collect();
    assert!(
        !entries.is_empty(),
        "the deferred section lists nothing at all, so this rule is measuring a list that no \
         longer exists:\n{deferred}"
    );
    for closed in ["macOS launch", "Windows launch"] {
        assert!(
            !entries.iter().any(|entry| entry.contains(closed)),
            "`{closed}` is still an entry of the deferred list, and the job that closes it \
             packages an artifact and starts it on every push. `tests/regressions/\
             e23_the_documents_said_the_launches_had_never_happened.rs` derives that from the \
             workflow:\n{entries:#?}"
        );
    }
    // Exactly one, because the section's own first word is "One". A new
    // deferred item is a real thing to add — but adding it silently under a
    // sentence that counts them is how a checklist starts disagreeing with
    // itself, and this is the document where that matters most.
    assert_eq!(
        entries.len(),
        1,
        "the deferred section opens by saying **one** kind of work is CI-gated, and lists \
         {}. The count and the sentence move together:\n{entries:#?}",
        entries.len()
    );
    assert!(
        entries[0].contains("provenance"),
        "the one deferred entry has to be the release provenance, the one thing that has never \
         executed:\n{entries:#?}"
    );
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

// The two changelog scanners live in `crate::common::version`: they are stated
// over the shape release-please's own `versionHeaderRegex` uses rather than
// over one spelling, so `## 0.2.0 - 2026-09-02` and a link to any tag are seen
// as well. See
// `tests/regressions/e20_a_dangling_changelog_link_was_pinned_by_its_tag.rs`.

#[test]
fn the_changelog_documents_the_phases_and_what_v1_delivers() {
    // Re-aimed in E20: what matters is that the work is documented, not that it
    // sits under a version heading. Before the first release the same content
    // belongs under `[Unreleased]`; release-please moves it under a dated
    // heading when a release is actually cut. Either way the phases and the
    // headline capabilities have to be there.
    let changelog = read("CHANGELOG.md");
    for phase in ["Phase A", "Phase B", "Phase C", "Phase D", "Phase E"] {
        assert!(
            changelog.contains(phase),
            "the changelog does not account for `{phase}`"
        );
    }
    for capability in ["launcher", "cache", "catalog", "cross", "verify"] {
        assert!(
            changelog.to_lowercase().contains(capability),
            "the changelog does not mention the `{capability}` work"
        );
    }
}

/// What the release notes of this project's first release are made of: the
/// phase summary and the two Keep a Changelog sections under it.
const THE_RELEASE_NOTES: [&str; 7] = [
    "Phase A",
    "Phase B",
    "Phase C",
    "Phase D",
    "Phase E",
    "### Added",
    "### Changed",
];

#[test]
fn the_unreleased_section_holds_only_work_that_is_not_released() {
    // Which section that content belongs under is decided by the record, not by
    // a document: `.release-please-manifest.json` is what release-please writes
    // in the same commit as the changelog section, so the two cannot disagree
    // about which release the work went out in.
    let changelog = read("CHANGELOG.md");
    let unreleased = unreleased_section(&changelog);
    let Some(version) = last_released_version() else {
        for mark in THE_RELEASE_NOTES {
            assert!(
                unreleased.contains(mark),
                "{MANIFEST_FILE} records no release, so this work is done and unreleased and \
                 belongs under `## [Unreleased]`; `{mark}` is not there"
            );
        }
        return;
    };
    let released = released_section(&changelog, &version).unwrap_or_else(|| {
        panic!(
            "{MANIFEST_FILE} records {version} as released, and the changelog has no section for \
             it. release-please generates that heading when it prepares the release; a manifest \
             recording a version the changelog never names is drift between the two files \
             release-please writes together"
        )
    });
    for mark in THE_RELEASE_NOTES {
        assert!(
            !unreleased.contains(mark),
            "{MANIFEST_FILE} records {version} as released, so `{mark}` describes released work \
             and must not still sit under `## [Unreleased]`. release-please inserts its section \
             *above* that heading and moves nothing, so this is the edit `docs/RELEASE.md` asks \
             the maintainer to make while reviewing the release pull request"
        );
        assert!(
            released.contains(mark),
            "`{mark}` is part of the release notes for {version} and belongs under its heading; \
             release-please writes commit subjects and cannot write prose"
        );
    }
}

#[test]
fn the_changelog_claims_no_release_the_manifest_does_not_record() {
    let changelog = read("CHANGELOG.md");
    let last = last_released_version();
    let claimed = sections_claiming_a_release_not_recorded(&changelog, last.as_deref());
    assert!(
        claimed.is_empty(),
        "the changelog carries {claimed:?}, and {MANIFEST_FILE} records {}. A version heading is \
         release-please's output when a release is cut, and a heading for a version the record \
         does not have — hand-written, or left behind by a proposal that never merged — is a \
         release nobody made",
        last.as_deref().unwrap_or("nothing released")
    );
}

#[test]
fn the_changelog_links_no_tag_the_manifest_does_not_record() {
    let changelog = read("CHANGELOG.md");
    let last = last_released_version();
    let dangling = tag_references_not_recorded(&changelog, last.as_deref());
    assert!(
        dangling.is_empty(),
        "the changelog links {dangling:?}, and {MANIFEST_FILE} records {}; a reader following one \
         gets a 404 from the project's own release notes",
        last.as_deref().unwrap_or("nothing released")
    );
    let commits = "[Unreleased]: https://github.com/P4suta/ginary/commits/main";
    let compare = last.as_ref().map(|version| {
        format!("[Unreleased]: https://github.com/P4suta/ginary/compare/v{version}...HEAD")
    });
    assert!(
        changelog.contains(commits) || compare.is_some_and(|compare| changelog.contains(&compare)),
        "`[Unreleased]` has to point somewhere honest: the commit history, or a comparison \
         against the tag {MANIFEST_FILE} records. Both spellings are accepted so that a release \
         does not oblige anyone to rewrite this line"
    );
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
