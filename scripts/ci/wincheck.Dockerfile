# SPDX-License-Identifier: MIT OR Apache-2.0
#
# Type-checks the whole tree — src/, tests/ and all — for Windows, from Linux.
#
# The scan in tests/common/portability.rs is a proxy: it finds an ungated
# mention of `std::os::unix`, and it cannot find an ungated *call* of something
# that is already `#[cfg(unix)]`, because such a call names no `os::unix` for
# any scan to read. Two of those reached the tree and only a real compile found
# them. `mingw-w64` is the whole of what a Linux host was missing: the `zstd-sys`
# C sources need a Windows C compiler, which is why the msvc triple cannot do
# this and the gnu one can.
#
# Built and run by `mise run check:windows`; the recipe is also in
# docs/dev/testing.md.
# Pinned by digest, and watched: Scorecard's `PinnedDependencies` is right that
# a floating tag is an unreviewed input, and a digest with nothing updating it
# only trades that for an image that never gets a patch. `.github/dependabot.yml`
# carries a `docker` entry for this directory, so the pair holds together — the
# tag beside the digest is what tells the updater which tag to follow.
FROM rust:1-bookworm@sha256:9a73a5088750b4c95158ab26629c854c3d6fc4b173cb7bc8079ad252d8ed7bfa
RUN apt-get update \
 && apt-get install -y --no-install-recommends mingw-w64 \
 && rm -rf /var/lib/apt/lists/*
RUN rustup target add x86_64-pc-windows-gnu
