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
   appended to it. That is `TrailerError::Geometry` and exits **122**.
5. A non-zero `reserved` exits **122**.

The rule behind 3 to 5 is that a damaged application must never present ginary's help text. A
missing magic means "this is the tool"; a broken magic means "this is a broken application".

## Payload

The payload is a single zstd stream. Decompressed it is a tar archive whose entries are written
in sorted order with `HeaderMode::Deterministic` and `mtime` 0, so the same staging root always
produces the same bytes.

Entry order is fixed at the front:

1. `ginary.json` — the manifest. It is first so that `ginary inspect` can stop after one entry.
2. `ginary.index.json` — path, size, SHA-256 and mode of every other entry. It lets
   `ginary verify` check an artifact without extracting it and `ginary diff` compare two.

Everything after that is the staging root. Only `Regular` and `Directory` entries are legal.
Symlinks, hard links, devices and FIFOs are `UnsupportedEntry` errors. Absolute paths, paths
containing `..`, and paths with a tar prefix are rejected. An entry the tar crate declines to
unpack is an error, never a silent skip.

After the last entry the reader consumes the remainder of the stream and compares the SHA-256 it
computed against the trailer. A mismatch exits **123** and leaves no cache directory behind.

## Manifest: `ginary.json`

```json
{
  "format_version": 1,
  "app": "my_gleam_app",
  "app_version": "1.2.3",
  "gleam_version": "1.18.1",
  "otp_release": "29",
  "otp_version": "29.0.5",
  "erts_version": "17.0.5",
  "target": "linux-x86_64-gnu",
  "otp_applications": [{ "name": "kernel", "vsn": "11.0.3" }],
  "gleam_applications": ["my_gleam_app", "gleam_stdlib"],
  "launch": {
    "program": "erts-17.0.5/bin/erlexec",
    "bindir": "erts-17.0.5/bin",
    "boot": "bin/no_dot_erlang",
    "pa": ["lib/my_gleam_app/ebin", "lib/gleam_stdlib/ebin"],
    "eval": "'my_gleam_app@@main':run('my_gleam_app')",
    "erl_flags": [],
    "vm_args": null,
    "sys_config": null,
    "distribution": false,
    "filename_encoding": "utf8"
  },
  "native": [],
  "created_at": "2026-08-30T00:00:00Z",
  "ginary_version": "0.1.0"
}
```

- Every path is relative to the extracted root and uses `/` as separator on every platform. The
  launcher joins them with the native separator.
- `created_at` honours `SOURCE_DATE_EPOCH` so that builds are reproducible.
- The launcher needs nothing outside this manifest. Anything it must know at start-up belongs
  here.
- Unknown keys are preserved through a flattened `extra` map, so an older launcher can read a
  newer manifest as long as the format version still matches.
- `format_version` greater than 1 exits **122**.

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
