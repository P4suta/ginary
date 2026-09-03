<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# 0016 — The payload grows `__LINKEDIT`, ad-hoc signed; not a new section, not a plain trailer

Status: Accepted · 2026-09-01 · amended 2026-09-03 after two Macs ran it

> **Amendment (2026-09-03).** The original decision below — *the payload lives
> in a new `__GINARY,__payload` section* — was **falsified by the runner** and
> is superseded by the section "2026-09-03 — the section approach does not run,
> and what replaced it" at the foot of this file. The Context still holds; the
> Decision now reads: **the payload is appended inside the stub's existing
> `__LINKEDIT` segment, which is grown to cover it, no load command is added and
> no byte of code moves.** The prose from "## Decision" to that closing section
> is kept as the record of what was tried and why it was wrong, not as current
> guidance.

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

## 2026-09-03 — what a Mac reported (provisional; sufficiency not yet re-run)

The `macos-14` and `macos-15-intel` runners of
<https://github.com/P4suta/ginary/actions/runs/33712111530> are the milestone the Consequences
above named, and they ran this decision for the first time. **The section-plus-ad-hoc-signature
shape is not in question** — the payload belongs in a `__GINARY,__payload` section and the
artifact carries an ad-hoc `CodeDirectory`. What is recorded below is *provisional*: no macOS
runner has yet executed the fixed writer, so the *sufficiency* of the fix — that
`codesign --verify --strict` accepts the artifact and it runs to `exit 3` — is unverified until
a runner passes. What the runners falsified was the writer, not the ADR, and two independent
writer defects were found, one of them only on a later review pass.

Both jobs died at the same line, before `codesign` was reached at all:

```text
/Users/runner/work/_temp/84fd6172-....sh: line 10:  7695 Killed: 9   "$artifact" 0 hello world
##[error]Process completed with exit code 137.
```

`Killed: 9` before a program prints anything is the kernel refusing to map a page whose SHA-256
does not match the slot the `CodeDirectory` holds for it — and an *invalid* signature turns out
to be worse than no signature on x86_64 as well as on arm64, which is a fact this ADR's second
bullet stated only for arm64.

**Defect one — hash ordering.** `Writer::build` hashed the assembled body and *then* wrote
four more fields into it: `LC_CODE_SIGNATURE`'s `dataoff` and `datasize`, and `__LINKEDIT`'s
`vmsize` and `filesize`. All four live in the load-command area, which is page 0, so slot 0
described a page that no longer existed by the time the file was closed. The signature now
covers the finished file: the section is injected, the signature's own offset and size and
`__LINKEDIT`'s grown extent are patched in, the signature is aligned to a 16-byte boundary as
every linker-produced one is, and only then are the pages hashed. This is proven on Linux by
`tests/regressions/e8_the_ad_hoc_signature_did_not_cover_the_finished_file.rs` and
`tests/sign_macos.rs`, which parse the superblob field by field and recompute every page hash
without going through `src/sign_macos.rs`.

**Defect two — segment page alignment (found on the Fix round 1 review, not by the runner).**
The `Killed: 9` above does not, on its own, isolate the ordering defect as the *only* cause: a
misaligned load map is an independent reason the same binary cannot be mapped, and the writer
had one. The new `__GINARY` segment was inserted by shifting every following byte forward by the
raw load-command growth (152 bytes), which is not a multiple of a page, so on arm64 every
segment after `__TEXT` lost `0x4000` alignment and `round_page(__TEXT.vmsize)` swallowed
`__DATA_CONST` and `__DATA`. `Writer::plan` now rounds the shift up to a whole page and pads the
load-command area to match, and emits `__GINARY` before `__LINKEDIT` so segments stay in
increasing `vmaddr` order; this too is pure arithmetic and is pinned on Linux by
`tests/regressions/e8_the_injected_segment_broke_page_alignment.rs`. Whether these two together
are *sufficient* — whether the artifact now maps, runs, and passes `codesign --verify --strict`
— is the open question a macOS runner has still to answer. `docs/dev/log/E8.md` records both
defects and the run.

