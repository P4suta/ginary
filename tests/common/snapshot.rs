// SPDX-License-Identifier: MIT OR Apache-2.0
//! Making output that names a temporary directory snapshot-safe.
//!
//! A closure resolves real paths, so every message it produces carries the
//! `tempfile` directory the test built a tree in — a name that changes on
//! every run. [`scrub`] replaces those prefixes with stable placeholders, so
//! what the snapshot pins is the sentence and the shape of the path rather
//! than the machine it ran on.
//!
//! Longest prefix first, because `<tmp>/otp` is a prefix of nothing but
//! `<tmp>` is a prefix of both trees, and replacing the short one first would
//! leave `<tmp>/otp/lib` unrecognisable.
//!
//! The separator is scrubbed too, through [`crate::common::hostpath::slashed`].
//! A snapshot of a message that names a path is a claim about the sentence and
//! the shape of the path, not about which slash the host writes between two
//! components — and on Windows it is *both*, because a path built by joining a
//! `/`-spelled relative name onto an absolute one carries whichever separator
//! each half was written with. Four snapshots failed on the Windows runner
//! with diffs that were entirely spelling; see
//! `tests/regressions/e10_a_snapshot_pinned_the_hosts_own_path_spelling.rs`.
//! On a host that already writes `/` this changes nothing.

use std::path::Path;

use crate::common::hostpath::slashed;

/// Replaces each path with its placeholder, longest path first, and respells
/// every remaining separator as `/`.
pub fn scrub(text: &str, replacements: &[(&Path, &str)]) -> String {
    let mut pairs: Vec<(String, &str)> = replacements
        .iter()
        .map(|(path, name)| (slashed(&path.display().to_string()), *name))
        .collect();
    pairs.sort_by_key(|(path, _)| std::cmp::Reverse(path.len()));

    let mut scrubbed = slashed(text);
    for (path, name) in pairs {
        scrubbed = scrubbed.replace(&path, name);
    }
    scrubbed
}
