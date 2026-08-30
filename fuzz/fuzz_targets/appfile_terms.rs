// SPDX-License-Identifier: MIT OR Apache-2.0
//! The Erlang term parser, over arbitrary text.
//!
//! `.app` files come out of a shipment and out of an OTP library, neither of
//! which ginary wrote. The parser recurses, so the interesting inputs are the
//! deeply nested ones a random vector never produces and a coverage-guided
//! fuzzer finds in seconds.

#![no_main]

use ginary::appfile::parse_terms;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let _ = parse_terms(&text);
});
