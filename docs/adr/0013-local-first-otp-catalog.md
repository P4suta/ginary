<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# 13. The OTP catalog is local first, and nothing is published

Date: 2026-08-31

## Status

Accepted.

## Context

A cross-target build needs a BEAM runtime for a machine ginary is not running on. There are only
three ways to get one: build it (a cross-compiled Erlang/OTP, which is hours of work per target
and a toolchain per libc), copy it out of a container image (a docker daemon per build, and a
runtime whose provenance is a tag somebody can move), or download a prebuilt one.

Downloading needs an index: a document naming, per OTP version, per target, per variant, where a
tarball is, what it hashes to, and the handful of facts a build cannot read until it has already
unpacked it — the linkage, whether a NIF can be loaded, the libc floor. That index is the
catalog.

The obvious shape for it is a hosted `ginary-otp` repository: a release per OTP version, a
`catalog.json` served from a fixed URL, and a ginary that fetches it. That is where this is
going. It is not where it starts, for three reasons.

A hosted catalog is a **publishing commitment** before there is anything to publish: an artifact
served under a project's name is a promise to keep serving it, to rebuild it when upstream moves,
and to answer for what is in it. A repository created to hold a milestone's output is a
repository nobody has decided to maintain.

A hosted catalog is also **the wrong thing to test against**. Every claim in this milestone — the
digest, the length, the linkage, the target — is checked against the bytes on disk, and a test
that reached over the network to check them would be measuring somebody else's availability.

And the pipeline is the interesting part. Whether the tarballs are then uploaded is a deployment
question; whether they are correctly *made* is the engineering one.

## Decision

**The catalog lives locally.** `ginary otp repack` is the pipeline, run on a developer's machine:

1. map each requested `<target>[:<variant>]` to an upstream release asset of
   `gleam-community/erlang-linux-builds` (`x64`/`arm64`, no suffix for the fully static musl
   build, `-glibc` and `-musl` for the two dynamic ones);
2. fetch it, verified against the `digest` the GitHub release API reports for that asset — or
   read it out of `--upstream-dir` and hash it there;
3. unpack it, prune the fat a packaged runtime never reads (`src`, `doc`, `man`, `examples`,
   `c_src`, `emacs`, `misc`, `*.pdb`; `include` stays, because a NIF needs the headers);
4. dereference every symlink and assert that none remains;
5. read the emulator's own ELF header and refuse the asset if it is not for the target it was
   requested as, or not linked the way the variant claims;
6. repack deterministically — sorted paths, `mtime` 0, `uid`/`gid` 0, zstd 19 — and append the
   entry to a `catalog.json` beside the tarballs.

**The output directory is the distribution.** `dist/otp/catalog.json` is committed; the tarballs
beside it are not (`.gitignore`). An entry's `url` is a *file name relative to the catalog's own
directory*, and a URL with no scheme is resolved against that directory, so a checkout with the
tarballs beside it works with nothing hosted anywhere. A URL with a scheme is fetched, which is
what a hosted catalog will use, and the two live in one field on purpose: switching costs an
edit, not a code path.

**The embedded catalog is empty.** `catalog::EMBEDDED` is a valid schema-1 document with no
entries. A build that needs a runtime and finds nothing says so and names `--catalog` and
`ginary otp repack`.

**Every claim is checked twice.** The tarball is held to the `sha256` and the `size` the catalog
states, whether it was fetched or found on disk. The extracted runtime is then held to its own
`beam.smp`: the target, the linkage and the libc in the entry are compared against what the ELF
header says, and a disagreement is a hard error naming both sides. The catalog is an index, never
evidence.

## Consequences

A developer who wants to cross-build runs `mise run otp:repack` once, which costs about 130 MB of
downloads and a few minutes, and gets `dist/otp/catalog.json` plus three tarballs. A developer
who does not cross-build never touches any of it, and no build reaches the network unless a
catalog entry says it must.

The three facts nobody can read off a tarball's name are read off its emulator instead, so a
mislabelled upstream asset is caught in the pipeline rather than by a user whose loader refuses
the artifact.

**What flips when a hosted `ginary-otp` appears.** The pipeline does not change. What changes is
where its output goes and what a fresh ginary reads by default:

- `otp repack` grows an upload step, or a CI job runs it and attaches the tarballs to a release;
- the entries' `url` values become absolute (`https://github.com/.../otp-29.0.5-...tar.zst`),
  which `resolve_url` already handles;
- `catalog::EMBEDDED` becomes a snapshot of that catalog, or `otp update` gets a default URL, so
  a machine that has never run `repack` can still cross-build;
- the digests become the thing that makes a hosted artifact trustworthy, and they are already
  there, checked on both sides.

Nothing above requires a new format, a new command or a new check. Until then, no repository is
created, nothing is pushed, and nothing is published.
