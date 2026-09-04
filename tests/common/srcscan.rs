// SPDX-License-Identifier: MIT OR Apache-2.0
//! Two pure scanners over Rust source, for defects only another platform can
//! see.
//!
//! A unit test that asks the host what platform it is on asserts a different
//! thing on every machine, and the assertion that is wrong is the one nobody
//! runs. Both defects E7 read out of the first Windows runner are that shape —
//! `runtime_bins(.., Target::host())` compared against the unix spelling of a
//! program name, and a launcher message compared against the C library's own
//! `strerror` text — and neither can be reproduced from Linux, because on
//! Linux the host *is* the platform the expectation was written for.
//!
//! So they are scanned rather than compiled, the way
//! `tests/regressions/e6_the_test_helpers_did_not_compile_on_windows.rs`
//! scans for the unix-only standard library: a pure function over one file's
//! text, calibrated against source it is handed before it is turned loose on
//! the tree.

/// Every 1-based line on which `callee(..)` is called with `needle` somewhere
/// in its argument list.
///
/// The argument list is taken by balancing parentheses from the `(` that
/// follows `callee`, skipping over double-quoted string literals so that a
/// parenthesis inside a message does not close the call. A call that spans
/// several lines is reported at the line the callee's own name is on, which is
/// the line a reader would go to.
///
/// Deliberately syntactic: this is a scanner over text, not a parser. It is
/// exact enough for what it is asked — "does any call to this function name
/// the host" — and the self-tests beside its callers show the shapes it was
/// calibrated on.
pub fn calls_with(source: &str, callee: &str, needle: &str) -> Vec<usize> {
    let bytes = source.as_bytes();
    let mut lines = Vec::new();
    let mut from = 0;
    while let Some(at) = source[from..].find(callee) {
        let start = from + at;
        from = start + callee.len();
        // `runtime_bins` must not match `check_runtime_bins`.
        if start > 0 && is_ident_byte(bytes[start - 1]) {
            continue;
        }
        let mut open = start + callee.len();
        while open < bytes.len() && bytes[open].is_ascii_whitespace() {
            open += 1;
        }
        if open >= bytes.len() || bytes[open] != b'(' {
            continue;
        }
        let Some(close) = closing_paren(bytes, open) else {
            continue;
        };
        if source[open..close].contains(needle) {
            lines.push(line_of(source, start));
        }
    }
    lines
}

/// Every 1-based line on which `needle` appears in code rather than in prose.
///
/// A line whose first non-blank characters are `//` is a comment, and a rule
/// that could not be *described* in the file it governs would be a rule
/// nobody could read.
pub fn literal_sites(source: &str, needle: &str) -> Vec<usize> {
    source
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim_start().starts_with("//") && line.contains(needle))
        .map(|(index, _)| index + 1)
        .collect()
}

/// The offset of the `)` that closes the `(` at `open`, if there is one.
fn closing_paren(bytes: &[u8], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut at = open;
    while at < bytes.len() {
        match bytes[at] {
            b'"' => {
                at += 1;
                while at < bytes.len() && bytes[at] != b'"' {
                    at += if bytes[at] == b'\\' { 2 } else { 1 };
                }
            }
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(at);
                }
            }
            _ => {}
        }
        at += 1;
    }
    None
}

/// The 1-based line `offset` falls on.
fn line_of(source: &str, offset: usize) -> usize {
    source[..offset]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        + 1
}

/// Whether `byte` can appear inside a Rust identifier.
fn is_ident_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}
