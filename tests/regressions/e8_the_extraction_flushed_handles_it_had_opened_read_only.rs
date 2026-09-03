// SPDX-License-Identifier: MIT OR Apache-2.0
//! Every cold-cache extraction on Windows failed with `Access is denied`,
//! because the durability flush opened each file read-only and then asked the
//! kernel to flush it.
//!
//! **What went wrong.** No packaged application could run on Windows at all.
//! The first file of the first extraction stopped it:
//!
//! ```text
//! ---- a_cold_cache_extracts_into_the_key_directory stdout ----
//! a cold cache must extract: Cache {
//!   path: "\\\\?\\C:\\Users\\RUNNER~1\\...\\.1179d51043100e24.tmp-2928\\bin\\no_dot_erlang.boot",
//!   source: Os { code: 5, kind: PermissionDenied, message: "Access is denied." } }
//! ```
//!
//! Thirteen `tests/cache.rs` targets, the whole of `tests/e2e_hello.rs` and
//! the D2 launcher regression fail on that one line.
//!
//! (`Windows build and exit-code propagation`
//! <https://github.com/P4suta/ginary/actions/runs/33712111530/job/100513644404>.)
//!
//! **The input.** Any extraction on Windows. `sync_tree` walks the freshly
//! written tree with `File::open`, which asks for read access alone, and
//! calls `sync_all`. On unix that is `fsync(2)`, which asks nothing of the
//! descriptor's access mode. On Windows it is `FlushFileBuffers`, which the
//! kernel refuses with `ERROR_ACCESS_DENIED` unless the handle was opened
//! for writing.
//!
//! **The correct behaviour.** The flush opens the handle the platform's own
//! barrier needs. [`ginary::platform::flush_needs_write_access`] is that
//! rule, stated once so that a machine with no Windows kernel on it can still
//! be held to it. Read access stays the unix answer deliberately: a staged
//! tree holds files a build has already made read-only, and asking for write
//! access there would fail for the opposite reason.

use ginary::platform::flush_needs_write_access;
use ginary::target::Os;

#[test]
fn the_platform_that_needs_a_writable_handle_to_flush_is_named() {
    assert!(
        flush_needs_write_access(Os::Windows),
        "`FlushFileBuffers` is refused on a handle that was not opened for writing, and an \
         extraction that cannot flush is an extraction that cannot finish"
    );
}

#[test]
fn a_unix_flush_asks_nothing_of_the_descriptors_access_mode() {
    assert_eq!(
        [
            flush_needs_write_access(Os::Linux),
            flush_needs_write_access(Os::Macos),
        ],
        [false, false],
        "`fsync(2)` flushes a read-only descriptor, and a staged tree holds files a build has \
         already made read-only"
    );
}
