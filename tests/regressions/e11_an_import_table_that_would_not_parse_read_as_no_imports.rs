// SPDX-License-Identifier: MIT OR Apache-2.0
//! A PE whose import directory will not parse was reported as an object that
//! needs nothing, rather than as one nobody could read.
//!
//! **What went wrong.** `native::read_with_object_crate` asked the `object`
//! crate for the import table and dropped the error:
//!
//! ```text
//! if let Ok(imports) = file.imports() {
//! ```
//!
//! An `Err` there — an import directory whose RVA falls outside every section,
//! a truncated descriptor array — left `needed` empty and the function
//! answered `Ok`. That value is what `verify::describe` iterates to raise
//! `Issue::UnexpectedNeeded` and what `report::collect_elf_deps` turns into the
//! `needs:` line, so a Windows artifact carrying such a PE verified clean and
//! printed `needs: (none)`.
//!
//! This is the same defect one layer down that
//! `e11_the_deep_check_read_only_one_of_the_three_object_formats` was written
//! for: reporting the *absence of native dependencies* where what happened was
//! the absence of a reader for them. The two neighbouring error paths in the
//! same function — `File::parse` failing, and the missing `PE\0\0` signature —
//! both already answer `ObjectError::Unreadable`.
//!
//! **The input.** A PE32+ whose import data directory names an RVA past the
//! last section, which is what a truncated or a hostile `.dll` looks like.
//!
//! **The correct behaviour.** `ObjectError::Unreadable`, so the entry is a
//! listed row saying nobody could read it rather than a row saying it needs
//! nothing.
#![cfg(feature = "cli")]

use ginary::native::{self, ObjectError};

use crate::common::native::pe_bytes;
use crate::common::stubfile::PE_MACHINE_AMD64;

/// Where the PE32+ optional header's data directories begin.
///
/// `0x40` of DOS header, `4` of `PE\0\0`, `20` of COFF header, then the
/// optional header, whose sixteen `(rva, size)` pairs are its last `128`
/// bytes. Derived rather than written down: the helper that builds the file
/// asserts its own optional-header length, so the only number here is the
/// directory count.
const DATA_DIRECTORY_AT: usize = 0x40 + 4 + 20 + 240 - 16 * 8;

/// The data directory the import table is.
const IMPORT_DIRECTORY_INDEX: usize = 1;

/// A PE32+ whose import directory points where no section is mapped.
///
/// `0x9000` is past the `0x2000` the optional header gives as the size of the
/// image, so no section covers it and the reader has nowhere to look.
fn pe_with_an_unmapped_import_directory() -> Vec<u8> {
    let mut bytes = pe_bytes(PE_MACHINE_AMD64, true);
    let at = DATA_DIRECTORY_AT + IMPORT_DIRECTORY_INDEX * 8;
    assert_eq!(
        &bytes[at..at + 8],
        &[0u8; 8],
        "the import data directory of the fixture starts empty; the layout has moved"
    );
    bytes[at..at + 4].copy_from_slice(&0x9000u32.to_le_bytes());
    bytes[at + 4..at + 8].copy_from_slice(&0x100u32.to_le_bytes());
    bytes
}

#[test]
fn an_import_table_the_reader_cannot_follow_is_unreadable_and_not_empty() {
    let error = native::inspect_object_bytes(&pe_with_an_unmapped_import_directory())
        .err()
        .unwrap_or_else(|| {
            panic!(
                "a PE whose import directory falls outside every section is a file nobody could \
                 read; answering that it needs nothing reports the absence of dependencies where \
                 what happened was the absence of a reader for them"
            )
        });

    let ObjectError::Unreadable { message } = error else {
        panic!("the bytes do begin like an object, so this is not `NotAnObject`: {error:?}");
    };
    assert!(
        !message.is_empty() && !message.contains('\0'),
        "the reason reaches a terminal and a JSON document: {message:?}"
    );
}

#[test]
fn a_pe_whose_import_table_is_simply_absent_still_needs_nothing() {
    // The other side of the rule. An import data directory of `(0, 0)` is a
    // PE that imports nothing, which is a fact about the file and not a
    // failure to read one, or the change above would turn every fabricated
    // fixture in the suite into an unreadable object.
    let needs = native::inspect_object_bytes(&pe_bytes(PE_MACHINE_AMD64, true))
        .expect("a PE with no import directory is a readable object");

    assert!(needs.needed.is_empty(), "{:?}", needs.needed);
}
