// SPDX-License-Identifier: MIT OR Apache-2.0
//! Three user-facing sentences carried a run of literal spaces, because the
//! string literal holding them was wrapped at the source margin without the
//! `\` continuation the rest of the crate uses.
//!
//! `rustfmt` does not touch the inside of a string, and no test asserted on
//! the rendered text, so all five gates were clean while the strip report and
//! the repack report printed
//!
//! ```text
//! 4 native files are for another machine than this one (aarch64), and `strip` here                  reads x86_64; ...
//! ```
//!
//! The right behaviour is the one the rest of the crate already follows: a
//! literal that has to wrap ends the line with `\`, so the source wraps and
//! the message does not.
//!
//! This is a shape test rather than a behaviour test on purpose: the three
//! sites are in two modules and one of them is only reachable on a machine
//! with no Erlang at all, and what they have in common is the typo, not the
//! code path. The scan is the same one that found them.
#![cfg(feature = "cli")]

use std::path::{Path, PathBuf};

/// The longest run of spaces a sentence may hold: one.
///
/// Four is the threshold rather than two so that a deliberate two-space
/// sentence break, if anybody ever writes one, is not a failure; nothing in a
/// rendered sentence has any reason to hold four.
const RUN: usize = 4;

/// Every `.rs` file directly under `src/`.
fn sources() -> Vec<PathBuf> {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut found: Vec<PathBuf> = std::fs::read_dir(&src)
        .expect("the crate has a src directory")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
        .collect();
    found.sort();
    assert!(!found.is_empty(), "src/ holds Rust files");
    found
}

/// The columns of `line` inside a double-quoted literal.
///
/// Naive on purpose — it toggles on every unescaped `"` — because what it is
/// looking for is a run of spaces, and a mis-paired quote can only make the
/// scan look at more text, never less.
fn quoted_spans(line: &str) -> Vec<(usize, usize)> {
    let bytes: Vec<char> = line.chars().collect();
    let mut spans = Vec::new();
    let mut start: Option<usize> = None;
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            '\\' => index += 1,
            '"' => match start {
                Some(open) => {
                    spans.push((open + 1, index));
                    start = None;
                }
                None => start = Some(index),
            },
            _ => {}
        }
        index += 1;
    }
    if let Some(open) = start {
        spans.push((open + 1, bytes.len()));
    }
    spans
}

/// Whether `span` of `chars` holds a run of [`RUN`] spaces inside a sentence.
fn runs_words_together(chars: &[char], (from, to): (usize, usize)) -> bool {
    let mut index = from;
    while index < to {
        if chars[index] != ' ' {
            index += 1;
            continue;
        }
        let start = index;
        while index < to && chars[index] == ' ' {
            index += 1;
        }
        if index - start < RUN || start == from || index >= to {
            continue;
        }
        let before = chars[start - 1];
        let after = chars[index];
        // A word before and a word after: a wrapped sentence. Indentation
        // inside a raw literal has neither.
        if (before.is_ascii_lowercase() || matches!(before, ',' | ';' | '.'))
            && after.is_ascii_alphabetic()
        {
            return true;
        }
    }
    false
}

#[test]
fn no_message_in_the_crate_carries_a_run_of_literal_spaces() {
    let mut offenders: Vec<String> = Vec::new();
    for path in sources() {
        let text = std::fs::read_to_string(&path).expect("a source file");
        for (number, line) in text.lines().enumerate() {
            let chars: Vec<char> = line.chars().collect();
            for span in quoted_spans(line) {
                if runs_words_together(&chars, span) {
                    offenders.push(format!(
                        "{}:{}: {}",
                        path.file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                            .unwrap_or_default(),
                        number + 1,
                        line.trim()
                    ));
                    break;
                }
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "a wrapped string literal needs a trailing `\\`, or the run of spaces reaches the \
         reader:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn the_scan_itself_finds_the_shape_it_is_looking_for() {
    // Without this the test above would pass on a scanner that found nothing
    // at all, which is exactly what it is guarding against.
    let bad = r#"    "and `strip` here                  reads x86_64""#;
    let chars: Vec<char> = bad.chars().collect();
    assert!(
        quoted_spans(bad)
            .into_iter()
            .any(|span| runs_words_together(&chars, span)),
        "the scan catches the defect it was written for"
    );

    let indented = r#"    "one space between every word""#;
    let chars: Vec<char> = indented.chars().collect();
    assert!(
        !quoted_spans(indented)
            .into_iter()
            .any(|span| runs_words_together(&chars, span)),
        "and an ordinary sentence is not a defect"
    );
}
