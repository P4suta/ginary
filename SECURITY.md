<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# Security policy

## Supported versions

ginary is pre-alpha and has no released version. Only the `main` branch is supported.

## Reporting a vulnerability

Report privately through GitHub's [private vulnerability
reporting](https://github.com/P4suta/ginary/security/advisories/new). Please do not open a
public issue for a vulnerability.

Include the ginary revision, the host and target platforms, and the smallest reproduction you
can manage. Expect an acknowledgement within seven days.

## Threat model

ginary produces executables that carry a BEAM runtime and extract it to a per-user cache
directory before running it. The parts of that pipeline with security relevance are listed
below.

**None of these properties is implemented in the current revision.** They are the design the
implementation is held to, not a description of shipped behaviour: ginary is pre-alpha and there
is no trailer reader, no launcher and no runtime catalogue yet. See
[docs/dev/log/](docs/dev/log/) for what has actually landed, and this section for what each
piece must guarantee when it does.

- **Payload integrity.** The trailer records a SHA-256 of the payload, which the launcher
  verifies while extracting. A mismatch aborts before anything is executed.
- **Archive extraction.** The payload is a tar stream. Entries with absolute paths, `..`
  components, or a type other than regular file or directory are rejected rather than skipped.
- **Cache placement.** Extraction goes to a temporary sibling directory that is renamed into
  place, so a partially written runtime is never executed. The per-application directory is
  created with mode 0700.
- **Environment hygiene.** The launcher removes `ERL_LIBS`, `ERL_FLAGS`, `ERL_AFLAGS`,
  `ERL_ZFLAGS`, `ERL_ROOTDIR` and related variables before starting the runtime, so an
  attacker-controlled environment cannot inject code into a packaged application.
- **Runtime provenance.** Prebuilt OTP archives are verified against a checksum recorded in a
  signed catalogue before they are unpacked.

Out of scope: the security of the Gleam application being packaged, the security of the OTP
release it is packaged with, and any use of ginary to run untrusted third-party artifacts.
