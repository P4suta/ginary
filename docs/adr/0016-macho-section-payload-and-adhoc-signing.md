<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# 0016 — A Mach-O section carries the payload, ad-hoc signed, not an appended trailer

Status: Accepted · 2026-09-01

## Context

Every other target this crate builds for gets the same treatment: the stub, unmodified, with a
payload and a 64-byte trailer appended after it. `docs/format.md` states the layout and
`src/payload.rs` and `src/trailer.rs` are built on it. It works for ELF and PE because neither
format's loader cares what comes after the bytes its own headers describe — an ELF has no
required trailing content and `PT_LOAD` segments name their own extent, and a PE's own
`SizeOfImage`/section table stop where the linker put them.

A Mach-O is different in a way that matters here specifically because macOS code signing is not
optional the way it is on Linux and Windows. Two facts, both cited in the approved plan
(`docs/dev/log/` records the plan reference; see also Apple's own `codesign(1)` and the arm64
boot-args documentation):

- **`codesign --strict` rejects a Mach-O with anything appended after its last segment.** A
  Mach-O's `CodeDirectory`, when one is present, covers the file up to (and including) the
  `__LINKEDIT` segment's own end — `__LINKEDIT` is required to be the *last* segment in the
  file precisely so that a signature computed over "everything" has a well-defined boundary.
  Bytes appended past that boundary are bytes the signature does not cover, and `--strict`
  verification refuses a file shaped that way outright, independent of whether those bytes
  happen to be garbage or a well-formed payload.
- **An arm64 Mac's kernel refuses to map an unsigned executable page at all**, not merely one
  whose signature fails to verify. Every process on Apple Silicon needs *some* code signature —
  ad-hoc is enough — before the kernel will execute any of its pages. This is a stronger
  requirement than x86_64 macOS, and stronger than "signed if you want Gatekeeper to be happy":
  it is a precondition for the binary to run under the kernel's page-in path at all.

Together these rule out the append-a-trailer approach for macOS specifically: appending breaks
the one verification mode (`--strict`) that a careful build wants to pass, and on arm64 an
*unsigned* binary — appended-to or not — will not launch regardless. Some signature has to be
applied to the finished artifact, and that signature has to cover a file that still has
`__LINKEDIT` last.

## Decision

**The payload lives in a `__GINARY,__payload` Mach-O section, and the finished artifact carries
a plain, unsigned, ad-hoc `CodeDirectory` applied after the section is written.**

A section is ordinary content a Mach-O carries — the same mechanism a compiler uses for
`__TEXT,__cstring` or a linker for `__DATA,__const` — so adding one is not appending bytes a
format wasn't shaped to hold; it is data the format already has a place for. `sign_macos.rs`
inserts a new segment carrying the section immediately before `__LINKEDIT`'s own data, shifts
`__LINKEDIT` (and every offset a `symtab`/`dysymtab`/`dyld_info`/`linkedit_data` command points
into it) by exactly that much, and drops whatever `LC_CODE_SIGNATURE` the stub already carried
— a signature that covered the old bytes says nothing about the new ones. Then, when asked, it
applies the real signature: an *ad-hoc* `CodeDirectory`, meaning it asserts no identity and is
signed by nobody — it is a hash over ginary's own output, not a claim about who produced it.
This is the same technique the Burrito and Bakeware self-contained-executable tools use on
macOS, and the same kind an ordinary `.app` bundler applies to make its output loadable; nothing
here strips an existing signature to hide it, forges one, or evades a check — it satisfies the
kernel's load-time requirement the same way any macOS build tool has to. See `docs/dev/log/D3.md`
for the exact crates this was implemented with, including the one the plan named that turned out
not to fit as a dependency and what was adapted from it instead.

The trailer struct itself is unchanged: the section's first 64 bytes are the exact same layout
`docs/format.md`'s trailer table describes, with `payload_offset` read as relative to the
section's own start (fixed at 64 by this format, since the payload always immediately follows
the trailer) rather than to the file. `payload::locate` is the one place that knows there are two
containers; everything downstream — the launcher, `ginary inspect`, `ginary verify`,
`cache::ensure_extracted` — reads a plain `(file, offset, len)` stream regardless of which one
produced it. See `docs/format.md` ("Mach-O section container") for the exact geometry and its
validation rules, and `docs/dev/log/D3.md` for the crate and API this was implemented with.

### Alternatives considered

- **Append the trailer anyway, and accept that `codesign --strict` fails.** Rejected: this is
  not "unsigned, so no worse than today" — an *appended-to* Mach-O without any signature still
  will not launch on arm64, because the kernel's requirement is for a signature to be present,
  not merely for `--strict` to pass. Appending buys nothing an ad-hoc section-based signature
  does not also give, and it additionally fails the one verification mode a careful build wants
  to pass.
- **Widen an existing segment (e.g. grow `__LINKEDIT`) to hold the payload instead of adding a
  new section.** Rejected as more invasive for no benefit: it means rewriting an existing
  segment's own bookkeeping (its `filesize`, and every section inside it) rather than appending
  one new, self-contained load command, for a payload that is often the majority of the file's
  bytes and has nothing to do with what `__LINKEDIT` otherwise holds (symbol tables, the
  existing code signature blob). A dedicated section keeps the payload's own concerns — where it
  starts, how long it is, its digest — out of a segment shaped for the linker's own bookkeeping.
- **A Developer ID (or self-signed, non-ad-hoc) signature at this milestone.** Out of scope for
  the reason `README.md`'s macOS section states plainly: it asserts an identity, needs a
  certificate this project does not hold or want to hold on a contributor's behalf, and answers
  a distribution question (does Gatekeeper trust who built this) that D3 is not trying to
  answer. Ad-hoc signing answers only "can the kernel load this file at all", which is what a
  build tool owes every artifact regardless of who eventually ships it. Real Developer ID
  signing is left as a stated later option, same as ELF/PE code-signing for those platforms.

## Consequences

- `payload::locate`, `stub::verify` and `sign_macos::inject_and_sign` all depend on
  `macho.rs` being able to answer three questions honestly: a file's `cputype` (there is no
  libc distinction on macOS the way there is on Linux, so the `cputype` is the whole of the
  target), whether it is fat (a fat binary carries no single section to look the payload up
  in, and every caller here needs one architecture), and where a named section is.
- Real verification — `codesign --verify --strict`, Gatekeeper's own quarantine gate, an actual
  launch — needs a Mac and cannot be done from this Linux host. What can be proven here is
  structural: the section lands where `macho.rs` itself says it does, an `LC_CODE_SIGNATURE`
  load command is present exactly when signing was asked for, and `payload::locate` round-trips
  the exact bytes and digest that went in. `docs/dev/log/D3.md` records exactly that split, and
  CI on a `macos-15-intel`/`macos-14` runner is the GitHub Actions milestone that closes it.
- A Mach-O object found *inside* a payload (a NIF, a port program) is not required to carry its
  own code signature: only the artifact itself, the one the kernel loads directly, needs one.
  Giving `ginary verify`'s native-object scan the same `cputype`/target awareness for an inner
  Mach-O that it already has for an inner ELF is follow-on work `docs/dev/log/D3.md` records as
  scoped out of this pass rather than silently skipped — the scan's `DT_NEEDED` allowlist logic
  is Linux-specific throughout, and building its macOS counterpart honestly needs the same kind
  of care this ADR gave the artifact-level signature, not a quick pattern match bolted on.
