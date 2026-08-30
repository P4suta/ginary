// SPDX-License-Identifier: MIT OR Apache-2.0
//! The first entry of a payload, over arbitrary bytes.
//!
//! `payload::read_manifest` is the deepest the launcher goes before it has
//! decided to trust anything: zstd, then tar, then serde, over bytes that came
//! out of the file the process is running from.
//!
//! `unpack` is deliberately *not* the target. It writes to disk and creates
//! directories, so a fuzzer would spend its time in the kernel and leave a
//! tree behind after every crash; the parsing this target covers is the half
//! that reads the untrusted structure.

#![no_main]

use ginary::payload::read_manifest;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = read_manifest(data);
});
