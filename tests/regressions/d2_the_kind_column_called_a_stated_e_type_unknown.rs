// SPDX-License-Identifier: MIT OR Apache-2.0
//! `doctor`'s `kind` column printed `unknown` over an `e_type` the file
//! stated, and the field's own documentation sent the reader somewhere the
//! number is not reported either.
//!
//! [`kind_of_elf`] translated `ET_DYN` and `ET_EXEC` and folded every other
//! `e_type` into [`NativeKind::Unknown`]. That word means one thing in this
//! table — a file whose magic says it is an object and which will not parse —
//! and an `ET_REL` left under `priv` by a build system parses perfectly well.
//! A reader who saw it went looking for a corrupt file, and the doc comment on
//! `doctor::NativeObject::kind` pointed at `ginary inspect` for the raw
//! number, which reports no ELF header at all: `ginary elf deps` is the
//! command that reads one, and it prints the needed list rather than `e_type`.
//! So the number the header held was written down nowhere.
//!
//! The right behaviour: translate the one distinction that has to be
//! translated — `DF_1_PIE`, because `e_type` alone calls a position-
//! independent program a shared object, which is the C4 fix beside this one —
//! and otherwise say what the header said. `relocatable`, `core`, and
//! `e_type <n>` for a number no standard assigns.
//!
//! Carrying the number cost the JSON its shape, which is the second half of
//! this file. `NativeKind` derived `Serialize`, so the new variant turned
//! `ginary doctor --json`'s `native[].kind` from the string it had always been
//! into `{"elf_type": 65024}` — a field whose type depends on the file being
//! described, which is the one thing a machine-readable report may not do. The
//! column and the field say the same words now: `e_type <n>`.
//!
//! [`kind_of_elf`]: ginary::native::kind_of_elf
//! [`NativeKind::Unknown`]: ginary::native::NativeKind::Unknown
#![cfg(feature = "cli")]

use std::time::SystemTime;

use ginary::doctor;
use ginary::native::NativeKind;

use crate::common::native::{elf_bytes, plant};
use crate::common::project::TempProject;
use crate::common::repack::EM_X86_64;

/// `ET_REL`, an unlinked object file.
const ET_REL: u16 = 1;

/// `ET_CORE`, a core dump.
const ET_CORE: u16 = 4;

/// `ET_LOOS`, the first operating-system-specific type, which stands here for
/// every number this crate has no name for.
const ET_LOOS: u16 = 0xfe00;

/// The object file a build system left behind.
const RELOCATABLE: &str = "esqlite/priv/esqlite3_nif.o";

/// The core dump somebody's test run wrote into `priv`.
const CORE: &str = "esqlite/priv/core";

/// The file whose `e_type` names nothing this crate knows.
const OTHER: &str = "esqlite/priv/strange.bin";

/// A project whose shipment holds one file of each `e_type`.
fn report() -> doctor::ProjectReport {
    let project = TempProject::new(
        "name = \"notify\"\nversion = \"0.1.0\"\n\n\
         [tools.ginary]\ntargets = [\"linux-x86_64-gnu\"]\n",
    );
    let shipment = project.empty_shipment();
    for (path, e_type) in [(RELOCATABLE, ET_REL), (CORE, ET_CORE), (OTHER, ET_LOOS)] {
        plant(&shipment, path, &elf_bytes(EM_X86_64, e_type, None));
    }
    doctor::project_context(project.root(), SystemTime::now()).expect("a project")
}

/// The `kind` of the one scanned object whose path is `path`.
fn kind(report: &doctor::ProjectReport, path: &str) -> NativeKind {
    report
        .native
        .iter()
        .find(|object| object.path == path)
        .unwrap_or_else(|| panic!("{path} was not scanned: {:?}", report.native))
        .kind
}

#[test]
fn an_e_type_the_rule_does_not_translate_reaches_the_report_as_itself() {
    let report = report();

    assert_eq!(kind(&report, RELOCATABLE), NativeKind::Relocatable);
    assert_eq!(kind(&report, CORE), NativeKind::Core);
    assert_eq!(
        kind(&report, OTHER),
        NativeKind::ElfType(ET_LOOS),
        "a number no standard assigns is still the only fact there is about \
         the file, so it travels rather than being rounded off"
    );
}

#[test]
fn the_kind_column_prints_the_e_type_rather_than_unknown() {
    let text = report().render();

    for (path, expected) in [
        (RELOCATABLE, "relocatable"),
        (CORE, "core"),
        (OTHER, "e_type 65024"),
    ] {
        let row = text
            .lines()
            .find(|line| line.starts_with(path))
            .unwrap_or_else(|| panic!("no row for {path} in:\n{text}"));
        assert!(
            row.contains(expected),
            "the cell says what the header said:\n{row}"
        );
        assert!(
            !row.contains("unknown"),
            "and `unknown` is reserved for a file nobody could read:\n{row}"
        );
    }
}

#[test]
fn a_file_nobody_could_read_is_still_the_unknown_one() {
    // The other half: the fix above must not have turned every row into a
    // stated fact. `unknown` keeps the meaning it had, and the shipment scan
    // — which lists what `doctor`'s walk leaves out — is where it is said.
    let dir = tempfile::tempdir().expect("a temporary directory");
    let mut truncated = elf_bytes(EM_X86_64, ET_REL, None);
    truncated.truncate(24);
    let path = plant(dir.path(), RELOCATABLE, &truncated);

    let described = ginary::native::describe_object(&path)
        .expect("the file is there")
        .expect("its magic says it is an object");

    assert_eq!(described.kind, NativeKind::Unknown);
    assert!(
        described.unreadable.is_some(),
        "and the row says why nobody could read it"
    );
}

#[test]
fn every_kind_reaches_the_json_as_the_string_the_table_prints() {
    for (kind, expected) in [
        (NativeKind::SharedObject, "shared_object"),
        (NativeKind::Executable, "executable"),
        (NativeKind::Relocatable, "relocatable"),
        (NativeKind::Core, "core"),
        (NativeKind::ElfType(ET_LOOS), "e_type 65024"),
        (NativeKind::Unknown, "unknown"),
    ] {
        let value = serde_json::to_value(kind).expect("a kind serialises");
        assert_eq!(
            value,
            serde_json::Value::String(expected.to_owned()),
            "`native[].kind` is a string for every variant: a field whose type \
             depends on the file it describes is one no reader can parse"
        );
    }
}

#[test]
fn the_json_a_doctor_writes_says_the_e_type_rather_than_an_object() {
    let json = serde_json::to_value(report()).expect("the report serialises");
    let objects = json
        .get("native")
        .and_then(serde_json::Value::as_array)
        .expect("the report carries the native array");

    for object in objects {
        let kind = object.get("kind").expect("every row has a kind");
        assert!(
            kind.is_string(),
            "`ginary doctor --json` promised a string here and this row holds \
             {kind}"
        );
    }
    let strange = objects
        .iter()
        .find(|object| object.get("path").and_then(serde_json::Value::as_str) == Some(OTHER))
        .expect("the file whose e_type names nothing was scanned");
    assert_eq!(
        strange.get("kind").and_then(serde_json::Value::as_str),
        Some("e_type 65024"),
        "and it says what the header said, in the words the table prints"
    );
}
