<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# Architecture decision records

Records are MADR-shaped: `Context`, `Decision`, `Consequences`, with a status line under the
title. They record why a choice was made, not what the code currently does; where a decision is
only partly built, the record says so under `Consequences`.

- [0001](0001-record-architecture-decisions.md) — Record architecture decisions
- [0002](0002-rust-single-binary-self-copy-stub.md) — One Rust binary that is both the packager
  and the stub
- [0003](0003-erlang-shipment-input-and-direct-erlexec.md) — Package an erlang-shipment and
  start it by exec'ing erlexec directly
- [0004](0004-payload-format-v1.md) — Payload format v1: deterministic tar + zstd behind a
  64-byte trailer
- [0005](0005-cache-layout-and-atomic-extraction.md) — Cache layout and atomic extraction
- [0006](0006-tdd-workflow-execution-model.md) — TDD workflow and the developer tooling that
  comes first
- [0007](0007-strip-elf-and-beam-debug-info.md) — Strip ELF and BEAM debug
  information, on by default
- [0008](0008-launcher-exit-codes-and-env-protocol.md) — Launcher exit codes 121 to 125, and
  maintenance through the environment
- [0009](0009-front-entries-in-their-own-zstd-block.md) — The manifest and the index get a zstd
  block of their own
- [0010](0010-cache-locking-and-pruning.md) — A cache entry is locked for the life of the
  runtime, and pruning honours the lock
- [0011](0011-verify-and-sbom.md) — What `verify` checks, and how the SBOM stays a function of
  the artifact
- [0012](0012-stub-identity-and-feature-split.md) — A stub says what it is, and the `cli` feature
  is what makes one
