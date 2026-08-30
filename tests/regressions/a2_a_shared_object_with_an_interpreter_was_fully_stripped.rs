// SPDX-License-Identifier: MIT OR Apache-2.0
//! A shared object that carries a program interpreter got `--strip-all`.
//!
//! **What went wrong.** The rule three documents state — a shared object is an
//! `ET_DYN` with no `PT_INTERP`, named `*.so` — was implemented without the
//! `ET_DYN` half: `ElfInfo` carried no object type at all, and the code read
//! `info.interp.is_none()` as if an absent interpreter were the same fact. It
//! is not, in either direction. A statically linked *executable* has no
//! interpreter either, and a real shared library may well have one: on a glibc
//! machine `ginary elf deps /lib/x86_64-linux-gnu/libc.so.6` prints
//! `interp /lib64/ld-linux-x86-64.so.2`. A NIF of that shape was therefore
//! classified as a program and stripped with `--strip-all`, which is exactly
//! the case the doc comment says "costs a loadable NIF its dynamic symbol
//! table".
//!
//! Nothing pinned the choice. `strip(1)` leaves no record of the arguments it
//! was given, and no test in the suite put a shared object into a staged tree,
//! so both constants could have been swapped without a single failure.
//!
//! **The input.** The host C library, which is an `ET_DYN` named `libc.so.6`
//! that carries a program interpreter, and the running test binary, which is a
//! program.
//!
//! **The correct behaviour.** The library gets `--strip-unneeded` and the
//! program gets `--strip-all`.

use std::path::{Path, PathBuf};

use ginary::elf::{self, ElfInfo};
use ginary::strip::{STRIP_ALL_ARGS, STRIP_UNNEEDED_ARGS, strip_arguments};

/// Where a glibc machine keeps its C library.
///
/// Named rather than searched: this test is about one concrete shape — an
/// `ET_DYN` that has a `PT_INTERP` — and a machine that has no file of that
/// shape reports a skip instead of quietly asserting nothing.
const LIBC_CANDIDATES: [&str; 4] = [
    "/lib/x86_64-linux-gnu/libc.so.6",
    "/lib/aarch64-linux-gnu/libc.so.6",
    "/lib64/libc.so.6",
    "/usr/lib/libc.so.6",
];

/// The first candidate that is a shared object carrying an interpreter.
fn host_libc() -> Option<(PathBuf, ElfInfo)> {
    LIBC_CANDIDATES.iter().find_map(|candidate| {
        let path = Path::new(candidate);
        let info = elf::inspect(path).ok()?;
        info.interp.as_ref()?;
        Some((path.to_path_buf(), info))
    })
}

#[test]
fn a_shared_object_that_has_an_interpreter_keeps_its_dynamic_symbols() {
    let Some((path, info)) = host_libc() else {
        eprintln!("skipping: no host libc.so.6 carrying a program interpreter");
        return;
    };

    assert_eq!(
        strip_arguments(&path, &info),
        STRIP_UNNEEDED_ARGS,
        "{} is a shared object; --strip-all would take its dynamic symbol table",
        path.display()
    );
}

#[test]
fn a_program_is_stripped_all_the_way() {
    let exe = std::env::current_exe().expect("the running test binary");
    let info = elf::inspect(&exe).expect("the test binary is an ELF file");

    assert_eq!(strip_arguments(&exe, &info), STRIP_ALL_ARGS);
}
