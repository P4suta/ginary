<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# Payload format v1

A ginary artifact is a copy of the ginary binary for the target, followed by a payload, followed
by a 64-byte trailer. The stub is unmodified, so the artifact is a valid executable for its
platform and the launcher code inside it is the same code that ships in the CLI.

```
+-----------------------------+  offset 0
|  stub (a ginary executable) |
+-----------------------------+  offset = trailer.payload_offset
|  payload (zstd stream)      |
+-----------------------------+  offset = file_len - 64
|  trailer (64 bytes)         |
+-----------------------------+  offset = file_len
```

On macOS the trailer and payload are not appended: they are placed in a `__GINARY,__payload`
Mach-O section, with the same 64-byte structure at the start of the section and
`payload_offset` relative to the section start. Appending bytes to a Mach-O file breaks
`codesign --strict` and is killed by the kernel on arm64. Everything downstream of the locator
sees only a `(file, offset, len)` stream, so ELF, PE and Mach-O share one implementation.

## Trailer

64 bytes, little-endian, at the very end of the file.

| offset | length | field            | notes                                                |
|--------|--------|------------------|------------------------------------------------------|
| 0      | 8      | `magic`          | `GINARY\0\x01`; byte 7 is the trailer format version |
| 8      | 8      | `payload_offset` | u64, absolute offset of the first payload byte        |
| 16     | 8      | `payload_len`    | u64, payload length in bytes                          |
| 24     | 32     | `sha256`         | SHA-256 of exactly `payload_len` payload bytes        |
| 56     | 8      | `reserved`       | must be zero; a non-zero value is rejected            |

### Validation

1. A file shorter than 64 bytes has no trailer.
2. If `magic[0..7]` does not equal `GINARY\0`, there is no trailer. The binary is the ginary
   CLI and parses `argv`.
3. If the magic matches but `magic[7]` is not a supported trailer version, the launcher exits
   **122**. It does not fall back to the CLI.
4. If `payload_offset + payload_len != file_len - 64`, the file was truncated or something was
   appended to it. That is `TrailerError::Geometry` and exits **122**. The addition is checked,
   so an offset near `u64::MAX` cannot wrap around to a length that matches.
5. A `payload_len` of zero is `TrailerError::EmptyPayload` and exits **122**. It is a separate
   error from the geometry one because it is a separate fault: such a file is not truncated and
   its lengths do add up, it simply carries no application, and the message says that rather
   than naming a length that was never missing.
6. A non-zero `reserved` exits **122**.

The rule behind 3 to 6 is that a damaged application must never present ginary's help text. A
missing magic means "this is the tool"; a broken magic means "this is a broken application".

The cache key is the first eight bytes of `sha256`, in lower-case hexadecimal: sixteen
characters, `Trailer::cache_key`.

## Payload

The payload is a single zstd stream. Decompressed it is a tar archive whose entries are written
in sorted order with `HeaderMode::Deterministic` and `mtime` 0, so the same staging root always
produces the same bytes.

Entry order is fixed at the front:

1. `ginary.json` — the manifest. It is first so that `ginary inspect` can stop after one entry.
2. `ginary.index.json` — path, size, SHA-256 and mode of every other entry. It lets
   `ginary verify` check an artifact without extracting it and `ginary diff` compare two.

Everything after that is the staging root, sorted by path, one entry per file.
`ginary.stage.json` is **not** packed: it is the listing `assemble::stage` writes, it is what
`ginary.index.json` is built from, and a file that describes a tree it is itself inside cannot be
reproduced. The index supersedes it.

`ginary.json` and `ginary.index.json` are **reserved**: no entry after the front matter may
carry either name. `pack` refuses a staging listing that names one, so ginary cannot write an
artifact its own reader would refuse, and `unpack` refuses one that reaches it.

There are no directory entries for directories that hold files — the reader creates the parents
of every entry — so a directory entry appears only for a directory that would otherwise be lost,
and a staging root has none.

### Header fields

Each header is written with `HeaderMode::Deterministic` and then `mtime` set to 0:

| field | value |
|-------|-------|
| `mtime` | `0` |
| `uid`, `gid` | `0`, with empty user and group names |
| `mode` | `0755` if the staged file has the user execute bit, `0644` otherwise |
| `size` | the file's own length |

