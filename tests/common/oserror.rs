// SPDX-License-Identifier: MIT OR Apache-2.0
//! What the operating system says, in its own words, for an assertion that
//! has to hold on every host.
//!
//! `src/error.rs`'s unit tests have had this since E7, under the same name
//! and for the same reason: the text after ginary's own colon is not ginary's
//! to spell. Every C library renders `strerror` in its own words and some
//! render them in the user's language, so an expectation that quotes one of
//! them is exact on one host and wrong on the next. The integration suite
//! spelled the same fact in English until a Japanese Windows read it back —
//! see
//! `tests/regressions/e23_a_test_expected_the_operating_systems_words_in_english.rs`
//! — so the helper now lives where both halves of the suite can reach it.

/// What this host says about the raw operating-system error `code`.
///
/// The whole of `io::Error`'s own `Display`, tail included: `No such file or
/// directory (os error 2)` on glibc, `The system cannot find the file
/// specified. (os error 2)` on an English Windows, and the same sentence in
/// the user's language on one that is not.
///
/// A test asserts that this text *survives* into a message. What ginary owns
/// — the prefix, the path, the numbered exit code — is pinned separately and
/// in ginary's own words.
pub fn os_words(code: i32) -> String {
    std::io::Error::from_raw_os_error(code).to_string()
}

/// The raw code every platform reports for a file that is not there:
/// `ENOENT` on unix and `ERROR_FILE_NOT_FOUND` on Windows, which are the same
/// number.
pub const NOT_FOUND: i32 = 2;
