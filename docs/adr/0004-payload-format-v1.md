<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# 0004 — Payload format v1: deterministic tar + zstd behind a 64-byte trailer

Status: Accepted · 2026-08-30

## Context

The stub has to find its payload, know it is intact, and stream it out without holding it in
memory: a trimmed runtime plus an application is around 25 MB raw and roughly 8 MB compressed.
The format also has to be inspectable — `ginary inspect` should not decompress 8 MB to print an
application name — and reproducible, because "the same input produces the same artifact" is a
gate the project intends to keep.

The prior art splits both ways. Bakeware appends a trailer and uses CPIO with zstd; Burrito uses
gzip and a wrapper script. Locating the payload from a header at the front is not workable,
because the stub's own length is only known after the copy is made.

Two hazards were identified up front. First, the `tar` crate's `Entry::unpack_in` returns
`Ok(false)` — a silent skip — for an entry whose path contains `..`, which would produce a cache
directory that is missing files and looks complete. Second, a truncated download or a virus
scanner appending bytes must be detectable, and the failure must not degrade into "print the
ginary help text", because the user would then see a tool they never installed.

## Decision

Payload format version 1 is:

- a **64-byte little-endian trailer** at the end of the file: magic `GINARY\0\x01` where byte 7
  is the trailer version, `payload_offset: u64`, `payload_len: u64`, `sha256` of the payload,
  and 8 reserved bytes that must be zero;
- a **payload** that is one zstd stream wrapping a tar archive whose first entry is
  `ginary.json` (the manifest) and whose second is `ginary.index.json` (path, size, SHA-256 and
  mode of every other entry);
- a **manifest** carrying `format_version: 1` and everything the launcher needs — application,
  versions, target, the application list, and a `launch` object with `program`, `bindir`,
  `boot`, `pa`, `eval`, `erl_flags` and the optional `vm_args`, `sys_config`, `distribution` and
  `filename_encoding`. Unknown keys are preserved through a flattened `extra` map. All paths are
  root-relative and `/`-separated, joined natively at run time.

Validation rules:

- no magic means CLI mode;
- a magic with an unsupported version, a non-zero `reserved`, or
  `payload_offset + payload_len != file_len - 64` exits **122**, never falling back to the CLI;
- a manifest `format_version` above 1 exits **122**;
- a SHA-256 mismatch or an illegal tar entry exits **123**.

Only `Regular` and `Directory` entries are legal; symlinks, hard links, devices and FIFOs are
errors, as are absolute paths, `..` components and tar prefixes. `unpack_in` returning `false`
is treated as an error, not a skip.

Determinism comes from a sorted traversal, `HeaderMode::Deterministic`, `mtime` 0,
single-threaded zstd, and `SOURCE_DATE_EPOCH` feeding `created_at`.

The trailer version and the manifest `format_version` are versioned independently: the first
changes when the 64 bytes are re-laid-out, the second when a launcher needs a field it cannot
ignore.

macOS puts the same 64 bytes at the start of a `__GINARY,__payload` section instead of at the
end of the file, with `payload_offset` relative to the section. Every consumer downstream of
`payload::locate` sees only `(file, offset, len)`.

## Consequences

`inspect` reads the trailer and the first tar entry and stops, so it is fast on any artifact
size. `verify` can check every file against `ginary.index.json` without extracting, and `diff`
can compare two artifacts. Extraction streams through a hashing reader into the tar unpacker,
so peak memory is a buffer rather than a runtime.

The strict trailer rules mean a truncated or padded artifact fails loudly with a documented exit
code instead of behaving like an unrelated program. The cost is that a user who appends
something to an artifact — some installers and signing tools do — gets a hard failure rather
than a best-effort run; this is deliberate.

Rejecting symlinks means the packaged runtime must not rely on them, which is true of the
pruned OTP trees ginary assembles but is a constraint on future layout changes.

`beam_lib:strip_release` writes chunks zlib-compressed, which reduces what zstd can achieve. A
Rust IFF chunk filter that rewrites them uncompressed is a possible later optimisation, to be
decided after measurement rather than before.