The mode is normalised rather than copied, which is what keeps a build machine's umask, ACLs and
any set-user-ID bit out of the artifact. `ginary.index.json` records the staged file's own
permission bits, so the two agree for a `0644`/`0755` tree and the index is the record of what
was staged rather than of what was packed.

The tar crate's own `Deterministic` mode writes a fixed *non-zero* `mtime`, as a workaround for
tools that mishandle a zero one. This format fixes it at 0, so ginary sets that one field itself.

### Reading

Only `Regular` and `Directory` entries are legal. Symlinks, hard links, devices, FIFOs,
contiguous files and every GNU or PAX extension header that reaches the reader are
`UnsupportedEntry` errors naming the path and the kind. Absolute paths, paths containing `..`,
paths with a `ustar` prefix that climbs out, and a path with no ordinary component at all are
`UnsafePath`, checked before the path is used for anything.

`PathEscape` is the tar crate declining to unpack an entry — it answers `false` rather than
failing — and a skipped file is exactly the outcome this format may not have. No archive can
reach it today: the tar crate answers `false` only for a `..` component or a destination with no
parent, and `UnsafePath` has already refused the first. It is kept as defence in depth, so that
a tar crate which starts declining for a reason ginary did not anticipate ends as a report and
not as a missing file, and `src/payload.rs` pins the mapping in a unit test.

The front of the payload is fixed for every reader, not only for the streaming ones: entry 0
must be `ginary.json` and entry 1 must be `ginary.index.json`, or the payload is
`UnexpectedEntry`/`MissingEntry` naming the position. A cache directory without an index is a
directory `ginary verify` cannot read.

An entry at position 2 or later whose destination is `ginary.json` or `ginary.index.json` is
`DuplicateEntry`, naming the position, the name and the position the format fixes that name at.
The comparison is against the path the entry would land on — the `/`-joined ordinary components,
which is what the tar crate extracts to — so `./ginary.json` is the same name as `ginary.json`.
This is not redundant with the overwrite rule: entries 0 and 1 are read rather than unpacked, so
a repeat of either name is the one path in an archive that `set_overwrite(false)` does not stand
in front of.

Entries 0 and 1 are refused above 8 MiB (`payload::MAX_FRONT_ENTRY_BYTES`): they are the two a
launcher holds whole, and a few kilobytes of zstd can claim a terabyte of tar entry.

Nothing already in the destination is overwritten — every entry is unpacked with
`set_overwrite(false)` and `ginary.json` is created with `create_new` — so a second extraction
into a populated directory fails instead of half-replacing what is there.

After the last entry the reader consumes the remainder of the stream and compares the SHA-256 it
computed against the trailer. A mismatch exits **123**.

Entry 0 is read into memory rather than unpacked, because it is parsed, and it is written to
`<root>/ginary.json` **last**, after the digest has matched. That file's presence is what a cache
entry's completeness is judged by, so it may never precede a tree that was not finished: a
payload rejected for any reason leaves a partial directory *without* the marker, which the cache
treats as incomplete and removes.

## Manifest: `ginary.json`

```json
{
  "format_version": 1,
  "app": "my_gleam_app",
  "app_version": "1.2.3",
  "gleam_version": "1.18.1",
  "otp_release": 29,
  "otp_version": "29.0.5",
  "erts_version": "17.0.5",
  "target": "linux-x86_64-gnu",
  "otp_applications": [{ "name": "kernel", "vsn": "11.0.3" }],
  "gleam_applications": ["my_gleam_app", "gleam_stdlib"],
  "launch": {
    "program": "erlexec",
    "bindir": "erts-17.0.5/bin",
    "boot": "bin/no_dot_erlang",
    "pa": ["lib/my_gleam_app/ebin", "lib/gleam_stdlib/ebin"],
    "eval": "'my_gleam_app@@main':run('my_gleam_app')",
    "erl_flags": []
  },
  "native": [],
  "created_at": "2026-08-30T00:00:00Z",
  "ginary_version": "0.1.0"
}
```

- `launch.program` is a bare program name inside `launch.bindir`, not a path. The launcher needs
  the directory on its own regardless — it is what `BINDIR` is set to — and a joined path would
  say the same thing twice.
