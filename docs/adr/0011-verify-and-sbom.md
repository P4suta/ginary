<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# 0011 — What `verify` checks, and how the SBOM stays a function of the artifact

Status: Accepted · 2026-08-31

## Context

B2 added two readers that produce a *document* about an artifact rather than a fact: `ginary
verify` and `ginary sbom`. Three decisions in them are not obvious from the code, and each one
is the kind that is expensive to change once anything consumes the output.

**One: what makes a `DT_NEEDED` a finding.** An artifact carries its own BEAM and cannot carry a
libc. Everything it names in `DT_NEEDED` is therefore either part of the C runtime — which every
glibc system has — or a file it expects a stranger's machine to already have, which is the whole
of the portability promise broken. There is no way to compute the difference; it has to be
written down.

**Two: what an SPDX document is unique under.** SPDX 2.3 requires a `documentNamespace` that is
unique per document, and every tool that writes one uses a random version 4 UUID and a clock.
Doing that here would make the bill of materials the one part of a reproducible build that is
not reproducible: two runs of `ginary sbom` over one file would disagree, and so would two
builds of one project that are otherwise byte-identical.

**Three: what `ginary verify` costs.** `inspect --verify` is one streaming pass past a hasher.
Checking every file against the index and inspecting every native binary is a second pass and,
for the binaries, memory. An artifact's tar header can claim any length, so a verifier that
believed it would be killed rather than report anything.

## Decision

**The allowlist is a constant with a documented membership rule.**
`verify::NEEDED_ALLOWLIST` names the eight libraries a glibc system supplies:
`libc.so.6`, `libm.so.6`, `libpthread.so.0`, `libdl.so.2`, `librt.so.1`, `libgcc_s.so.1`,
`libstdc++.so.6`, `libtinfo.so.6`. The dynamic loader cannot be on it by name, because its
soname carries the architecture — `ld-linux-x86-64.so.2` on one target, `ld-linux-aarch64.so.1`
on another — so it is matched by the `ld-linux-` prefix. That prefix rule is conditional on the
allowlist naming `libc.so.6`: the loader *is* glibc, so an allowlist that admits glibc's own
libc admits its loader, and one that does not admits neither. An empty allowlist therefore means
what it says, which is what makes the list injectable and the rule testable at all.

**The SPDX namespace is derived from the payload digest.** `sbom::uuid_from_sha256` takes the
first sixteen bytes of the payload's SHA-256 and sets the version and variant nibbles RFC 4122
fixes, and the namespace is `<prefix>/<app>-<version>-<uuid>`. Two different artifacts still get
two different namespaces, because the bytes underneath those nibbles are still the digest's, and
one artifact always gets one namespace. `creationInfo.created` is the manifest's own timestamp
and `creators` is the ginary that *built* the artifact rather than the one reading it, for the
same reason.

Nothing about a package's origin is invented. A shipment records what an application is, never
where it came from, so a download location comes from the project's `manifest.toml` when there
is one to read and is `NOASSERTION` otherwise. `ginary sbom <exe>` on an artifact with no
project around it is the `NOASSERTION` case by construction.

**`verify` reads a payload entry into memory only when its first bytes are the ELF magic, and
only up to 100 MB.** Every entry is hashed against the index as it streams past; the bytes are
kept only for a file that begins `\x7fELF`, and an entry whose header claims more than
`verify::MAX_OBJECT_BYTES` is reported as unreadable rather than held. A payload whose digest
does not match the trailer is not read at all past that point: every entry after the damage is
bytes nobody wrote, and a table of findings about them would describe the damage.

## Consequences

The two commands stay two commands. `inspect --verify` is unchanged — one pass, the payload
hash, and the check the launcher itself makes — and its documentation points at `verify` for the
rest. A caller that wants the cheap answer keeps paying the cheap price.

An artifact built against a library the allowlist does not name is a *finding*, not an error:
`verify` exits 1 and names the file and the soname, and the reader decides. Adding a library to
the list is a deliberate widening of what ginary promises about a target machine, and it is one
line and one changed test.

`verify` gained four issue kinds beyond the four the plan names, all for the same reason: the
house rule is that skipping is a reported decision and never a default, and there was nowhere
else for those decisions to go. `UnreadableObject` is a file that begins with the ELF magic and
does not parse as one, or is too large to inspect. `UnsafePath` is an entry whose name is
absolute, holds `..`, or normalises to nothing — `payload::destined_path` is the one rule both
commands apply, so an artifact `unpack` refuses at run time cannot verify clean at build time.
`ReservedEntry` is an entry at position 2 or later that lands on `ginary.json` or
`ginary.index.json` — `payload::unpack` refuses the whole payload for one, so `verify` may not
pass over it because of what it is *called*; the front matter is positions 0 and 1 and nothing
else. `UnsupportedEntry` is an entry that is neither a regular file nor a directory, in the
vocabulary `PayloadError::UnsupportedEntry` already uses. A directory entry is the one shape
that is passed over in silence, because `docs/format.md` permits it, the index lists files only,
and it carries no bytes to check.

`UnsafePath` is raised before the index is consulted, so an escaping entry is counted in neither
`files_checked` nor `IndexOrphan`. That is deliberate: an entry no launcher may write is not a
file the index can account for, and an artifact whose index carries a matching row for it has
told two lies rather than none — the row is then `IndexMissing`, which is the second of them.

`verify::MAX_OBJECT_BYTES` and the allowlist are both injectable through `VerifyOptions`, for
the same reason: a hundred megabytes is not a payload a test can produce, and every ELF a test
can build on the machine it runs on links against libraries the real allowlist names. Injection
is what makes "this is refused" an assertion rather than a hope.

A consumer may treat the SPDX `documentNamespace` as an identifier for the artifact: it changes
if and only if the payload, the application name or the version changes. The cost is that the
namespace is not a URL anything resolves — it never was, for any producer — and that a second
artifact built from an identical payload under a different application name gets a different
namespace, which is correct.
