// SPDX-License-Identifier: MIT OR Apache-2.0
//! Tests that assert glibc's own shape were gated on Linux, and musl Linux is
//! Linux.
//!
//! **What went wrong.** `#[cfg(target_os = "linux")]` is true on every Linux
//! target, and the two Linux C libraries this project ships for differ in
//! exactly the facts these tests assert. A gnu binary needs `libc.so.6`, names
//! `ld-linux-*.so.2` as its interpreter and carries a `GLIBC_x.y` floor in
//! `.gnu.version_r`. A musl one needs `libc.musl-<arch>.so.1`, and the static
//! musl build this project treats as its portability story needs nothing at
//! all, names no interpreter and has no symbol versions to derive a floor
//! from. Run there, the assertions fail against a runtime that is healthy:
//!
//! ```text
//! a dynamically linked Rust binary needs libc: ["libc.musl-x86_64.so.1"]
//! ```
//!
//! Nothing in the suite says so, because every machine and every job that runs
//! it today is gnu. `x86_64-unknown-linux-musl` and
//! `aarch64-unknown-linux-musl` are two of the seven targets `distribute.yml`
//! builds and `tests/e2e_cross.rs` exercises, so the host these tests would
//! fail on is one the project already claims to support — it has simply never
//! been the host that ran `cargo test`.
//!
//! **The input.** `cargo test` on any musl Linux host, or with a musl target
//! selected.
//!
//! **The correct behaviour.** A test that asserts something only glibc
//! provides says so in its gate: `#[cfg(all(target_os = "linux", target_env =
//! "gnu"))]`. The claim is about the C library and the gate names the C
//! library, so the test runs where it is true and is absent where it is not —
//! rather than being run everywhere and being wrong half the time. A test that
//! asserts something every Linux binary has keeps the gate it already has;
//! this is not a rule about Linux tests, it is a rule about glibc claims.
//!
//! The scanner is [`crate::common::portability::gnu_gate_sites`], a pure
//! function over one file's text, calibrated below against a committed fixture
//! before it is turned loose on the tree.

use crate::common::portability::{GnuGateSite, gnu_gate_sites, tracked_test_sources};
use crate::common::repo::read;

/// The calibration fixture: seven `#[test]` items covering every shape the
/// rule has to tell apart.
///
/// A committed file rather than a raw string in this one, because the scan
/// below reads every tracked `.rs` file under `tests/` and a fixture written
/// inline would be read as source of this file's own. The `.rs.txt` name keeps
/// it out of both that listing and cargo's.
const FIXTURE: &str = "tests/fixtures/portability/gnu_gated_tests.rs.txt";

/// The sites as `(line, name, claim)`, for an assertion that reads.
fn seen(sites: &[GnuGateSite]) -> Vec<(usize, &str, &str)> {
    sites
        .iter()
        .map(|site| (site.line, site.name.as_str(), site.claim.as_str()))
        .collect()
}

/// The sites, one per line, for a failure that can be read without the files
/// open beside it.
fn render(sites: &[GnuGateSite]) -> String {
    sites
        .iter()
        .map(GnuGateSite::to_string)
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn only_a_glibc_claim_under_a_libc_blind_gate_is_a_site() {
    let sites = gnu_gate_sites(FIXTURE, &read(FIXTURE));
    assert_eq!(
        seen(&sites),
        vec![
            (6, "a_linux_gate_over_a_glibc_claim", "libc.so.6"),
            (45, "a_second_linux_gate_over_a_loader_claim", "ld-linux"),
        ],
        "the two `target_os = \"linux\"` gates over a glibc claim are the sites. The gnu-gated \
         pair are not, on one line or wrapped over four; the PIE claim is true of a musl binary \
         too; the ungated test makes a real glibc claim and is a site only if the \
         `target_os = \"linux\"` precondition is dropped, which is what calibrates it; and a \
         needle in a comment is prose:\n{}",
        render(&sites)
    );
}

#[test]
fn no_tracked_test_asserts_a_glibc_only_fact_outside_a_gnu_gate() {
    let Some(sources) = tracked_test_sources() else {
        eprintln!("skipping: `git ls-files` did not answer, so `tracked` would be a guess");
        return;
    };
    assert!(
        sources.unreadable.is_empty(),
        "a tracked source the scan cannot read is a file it has no answer for, and reporting it \
         as clean is the silent skip CLAUDE.md forbids:\n{}",
        sources.unreadable.join("\n")
    );
    assert!(
        sources.files.len() > 40,
        "only {} tracked test sources were read; the scan has lost its subject",
        sources.files.len()
    );

    let mut sites: Vec<GnuGateSite> = Vec::new();
    for (name, text) in &sources.files {
        sites.extend(gnu_gate_sites(name, text));
    }

    assert!(
        sites.is_empty(),
        "`target_os = \"linux\"` is true on musl Linux, where none of these facts hold: a static \
         musl binary needs no `libc.so.6`, names no `ld-linux` interpreter and carries no \
         `GLIBC_x.y` floor. Two of the seven targets this project distributes are musl, so the \
         host these would fail on is one it already supports. Gate each on the C library its \
         claim is about — `#[cfg(all(target_os = \"linux\", target_env = \"gnu\"))]`:\n{}",
        render(&sites)
    );
}