## 2026-09-03 — the section approach does not run, and what replaced it

The `macos-14` and `macos-15-intel` runners of
<https://github.com/P4suta/ginary/actions/runs/33724862229> answered that open question, and the
answer is that **the section approach is wrong at the root, not in its details.** The E8 writer
produced a Mach-O whose ad-hoc signature is genuinely valid — `codesign --verify --strict
--verbose=4` now exits `0`, `valid on disk` and `satisfies its Designated Requirement` both
print — and which then **segfaults on exec** (`Segmentation fault: 11`, exit `139`) before it
runs a single useful instruction. A valid signature over a structurally broken image is exactly
what a section-plus-shift writer produces, and no further round of shifting fixes it.

The cause is structural and unavoidable for a *new section*. A section needs a new
`LC_SEGMENT_64` load command, and a linker leaves almost no room in the load-command area — the
committed arm64 fixture has forty spare bytes before its first section and a segment-with-section
command is a hundred and fifty-two. So the command cannot be added without sliding every byte of
code and data that follows it forward by a page. That slide is fatal twice over:

- **The entry point.** `LC_MAIN`'s `entryoff` is a file offset into `__TEXT`. Move `__TEXT`'s
  contents and leave `entryoff` alone and the kernel jumps into the load commands; move both and
  you are relocating a linked image by hand.
- **Every rebase.** `LC_DYLD_CHAINED_FIXUPS` encodes each rebase target as an offset from the
  image base. Slide the segment a target points into and the stored offset is stale: dyld writes
  a pointer to where the datum *used to be*. The signature covers the moved bytes faithfully —
  it just describes an image dyld can no longer fix up. This is why `codesign --verify` passes
  and the process still dies.

The "widen an existing segment" alternative this ADR rejected in 2026-09-01 as "more invasive
for no benefit" turns out to be the *only* shape that runs, and the benefit is decisive: it adds
no load command, so nothing slides, so the entry point and every fixup keep the offsets the
linker gave them.

**Amended decision.** `sign_macos.rs` appends the payload — the packed bytes followed by the
64-byte trailer — immediately after `__LINKEDIT`'s existing content, grows `__LINKEDIT`'s
`filesize` and `vmsize` so the segment still ends the file, reuses the `LC_CODE_SIGNATURE`
command the linker already left (repointing its `dataoff`/`datasize`; **no** load command is
added or removed for a signed build), and computes a fresh ad-hoc `CodeDirectory` over the
finished bytes. This is precisely how `codesign(1)` itself embeds a signature: as more bytes at
the tail of `__LINKEDIT`, covered by the hashes, described by no section. The entry point, every
segment's `fileoff`/`vmaddr`, and every fixup are byte-for-byte what the stub carried.

`payload::locate` finds the payload by reading `LC_CODE_SIGNATURE`'s `dataoff` and parsing the
trailer at `dataoff - 64` (`PayloadVia::MachOAppended`); an unsigned build — only the tests ask
for one — has no signature after the payload, so its trailer is the last 64 bytes of the file and
the ordinary end-of-file reader finds it. The `__GINARY,__payload` section an earlier ginary
wrote is still *read* (`macho::section`, `PayloadVia::MachOSection`) so an artifact built by an
older build still opens, but it is no longer *written*.

What a Linux host can prove about this is structural and is proven, in `tests/sign_macos.rs`: the
finished artifact's mapped entry holds the stub's own first instructions (an image that jumps
into its load commands is the segfault above), the `CodeDirectory` carries `CS_ADHOC` and **not**
`CS_LINKER_SIGNED` (ginary rewrote this binary; it did not come from a linker), the payload lies
inside what the hashes cover, and `__LINKEDIT` ends the file. That the artifact now also *runs*
and passes `codesign --verify --strict` is what the macOS runners confirm, with the load-command
diff the job prints beside the run as the standing evidence. `docs/dev/log/E9.md` records the run
and the fix.
