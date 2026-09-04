// SPDX-License-Identifier: MIT OR Apache-2.0
//! The one allowlisted Windows library spelled with an upper-case suffix was
//! matched by exact equality only, so the lower-case spelling of it was
//! reported as a dependency the target may not have.
//!
//! **What went wrong.** `verify::needed_is_allowed` sends a name that is not an
//! exact match to a case-insensitive arm, and that arm decides whether an
//! allowlist entry is a Windows library by asking `known.ends_with(".dll")` —
//! a case-*sensitive* test, applied to the entry rather than to a lower-cased
//! copy of it:
//!
//! ```text
//! .any(|known| known.ends_with(WINDOWS_LIBRARY_SUFFIX) && known.eq_ignore_ascii_case(name))
//! ```
//!
//! `WINDOWS_NEEDED_ALLOWLIST` holds `IPHLPAPI.DLL`, whose suffix is spelled
//! `.DLL`. `"IPHLPAPI.DLL".ends_with(".dll")` is `false`, so that entry never
//! reached `eq_ignore_ascii_case` and only its own spelling was admitted.
//!
//! **The input.** A PE import table naming `iphlpapi.dll`. Windows file names
//! are case-insensitive and an import table records whatever spelling the
//! linker was handed, which is why the module's own documentation says every
//! name on the list is matched case-insensitively — `ginary verify` raised a
//! spurious `UnexpectedNeeded` finding and exited non-zero over the spelling.
//!
//! **The correct behaviour.** Every entry of the list is admitted in its own
//! spelling, in all-lower and in all-upper case; and a narrowed allowlist
//! still admits nothing, so the injection seam stays a seam.
#![cfg(feature = "cli")]

use ginary::verify::{self, WINDOWS_CRT_COMPANION, WINDOWS_CRT_PREFIX, WINDOWS_NEEDED_ALLOWLIST};

#[test]
fn every_windows_library_is_admitted_in_every_spelling_of_its_name() {
    for known in WINDOWS_NEEDED_ALLOWLIST {
        for spelling in [
            known.to_owned(),
            known.to_ascii_lowercase(),
            known.to_ascii_uppercase(),
        ] {
            assert!(
                verify::needed_is_allowed(&spelling, &WINDOWS_NEEDED_ALLOWLIST),
                "a PE import table records whatever spelling the linker was handed, and \
                 {spelling:?} is {known:?}"
            );
        }
    }
}

#[test]
fn the_universal_crt_family_is_admitted_in_every_spelling_too() {
    let member = format!("{WINDOWS_CRT_PREFIX}heap-l1-1-0.dll");
    for spelling in [member.clone(), member.to_ascii_uppercase()] {
        assert!(
            verify::needed_is_allowed(&spelling, &WINDOWS_NEEDED_ALLOWLIST),
            "the forwarding libraries are the runtime {WINDOWS_CRT_COMPANION} names: {spelling}"
        );
    }
}

#[test]
fn a_narrowed_allowlist_still_admits_nothing() {
    // The other side of the rule: the case-insensitive arm reads the list it
    // was handed, so a test that narrows the list to nothing gets nothing.
    for spelling in [
        "iphlpapi.dll",
        "IPHLPAPI.DLL",
        "api-ms-win-crt-heap-l1-1-0.dll",
    ] {
        assert!(
            !verify::needed_is_allowed(spelling, &[]),
            "an empty allowlist admits nothing, or the seam a test verifies through is not one: \
             {spelling}"
        );
    }
}
