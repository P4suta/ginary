<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# Contributing

## Test-driven development is mandatory

Every behaviour change goes through the same five phases, in order. A change that skips a phase
is sent back.

**RED.** Write the smallest failing test that states the wanted behaviour, then run it and read
the failure. A test that fails to compile is not RED: the failure must be an assertion failure,
which proves the test can distinguish right from wrong. Record the failure output when it is a
milestone deliverable.

**GREEN.** Write the smallest production change that makes the new test pass without breaking a
passing one.

**REVIEW.** Read the change adversarially against boundary values, concurrency, error paths,
clippy with warnings denied, and the documentation. Write the findings down.

**FIX.** For each finding, add a regression test first, watch it fail, then fix it. A fix with
no test is not a fix.

**REFACTOR.** Clean up with the suite green, and finish on `mise run check`.

Bugs are never fixed directly. A bug becomes a test under `tests/regressions/` named after its
issue, that test is confirmed to fail, and only then is the code changed.

## Local gates

```console
mise run check
```

which is `mise run lint`, `mise run test` and `mise run doc`, or individually:

```console
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
RUSTDOCFLAGS=-Dwarnings cargo doc --no-deps
```

Tests that need `gleam`, `erl`, `strip` or `docker` skip themselves when the tool is missing.
Set `GINARY_REQUIRE_TOOLCHAIN=1` to turn a skip into a failure; CI does.

Never weaken an assurance threshold, delete an assertion or unpin an action SHA to make a gate
pass.

## Sandbox note: `CARGO_HOME`

The development sandbox for this project has a read-only `~/.cargo/registry`. `mise.toml` sets

```toml
[env]
CARGO_HOME = "{{config_root}}/.cache/cargo-home"
```

so any cargo command run through mise works. When invoking cargo directly, export the same
value first:

```console
export CARGO_HOME="$PWD/.cache/cargo-home"
```

`.cache/` is git-ignored. Do not try to disable the sandbox.

## Commits and releases

Commit messages follow [Conventional Commits](https://www.conventionalcommits.org/): `feat`,
`fix`, `test`, `docs`, `refactor`, `chore`, `ci`, with a scope naming the module
(`feat(cache): ...`, `test(trailer): ...`). One milestone is one commit unless the work
genuinely splits.

Stage explicit paths. Never run `git add -A` in this repository: the development sandbox places
character-device shims (`.bashrc`, `.zshrc`, `.idea`, `.vscode`, `.gitconfig`, `.mcp.json` and
similar) in the working tree, and they are not project files.

Never tag, publish, push, create a hosted release or change the package version without a
separate explicit request. Releases are prepared by release-please as drafts and are promoted by
hand.

## Documentation

Prose hard-wraps at about 100 columns. Markdown table rows and code are exempt, including an
unbreakable string literal such as an `-eval` expression: wrapping either would break it. No
emoji and no marketing language. Architecture decisions get an ADR under `docs/adr/` in MADR
form, listed in `docs/adr/README.md`. Each milestone appends its result, its measurements and
its open questions to `docs/dev/log/<milestone>.md`.

Every source file carries `SPDX-License-Identifier: MIT OR Apache-2.0`; `REUSE.toml` covers the
rest of the tree.
