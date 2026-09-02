<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# Cutting a release

This document is what a maintainer runs to cut a ginary release. It is written for `v0.1.0`, the
first one, but every later release is the same three moves: let release-please prepare the
version, publish the draft, let distribute verify and flip it.

No release has been cut yet. The repository is now live at <https://github.com/P4suta/ginary>
and the workflows run for real, but the house rule stands: a tag, a push or a publish waits for an
explicit request. What follows is the procedure the workflows carry out when one is made — and,
first, the one part of it that no workflow can do for itself.

## One-time setup

`release.yml` authenticates as a **GitHub App**, not as GitHub Actions. It has to: this repository
sets `can_approve_pull_request_reviews` to `false`, so the default `GITHUB_TOKEN` may not create a
pull request, and release-please's whole job is to maintain one. The first live run failed with
`GitHub Actions is not permitted to create or approve pull requests`, which is that hardening
working as designed. The App is the way through it that leaves the hardening alone, and it is the
same pattern the strict siblings (`release-glz`, `beamtrace`) use.

Installing an App and holding its private key needs a human with admin rights, so until a
maintainer does the three steps below, the `release-please` job **skips** and a
`credentials-notice` job prints what is missing and exits 0. The Release workflow is green and
says why it did nothing; it never goes red for a credential nobody in the tree can add.

A maintainer with admin rights on `P4suta/ginary` does this once:

1. **Create or install the App.** Install a release-please GitHub App on `P4suta/ginary`,
   granting it `contents: write`, `pull-requests: write` and `issues: write` — the version bump
   and changelog, the pull request itself, and the label it carries. Nothing wider: the
   installation token `actions/create-github-app-token` mints is narrowed to exactly those three
   scopes in the workflow.
2. **Add the repository variable `RELEASE_PLEASE_APP_CLIENT_ID`**, set to the App's client id.
   Settings → Secrets and variables → Actions → Variables. It is a variable rather than a secret
   because it is not one: a client id is public, and the job's `if:` reads it to decide whether
   the credentials exist at all.
3. **Add the repository secret `RELEASE_PLEASE_APP_PRIVATE_KEY`**, set to the App's PEM private
   key. Settings → Secrets and variables → Actions → Secrets.

The next push to `main` then runs release-please for real.

The two credentials are not symmetric, because a job's `if:` can read `vars.` and cannot read
`secrets.`. Removing the **variable** returns the workflow to the skip-and-report state: the
`release-please` job skips, `credentials-notice` prints what to add, and the run is green.
Removing only the **secret** leaves the variable saying "automation is configured" when it is
not, so the first step of the `release-please` job checks the private key through its `env:` and
fails with a message naming `RELEASE_PLEASE_APP_PRIVATE_KEY`. That state is red on purpose: a
maintainer set the variable, the automation they asked for is not running, and the fastest way to
say so is the name of the credential that is missing.

## The version is one number, everywhere

ginary is **version-locked** to its stubs. Every artifact of one release — the command line
tool, the seven stubs, and the OTP catalog tarballs — shares a single version, because a
launcher only reads the payload format its own build writes. A stub from `0.1.0` and a payload
from `0.2.0` is exactly the mismatch the version lock exists to prevent.

That single number lives in `Cargo.toml`. `.release-please-manifest.json` mirrors it, and a
release tag has to equal it. `scripts/ci/version-consistency.sh` is the check that proves it, and
`distribute.yml` runs that check before it builds or uploads anything: a tag of `v0.1.0` against
a `Cargo.toml` of `0.1.0` passes, and any drift fails the release, naming both sides.

## The three steps

### 1. release-please prepares the version

`release.yml` runs `release-please` on every push to `main`, authenticated with the App token from
the one-time setup above. It reads the Conventional Commits since the last release and maintains a
**release pull request** that bumps the version in `Cargo.toml` and
`.release-please-manifest.json` and rewrites the `[Unreleased]` section of `CHANGELOG.md` into a
dated release section.

For `v0.1.0`, review that pull request: confirm the version is `0.1.0`, that the changelog reads
correctly, and that `Cargo.toml` and the manifest agree. Merging it is a deliberate act — the
version bump and the changelog are a human decision, not an automatic one.

### 2. The draft release is created

When the release pull request merges, release-please creates the tag `v0.1.0` and a **draft**
GitHub release (`draft: true` in `release-please-config.json`). Nothing is public yet: a draft
release is visible only to maintainers, and its assets do not exist until distribute builds them.

### 3. distribute verifies, then publishes

Publishing the draft release triggers `distribute.yml`, which mirrors a strict
verify-then-publish discipline:

1. `version-consistency.sh` proves the tag equals `Cargo.toml`.
2. The build matrix produces, for all seven targets, the full `ginary` binary and the
   launcher-only `ginary-stub`: the four Linux targets via `cross`, the two macOS targets built
   natively on `macos-13` and `macos-14`, and `windows-x86_64` on `windows-2022`. `ginary otp
   repack` produces the OTP catalog tarballs on the appropriate runners.
3. `actions/attest-build-provenance` signs a provenance attestation for every asset, and a
   `SHA256SUMS` manifest is computed.
4. The release is created as a **draft** and the assets are uploaded to it.
5. The assets are **re-downloaded** and checked: `sha256sum --check SHA256SUMS`, and
   `gh attestation verify` against each one. A corrupt upload or a bad attestation fails here,
   while the release is still a draft and nothing is public.
6. Only when every check has passed does distribute flip the release out of draft
   (`gh release edit --draft=false`). An asset that failed its checks never becomes part of a
   published release.

## What a maintainer actually types

For `v0.1.0`, once the repository is published and CI is green:

```console
# 1. Merge the release-please pull request titled "chore(main): release 0.1.0".
#    The tag v0.1.0 and the draft release appear when it merges.

# 2. Publish the draft release from the GitHub UI (or with gh):
$ gh release edit v0.1.0 --draft=false   # only to trigger distribute; distribute re-drafts
```

In practice the maintainer publishes the release-please draft, distribute builds and re-verifies
the assets, and the final flip out of draft is distribute's own last step. The maintainer's job
is to review the release pull request and to publish the draft; the workflows do the rest, and
refuse to publish anything that does not check out.

## Nothing is tagged or published outside this flow

Do not `git tag`, `cargo publish`, or create a release by hand. The version lock, the checksums
and the attestations are only meaningful when the whole flow runs; a hand-cut tag skips
`version-consistency.sh` and the re-download check, which is exactly the discipline this document
exists to keep.
