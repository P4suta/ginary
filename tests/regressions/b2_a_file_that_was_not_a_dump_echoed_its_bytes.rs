// SPDX-License-Identifier: MIT OR Apache-2.0
//! `crashdump` put a whole line of a stranger's file into its refusal.
//!
//! `CrashdumpError::NotACrashDump` carried the file's first line verbatim, and
//! the only bound on that line was the reader's own buffer: sixty-four
//! kilobytes of value plus the key margin. `ginary crashdump` over a binary, or
//! over a file whose first line is fifty thousand ANSI escape sequences, wrote
//! all of it to standard error — control characters included, so the file
//! decided what the reader's terminal did. The field's own documentation said
//! "truncated for the message", and nothing truncated it.
//!
//! The correct behaviour is a short, escaped prefix: at most
//! [`ginary::crashdump::MAX_FOUND_CHARS`] characters of the line, each one
//! escaped so that no byte of the file reaches a terminal as a control
//! sequence, and an ellipsis when there was more.

use std::io::Cursor;

use ginary::crashdump::{self, CrashdumpError};

/// The refusal, for a file whose first line is `bytes`.
fn refusal(bytes: Vec<u8>) -> String {
    match crashdump::parse(Cursor::new(bytes)) {
        Err(CrashdumpError::NotACrashDump { found }) => found,
        Err(other) => panic!("expected NotACrashDump, got {other:?}"),
        Ok(dump) => panic!("expected a refusal, got {dump:?}"),
    }
}

#[test]
fn a_long_first_line_is_cut_before_it_reaches_the_message() {
    let found = refusal(b"A".repeat(50_000));

    assert!(
        found.len() <= 1024,
        "the message quotes {} bytes of the file",
        found.len()
    );
}

#[test]
fn an_escape_sequence_in_the_file_never_reaches_a_terminal() {
    let found = refusal(b"\x1b[31mred\x07\x00".repeat(4_000));

    assert!(
        !found.chars().any(char::is_control),
        "a control character survived into the message: {found:?}"
    );
    assert!(
        found.len() <= 1024,
        "the message quotes {} bytes of the file",
        found.len()
    );
}
