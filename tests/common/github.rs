// SPDX-License-Identifier: MIT OR Apache-2.0
//! The names a GitHub token travels under, for the targets that cannot ask the
//! crate.
//!
//! `ginary::download::GITHUB_TOKEN_VARS` is the source of this list, and
//! `tests/download.rs` holds the two equal. The copy exists because the rule
//! that needs it most is in `tests/ci_matrix.rs`, which reads `.github/` in both
//! flavors of the suite and therefore cannot import a `cli`-gated module: a
//! workflow step that fetches an OTP asset has to be handed a token, and the
//! variable it hands it under has to be one the downloader actually reads.
//!
//! That pairing is the whole point. Rename the variable in `src/download.rs`
//! and leave the workflow alone and nothing fails loudly — the reads simply go
//! back to being anonymous, counted against the runner's shared address, until
//! a job somewhere hits 403 and the fix looks like an outage rather than a
//! regression. E22 is that failure once already.

/// The variables a GitHub token is taken from, in the order they are tried.
///
/// Held equal to `ginary::download::GITHUB_TOKEN_VARS` by
/// `the_downloader_reads_the_variables_the_workflows_are_held_to` in
/// `tests/download.rs`.
pub const TOKEN_VARS: [&str; 3] = ["GINARY_GITHUB_TOKEN", "GH_TOKEN", "GITHUB_TOKEN"];
