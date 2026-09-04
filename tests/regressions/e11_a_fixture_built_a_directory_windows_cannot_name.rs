// SPDX-License-Identifier: MIT OR Apache-2.0
//! Two fixtures built directories called `out*` and `x:`, which are not file
//! names on every platform, and failed before the claim they carry was
//! reached.
//!
//! **What went wrong.** Both tests deliberately choose an awkward name,
//! because the awkwardness is the subject. `a2_the_staged_root_became_a_wildcard`
//! stages into `out*` to prove that `beam_lib:strip_files/1` treats the root as
//! a path and not as a `filelib:wildcard` prefix that would reach the sibling
//! `outer`. `c3_otp_update_truncated_the_catalog_it_replaced` writes a
//! catalogue under a directory called `x:` so that the path holds `//` and can
//! be told from a URL. Neither name exists on Windows, where nine printable
//! characters are reserved:
//!
//! ```text
//! ---- a_root_named_with_a_star_leaves_its_neighbours_alone ----
//! an ebin directory: Os { code: 123, kind: InvalidFilename,
//!   message: "The filename, directory name, or volume label syntax is incorrect." }
//!
//! ---- a_path_that_holds_a_double_slash_is_a_path_and_not_a_url ----
//! a catalog at an awkward path: Os { code: 3, kind: NotFound, ... }
//! ```
//!
//! (`Windows build and exit-code propagation`
//! <https://github.com/P4suta/ginary/actions/runs/33751715516/job/100636537290>,
//! `tests/regressions/a2_the_staged_root_became_a_wildcard.rs:44` and
//! `tests/regressions/c3_otp_update_truncated_the_catalog_it_replaced.rs:133`.)
//!
//! **The input.** Any name holding `< > : " / \ | ? *`, or ending in a dot or
//! a space. Note that the sibling test in the same file — the one that stages
//! into `out[1]` — passes there: `[` and `]` are perfectly good Windows file
//! names and are `filelib:wildcard` metacharacters just the same, so the claim
//! is still made on that host by the name the platform permits.
//!
//! **The correct behaviour.** Which names a platform will create is a fact
//! about the platform, so it is `ginary::platform::is_legal_file_name`, and a
//! fixture picks from the names that are legal where it is running — reporting
//! the ones it had to leave out rather than failing on them.

use ginary::platform::is_legal_file_name;
use ginary::target::Os;

/// The `filelib:wildcard/1` metacharacters, which is the set
/// `a2_the_staged_root_became_a_wildcard` needs a legal name from.
const GLOB_METACHARACTERS: [&str; 6] = ["out*", "out?", "out[1]", "out]", "out{a}", "out}"];

#[test]
fn the_reserved_characters_are_reserved_only_where_they_are() {
    for name in ["out*", "out?", "a:b", r"a\b", "a<b", "a>b", "a|b", "a\"b"] {
        assert!(
            !is_legal_file_name(Os::Windows, name),
            "{name:?} holds a character Windows reserves"
        );
    }
    for name in ["out*", "out?", "x:", "a<b", "a|b"] {
        assert!(
            is_legal_file_name(Os::Linux, name),
            "{name:?} is an ordinary unix file name"
        );
        assert!(is_legal_file_name(Os::Macos, name));
    }
}

#[test]
fn a_name_that_would_be_normalised_away_is_not_a_name() {
    for name in ["out.", "out ", "out..", ""] {
        assert!(
            !is_legal_file_name(Os::Windows, name),
            "{name:?} is normalised to something else before it reaches the filesystem"
        );
    }
    assert!(
        is_legal_file_name(Os::Linux, "out."),
        "a trailing dot is part of a unix file name"
    );
}

#[test]
fn a_separator_is_never_one_component_and_neither_is_nothing() {
    for os in [Os::Linux, Os::Macos, Os::Windows] {
        assert!(!is_legal_file_name(os, ""), "{os:?}");
        assert!(!is_legal_file_name(os, "a/b"), "{os:?}");
        assert!(!is_legal_file_name(os, "a\0b"), "{os:?}");
    }
}

#[test]
fn every_platform_keeps_at_least_one_glob_metacharacter_to_test_with() {
    // The claim `a2_the_staged_root_became_a_wildcard` makes is that a root
    // whose name is a `filelib:wildcard` pattern is still only a path. A
    // platform on which no such name can be created could not make the claim
    // at all; each of the three can.
    for os in [Os::Linux, Os::Macos, Os::Windows] {
        let usable: Vec<&str> = GLOB_METACHARACTERS
            .into_iter()
            .filter(|name| is_legal_file_name(os, name))
            .collect();
        assert!(
            !usable.is_empty(),
            "{os:?} permits none of {GLOB_METACHARACTERS:?}, so the wildcard claim cannot be \
             made there at all"
        );
    }
    assert_eq!(
        GLOB_METACHARACTERS
            .into_iter()
            .filter(|name| is_legal_file_name(Os::Windows, name))
            .collect::<Vec<_>>(),
        ["out[1]", "out]", "out{a}", "out}"],
        "`*` and `?` are the two Windows will not create, and the brackets and braces are \
         metacharacters just the same"
    );
}
