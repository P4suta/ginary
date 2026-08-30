// SPDX-License-Identifier: MIT OR Apache-2.0
//! The chunk table of a compiled module, over arbitrary bytes.
//!
//! Every length field in an IFF form is attacker-controlled once the module
//! comes out of an artifact somebody edited, and a stripped module is a gzip
//! member, so this target reaches the decompression path as well as the
//! offsets.

#![no_main]

use ginary::beam::chunks;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = chunks(data);
});
