<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# F1 — real native builds need only tools their host uses

The final Windows readiness review found that `tests/e2e_hello.rs` unconditionally required
`strip`, despite native Windows staging producing PE objects and the product's native
stripper explicitly reporting that format as skipped. This prevented real installed Gleam
and Erlang from exercising the build and launcher acceptance paths when no executable named
`strip` was on PATH.

`tests/common/built.rs::HOST_BUILD_TOOLS` now defines the prerequisites of the native
`hello_ffi` fixture: Gleam and Erlang on all hosts, plus strip on Linux ELF hosts. The E2E,
real SBOM/verify, native stage-and-strip, and real SBOM-publication regression gates use it.
The stage tests already distinguish BEAM stripping, which runs everywhere, from native
ELF savings, which they only require for an ELF host. Direct stripper tests retain their
own prerequisites; the hand-assembled Unix artifact and cross-stub fixtures are unchanged.

The header of `tests/windows_build.rs` now describes its synthetic structural fixtures and
points to the separate real runtime acceptance tests. A historical milestone's absence of
a runtime is not a statement about every future machine that reads those tests.

This is a test prerequisite correction; production build, stripping and launcher behavior
are unchanged. No Cargo commands were run in the agent while the root task owned final
validation. The root's actual native Windows E2E run is the acceptance evidence; merely
removing an unnecessary gate is not counted as a successful real build. `git diff --check`
passed after the changes. No release, upload, tag or push was performed.
