// SPDX-License-Identifier: MIT OR Apache-2.0
//! `distribute.yml` published the OTP runtime tarballs and `catalog.json` as
//! release assets but excluded them from `SHA256SUMS`, the provenance
//! attestation, and the re-download verification.
//!
//! **What went wrong.** Every integrity step in the `publish` job globbed
//! `ginary-*`: `sha256sum ginary-* > SHA256SUMS`, the attestation
//! `subject-path: dist/release/ginary-*`, the re-download `--pattern
//! 'ginary-*'`, and the `for asset in ginary-*` verify loop. But `ginary otp
//! repack` writes `otp-<version>-<target>-<variant>.tar.zst` plus
//! `catalog.json`, and the release itself was created from `dist/release/*`.
//! So those runtime tarballs -- the actual BEAM bytes a user executes --
//! reached the published release with no checksum, no attestation, and no
//! re-download check, contradicting the workflow's own "an asset that fails
//! its checksum or its attestation never becomes part of a published release"
//! header and the house rule against silently skipping a verification.
//!
//! **The input.** The committed `.github/workflows/distribute.yml`.
//!
//! **The correct behaviour.** The checksum manifest, the attestation
//! subject-path, and the re-download-and-verify loop cover every published
//! asset -- the OTP runtime tarballs and `catalog.json` as well as the
//! `ginary` binaries -- so nothing reaches a published release unverified.

use crate::common::repo::read;

/// The body of the `publish:` job (from its header to the end of the file).
fn publish_job() -> String {
    let distribute = read(".github/workflows/distribute.yml");
    distribute
        .split_once("\n  publish:")
        .map(|(_, tail)| tail.to_owned())
        .expect("distribute.yml declares a `publish:` job")
}

/// The command a step runs, with its comment lines removed.
///
/// A step's `# ...` comments explain intent and often name the very files the
/// command must touch, so matching against the whole step can be satisfied by
/// prose the runner never executes. This keeps only the lines the shell runs.
fn command_of(step: &str) -> String {
    step.lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The text of the step whose `name:` contains `needle`, up to the next step.
fn step_containing(job: &str, needle: &str) -> String {
    let start = job
        .find(needle)
        .unwrap_or_else(|| panic!("the publish job has no step mentioning `{needle}`:\n{job}"));
    let from_name = &job[start..];
    // A step boundary is the next `- name:`/`- uses:` at the step indent.
    let end = from_name[1..]
        .find("\n      - ")
        .map(|i| i + 1)
        .unwrap_or(from_name.len());
    from_name[..end].to_owned()
}

#[test]
fn the_checksum_manifest_covers_the_otp_tarballs_and_catalog() {
    let job = publish_job();
    let step = step_containing(&job, "Compute SHA256SUMS");
    // Match the command, not the comment: the step's prose names `otp-` and
    // `catalog.json`, so asserting the whole step contains them passes even
    // on a command narrowed back to `ginary-*`.
    let command = command_of(&step);
    assert!(
        !command.contains("ginary-*"),
        "the SHA256SUMS command must not narrow to `ginary-*`, which drops the \
         OTP runtime tarballs (`otp-<version>-<target>-<variant>.tar.zst`) and \
         `catalog.json`; the runtime bytes a user executes cannot ship \
         unchecksummed:\n{command}"
    );
    assert!(
        command.contains("find") && command.contains("sha256sum"),
        "the SHA256SUMS command must checksum every asset in the directory \
         (a `find ... | sha256sum` glob-all form), so a new asset kind cannot \
         slip past the manifest:\n{command}"
    );
}

#[test]
fn the_attestation_covers_the_otp_tarballs_and_catalog() {
    let job = publish_job();
    let step = step_containing(&job, "Attest build provenance");
    assert!(
        step.contains("otp-"),
        "the provenance attestation subject-path must cover the OTP runtime \
         tarballs, not only `ginary-*`:\n{step}"
    );
    assert!(
        step.contains("catalog.json"),
        "the provenance attestation subject-path must cover `catalog.json`:\n{step}"
    );
}

#[test]
fn the_redownload_verification_is_not_narrowed_to_ginary_only() {
    let job = publish_job();
    let step = step_containing(&job, "Re-download the uploaded assets");
    assert!(
        !step.contains("ginary-*"),
        "the re-download and the attestation-verify loop must not restrict to \
         `ginary-*`, which drops the OTP runtime tarballs and `catalog.json` \
         from `sha256sum --check` and `gh attestation verify`:\n{step}"
    );
}
