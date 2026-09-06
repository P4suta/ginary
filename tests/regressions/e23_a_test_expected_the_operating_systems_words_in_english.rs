// SPDX-License-Identifier: MIT OR Apache-2.0
//! A command line test quoted glibc's words for a missing file, so it failed
//! on the first development host whose Windows speaks Japanese.
//!
//! **What went wrong.** `appfile_parse_reports_a_missing_file_and_exits_one`
//! runs the real binary against a path that is not there and asserts that the
//! operating system's own reason survives into the message. It asserted it by
//! naming the reason:
//!
//! ```rust,ignore
//! assert!(
//!     stderr.contains("No such file or directory") || stderr.contains("cannot find the file"),
//!     "the operating system's own reason must survive: {stderr}"
//! );
//! ```
//!
//! Two spellings of one language. On a Japanese Windows the CRT renders
//! `ERROR_FILE_NOT_FOUND` in the user's language, and the test reported the
//! very message it exists to require:
//!
//! ```text
//! ---- appfile_parse_reports_a_missing_file_and_exits_one stdout ----
//! thread '...' panicked at tests\cli.rs:165:5:
//! the operating system's own reason must survive: error: cannot read the
//! application file `tests/fixtures/app/does_not_exist.app`
//!   caused by: cannot read `tests/fixtures/app/does_not_exist.app`:
//!             指定されたファイルが見つかりません。 (os error 2)
//!   caused by: 指定されたファイルが見つかりません。 (os error 2)
//! ```
//!
//! **The input.** Any host whose C library does not render its messages in
//! English. Linux in the `C` locale and the English `windows-2022` runner are
//! both hosts where the assertion is true and says nothing, which is why no
//! run before this one saw it.
//!
//! **The correct behaviour.** This is E7's rule, at a site E7 did not reach.
//! `tests/regressions/e7_the_unit_tests_asked_the_host_what_platform_it_was.rs`
//! established that the text after ginary's own colon belongs to
//! `io::Error`'s `Display`, and scanned `src/error.rs` for the `(os error `
//! tail to keep it that way. The command line suite spells the same fact
//! differently — it quotes the *words* rather than the tail — so the scan
//! there could not see it. An expectation built from
//! [`crate::common::oserror::os_words`] is exact on every host and still pins
//! what ginary owns: that the cause reaches the user at all.
//!
//! The scan below is over every tracked test source rather than over the one
//! file, because the defect is a habit rather than a line, and it looks for
//! the two spellings a *missing file* produces in English. Those two phrases
//! appear nowhere else in the tree outside prose: the other operating-system
//! words a test names — `Read-only file system`, `Permission denied` — are
//! values `tests/doctor.rs` supplies to a renderer and then reads back, which
//! is a test asserting its own input rather than the host's language.

use crate::common::portability::tracked_test_sources;
use crate::common::srcscan::literal_sites;

/// The English renderings of "that file is not there", one per C library.
///
/// glibc's `strerror(ENOENT)` and the substring both the Windows CRT's
/// `The system cannot find the file specified.` and its `The system cannot
/// find the file specified` share.
const ENOENT_IN_ENGLISH: [&str; 2] = ["No such file or directory", "cannot find the file"];

#[test]
fn the_scanner_reads_a_quoted_reason_in_code_and_leaves_the_prose_alone() {
    // The instrument: the phrase in an assertion, the phrase in a line
    // comment, and the phrase in a doc comment that has to be able to state
    // the rule it governs.
    let planted = "\
// glibc says `No such file or directory` and the Windows CRT says something else\n\
    assert!(stderr.contains(\"No such file or directory\"), \"{stderr}\");\n\
/// The message ends in `cannot find the file specified` on an English Windows.\n\
    assert!(stderr.contains(&os_words(2)), \"{stderr}\");\n";

    assert_eq!(
        literal_sites(planted, ENOENT_IN_ENGLISH[0]),
        vec![2],
        "a comment may describe the reason; a line of code may not require one language's"
    );
    assert_eq!(
        literal_sites(planted, ENOENT_IN_ENGLISH[1]),
        Vec::<usize>::new(),
        "the doc comment naming the English spelling is prose, and prose is not the subject"
    );
}

#[test]
fn no_tracked_test_requires_a_missing_file_to_be_reported_in_english() {
    let Some(sources) = tracked_test_sources() else {
        eprintln!("skipping: `git ls-files` did not answer, so `tracked` would be a guess");
        return;
    };
    assert!(
        sources.unreadable.is_empty(),
        "a tracked source the scan cannot read is a file it has no answer for, and reporting it \
         as clean is the silent skip CLAUDE.md forbids:\n{}",
        sources.unreadable.join("\n")
    );
    assert!(
        sources.files.len() > 40,
        "only {} tracked test sources were read; the scan has lost its subject",
        sources.files.len()
    );

    // This file spells both phrases in order to look for them — once as the
    // needles themselves, once in the sample the instrument above is
    // calibrated against — and neither is an assertion requiring a language.
    // `file!()` is the scanner's own path, so the exclusion cannot go stale
    // under a rename.
    let myself = file!().replace('\\', "/");

    let mut sites: Vec<String> = Vec::new();
    for (name, text) in &sources.files {
        if name == &myself {
            continue;
        }
        for phrase in ENOENT_IN_ENGLISH {
            for line in literal_sites(text, phrase) {
                sites.push(format!("{name}:{line}: {phrase}"));
            }
        }
    }
    sites.sort();
    sites.dedup();

    assert!(
        sites.is_empty(),
        "the words an operating system reports a missing file with are its own, and a test that \
         names them in English is a test that fails on a host that speaks something else. Build \
         the expectation from `common::oserror::os_words`:\n{}",
        sites.join("\n")
    );
}
