<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# Cutting a release

This document is what a maintainer runs to cut a ginary release. It is written for `v0.1.0`, the
first one, but every later release is the same three moves: let release-please prepare the
version, rehearse distribution against its tag, then explicitly authorize verified publication.

Whether anything has been released is not written down here. It is recorded in
`.release-please-manifest.json`, whose `0.0.0` is release-please's own spelling of "this package
has never been released" and whose any other value is the last released version. That record moves
in the same commit as the changelog section it has to agree with, so it is the one answer that
cannot be left behind; a sentence in this document could only be a copy of it that somebody has to
remember to delete. `git log --oneline -- .release-please-manifest.json` is the history of it.

The repository is live at <https://github.com/P4suta/ginary> and the workflows run for real, but
the house rule stands: a tag, a push or a publish waits for an explicit request. What follows is
the procedure the workflows carry out when one is made — and, first, the one part of it that no
workflow can do for itself.

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

ginary is **version-locked** to its stubs. The command line tool and seven stubs share the
ginary version, because a launcher only reads the payload format its own build writes. A stub
from `0.1.0` and a payload from `0.2.0` is exactly the mismatch the version lock exists to
prevent. OTP archives retain their independent OTP version, recorded in the merged catalog;
the distribution inventory records the ginary version and every archive's exact bytes.

That single number lives in `Cargo.toml`, and a release tag has to equal it.

`.release-please-manifest.json` is **not** a second copy of it. release-please reads that file as
the **last released version** and derives the next proposal from it, so before the first release
the two records legitimately differ: the manifest records `0.0.0` — release-please's own spelling
of "this package has never been released" — while `Cargo.toml` carries the version being prepared.
Recording anything else there asserts a release that was made, and a manifest of `0.1.0` for a
repository with no tag is what made release-please propose `0.2.0` for a project that had released
nothing (`docs/dev/log/E20.md`). From the first release onward release-please writes `Cargo.toml`
and the manifest in one commit, and the two hold one version between them.

`scripts/ci/version-consistency.sh` checks the tag, `Cargo.toml` and `.release-please-manifest.json`.
`distribute.yml` runs it before it builds or uploads anything: a tag of `v0.1.0` against a `Cargo.toml` of `0.1.0`
and a manifest recording `0.1.0` passes. A tag cut while the manifest still records `0.0.0` fails —
that is a hand-cut tag, because release-please writes the version into the manifest before it
creates the tag — and so does any other drift, naming both sides.

`release-please-config.json` states the first version this repository will release,
`"initial-version": "0.1.0"`. `release-type: rust` independently starts a never-released package at
`0.1.0`, so the key does not change today's answer; it says in the file release-please reads what
the strategy does silently, and the two cannot drift apart unnoticed.

## The three steps

### 1. release-please prepares the version

`release.yml` runs `release-please` on every push to `main`, authenticated with the App token from
the one-time setup above. It reads the Conventional Commits since the last release and maintains a
**release pull request** that bumps the version in `Cargo.toml` and
`.release-please-manifest.json` and writes a dated release section into `CHANGELOG.md`, built out
of those commit messages.

**Where that section lands, and what it does not touch.** release-please's changelog updater
searches the file for its own `\n###? v?[0-9[]` and splices the generated section in immediately
above the first line that matches. In this changelog that line is `## [Unreleased]` — the `[` is
inside that character class — so the new section is inserted **above the `## [Unreleased]`
heading** and everything under `[Unreleased]` is left exactly where it was. It is not rewritten,
moved or consumed. (`src/updaters/changelog.ts`, quoted in `docs/dev/log/E20.md`.)

Review that pull request: confirm the version, that `Cargo.toml` and the manifest agree, and that
the generated section reads correctly. Then **clear the `[Unreleased]` section by hand** in the
same pull request, because the work it describes is the work being released and release-please
will not move it. Concretely, three edits to `CHANGELOG.md`:

1. **Fold the hand-written body into the generated section.** Everything under `## [Unreleased]`
   — the prose, `### Added`, `### Changed` — describes the release being cut. Move it under the
   `## <version>` heading release-please generated, above or below its commit lists as reads
   best. release-please writes commit subjects and cannot write prose; this is the only part of
   the release notes a person has to place.
