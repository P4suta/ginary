<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# Security policy

## Supported versions

ginary is a single crate released from `main`. Only the most recent release and the current
`main` receive fixes; older releases are not patched.

| version | supported |
| --- | --- |
| 0.1.x | yes |
| < 0.1 | no, there was no such release |

The crate is not published to crates.io. "The most recent release" means the newest `v*` tag in
this repository and the artifacts the release workflow built from it.

## Reporting a vulnerability

Report privately through GitHub's [private vulnerability
reporting](https://github.com/P4suta/ginary/security/advisories/new). Please do not open a
public issue, a discussion, or a pull request for a vulnerability: an advisory stays private
until a fix exists, and the other three do not.

Expect an acknowledgement within seven days, and an assessment of whether the report is
accepted, with the fix or mitigation planned, within a further fourteen. If seven days pass in
silence, treat that as a failure of this policy and say so in the advisory thread.

Useful in a report: the ginary version or commit, the host platform and the target the artifact
was built for, the smallest reproduction you can manage, and what an attacker gains.

**What to strip before you send it.** A reproduction for ginary is a manifest, a log, or an
artifact, and all three routinely carry things that are yours rather than the project's. Remove
secrets and credentials of every kind — API tokens, registry credentials, environment variables
holding either — along with code-signing identities and certificates, and home-directory paths
that name you or your employer. A reproduction that needs a secret to be a reproduction is one
we would rather receive as a description than as a file.

We do not run a bounty programme, and we will credit you in the advisory and the release notes
unless you ask us not to.

## Threat model

ginary produces executables that carry a BEAM runtime and extract it to a per-user cache
directory before running it. The properties below are what the implementation is held to, and
each one is exercised by the test suite; where a property is narrower than it sounds, the
narrowing is stated.

- **Payload integrity.** The trailer records a SHA-256 of the payload, which the launcher
  verifies while extracting. A mismatch aborts before anything is executed. This detects
  corruption and truncation; it is not a signature, so it does not by itself establish who built
  an artifact.
- **Archive extraction.** The payload is a tar stream. Entries with absolute paths, `..`
  components, or a type other than regular file or directory are rejected rather than skipped.
- **Cache placement.** Extraction goes to a temporary sibling directory that is renamed into
  place, so a partially written runtime is never executed. The per-application directory is
  created with mode 0700 on Unix, and pruning never removes an entry another process holds a
  lock on.
- **Environment hygiene.** The launcher removes `ERL_LIBS`, `ERL_FLAGS`, `ERL_AFLAGS`,
  `ERL_ZFLAGS`, `ERL_ROOTDIR` and related variables before starting the runtime, so an
  attacker-controlled environment cannot inject code into a packaged application.
- **Runtime provenance.** A prebuilt OTP archive is verified against the SHA-256 the catalogue
  records before it is unpacked, and the unpacked emulator is checked to be the target it
  claims. The catalogue itself is local first: no hosted catalogue exists, the embedded one is
  empty, and the entries a developer produces are as trustworthy as the machine that produced
  them. There is no signature over the catalogue today; see
  [docs/adr/0013-local-first-otp-catalog.md](docs/adr/0013-local-first-otp-catalog.md) for what
  changes when a hosted one appears.
- **macOS signing.** The macOS artifact is ad-hoc signed, which makes it loadable on arm64. An
  ad-hoc signature carries no identity: it is not a Developer ID signature and it is not
  notarised.

Out of scope: the security of the Gleam application being packaged, the security of the OTP
release it is packaged with, the machine that runs a build, and any use of ginary to run
untrusted third-party artifacts.
