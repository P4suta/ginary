// SPDX-License-Identifier: MIT OR Apache-2.0
//! Documentation completeness, proved by scanning the sources.
//!
//! `#![warn(missing_docs)]` plus `clippy -D warnings` already fails the gate on
//! an undocumented item that is part of the crate's *public* API. It says
//! nothing about a `pub(crate) fn` in a private module, which is most of
//! ginary: every module below the four the library re-exports is `mod`, not
//! `pub mod`, so its `pub` items never reach the public surface `missing_docs`
//! guards. This file is that guard for the rest of the tree — every source
//! file carries a module doc, and every public item declared in it carries a
//! doc comment — and it is a simple line scan rather than a real parser on
//! purpose: the rule it enforces is a house rule, and a house rule a person can
//! read off the diff is one they can keep.
//!
//! It is a guard, not a feature, so on a clean tree it passes. Its red is
//! demonstrated by removing one doc comment; see `docs/dev/log/E1.md`.
//!
//! Ungated: the rule is about every source file, and the launcher-only stub is
//! built from the same tree.

use std::path::PathBuf;

/// The `src/` directory of the crate under test.
fn src_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src")
}

/// Every `*.rs` file directly under `src/`, sorted, as (name, contents).
fn sources() -> Vec<(String, String)> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(src_dir()).expect("read src/") {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .expect("utf8 file name")
            .to_owned();
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        out.push((name, text));
    }
    out.sort();
    out
}

/// A `pub` item declaration this scan holds to carrying a doc comment.
///
/// `pub mod` is excluded: a module's documentation is the `//!` in its own
/// file, which the module-doc scan checks. `pub use` is excluded: a re-export
/// documents nothing new. Everything else a `pub` (or `pub(crate)`) keyword
/// introduces — `fn`, `struct`, `enum`, `trait`, `type`, `const`, `static`,
/// `union` — is in scope.
fn is_documentable_pub_item(trimmed: &str) -> Option<&'static str> {
    let rest = trimmed.strip_prefix("pub")?;
    // Skip an optional visibility restriction, e.g. `pub(crate)`.
    let rest = match rest.strip_prefix('(') {
        Some(after) => after.split_once(')')?.1,
        None => rest,
    };
    let rest = rest.trim_start();
    // Strip modifiers that can sit between the visibility and the keyword.
    let mut head = rest;
    loop {
        let mut advanced = false;
        for modifier in ["async ", "unsafe ", "const ", "default ", "extern "] {
            if let Some(after) = head.strip_prefix(modifier) {
                // `extern "C"` carries an ABI string; drop to the next token.
                head = if modifier == "extern " {
                    after
                        .trim_start()
                        .trim_start_matches('"')
                        .split_once('"')
                        .map_or(after, |x| x.1)
                } else {
                    after
                };
                head = head.trim_start();
                advanced = true;
            }
        }
        if !advanced {
            break;
        }
    }
    for (keyword, label) in [
        ("fn ", "fn"),
        ("struct ", "struct"),
        ("enum ", "enum"),
        ("trait ", "trait"),
        ("type ", "type"),
        ("const ", "const"),
        ("static ", "static"),
        ("union ", "union"),
    ] {
        if head.starts_with(keyword) {
            return Some(label);
        }
    }
    None
}

/// Counts the net bracket balance a single line contributes, `[`+`(` opened
/// minus `]`+`)` closed. Only used to decide whether an attribute continues.
fn bracket_delta(line: &str) -> i32 {
    let mut d = 0i32;
    for ch in line.chars() {
        match ch {
            '[' | '(' => d += 1,
            ']' | ')' => d -= 1,
            _ => {}
        }
    }
    d
}

#[test]
fn every_source_file_has_a_module_doc() {
    let mut missing = Vec::new();
    for (name, text) in sources() {
        if !text.lines().any(|l| l.trim_start().starts_with("//!")) {
            missing.push(name);
        }
    }
    assert!(
        missing.is_empty(),
        "these source files carry no `//!` module doc: {missing:?}"
    );
}

#[test]
fn every_public_item_has_a_doc_comment() {
    let mut gaps: Vec<String> = Vec::new();
    for (name, text) in sources() {
        let lines: Vec<&str> = text.lines().collect();
        let mut i = 0usize;
        // Whether the most recent non-blank, non-attribute line was a doc
        // comment, so an attribute may sit between the doc and the item.
        let mut pending_doc = false;
        // Brace depth, and the depth of the nearest `#[cfg(test)]` module: a
        // public item inside the unit-test module is not part of the crate.
        let mut depth: i32 = 0;
        let mut test_mod_depth: Option<i32> = None;
        let mut pending_test_mod = false;
        while i < lines.len() {
            let raw = lines[i];
            let trimmed = raw.trim_start();

            // Multi-line attribute: consume through the line that closes it,
            // leaving `pending_doc` untouched, so `#[command(..)]` between a
            // doc comment and its `struct` does not read as code.
            if trimmed.starts_with("#[") || trimmed.starts_with("#![") {
                let is_doc = trimmed.starts_with("#[doc") || trimmed.starts_with("#![doc");
                if is_doc {
                    pending_doc = true;
                }
                let mut balance = bracket_delta(raw);
                while balance > 0 && i + 1 < lines.len() {
                    i += 1;
                    balance += bracket_delta(lines[i]);
                }
                i += 1;
                continue;
            }

            if trimmed.starts_with("///") || trimmed.starts_with("//!") {
                pending_doc = true;
                i += 1;
                continue;
            }
            if trimmed.is_empty() || trimmed.starts_with("//") {
                i += 1;
                continue;
            }

            if trimmed.starts_with("#[cfg(test)]") {
                pending_test_mod = true;
            }
            if pending_test_mod && trimmed.starts_with("mod ") {
                test_mod_depth = Some(depth);
                pending_test_mod = false;
            }
            let inside_test = matches!(test_mod_depth, Some(d) if depth > d);

            if !inside_test
                && let Some(label) = is_documentable_pub_item(trimmed)
                && !pending_doc
            {
                gaps.push(format!(
                    "{name}:{}: pub {label} without a doc comment",
                    i + 1
                ));
            }

            depth += bracket_delta_braces(raw);
            if let Some(d) = test_mod_depth
                && depth <= d
            {
                test_mod_depth = None;
            }
            pending_doc = false;
            i += 1;
        }
    }
    assert!(
        gaps.is_empty(),
        "every public item must be documented; {} without a doc comment:\n{}",
        gaps.len(),
        gaps.join("\n")
    );
}

/// Net brace balance a line contributes, `{` opened minus `}` closed.
fn bracket_delta_braces(line: &str) -> i32 {
    let mut d = 0i32;
    for ch in line.chars() {
        match ch {
            '{' => d += 1,
            '}' => d -= 1,
            _ => {}
        }
    }
    d
}
