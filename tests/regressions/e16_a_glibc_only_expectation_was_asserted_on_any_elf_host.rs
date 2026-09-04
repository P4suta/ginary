// SPDX-License-Identifier: MIT OR Apache-2.0
//! A `needs:` assertion keyed on the object format asserted glibc's shape
//! against a musl host.
//!
//! **What went wrong.** E16 gated six `#[cfg(target_os = "linux")]` tests on
//! `target_env = "gnu"` as well, because `target_os = "linux"` is true on musl
//! Linux and the six asserted things only glibc provides. A seventh made the
//! same claim in the one shape the scanner written for that sweep cannot see:
//! `tests/stage_run.rs::the_needs_line_lists_the_libraries_the_runtime_loads`
//! carries no `cfg` attribute at all and chooses its expectation at *run* time,
//! from `platform::object_format(platform::HOST)`.
//!
//! `object_format` maps `Os::Linux` to `ObjectFormat::Elf` with no C library in
//! the question — that is the whole of its job — so the ELF arm asserted
//! glibc's four sonames and a `(GLIBC_` floor on every Linux host. On Alpine
//! the host `erl` names `libc.musl-<arch>.so.1`, carries no symbol versions to
//! derive a floor from, and the test fails against a machine with nothing wrong
//! with it. A guard that fails a good tree is a guard the next author deletes,
//! which is the argument the sweep was made from.
//!
//! **The input.** Any Linux host whose C library is not glibc.
//!
//! **The correct behaviour.** The expectation is a fact about the *runtime's*
//! C library and not about the container format it happens to be written in,
//! so it is keyed on both. `common::portability::host_needs_expectation` is
//! that rule as a pure function, which is what makes it assertable from a host
//! that is not the one it describes; `tests/stage_run.rs` reads it rather than
//! writing the names down a second time.

use ginary::target::{Arch, Libc, Os, Target};

use crate::common::portability::host_needs_expectation;

#[test]
fn a_musl_host_is_not_expected_to_name_glibc() {
    let expectation = host_needs_expectation(Target::new(Os::Linux, Arch::X86_64, Libc::Musl));
    assert_eq!(
        expectation.libraries,
        vec!["libc.musl-x86_64.so.1".to_owned()],
        "musl publishes one C library and names it after the architecture; `libc.so.6` is \
         glibc's own soname and a musl runtime never loads it"
    );
    assert!(
        !expectation.glibc_floor,
        "musl carries no symbol versions, so there is no `(GLIBC_` floor to read and asserting \
         one fails a healthy host"
    );
    assert_eq!(
        host_needs_expectation(Target::new(Os::Linux, Arch::Aarch64, Libc::Musl)).libraries,
        vec!["libc.musl-aarch64.so.1".to_owned()],
        "and the name carries the architecture, so one row cannot stand for both"
    );
}

#[test]
fn a_gnu_host_is_still_held_to_every_name_it_had_before() {
    let expectation = host_needs_expectation(Target::new(Os::Linux, Arch::X86_64, Libc::Gnu));
    assert_eq!(
        expectation.libraries,
        vec![
            "libc.so.6".to_owned(),
            "libtinfo.so.6".to_owned(),
            "libstdc++.so.6".to_owned(),
            "libgcc_s.so.1".to_owned(),
        ],
        "narrowing the rule for musl must not weaken it for the host it was written on"
    );
    assert!(
        expectation.glibc_floor,
        "the glibc floor is the number a user needs most, and a gnu host still has to print it"
    );
    assert!(!expectation.fold_case, "an ELF soname is spelled one way");
}

#[test]
fn the_two_hosts_with_one_system_c_library_claim_no_floor() {
    for host in [
        Target::new(Os::Macos, Arch::Aarch64, Libc::None),
        Target::new(Os::Windows, Arch::X86_64, Libc::None),
    ] {
        let expectation = host_needs_expectation(host);
        assert!(
            !expectation.glibc_floor,
            "{} has no glibc and therefore no floor",
            host.name()
        );
        assert!(
            !expectation.libraries.is_empty(),
            "{} still has one name that must be there, or the assertion is measuring nothing",
            host.name()
        );
    }
    assert!(
        host_needs_expectation(Target::new(Os::Windows, Arch::X86_64, Libc::None)).fold_case,
        "a PE import table spells one file name both ways"
    );
}
