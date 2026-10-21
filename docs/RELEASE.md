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

The release job authenticates through the purpose-scoped Doppler config `ginary.automation`.
Install the release App on `P4suta/ginary` with contents, pull requests, and issues write access.
Nothing wider belongs in the installation token.
Keep the existing App credential in Doppler as `GITHUB_APP_CLIENT_ID` and `GITHUB_APP_PRIVATE_KEY`, using the shared references described in [credential delivery](credential-delivery.md).

Keep administrator bypass disabled with `can_admins_bypass: false`, alongside the existing `release` environment policy for the main branch and `v*` tags.
Set the public `DOPPLER_AUTOMATION_IDENTITY_ID` selector under Settings -> Environments -> release -> Environment variables.
The selected identity has read-only access to `ginary.automation` and checks the exact repository, workflow, event, environment, and ref claims.
Provider signing and distribution publication retain their separate approval boundaries.

Doppler retrieval is required.
A denied fetch or a missing, empty, or unresolved required field fails before token creation or release-please.
The old optional-credential notice remains only in a historical test fixture.
Keep the current delivery path until the merged workflow passes its exact live positive and negative checks.

GitHub permits any job of any workflow to declare an environment.
Doppler adds its own workload claims, and jobs with the same matching context can select the same identity.
After retrieval, the App private key still carries the App's installation authority.
A read-only token created by the verifier limits that token, while the stored key retains its provider authority.

Two tests in `tests/release_workflow.rs` hold the repository's declared readers to their reviewed contexts:

- `no_job_of_any_workflow_reads_the_release_credentials_without_declaring_the_environment` checks every credential reference.
Its regression is `tests/regressions/e17_the_release_credentials_were_read_outside_their_environment.rs`.
- `only_release_and_manual_verification_jobs_declare_the_release_environment` permits the release job and the main-only manual authentication verifier.
Its regression is `tests/regressions/e18_nothing_bounded_the_number_of_jobs_that_may_read_the_credentials.rs`.

The verifier creates only a repository-scoped contents-read token and checks unrelated config denial using the same selected identity.
It performs no signing or publication.
The existing `release.yml` trigger stays limited to pushes on main.
Merging an implementation PR grants no authority to merge a release PR or publish its draft.

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
The workflow must itself run from that same tag. Before checkout it rejects a branch execution
context or a different tag, so code selected by a tag input cannot execute with the default
branch's cache privileges. Every builder checks out the immutable event commit. Before any
repository script runs, the version job checks that the fetched tag still identifies that
commit and records its commit time as `SOURCE_DATE_EPOCH`.

After separate authorization to run a hosted rehearsal against an existing version tag:

```console
$ gh workflow run distribute.yml --ref v0.1.0 -f tag=v0.1.0 -f publish=false
```

Selecting `main` as the workflow ref while supplying a tag input fails before checkout. A
reusable-workflow caller must also run from the same tag: `workflow_call` inherits the caller's
event ref and commit. For manual dispatch, the workflow file comes from the selected tag,
so older tags retain their older workflow behavior; changes on main do not retroactively
update them.

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