2. **Put an empty `## [Unreleased]` back above that heading.** release-please inserted its section
   above the old one, so `[Unreleased]` is now *below* a release. Left there it is also below the
   insertion point of the next release, and sinks one section further every time — the section for
   work that is not released ends up at the bottom of the file for ever.
3. **Point the `[Unreleased]` link at the new tag**, `compare/v<version>...HEAD`, or leave it at
   `commits/main`. Both are honest; a link to a tag past the one the manifest records is not.

The suite holds all three: `the_unreleased_heading_is_the_first_version_header` fails while
`[Unreleased]` is not back on top, and `the_unreleased_section_holds_only_work_that_is_not_released`
fails while the released work is still filed under it. A red release pull request here is that
check working — the edits are what turn it green, and they are why merging is a deliberate act
rather than an automatic one. release-please regenerates its branch when new commits reach `main`,
which would discard them, so make the edits when the merge is next.

### 2. The draft release is created

When the release pull request merges, release-please creates the tag `v0.1.0` and a **draft**
GitHub release (`draft: true` in `release-please-config.json`). Nothing is public yet: a draft
release is visible only to maintainers, and its assets do not exist until distribute builds them.

### 3. Rehearse distribution before authorizing publication

`distribute.yml` has `workflow_dispatch` and `workflow_call` inputs: a required existing `tag`
and `publish`, which defaults to `false`. A published-release event never starts distribution.
The tag is resolved once, and every builder checks out that same commit with its commit time as
`SOURCE_DATE_EPOCH`.

The seven native/cross builders produce a full binary, a stub and an OTP runtime for each target.
Linux repacks the verified upstream archive; Windows and macOS copy and validate the OTP root
installed by the pinned setup action. Root provenance explicitly identifies a local tree and
its digest; it does not claim verification of an upstream archive that was never inspected.

Each target uploads a separate fragment directory. `ginary otp merge --inputs DIR --out DIR
--version VERSION` checks all seven binary identities, object architectures and flavors, the
runtime digests and lengths, and agreement on the OTP version. It refuses missing/extra assets,
conflicting catalog entries and an existing output. The merged catalog, inventory and
`SHA256SUMS` are written as one completed output directory. The default workflow preserves this
result for review and performs no hosted release operation.

Local assembly can be rehearsed without GitHub:

The output directory must be new. Assembly verifies the copied binaries, stubs and runtimes,
reserves the directory exclusively, then publishes files without replacing existing paths.
`SHA256SUMS` is written last and marks a complete distribution. If publication fails, the error
names the retained staging directory; the partial output can be inspected, and a retry should
use a new output path. An existing directory is never removed or reused automatically.

```console
$ ginary otp repack --upstream-tag OTP-29.0.5 --targets windows-x86_64 --root PATH --out dist/dist-windows-x86_64
$ ginary otp merge --inputs dist/fragments --out dist/verified --version 0.1.0
$ cargo test --test distribution
```

The merge input contains `dist-<target>` directories from all seven builders, each containing its
versioned full binary, versioned stub, runtime tarball and `catalog.json`. The local transaction
test uses a readonly mock `gh` shell function and cannot create, upload or publish anything.

### 4. Optional publication requires separate authorization

Only an explicitly authorized `publish=true` run uses the hosted path. It requires the release
for the tag to already exist as a draft; it never creates a release and refuses an already-public
one. It attests the complete inventory, uploads to the existing draft, downloads every asset,
checks the exact asset set and checksums, verifies every attestation, rechecks draft state, and
only then changes `draft=false`. Failures leave the draft unpublished. Same-tag workflow runs are
serialized; reruns may replace the same draft assets but cannot modify an existing public release.

No actual release operation is authorized by the development work recorded in F1. No tag,
draft, release, hosted release asset, or publication was created by that work. Publishing remains
a separate maintainer action after the relevant native CI and local rehearsal evidence is read.
## Nothing is tagged or published outside this flow

Do not `git tag`, `cargo publish`, or create a release by hand. The version lock, the checksums
and the attestations are only meaningful when the whole flow runs; a hand-cut tag skips
`version-consistency.sh` and the re-download check, which is exactly the discipline this document
exists to keep.