- Every path is relative to the extracted root and uses `/` as separator on every platform. The
  launcher joins them with the native separator. `LaunchSpec::validate` refuses an empty value,
  an absolute one, a backslash, and any `.`, `..` or empty component, in `program`, `bindir`,
  `boot` and every element of `pa`; `program` must additionally be a single component.
- `otp_release` is a number, the same `u32` `OtpInfo` and `ginary.stage.json` carry.
- `created_at` honours `SOURCE_DATE_EPOCH` so that builds are reproducible. A value that is not a
  count of seconds is an error rather than a silent fall back to the clock; an empty value is an
  unset one.
- The launcher needs nothing outside this manifest. Anything it must know at start-up belongs
  here.
- Unknown keys are preserved through a flattened `extra` map, so an older launcher can read a
  newer manifest as long as the format version still matches.
- `format_version` greater than 1 exits **122**.


### Fields

| key | type | notes |
|-----|------|-------|
| `format_version` | number | `1`; a greater value parses and is then refused |
| `app` | string | the packaged application's name |
| `app_version` | string | its version |
| `gleam_version` | string or null | the compiler that built the shipment, when it said so |
| `otp_release` | number | `erlang:system_info(otp_release)` |
| `otp_version` | string | `erlang:system_info(version)` |
| `erts_version` | string | also the `erts-<vsn>` directory name |
| `target` | string | the canonical `<os>-<arch>[-<libc>]` name |
| `otp_applications` | array of `{name, vsn}` | the applications from the OTP library |
| `gleam_applications` | array of string | the applications from the shipment |
| `launch` | object | below |
| `native` | array of `{path, kind}` | `kind` is `elf`, `macho` or `pe` |
| `created_at` | string | RFC 3339 in UTC, `SOURCE_DATE_EPOCH` honoured |
| `ginary_version` | string | the ginary that built the artifact |

`launch`:

| key | type | notes |
|-----|------|-------|
| `program` | string | a bare program name inside `bindir` |
| `bindir` | string | the runtime's `bin`, root-relative |
| `boot` | string | the boot script, root-relative, without `.boot` |
| `pa` | array of string | one root-relative directory per `-pa` |
| `eval` | string | the expression `-eval` is given |
| `erl_flags` | array of string | extra flags, before `-extra` |

The keys are serialised in the order of this table, which is the order the Rust struct declares
them: `serde_json` preserves declaration order for a struct, and `tests/manifest.rs` pins it with
a snapshot. Reading does not depend on the order.

## Index: `ginary.index.json`

```json
{
  "files": [
    {
      "path": "bin/no_dot_erlang.boot",
      "size": 512,
      "mode": 420,
      "sha256": "e3b0c442...",
      "category": "boot"
    }
  ]
}
```

One object per entry of the payload other than the two at the front, sorted by `path`. `mode` is
the staged file's `st_mode & 0o7777` as a number, and `category` is the same word
`ginary.stage.json` uses: `erts_binary`, `boot`, `otp_beam`, `gleam_beam`, `priv`,
`app_resource` or `other`. The categories are carried over from the staging listing rather than
guessed a second time, so the index and the listing cannot disagree about what a file is.

The index is what makes `ginary verify` possible without extracting an artifact, and it is why
`ginary.stage.json` does not travel.

## Determinism

Packing the same staging root with the same manifest produces the same bytes, on any machine.
What that rests on:

- entries in path order, never in `readdir` order;
- `mtime` 0 and `uid`/`gid` 0 in every header, and a mode reduced to the execute bit;
- a single-threaded zstd encoder — the `zstd` crate is built with `default-features = false`, so
  the multi-threaded one is not compiled in and a thread count cannot vary the output;
- `created_at` taken from `SOURCE_DATE_EPOCH` when it is set, and never read from a clock inside
  the format code;
- `ginary.stage.json` excluded, because a file inside the tree that describes the tree cannot
  describe itself.

A staging root that holds a file its own listing does not name is an error rather than a choice
between packing something the index does not describe and dropping it silently.

## Versioning

The trailer version (byte 7 of the magic) and the manifest `format_version` move independently.
The trailer version changes only when the 64 bytes are re-laid-out; the manifest version changes
when a launcher needs a field that older launchers cannot ignore. Adding a key that a launcher
may ignore is not a version bump.

