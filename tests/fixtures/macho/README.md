<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# Mach-O fixture

One real, unmodified Mach-O binary, so that `src/macho.rs` is tested against a
file it did not write, the same reason `tests/fixtures/beam/` and
`tests/fixtures/app/otp/` exist.

| file | bytes | cputype | source |
|---|---|---|---|
| `inet_gethost-aarch64-apple-darwin` | 71680 | arm64 (`0x0100000c`) | `erts-17.0.5/bin/inet_gethost`, `erlef/otp_builds` release `OTP-29.0.5`, asset `otp-aarch64-apple-darwin.tar.gz` |

Downloaded once from
`https://github.com/erlef/otp_builds/releases/download/OTP-29.0.5/otp-aarch64-apple-darwin.tar.gz`
(sha256 of the tarball itself:
`24b9e00da2b9ad25b1f182e2efd73ff316e46ec4b143c0cc3c69dbd27d5a594d`), and
`erts-17.0.5/bin/inet_gethost` extracted from it verbatim. `inet_gethost`
rather than `epmd` because it is the smaller of the two small binaries in
that tree (`epmd` is 87992 bytes), and neither carries anything this project
would rather not commit: both are the Erlang port program used to resolve
host names and to register a node, with no embedded strings specific to a
build machine.

It is a real linker's output, which is exactly the point: thin (not fat),
`__LINKEDIT` last (fileoff 65536, filesize 6144, ending at the file's own
71680 bytes) as ADR
[0016](../../../docs/adr/0016-macho-section-payload-and-adhoc-signing.md)
says a macOS binary's last segment always has to stay, and it already carries
an `LC_CODE_SIGNATURE` load command (dataoff 70976, datasize 704) — arm64
macOS refuses to run an unsigned binary at all, so upstream's own build already
ad-hoc signs it. That makes it double duty: the one committed fixture proves
both "a real Mach-O's section table reads correctly" and "a real Mach-O
already carrying a code signature is detected as such", without needing a
second file.

Every other Mach-O fixture used in the test suite — a minimal thin header, a
fat header, a header carrying a hand-built `__GINARY,__payload` section — is
written by hand in `tests/common/macho.rs`, the technique
`tests/common/native.rs` and `tests/common/stubfile.rs` already use for ELF
and PE: there is no macOS toolchain on this machine, so nothing here can be
compiled, only written field by field or copied from what erlef already built.

## Licensing

This is not ginary's file: it is compiled from Erlang/OTP, which is
`Apache-2.0` (<https://github.com/erlang/otp/blob/master/LICENSE.txt>). A
binary carries no SPDX header, so `REUSE.toml` declares the path instead.
