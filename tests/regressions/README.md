<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# Regression tests

One file per fixed bug. A bug fix without a file here is rejected.

## The convention

- The file is named after the bug: `<module>_<what_went_wrong>.rs`, for example
  `appfile_quoted_atom_escape.rs` or `otp_kernel_doc_matched_the_glob.rs`. When
  the bug has an issue number, prefix it: `issue_42_<short_name>.rs`.
- Every file starts with the SPDX header and a module comment that says, in
  order: what the wrong behaviour was, what input produced it, and what the
  correct behaviour is. A reader who has never seen the bug must be able to
  judge whether the test still earns its place.
- One file proves one bug. If a fix needs several assertions they belong in that
  one file, not spread across the suite.
- The test is written **before** the fix and is watched failing. A regression
  test that has never failed proves nothing.
- Files here are never deleted when the code they cover is refactored. They are
  deleted only when the behaviour they pin is deliberately removed, and then the
  commit message says so.

Cargo compiles every `.rs` file directly under `tests/` as its own integration
test binary, so files in this subdirectory are *not* test targets on their own.
Each one is included from `tests/regressions.rs`, which is the single target
that carries them:

```rust
// tests/regressions.rs
#[path = "regressions/appfile_quoted_atom_escape.rs"]
mod appfile_quoted_atom_escape;
```

`tests/regressions.rs` exists and lists every file here.
