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

use std::path::Path;

/// Replaces each path with its placeholder, longest path first.
pub fn scrub(text: &str, replacements: &[(&Path, &str)]) -> String {
    let mut pairs: Vec<(String, &str)> = replacements
        .iter()
        .map(|(path, name)| (path.display().to_string(), *name))
        .collect();
    pairs.sort_by_key(|(path, _)| std::cmp::Reverse(path.len()));

    let mut scrubbed = text.to_owned();
    for (path, name) in pairs {
        scrubbed = scrubbed.replace(&path, name);
    }
    scrubbed
}