## Launcher exit codes

These are the launcher's own codes and are distinct from the packaged application's.

| code | meaning                                                          |
|------|------------------------------------------------------------------|
| 121  | cannot open own executable, or an internal error (panic hook)     |
| 122  | trailer invalid, or an unsupported format version                 |
| 123  | payload corrupt: SHA-256 mismatch or an illegal tar entry         |
| 124  | cache I/O error                                                   |
| 125  | the runtime could not be started (`execve` failed)                |

## Changes

The format is versioned, and this section records what each decision was and why. Nothing here
is a change to a *released* format: v1 has not shipped.

### v1, milestone A3a — the format as implemented

- **`launch.program` is a bare name.** It was written as `erts-17.0.5/bin/erlexec`, which
  repeated `bindir`. The launcher needs `bindir` on its own to set `BINDIR`, so the path is
  assembled from the two rather than stored twice.
- **`otp_release` is a number.** It was written as a string. `OtpInfo`, `StageListing` and
  `assemble` all carry it as a `u32`, and the manifest would have been the only place that
  re-stringified it.
- **`launch` lost four keys that nothing wrote.** `vm_args`, `sys_config`, `distribution` and
  `filename_encoding` were in the document and in no code. A key nobody writes is a placeholder;
  when a launcher needs one it arrives with the code that sets it, and adding a key a launcher
  may ignore is not a version bump.
- **An unsupported trailer version is an error, not the CLI.** The magic deciding and the version
  byte deciding are different answers: `magic[0..7]` says whether this file is an artifact at
  all, `magic[7]` says whether this build can read it. Only the first can produce ginary's help
  text.
- **`SOURCE_DATE_EPOCH` that is not a second count is an error.** A build that was asked to be
  reproducible and quietly was not is worse than a build that stops.
- **`mtime` is 0.** The tar crate's `HeaderMode::Deterministic` writes a fixed non-zero
  timestamp; this format overrides it, because 0 is what the format says and a value chosen by a
  dependency is a value that can change under it.
- **Entry modes are normalised to `0644`/`0755`.** That is `HeaderMode::Deterministic`, and it
  means an artifact never carries a set-user-ID, set-group-ID or sticky bit, nor a mode that a
  build machine's umask happened to produce.

### v1, milestone A3a, review round 1

- **A payload of no bytes has its own error.** It was folded into the geometry check by
  computing `payload_offset + payload_len.max(1) + 64`, so the message accused a file of being
  one byte short when nothing had been truncated. `TrailerError::EmptyPayload` says what is
  actually wrong.
- **`ginary.json` is written last, and with `create_new`.** It was written as soon as it parsed,
  which left the completeness marker behind on a payload that then failed its SHA-256 check, and
  it was written with a call that overwrites while every other entry refused to. Both are the
  same decision: the marker follows the digest, and nothing in the destination is replaced.
- **`unpack` enforces entry 1.** Only `read_index` did, so an artifact whose index was misnamed
  or missing extracted happily into a cache directory that `ginary verify` could not read.
- **A contiguous file (`typeflag '7'`) is refused.** The allowlist read
  `Regular | Continuous | Directory` while this document said `Regular` and `Directory`. `pack`
  has never written one, so the extra type widened only what a hostile archive could contain.
- **`PathEscape` is documented as unreachable.** It is defence in depth behind the path check
  rather than a rejection any archive can produce; the alternative — deleting it — would make a
  future `false` from the tar crate a silently skipped file.

### v1, milestone A3a, review round 2

- **`ginary.json` and `ginary.index.json` are reserved names.** Holding entry 0 back until the
  digest matched took it out of the unpack loop, and with it the `set_overwrite(false)` that had
  been standing between a *second* entry of that name and the destination: a payload whose entry
  2 was called `ginary.json` planted attacker-chosen bytes at `<root>/ginary.json` and then
  failed with a bare `AlreadyExists`, which is the completeness marker surviving a rejection
  after all. Both ends now refuse the repeat by name — `unpack` with `DuplicateEntry`, `pack`
  with `ReservedName` — rather than leaving it to a file-system race between two writers.
