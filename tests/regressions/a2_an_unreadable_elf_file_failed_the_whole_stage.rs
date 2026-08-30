// SPDX-License-Identifier: MIT OR Apache-2.0
//! One file that starts like an ELF and is not one failed the whole stage.
//!
//! **What went wrong.** The two modules that look at the same tree disagreed
//! about how much one odd file is worth. `report::measure` says so in its own
//! doc comment — "a file that starts like an ELF and cannot be parsed is a
//! warning rather than a failure: one odd file in a staged tree must not cost
//! the reader the whole account" — and pushes a line into
//! `SizeReport::warnings`. `strip::strip_elf` took the same file and returned
//! `StripError::Elf` from a bare `?`, before `strip` had even been looked for.
//! So four bytes of `\x7fELF` under `priv` — inert data, a fixture, a truncated
//! download — made `ginary stage` exit non-zero, and CLAUDE.md's rule that a
//! skip is a reported decision was satisfied in one module and not the other.
//!
//! **The input.** A tree holding one file whose whole content is the four
//! bytes of the ELF magic.
//!
//! **The correct behaviour.** Stripping succeeds, the file is skipped, and the
//! skip is reported by name in the strip table rather than being silent.

use ginary::strip::{self, ElfOutcome, StripOptions};

use crate::common::fake_otp::FakeOtp;

#[test]
fn four_bytes_of_elf_magic_are_a_reported_skip_and_not_a_failure() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    std::fs::create_dir_all(dir.path().join("otp")).expect("the otp root");
    let otp = FakeOtp::new().build_in(dir.path().join("otp"));
    let info = ginary::otp::inspect_root(&otp.root).expect("a usable OTP root");
    let root = dir.path().join("out");
    std::fs::create_dir_all(root.join("lib/notify/priv")).expect("a priv directory");
    std::fs::write(root.join("lib/notify/priv/data.bin"), b"\x7fELF").expect("the odd file");

    let report = strip::strip(
        &root,
        &info,
        &StripOptions {
            elf: true,
            beams: false,
        },
    )
    .unwrap_or_else(|error| panic!("one unreadable file must not fail the stage: {error}"));

    assert_eq!(
        report.elf,
        ElfOutcome::NothingToStrip,
        "the tree holds no ELF file `strip` could work on"
    );
    let table = report.to_string();
    assert!(
        table.contains("lib/notify/priv/data.bin"),
        "the skip has to name the file it skipped:\n{table}"
    );
}
