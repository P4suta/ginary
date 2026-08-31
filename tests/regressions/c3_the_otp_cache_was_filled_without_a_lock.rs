// SPDX-License-Identifier: MIT OR Apache-2.0
//! Two builds filling one OTP cache entry raced each other.
//!
//! `catalog::ensure_otp` checked `is_complete(&dir)` and then handed the work
//! to `extract_into_cache`, which begins
//!
//! ```rust
//! if dir.exists() { std::fs::remove_dir_all(dir)?; }
//! ```
//!
//! and ends with a `rename` of `<dir>.tmp-<pid>` onto `<dir>`. Nothing held a
//! lock, and the staging directory's only distinguishing mark is the process
//! id — so two threads of one process share it outright, and two processes
//! that both saw an incomplete entry both remove and both rename. The second
//! rename lands on a directory that is no longer empty and fails `ENOTEMPTY`,
//! which surfaces as `CatalogError::Io { message: "Directory not empty" }` for
//! a runtime that is, by then, perfectly cached; worse, a process that checked
//! `is_complete` a moment too early deletes the tree its peer has just
//! renamed into place while a third reads out of it.
//!
//! The crate already owns the answer — `src/cache.rs` takes
//! `cache_lock::try_exclusive` around exactly this operation for the payload
//! cache — and `src/catalog.rs` did not mention `cache_lock` at all.
//!
//! The right behaviour: one filler at a time, and everybody else either waits
//! or finds the entry complete. Every caller gets the same directory back and
//! nobody gets an error.
#![cfg(feature = "cli")]

use std::path::PathBuf;

use ginary::catalog::{self, CatalogError, EnsureContext, OtpReq};
use ginary::diag::Diag;
use ginary::download::Net;

use crate::common::catalog::{CatalogBuilder, ERTS_VSN, RELEASE, VERSION, static_variant};
use crate::common::fake_otp::FakeOtp;
use crate::common::payload::sha256_hex;

/// The target the fixture entry is for.
const MUSL: &str = "linux-x86_64-musl";

/// How many callers race for the one entry.
const RACERS: usize = 6;

#[test]
fn several_callers_filling_one_cache_entry_all_get_the_same_runtime() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let source = dir.path().join("source");
    FakeOtp::new()
        .erts_vsn(ERTS_VSN)
        .release(RELEASE)
        .otp_version(VERSION)
        .build_in(&source);
    let tarball = crate::common::catalog::runtime_tarball(&source);
    let digest = sha256_hex(&tarball);

    // A file URL beside the catalogue, so the race is over the extraction and
    // not over a socket.
    let catalog_dir = dir.path().join("dist/otp");
    std::fs::create_dir_all(&catalog_dir).expect("the catalog directory");
    std::fs::write(catalog_dir.join("otp.tar.zst"), &tarball).expect("the runtime tarball");

    let catalog = CatalogBuilder::new()
        .entry(
            VERSION,
            RELEASE,
            ERTS_VSN,
            MUSL,
            "static",
            static_variant("otp.tar.zst", &digest, tarball.len() as u64),
        )
        .build();

    let cache = dir.path().join("cache/otp");
    std::fs::create_dir_all(&cache).expect("the cache root");

    let results: Vec<Result<PathBuf, CatalogError>> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..RACERS)
            .map(|_| {
                let catalog = &catalog;
                let cache = cache.as_path();
                let catalog_dir = catalog_dir.as_path();
                scope.spawn(move || {
                    let selected = catalog
                        .select(&OtpReq::Host(RELEASE), MUSL, None, "the fixture")
                        .expect("the fixture holds it");
                    let net = Net::offline();
                    let diag = Diag::disabled();
                    catalog::ensure_otp(
                        &selected,
                        &EnsureContext {
                            cache_root: cache,
                            catalog_dir: Some(catalog_dir),
                            net: &net,
                            diag: &diag,
                        },
                    )
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().expect("no filler panicked"))
            .collect()
    });

    let expected = cache.join(format!("{VERSION}-{MUSL}-static"));
    for (index, result) in results.iter().enumerate() {
        match result {
            Ok(path) => assert_eq!(
                *path, expected,
                "filler {index} answered about another directory"
            ),
            Err(error) => panic!(
                "filler {index} failed over a runtime every other caller has: {error}\n\
                 a shared cache needs a lock, not a hope"
            ),
        }
    }
    assert!(
        expected
            .join(format!("erts-{ERTS_VSN}/bin/beam.smp"))
            .is_file(),
        "and what is left behind is one whole runtime"
    );
    assert!(
        catalog::is_complete(&expected),
        "with its completion marker in place"
    );
}
