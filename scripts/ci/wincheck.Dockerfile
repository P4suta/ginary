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
FROM rust:1-bookworm
RUN apt-get update \
 && apt-get install -y --no-install-recommends mingw-w64 \
 && rm -rf /var/lib/apt/lists/*
RUN rustup target add x86_64-pc-windows-gnu
