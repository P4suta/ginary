<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# ELF fixture

One real, unmodified-in-shape ELF binary a linker wrote, so that the tests
that plant "a real native object" under an application's `priv` have a file
whose machine is a fact of the *file* rather than of whichever host is running
the suite.

| file | bytes | machine | source |
|---|---|---|---|
| `inet_gethost-x86_64-linux-gnu` | 59592 | `x86_64` (`EM_X86_64`, 62) | `erts-17.0.5/bin/inet_gethost`, Erlang/OTP 29.0.5, stripped |

## Why this exists

`tests/common/repack.rs` and several `tests/verify.rs` / `tests/doctor.rs`
tests need a real ELF in the payload — a file with a real `PT_INTERP`, real
`DT_NEEDED` entries and a real `e_machine`, not one a test made up field by
field. Until E9 they used *this test run's own binary* (`current_exe()`) for
that. On a Linux host that binary is an ELF and the trick works; on the Windows
runner the same binary is a PE, so `elf::inspect_bytes` refuses it, the object
table comes back empty, and every "a real ELF is listed with what it needs"
assertion fails against a healthy artifact — a defect in the *test*, not in
`ginary verify`. See `docs/dev/log/E9.md`.

A committed ELF fixture removes the host from the question: the planted object
is a genuine `x86_64` Linux ELF whatever machine reads it, exactly as
`tests/fixtures/macho/inet_gethost-aarch64-apple-darwin` is a genuine arm64
Mach-O whatever machine reads it.

## What it is, and what it carries

It is `inet_gethost`, the Erlang port program used to resolve host names, taken
from the `erts-17.0.5/bin` of the Erlang/OTP 29.0.5 toolchain ginary is
developed against, then `strip --strip-all` and its `.comment`/`.note` sections
removed. The strip is why it is committable: it leaves a real linker's ELF —
`ET_DYN` (a PIE executable), `e_machine` `EM_X86_64`, interpreter
`/lib64/ld-linux-x86-64.so.2`, `DT_NEEDED` of `libm.so.6` and `libc.so.6` (both
on `verify`'s `NEEDED_ALLOWLIST`, so a healthy artifact carrying it verifies
clean) — while removing every symbol, debug section and build note, so the file
holds no path, host name or other string specific to the machine it was built
on. `inet_gethost` rather than `epmd` for the same reason the Mach-O fixture
chose it: it is the smaller of the two small binaries in that tree.

`inet_gethost.debug` is not committed and neither is any unstripped copy: only
these 59592 bytes, which carry nothing this project would rather not commit.

## Licensing

This is not ginary's file: it is compiled from Erlang/OTP, which is
`Apache-2.0` (<https://github.com/erlang/otp/blob/master/LICENSE.txt>). A binary
carries no SPDX header, so `REUSE.toml` declares the path instead, exactly as it
does for `tests/fixtures/macho/` and `tests/fixtures/beam/`.
