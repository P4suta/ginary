// SPDX-License-Identifier: MIT OR Apache-2.0
//! The release and distribute workflows, held against the repository.
//!
//! These two workflows are authored, never run: an Action needs a remote, and
//! the house rule is that nothing is tagged or published without an explicit
//! request. So the deliverable is a workflow that is correct by inspection —
//! `release-please` drives the version bump and the draft, and `distribute.yml`
//! builds every artifact, checks it, and only then flips the release out of
//! draft. This file pins the discipline that makes "verify then publish" a
//! property of the YAML rather than of the person who runs it: the seven
//! targets, the checksums, the attestations, the re-download-and-check, and the
//! order the draft is flipped in.
//!
//! Neither file exists yet; every test fails at the read that looks for it.
//!
//! Ungated: a workflow is neither half of the crate.

mod common;

use crate::common::repo::read;

// ------------------------------------------------------- release.yml --

#[test]
fn the_release_workflow_is_driven_by_release_please() {
    let release = read(".github/workflows/release.yml");
    assert!(
        release.contains("release-please"),
        "release.yml drives version bumps and the draft release through release-please:\n{release}"
    );
    assert!(
        release.contains("permissions: {}"),
        "release.yml sets the default permissions to none and widens per job"
    );
}

#[test]
fn a_published_release_triggers_the_distribute_workflow() {
    let release = read(".github/workflows/release.yml");
    let distribute = read(".github/workflows/distribute.yml");
    // distribute runs on `release: published` or is called by release.yml.
    assert!(
        distribute.contains("workflow_call") || distribute.contains("release:"),
        "distribute.yml runs on a published release or as a reusable workflow:\n{distribute}"
    );
    assert!(
        release.contains("distribute") || distribute.contains("workflow_call"),
        "the two workflows are wired together"
    );
}

// ---------------------------------------------------- distribute.yml --

#[test]
fn distribute_builds_every_target_of_the_release() {
    let distribute = read(".github/workflows/distribute.yml");
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
            distribute.contains(target),
            "distribute.yml does not produce artifacts for `{target}`; a release must carry all \
             seven"
        );
    }
}

#[test]
fn distribute_produces_both_the_binary_and_the_stub_and_the_otp_tarballs() {
    let distribute = read(".github/workflows/distribute.yml");
    assert!(
        distribute.contains("ginary-stub") || distribute.contains("--no-default-features"),
        "each target ships the launcher-only stub as well as the full binary"
    );
    assert!(
        distribute.contains("otp repack"),
        "the OTP catalog tarballs are produced by `ginary otp repack` on the right runner"
    );
}

#[test]
fn distribute_verifies_before_it_publishes() {
    let distribute = read(".github/workflows/distribute.yml");
    // The checksums, the attestation, the re-download check, and the flip.
    for needle in [
        "SHA256SUMS",
        "attest-build-provenance",
        "sha256sum",
        "--check",
    ] {
        assert!(
            distribute.contains(needle),
            "distribute.yml is missing `{needle}`: the verify-then-publish discipline is \
             incomplete"
        );
    }
    assert!(
        distribute.contains("attestation verify") || distribute.contains("gh attestation"),
        "the attestation is verified after re-download, not only produced"
    );
}

#[test]
fn distribute_creates_a_draft_first_and_flips_it_only_after_the_checks() {
    let distribute = read(".github/workflows/distribute.yml");
    let draft = distribute
        .find("draft: true")
        .or_else(|| distribute.find("--draft"))
        .expect("the release is created as a draft first");
    let flip = distribute
        .find("draft=false")
        .or_else(|| distribute.find("draft: false"))
        .or_else(|| distribute.find("--draft=false"))
        .expect("and the draft is flipped to a published release at the end");
    assert!(
        draft < flip,
        "the draft is created before it is flipped: an artifact that fails its checks never \
         becomes a published release"
    );
    let check = distribute
        .find("--check")
        .expect("sha256sum --check runs on the re-downloaded assets");
    assert!(
        check < flip,
        "the checksum re-check comes before the flip, or a bad asset would already be public"
    );
}

#[test]
fn distribute_runs_the_version_consistency_check() {
    let distribute = read(".github/workflows/distribute.yml");
    assert!(
        distribute.contains("version-consistency.sh"),
        "the tag and Cargo.toml are proved equal before anything is uploaded:\n{distribute}"
    );
}
