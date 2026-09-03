// SPDX-License-Identifier: MIT OR Apache-2.0
//! A Windows `ginary.exe` carried the identity needle twice, so every scan of
//! it answered `Ambiguous` and the whole stub-identity mechanism stopped
//! working on that platform.
//!
//! **What went wrong.** `cargo test` on the Windows runner found two markers
//! in the one binary it had just built:
//!
//! ```text
//! ---- the_binary_this_test_run_built_carries_exactly_one_marker stdout ----
//! a ginary binary carries one identity marker, at one offset: [10961490, 10971432]
//!
//! ---- scanning_this_builds_own_binary_reports_its_identity stdout ----
//! the ginary binary is a stub: Ambiguous { count: 2 }
//! ```
//!
//! Sixteen `stub` and `stubid` targets failed with it, including
//! `the_running_ginary_verifies_as_a_host_stub`: on Windows, ginary could not
//! recognise itself.
//!
//! (`Windows build and exit-code propagation`
//! <https://github.com/P4suta/ginary/actions/runs/33739517757/job/100597889388>.)
//!
//! **The input.** Any Windows build. `stubid` never stores the needle whole —
//! it stores two halves and joins them at run time, precisely so that a
//! scanner does not match itself — but the two halves are two constants, and a
//! linker is free to lay two constants out next to each other. When it does,
//! the file holds `GINARY-STUB` immediately followed by `-ID\0`, which is the
//! needle, at an address no code in this crate chose.
//!
//! **The correct behaviour.** Two claims, because either one alone leaves the
//! mechanism at the mercy of the next linker:
//!
//! 1. No two of the byte images a build stores may be joinable into the
//!    needle, in any order. Splitting a constant is only a defence while the
//!    pieces cannot be put back together by accident.
//! 2. A scan counts *records* — 128 whole bytes, terminated, zero-padded,
//!    naming the four fields — not needle hits. Fifteen bytes of unrelated
//!    data are not an identity, so a file holding one record and one stray hit
//!    is a stub, and only two whole records are an ambiguity.

use crate::common::stubfile::{Marker, fragments, needle, noise, stray_needle};
use ginary::stubid::{self, StubIdError};

/// Every ordered pair of `stored`, including a fragment with itself: a linker
/// chooses the order, not the module that split the constant.
fn ordered_pairs(stored: &[&'static [u8]]) -> Vec<Vec<u8>> {
    let mut pairs = Vec::new();
    for first in stored {
        for second in stored {
            let mut joined = first.to_vec();
            joined.extend_from_slice(second);
            pairs.push(joined);
        }
    }
    pairs
}

#[test]
fn no_two_stored_fragments_can_be_laid_out_into_the_needle() {
    let needle = needle();

    for stored in [stubid::needle_fragments(), fragments()] {
        for joined in ordered_pairs(&stored) {
            assert!(
                !joined.windows(needle.len()).any(|w| w == &needle[..]),
                "a linker that placed these two side by side would put a second identity into \
                 every binary that holds them: {joined:?}"
            );
        }
    }
}

#[test]
fn a_binary_holding_one_record_and_one_stray_needle_is_that_record() {
    let mut bytes = noise(1024, 0x5eed);
    bytes.extend_from_slice(&stray_needle());
    bytes.extend_from_slice(&noise(256, 0x5eed_beef));
    let record_at = bytes.len();
    bytes.extend_from_slice(&Marker::host().bytes());

    let id = stubid::scan(&bytes).expect("one whole record is one identity, stray hits or not");

    assert_eq!(id.target, ginary::target::Target::host());
    assert_eq!(stubid::records(&bytes), vec![record_at]);
    assert_eq!(stubid::candidates(&bytes).len(), 2);
}

#[test]
fn two_whole_records_are_still_refused() {
    let marker = Marker::host().bytes();
    let mut bytes = noise(64, 0xfeed);
    bytes.extend_from_slice(&marker);
    bytes.extend_from_slice(&noise(64, 0xfeed_beef));
    bytes.extend_from_slice(&marker);

    let error = stubid::scan(&bytes).expect_err("a file with two identities has none");

    assert_eq!(error, StubIdError::Ambiguous { count: 2 });
    assert_eq!(
        stubid::records(&bytes).len(),
        2,
        "counting records rather than needle hits must not stop counting the real thing"
    );
}
