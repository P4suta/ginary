<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# The calibration fixture for the settled-item rule

This is not a document anyone maintains. It is the shape
`docs/dev/v1-readiness.md` was in on 2026-09-13 — one item proved by a hosted
run and still sitting in the list of what has not happened — beside the shapes
the rule must leave alone, so that a scan which flagged everything and a scan
which flagged nothing both fail here rather than on the real sweep.

## The evidence, by phase

### Phase D — Windows and macOS

| item | evidence | status |
|---|---|---|
| Windows cfg split, resident launcher, stub | `tests/windows.rs` | done (packaging) — `380de43` (D2) |
| macOS artifact **launch**, `codesign --verify` | `ci.yml` `macos` job, run 34281075949 | done — F1 |
| Mach-O section payload, ad-hoc signing | `tests/macho.rs` | done (packaging) — `5b35ecf` (D3) |

### Phase E — the verification matrix

| item | evidence | status |
|---|---|---|
| Catalog publishing, release **provenance** | `distribute.yml` | CI-gated — authored in E1 |
| Local-first OTP catalog, `ginary otp` | `tests/catalog.rs` | done — `304025b` (C3) |

A three-column table whose last column is not a status, so that a row reader
which took the third cell of every row on trust has something to be wrong
about. Its status does not begin `done`, so the rule passes over it:

| flag | default | meaning |
|---|---|---|
| `--target` | the host | which target to build, done or otherwise |

## The deferred items, restated plainly

What is listed here is what has **not** happened.

- **macOS launch** — `ci.yml` `macos` job, `macos-15-intel` and `macos-14` runners. Builds the
  darwin stub natively, packages and runs a `hello_ffi` artifact, and runs
  `codesign --verify --strict`.
- **A console control event reaching the Windows launcher** — nothing, yet. Declined in E23 with
  a reason: delivering one needs `GenerateConsoleCtrlEvent`.
- **Catalog publishing and release provenance** — `distribute.yml`. Runs when a maintainer cuts
  a release.
- **A green mutation campaign** — `nightly.yml`. No reconciliation with no survivor has happened.

Prose at the left margin after the list, which is not an entry in it and must
not be read as one.
