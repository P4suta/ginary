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
maintainer does the three steps below, the `release-please` job runs, finds nothing, prints what
is missing and exits 0. The Release workflow is green and says why it did nothing; it never goes
red for a credential nobody in the tree can add.

Both values live in the **`release` environment** of the repository rather than at repository
scope, and that is the point of the setup rather than a detail of it. An environment applies two
restrictions repository scope does not. Its variables and secrets reach a job **only when the job
declares that environment**, so a job that does not name it is handed nothing at all; and they
are released only on a ref the environment's own protection rules admit, which here is a
deployment-branch policy of the `main` branch and the `v*` tags and nothing else. A value at
repository scope carries neither restriction: no job has to ask for it, and no branch policy
stands in front of it.

That policy is only as strong as the bypass beside it. GitHub lets a repository administrator
force a waiting job past an environment's protection rules by default, and a job released that
way is handed the environment's secrets like any other. The `release` environment of this
repository therefore has **Allow administrators to bypass configured protection rules turned
off** (`can_admins_bypass: false`), which is what lets the claim be made without a qualifier: the
App's private key is unreachable from a pull request, from a fork, and from every branch that is
not `main`. Turn that setting back on and the claim stops being true, so it is part of the setup
rather than a preference — a repository restoring this environment from these notes has to set
it.

What the environment does **not** do is make the release job the only reader. Declaring an
environment is not a privilege GitHub hands to one job: any job of any workflow in this repository
may write `environment: release`, and on a ref the branch policy admits it is handed the same
client id and the same private key. A second reader is one line of YAML away, and nothing on
GitHub's side says no.

What says no is two of this repository's own tests, both in `tests/release_workflow.rs`, and it is
worth being exact about which half each one covers — a control credited with more than it does is
the same mistake as crediting the platform with it:

- `no_job_of_any_workflow_reads_the_release_credentials_without_declaring_the_environment` walks
  every scalar of every workflow for either credential name and requires each site it finds to sit
  in a job that declares the environment. That bounds **where** a credential may be read, not how
  many jobs read it: a second job that declares the environment and reads the private key
  satisfies it. It is pinned by
  `tests/regressions/e17_the_release_credentials_were_read_outside_their_environment.rs`.
- `exactly_one_job_of_this_repository_declares_the_release_environment` is the half that bounds
  the **number**. It collects every job of every workflow whose `environment:` is `release` — the
  jobs GitHub hands the values to, whether or not they name a credential — and requires there to
  be exactly one.
  `tests/regressions/e18_nothing_bounded_the_number_of_jobs_that_may_read_the_credentials.rs` pins
  it, against a fixture workflow that holds a second declaring job.

Together they make a second reader a red suite in a pull request a human reviews — a real control,
and ours rather than the platform's, which is why it is written down here: delete both tests and
nothing outside this repository objects. A later milestone that needs a second environment-bound
job should give it an environment of its own rather than widening that list.

A maintainer with admin rights on `P4suta/ginary` does this once:

1. **Create or install the App.** Install a release-please GitHub App on `P4suta/ginary`,
   granting it `contents: write`, `pull-requests: write` and `issues: write` — the version bump
   and changelog, the pull request itself, and the label it carries. Nothing wider: the
   installation token `actions/create-github-app-token` mints is narrowed to exactly those three
   scopes in the workflow.
2. **Create the `release` environment**, under Settings -> Environments, and give it a
   deployment-branch policy of exactly the branch `main` and the tag pattern `v*`. (On
   `P4suta/ginary` it already exists, with that policy.) That policy is the whole of its
   configuration: **add no other protection rule to it.** `release.yml`'s one job is bound to
   this environment, and every push to `main` therefore requests a deployment to it — including
   the pushes that release nothing and end in the notice below. A required reviewer or a wait
   timer on an environment named `release` would suspend all of them pending approval, which
   turns the green "nothing to release" run into a pending one nobody asked for. A later
   milestone that wants a reviewer gate should put it on a separate environment for
   `distribute.yml`, which is where publishing actually happens.
3. **Add both credentials to that environment**, and to nothing else:
   - `RELEASE_PLEASE_APP_CLIENT_ID`, set to the App's client id, under
     Settings -> Environments -> release -> Environment variables. It is a variable rather than
     a secret because it is not one: a client id is public.
   - `RELEASE_PLEASE_APP_PRIVATE_KEY`, set to the App's PEM private key, under
     Settings -> Environments -> release -> Environment secrets.

The next push to `main` then runs release-please for real.

The `release-please` job declares `environment: release` and carries no `if:` of its own, because
a job condition is evaluated before the job's environment is bound — it cannot see an
environment's variables, and it cannot see a `secrets` context at all. Its first step reads both
values through its `env:`, and every later step is guarded on what that step found:

- **Neither credential.** The job prints the notice naming the two values and the environment
  they belong in, and exits 0. This is the state of a fork, and of any repository that has not
  done the setup above.
- **Both credentials.** The checkout, the App token and release-please run.
- **One of the two.** The job fails with a message naming the missing credential. That state is
  red on purpose: a maintainer added half the pair, the automation they asked for is not running,
  and the fastest way to say so is the name of the credential that is absent. Removing the other
  half returns the workflow to the report-and-stay-green state.

Because the job is bound to the environment, `release.yml` triggers on a push to `main` and on
nothing else. A job that declares an environment the current ref may not deploy to does not skip:
the run fails with `Branch is not allowed to deploy to release due to environment protection
rules`. A `pull_request` or `workflow_dispatch` trigger here would be exactly that, so the
workflow does not carry one, and `tests/release_workflow.rs` holds it to that.

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
   natively on `macos-15-intel` and `macos-14`, and `windows-x86_64` on `windows-2022`. `ginary otp
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
