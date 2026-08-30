<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# Agent guide

ginary is a single Rust crate (edition 2024) that packages a Gleam application and a trimmed
BEAM runtime into one executable. The same binary is both the build tool and, with a payload
appended to it, the launcher that a packaged application runs under.

Read [docs/dev/architecture.md](docs/dev/architecture.md) before structural work,
[docs/format.md](docs/format.md) before touching the payload, and
[docs/dev/testing.md](docs/dev/testing.md) before behaviour changes. The approved plan and the
per-milestone record live in [docs/dev/log/](docs/dev/log/).

## Two modes, one binary

- `main()` decides mode before anything else: a valid trailer at the end of the running
  executable means launcher mode, no trailer means CLI mode.
- The launcher never parses `argv`. Everything after `argv[0]` belongs to the packaged
  application. Maintenance commands travel in `GINARY_CMD`.
- clap, the network, and the build-side dependencies are CLI mode only. Nothing on the launcher
  path may pull them in.

## TDD loop

1. Write the smallest failing test and run it. A compile error is not RED; it must fail on an
   assertion.
2. Make the smallest production change that turns it green without breaking anything else.
3. Review adversarially: boundaries, concurrency, error paths, clippy, docs.
4. For each finding, add a regression test, watch it fail, then fix it.
5. Refactor with the suite green.

A bug fix without a regression test under `tests/regressions/` is rejected.

## Gates

`mise run check` is the gate: `lint` (fmt check plus clippy with warnings denied), `test`,
`doc` (rustdoc with warnings denied) and `deny` (`cargo deny check`).

```console
mise run lint
mise run test
mise run test:fast
mise run test:nextest
mise run doc
mise run deny
mise run cov
mise run mutants
```

`cov`, `mutants` and `test:nextest` are the assurance tasks; see
[docs/dev/testing.md](docs/dev/testing.md).

Use the default `CARGO_HOME`. If `~/.cargo` is read-only (sandboxed agent), export
`CARGO_HOME=$PWD/.cache/cargo-home`. Never attempt to disable a sandbox.

## Prohibitions

- No placeholder exports, stub subcommands, or `todo!()` presented as a feature. A command that
  is not implemented does not appear in the CLI.
- No `panic!`, `unwrap`, `expect`, `unreachable!` or arithmetic that can overflow on the
  launcher path. The launcher reports a numbered exit code (121 to 125) and a hint instead.
- No `unsafe`; the crate declares `#![forbid(unsafe_code)]`.
- Never `git add -A` or `git add .`. The sandbox puts character-device shims (`.bashrc`,
  `.zshrc`, `.idea`, `.vscode`, `.gitconfig`, `.gitmodules`, `.mcp.json`, `.profile`,
  `.ripgreprc`, `.bash_profile`, `.zprofile`) in the working tree. Stage explicit paths only,
  and never touch those entries.
- Never commit unless asked. Never tag, publish, push, create a hosted release, or change the
  package version without a separate explicit request.
- Do not silently skip a tar entry, a missing tool, or a failed verification. Skipping is a
  reported decision or an error, never a default.
- Do not add a dependency the current milestone does not use.
- Do not weaken an assertion or unpin an action SHA to make a gate pass.

## Style

Conventional Commits with a module scope. Prose hard-wraps at about 100 columns, no emoji, no
marketing language. Every source file starts with `SPDX-License-Identifier: MIT OR Apache-2.0`.
Public items are documented; `missing_docs` is warned on and clippy denies warnings, so an
undocumented public item fails the gate.
