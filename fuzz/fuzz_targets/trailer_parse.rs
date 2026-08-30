// SPDX-License-Identifier: MIT OR Apache-2.0
//! The 64 bytes at the end of a file, and the length the file claims to have.
//!
//! Those bytes are whatever is at the end of the running executable: a virus
//! scanner, an installer or a truncated download can leave anything there, and
//! `main()` reads them before it does anything else. The property is the one
//! `tests/trailer.rs` states as a proptest and this target explores with
//! coverage feedback: a value or a typed error, never a panic.
//!
//! The input is taken apart by hand rather than through `Arbitrary`, so that a
//! seed file is exactly what the end of a real artifact holds — 64 trailer
//! bytes followed by the file length — instead of whatever encoding a derived
//! `Arbitrary` happens to use. Everything past the magic is only reachable at
//! all if those seven bytes are right, and a fuzzer does not guess them.

#![no_main]

use ginary::trailer::Trailer;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() < 72 {
        return;
    }
    let mut raw = [0u8; 64];
    raw.copy_from_slice(&data[..64]);
    let mut length = [0u8; 8];
    length.copy_from_slice(&data[64..72]);

    let _ = Trailer::parse(&raw, u64::from_le_bytes(length));
});
