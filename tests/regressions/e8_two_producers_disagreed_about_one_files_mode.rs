// SPDX-License-Identifier: MIT OR Apache-2.0
//! `ginary verify` reported five mode mismatches against an artifact nothing
//! had damaged, because the `mode` column has two producers and they answer
//! differently on a filesystem with no permission bits.
//!
//! **What went wrong.** The Windows job ran the suite for the first time and
//! `ginary verify` found five issues in a freshly staged artifact — every one
//! of them a `index_mode_mismatch`, and every one of them on a program:
//!
//! ```text
//! ---- verify_json_carries_the_documented_keys stdout ----
//! stderr="error: C:\\Users\\RUNNER~1\\...\\hello: 5 issue(s) found"
//!   { "issue": "index_mode_mismatch", "path": "erts-17.0.5/bin/beam.smp",
//!     "indexed": "0644", "expected": "0644", "actual": "0755" }
//! ```
//!
//! The five are `erts-17.0.5/bin/{beam.smp,erl_child_setup,erlexec,
//! inet_gethost}` and `lib/hello/priv/bin/tool`.
//!
//! (`Windows build and exit-code propagation`
//! <https://github.com/P4suta/ginary/actions/runs/33712111530/job/100513644404>.)
//!
//! **The input.** Any staging tree read on a filesystem that has no POSIX
//! mode word. Three pieces of code record the mode of one staged file — the
//! listing in [`ginary::assemble`], the index in
//! [`ginary::manifest::Index::from_staged`], and the payload header the `tar`
//! crate writes — and on such a filesystem the first answered `0` (or
//! whatever the caller asked for and the filesystem discarded) while the
//! other two answered `0o644`. `ginary verify` then compared two different
//! facts and reported the difference, which is what it is for.
//!
//! **The correct behaviour.** One rule, one answer.
//! [`ginary::platform::modeless_mode`] is the mode column a platform without
//! permission bits records, and it is the value the `tar` crate itself writes
//! into a header there, so the index and the header agree and the comparison
//! can never fire over a fact the machine cannot see. The check is not
//! weakened: an index that really does disagree with its payload is still an
//! issue, on every platform.

use ginary::platform::{has_unix_modes, modeless_mode, recorded_mode};
use ginary::target::Os;

#[test]
fn the_recorded_mode_reads_the_filesystem_only_where_it_has_one() {
    // The wiring both producers share, split out from the `#[cfg(unix)]`
    // `st_mode` read so both arms are asserted on one machine: where the
    // platform has a mode word the raw value is recorded, and where it has
    // none the raw value is discarded for the header's own answer.
    assert_eq!(
        recorded_mode(true, 0o755, false),
        0o755,
        "a unix listing records what the filesystem carries"
    );
    assert_eq!(
        recorded_mode(true, 0o644, false),
        0o644,
        "including a file that is not executable"
    );
    assert_eq!(
        recorded_mode(false, 0o755, false),
        0o644,
        "a modeless filesystem cannot see the requested 0o755; it records the mode the header \
         will carry"
    );
    assert_eq!(
        recorded_mode(false, 0, true),
        0o755,
        "a directory on a modeless filesystem records the header's directory mode"
    );
}

#[test]
fn a_platform_without_permission_bits_records_the_mode_a_header_carries() {
    assert_eq!(
        (modeless_mode(false), modeless_mode(true)),
        (0o644, 0o755),
        "the two values `HeaderMode::Deterministic` can write are the two values a modeless \
         filesystem records, so the index column and the header column are the same fact"
    );
}

#[test]
fn whether_a_platform_has_permission_bits_is_a_property_of_the_platform() {
    assert_eq!(
        [
            has_unix_modes(Os::Linux),
            has_unix_modes(Os::Macos),
            has_unix_modes(Os::Windows),
        ],
        [true, true, false],
        "a Windows file has an ACL and a read-only flag, and a POSIX mode word maps onto \
         neither; the other two carry one"
    );
}
