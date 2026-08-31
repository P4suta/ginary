<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# 0009 — The manifest and the index get a zstd block of their own

Status: Accepted · 2026-08-31

## Context

ADR 0004 fixes the payload as one zstd stream wrapping a tar archive whose entry 0 is
`ginary.json` and whose entry 1 is `ginary.index.json`, and it fixes that order so that a reader
which only wants to know what an artifact *is* pays for a few kilobytes rather than for tens of
megabytes. `payload::read_manifest` and `payload::read_index` are that reader, and the launcher's
cache lookup and `ginary inspect` are their two callers.

A4 asked for one more thing of the same shape: `ginary inspect` without `--verify` must still
print the manifest of an artifact that would *fail* `--verify`. That is how a user finds out what
a file they were given was supposed to be — which application, which OTP, which build — and it
is the only question worth asking about a file that is already broken.

zstd decodes a block at a time. A payload small enough to fit in one block therefore has no
front: a byte flipped anywhere in it, including in the last file of the tree, makes the decoder
report `Data corruption detected` before entry 0 comes out. The property held for large
artifacts by accident, because their front entries happen to land in an early block, and did not
hold at all for a small one. "By accident, for large inputs" is not a format guarantee.

## Decision

`payload::pack` flushes the compressor immediately after appending `ginary.index.json`, and
nowhere else. Entries 0 and 1 are therefore in a zstd block of their own, and the reader that
stops after them never touches a byte of the rest.

## Consequences

An artifact whose payload is damaged anywhere after its index still answers `ginary inspect`, and
still fails `ginary inspect --verify` — those are now two separate questions rather than one.
The launcher is unchanged: it hashes the whole payload before it trusts any of it, so a corrupt
tail is still exit 123 with nothing left in the cache.

Determinism is unaffected. The boundary is at a fixed position in the archive rather than at
whatever buffer boundary the encoder reached, so two packs of one staging root still produce
identical bytes; `docs/format.md` lists the flush among the things determinism rests on.

The cost is the compression ratio of one block boundary near the front of the stream, measured in
tens of bytes against a payload measured in megabytes. Nothing in the format's grammar changed,
so the trailer version and the manifest `format_version` both stay at 1 and every reader already
written keeps working.
