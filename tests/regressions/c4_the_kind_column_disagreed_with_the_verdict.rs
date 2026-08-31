// SPDX-License-Identifier: MIT OR Apache-2.0
//! `doctor`'s `kind` column and its verdict columns described the same file
//! two different ways.
//!
//! The table's `kind` cell came from `doctor`'s own ELF walk, which mapped
//! `e_type` straight through and rendered `ElfKind::SharedObject` as
//! `shared object`. The verdict cells came from `native::verdicts_for_target`,
//! which reads `NativeKind` — and `NativeKind` separates the two `ET_DYN`
//! shapes by `DF_1_PIE`, because every program a modern toolchain links is an
//! `ET_DYN` too. So one row said `kind = shared object` and, in the next
//! column, that a runtime which cannot `dlopen` anything was perfectly happy
//! with it: the two halves of one table contradicting each other about the
//! fact that decides the refusal.
//!
//! The right behaviour: one answer per file. The `kind` cell is
//! [`NativeKind`], the same value the verdict is reached from, so a port
//! program reads `executable` in both halves and the NIF beside it reads
//! `shared object` in both.
#![cfg(feature = "cli")]

use std::time::SystemTime;

use ginary::doctor;
use ginary::native::Verdict;

use crate::common::native::{plant, shared_object};
use crate::common::project::TempProject;
use crate::common::repack::{EM_X86_64, test_binary};

/// The target whose catalogue runtime cannot load a NIF.
const TARGET: &str = "linux-x86_64-musl";

/// The program: this test binary, which `cargo` links `-pie`.
const PROGRAM: &str = "tooling/priv/bin/helper";

/// The library: an `ET_DYN` with neither an interpreter nor `DF_1_PIE`.
const LIBRARY: &str = "esqlite/priv/esqlite3_nif.so";

/// The rendered project block, and the report it came from.
fn report() -> doctor::ProjectReport {
    let project = TempProject::new(&format!(
        "name = \"notify\"\nversion = \"0.1.0\"\n\n\
         [tools.ginary]\ntargets = [\"{TARGET}\"]\n\n\
         [tools.ginary.target.{TARGET}]\nerts = \"catalog\"\n"
    ));
    let shipment = project.empty_shipment();
    plant(&shipment, PROGRAM, &test_binary());
    plant(&shipment, LIBRARY, &shared_object(EM_X86_64, None));
    doctor::project_context(project.root(), SystemTime::now()).expect("a project")
}

/// The one rendered line that begins with `path`.
fn row(text: &str, path: &str) -> String {
    text.lines()
        .find(|line| line.starts_with(path))
        .unwrap_or_else(|| panic!("no row for {path} in:\n{text}"))
        .to_owned()
}

#[test]
fn a_position_independent_program_prints_as_a_program() {
    let text = report().render();

    let row = row(&text, PROGRAM);
    assert!(
        row.contains("executable"),
        "a program is what the verdict beside it is reached from:\n{row}"
    );
    assert!(
        !row.contains("shared object"),
        "and it is not also a library:\n{row}"
    );
}

#[test]
fn the_shared_object_beside_it_still_prints_as_one() {
    // The other half, or the fix above would be "nothing is a library".
    let text = report().render();

    let row = row(&text, LIBRARY);
    assert!(
        row.contains("shared object"),
        "a NIF is still a NIF:\n{row}"
    );
}

#[test]
fn the_kind_and_the_verdict_agree_about_which_file_a_static_runtime_refuses() {
    let report = report();

    let verdicts: Vec<(&str, Verdict)> = report
        .native
        .iter()
        .map(|row| {
            (
                row.path.as_str(),
                *row.verdicts
                    .get(TARGET)
                    .unwrap_or_else(|| panic!("no verdict for {TARGET}: {:?}", row.verdicts)),
            )
        })
        .collect();

    assert!(
        verdicts.contains(&(LIBRARY, Verdict::StaticRuntime)),
        "the runtime cannot open the library: {verdicts:?}"
    );
    assert!(
        !verdicts
            .iter()
            .any(|(path, verdict)| *path == PROGRAM && *verdict == Verdict::StaticRuntime),
        "and it never has to open the program: {verdicts:?}"
    );
}
