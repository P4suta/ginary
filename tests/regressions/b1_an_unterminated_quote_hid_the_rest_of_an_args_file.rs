// SPDX-License-Identifier: MIT OR Apache-2.0
//! An args file with a quote nobody closed swallowed everything after it, and
//! the flags in the tail were never linted.
//!
//! **What went wrong.** `config::tokenize_args_file` tracked the open quote
//! and, at the end of the input, simply flushed whatever it had built. A file
//! holding `-setcookie 'oops` therefore produced one token whose text was the
//! rest of the file, newlines included. `lint_args_file` walks tokens, so the
//! `-pa` on the next line was inside that token rather than being one, and the
//! build accepted an args file that puts a code path in front of ginary's own.
//! `erl -args_file` would not have read it that way.
//!
//! **The input.** An args file whose second line opens a quote that is never
//! closed, and whose third line holds `-pa`.
//!
//! **The correct behaviour.** A tokenization that cannot be trusted is not
//! linted, it is refused: the build names the file and the line the quote was
//! opened on, and says nothing about the flags it could not see.

use std::path::Path;

use ginary::config::{self, ConfigError};

/// A file whose quote opens on line 2 and is never closed.
const UNTERMINATED: &str = "+SDio 4\n-setcookie 'oops\n-pa /opt/lib\n";

/// The manifest an args file is named by, for the message.
const MANIFEST: &str = "/w/app/config/vm.args";

#[test]
fn an_args_file_with_an_unterminated_quote_is_refused_rather_than_reinterpreted() {
    let error = config::lint_args_file(UNTERMINATED, Path::new(MANIFEST))
        .expect_err("a quote nobody closed makes every token after it a guess");

    let message = error.to_string();
    assert!(
        message.contains(MANIFEST) && message.contains(":2"),
        "the refusal must name the file and the line the quote was opened on: {message}"
    );
}

#[test]
fn the_hidden_flag_is_not_what_gets_reported() {
    // The `-pa` on the third line is inside the token the old tokenizer built,
    // and reporting it would be reporting a reading of the file that `erl`
    // does not share. The quote comes first.
    let error =
        config::lint_args_file(UNTERMINATED, Path::new(MANIFEST)).expect_err("the file is refused");

    assert!(
        matches!(error, ConfigError::ArgsFileQuote { line: 2, .. }),
        "expected ConfigError::ArgsFileQuote on line 2, got {error:?}"
    );
}

#[test]
fn a_file_whose_quotes_all_close_is_still_accepted() {
    let text = "-setcookie 'a b'\n+S 2:2 # a comment with an apostrophe's quote\n";

    config::lint_args_file(text, Path::new(MANIFEST))
        .expect("a comment is not a place a quote can be left open");
}
