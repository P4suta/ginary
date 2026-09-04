// SPDX-License-Identifier: MIT OR Apache-2.0
//! The size and dependency account: `ginary::report`.
//!
//! Two halves, tested two ways. The rendering is pinned by an `insta` snapshot
//! over a *synthetic* report, because the sentence and the column alignment are
//! the contract and building them out of a real tree would make the snapshot a
//! recording of one machine's byte counts. The measurement is pinned against a
//! real staged tree, because that is where a category that is counted twice, a
//! file that is in the listing and not on the disk, or a total that does not
//! equal the sum of its parts actually shows up.
//!
//! The `needs:` line is the one output in the crate that a user acts on without
//! reading anything else: it is the artifact's portability floor. It is
//! asserted here in full, prefix included, rather than by substring.
// The command line half of the suite: every claim in this file is about a
// module the `cli` feature carries, so a `--no-default-features` build has
// nothing here to run. See `docs/dev/log/C2.md`.
#![cfg(feature = "cli")]

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use ginary::assemble::{self, Category, StageOptions, StagedRoot};
use ginary::closure::app_dependency_closure;
use ginary::report::{self, CategorySize, ElfDep, NeedsSummary, SizeReport};
use ginary::strip::{BeamOutcome, ElfOutcome, StripReport, StrippedFile};
use tempfile::TempDir;

use crate::common::fake_otp::{DUMMY_BEAM, FakeOtp, FakeShipment, beam_bytes};

/// A staged tree to measure, and the directory it lives in.
struct Staged {
    _dir: TempDir,
    root: StagedRoot,
}

impl Staged {
    /// Stages `notify` against a fake runtime into `<tmp>/out`.
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let shipment = FakeShipment::new()
            .app_with("notify", "1.0.0", |app| {
                app.applications(&["gleam_stdlib"])
                    .modules(&["notify", "notify@@main"])
                    .priv_file("greeting.txt", b"hello from priv\n")
            })
            .app_with("gleam_stdlib", "0.62.0", |app| {
                app.modules(&["gleam_stdlib", "gleam@list"])
            })
            .build_in(dir.path().join("shipment"));
        std::fs::create_dir_all(dir.path().join("otp")).expect("the otp root");
        let otp = FakeOtp::new().build_in(dir.path().join("otp"));
        let info = ginary::otp::inspect_root(&otp.root).expect("a usable OTP root");
        let set = app_dependency_closure(&shipment.root, &otp.lib(), &["notify".to_owned()], &[])
            .expect("the closure resolves");
        let root = assemble::stage(
            &set,
            &info,
            &StageOptions::default(),
            &dir.path().join("out"),
        )
        .expect("the scenario stages");
        Self { _dir: dir, root }
    }

    /// The staged tree on disk.
    fn path(&self) -> &Path {
        self.root.root()
    }
}

/// A strip report that says nothing was stripped, for a measurement that is
/// about the sizes rather than about the tools.
fn nothing_stripped() -> StripReport {
    StripReport {
        elf: ElfOutcome::NothingToStrip,
        beams: BeamOutcome::Stripped {
            files: 0,
            before: 0,
            after: 0,
        },
        per_file: Vec::new(),
        before_total: 0,
        after_total: 0,
        warnings: Vec::new(),
    }
}

/// The synthetic report the rendering snapshots are taken from.
///
/// Every number is made up and none of them comes from a machine, which is
/// what keeps the snapshot a contract about the layout rather than a recording
/// of one build.
fn synthetic() -> SizeReport {
    let categories = BTreeMap::from([
        (
            Category::ErtsBinary,
            CategorySize {
                files: 4,
                bytes_before: 41_675_352,
                bytes_after: 11_742_936,
            },
        ),
        (
            Category::Boot,
            CategorySize {
                files: 1,
                bytes_before: 12_345,
                bytes_after: 12_345,
            },
        ),
        (
            Category::OtpBeam,
            CategorySize {
                files: 280,
                bytes_before: 8_000_000,
                bytes_after: 2_500_000,
            },
        ),
        (
            Category::GleamBeam,
            CategorySize {
                files: 32,
                bytes_before: 1_382_144,
                bytes_after: 511_072,
            },
        ),
        (
            Category::Priv,
            CategorySize {
                files: 3,
                bytes_before: 4_096,
                bytes_after: 4_096,
            },
        ),
        (
            Category::AppResource,
            CategorySize {
                files: 40,
                bytes_before: 51_200,
                bytes_after: 51_200,
            },
        ),
    ]);

    SizeReport {
        categories,
        total_before: 51_125_137,
        total_after: 14_821_649,
        elf_deps: vec![ElfDep {
            path: "erts-17.0.5/bin/beam.smp".to_owned(),
            needed: vec![
                "libtinfo.so.6".to_owned(),
                "libstdc++.so.6".to_owned(),
                "libm.so.6".to_owned(),
                "libgcc_s.so.1".to_owned(),
                "libc.so.6".to_owned(),
            ],
            glibc_max: Some("2.38".to_owned()),
            interp: Some("/lib64/ld-linux-x86-64.so.2".to_owned()),
            machine: "x86_64".to_owned(),
        }],
        needs_summary: NeedsSummary {
            needed: BTreeSet::from([
                "libc.so.6".to_owned(),
                "libgcc_s.so.1".to_owned(),
                "libm.so.6".to_owned(),
                "libstdc++.so.6".to_owned(),
                "libtinfo.so.6".to_owned(),
            ]),
            glibc_max: Some("2.38".to_owned()),
        },
        warnings: vec![
            "lib/notify/priv/ghost.txt is in the listing and not in the tree".to_owned(),
        ],
    }
}

#[test]
fn the_report_renders_a_table_the_needs_line_and_the_warnings() {
    insta::assert_snapshot!("size_report_text", synthetic().render_text());
}

#[test]
fn the_needs_line_puts_the_glibc_floor_after_libc_and_sorts_the_rest() {
    // The floor belongs to `libc.so.6` and to no other entry: it is the version
    // of *that* library the artifact will not start without.
    assert_eq!(
        synthetic().needs_line(),
        "needs: libc.so.6 (GLIBC_2.38), libgcc_s.so.1, libm.so.6, libstdc++.so.6, libtinfo.so.6"
    );
}

#[test]
fn a_report_with_no_native_code_says_it_needs_nothing() {
    // An artifact with no ELF in it — every one built on a machine ginary does
    // not strip for — still has to answer the question, and the answer is not
    // a missing line.
    let report = SizeReport::default();

    assert_eq!(report.needs_line(), "needs: (none)");
}

#[test]
fn a_needs_line_without_a_glibc_floor_names_the_libraries_alone() {
    let mut report = SizeReport::default();
    report.needs_summary.needed = BTreeSet::from(["libSystem.B.dylib".to_owned()]);

    assert_eq!(report.needs_line(), "needs: libSystem.B.dylib");
}

#[test]
fn the_report_serialises_with_the_keys_the_json_form_documents() {
    let value = serde_json::to_value(synthetic()).expect("the report serialises");

    assert_eq!(value["total_before"], serde_json::json!(51_125_137u64));
    assert_eq!(value["total_after"], serde_json::json!(14_821_649u64));
    assert_eq!(
        value["categories"]["erts_binary"],
        serde_json::json!({"files": 4, "bytes_before": 41_675_352u64, "bytes_after": 11_742_936u64})
    );
    assert_eq!(
        value["needs_summary"]["glibc_max"],
        serde_json::json!("2.38")
    );
    assert_eq!(
        value["elf_deps"][0]["path"],
        serde_json::json!("erts-17.0.5/bin/beam.smp")
    );
    assert_eq!(value["elf_deps"][0]["machine"], serde_json::json!("x86_64"));
    assert_eq!(
        value["warnings"][0],
        serde_json::json!("lib/notify/priv/ghost.txt is in the listing and not in the tree")
    );
}

#[test]
fn measuring_an_unchanged_tree_reports_the_same_bytes_before_and_after() {
    let staged = Staged::new();

    let report =
        report::measure(&staged.root, &nothing_stripped(), staged.path()).expect("the tree reads");

    assert_eq!(report.total_before, staged.root.total_bytes());
    assert_eq!(report.total_after, staged.root.total_bytes());
    assert_eq!(report.saved(), 0);
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
}

#[test]
fn the_categories_agree_with_the_staged_listing_and_sum_to_the_total() {
    let staged = Staged::new();

    let report =
        report::measure(&staged.root, &nothing_stripped(), staged.path()).expect("the tree reads");

    let expected = staged.root.bytes_by_category();
    assert_eq!(
        report.categories.keys().copied().collect::<Vec<_>>(),
        expected.keys().copied().collect::<Vec<_>>(),
        "every category the tree holds is a row, and no other is"
    );
    for (category, size) in &report.categories {
        let (bytes, files) = expected[category];
        assert_eq!(size.files, files, "{category} file count");
        assert_eq!(size.bytes_before, bytes, "{category} bytes before");
    }
    assert_eq!(
        report
            .categories
            .values()
            .map(|size| size.bytes_before)
            .sum::<u64>(),
        report.total_before
    );
    assert_eq!(
        report
            .categories
            .values()
            .map(|size| size.bytes_after)
            .sum::<u64>(),
        report.total_after
    );
}

#[test]
fn a_file_that_shrank_is_measured_from_the_tree_and_not_from_the_listing() {
    // The report's whole reason to exist: `before` comes from what staging
    // wrote down and `after` from what is on the disk now. A report that read
    // one of them twice would show a saving of zero after every build.
    let staged = Staged::new();
    let victim = "lib/notify/ebin/notify.beam";
    let shrunk = beam_bytes(&[(*b"Code", b"x".as_slice())]);
    std::fs::write(staged.path().join(victim), &shrunk).expect("a shrunken module");

    let report =
        report::measure(&staged.root, &nothing_stripped(), staged.path()).expect("the tree reads");

    let saved = staged.root.total_bytes() - report.total_after;
    assert_eq!(
        saved,
        DUMMY_BEAM.len() as u64 - shrunk.len() as u64,
        "the only file that changed is the only saving"
    );
    let gleam = report
        .categories
        .get(&Category::GleamBeam)
        .expect("the tree holds Gleam modules");
    assert!(
        gleam.bytes_after < gleam.bytes_before,
        "the category the file belongs to is the one that shrank"
    );
}

#[test]
fn a_file_in_the_listing_that_is_not_in_the_tree_is_a_warning_and_not_a_failure() {
    // Nothing removes a staged file today. When something does, the report has
    // to keep printing and name what it could not measure, because a report
    // that refuses over one odd file is worse than one that says which.
    let staged = Staged::new();
    let missing = "lib/notify/priv/greeting.txt";
    std::fs::remove_file(staged.path().join(missing)).expect("a file to remove");

    let report =
        report::measure(&staged.root, &nothing_stripped(), staged.path()).expect("the tree reads");

    assert!(
        report
            .warnings
            .iter()
            .any(|warning| warning.contains(missing)),
        "the missing file has to be named: {:?}",
        report.warnings
    );
}

// gnu Linux, not Linux. The test binary this stages as a runtime is asserted
// to need `libc.so.6` and to carry a glibc floor, and both are glibc's own: a
// static musl build of the same binary needs nothing and has no symbol
// versions, so the assertions would fail against a healthy host. The rule is
// held over the tree by
// `tests/regressions/e16_a_glibc_only_assertion_ran_under_a_linux_gate.rs`.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
#[test]
fn every_elf_file_in_the_tree_is_listed_with_what_it_needs() {
    let staged = Staged::new();
    // The running test binary is the only real ELF a toolchain-free test can
    // reach. Put where `beam.smp` goes, it is what an artifact's runtime looks
    // like to this module.
    let exe = std::env::current_exe().expect("the running test binary");
    let native = staged.path().join("erts-17.0.5/bin/beam.smp");
    std::fs::copy(&exe, &native).expect("a real ELF in the staged tree");

    let report =
        report::measure(&staged.root, &nothing_stripped(), staged.path()).expect("the tree reads");

    let dep = report
        .elf_deps
        .iter()
        .find(|dep| dep.path == "erts-17.0.5/bin/beam.smp")
        .expect("the native binary is listed");
    assert!(dep.needed.iter().any(|name| name == "libc.so.6"));
    assert_eq!(dep.machine, std::env::consts::ARCH);
    assert!(dep.glibc_max.is_some());
    assert!(dep.interp.is_some());
    assert!(
        report.needs_summary.needed.contains("libc.so.6"),
        "the summary is the union of the files: {:?}",
        report.needs_summary
    );
    assert_eq!(report.needs_summary.glibc_max, dep.glibc_max);
}

#[cfg(target_os = "linux")]
#[test]
fn the_elf_entries_are_sorted_by_path() {
    let staged = Staged::new();
    let exe = std::env::current_exe().expect("the running test binary");
    for name in ["erts-17.0.5/bin/beam.smp", "erts-17.0.5/bin/erlexec"] {
        std::fs::copy(&exe, staged.path().join(name)).expect("a real ELF in the staged tree");
    }

    let report =
        report::measure(&staged.root, &nothing_stripped(), staged.path()).expect("the tree reads");

    let paths: Vec<&str> = report
        .elf_deps
        .iter()
        .map(|dep| dep.path.as_str())
        .collect();
    let mut sorted = paths.clone();
    sorted.sort_unstable();
    assert_eq!(paths, sorted);
    assert_eq!(paths.len(), 2, "both copies are listed: {paths:?}");
}

#[test]
fn a_tree_with_no_native_code_lists_no_dependencies_at_all() {
    let staged = Staged::new();

    let report =
        report::measure(&staged.root, &nothing_stripped(), staged.path()).expect("the tree reads");

    assert_eq!(report.elf_deps, Vec::<ElfDep>::new());
    assert_eq!(report.needs_summary, NeedsSummary::default());
}

#[test]
fn the_strip_reports_per_file_rows_do_not_change_what_the_tree_says() {
    // The strip report is an input to the account, not the account itself:
    // whatever it claims, the sizes come from the files.
    let staged = Staged::new();
    let lying = StripReport {
        elf: ElfOutcome::NothingToStrip,
        beams: BeamOutcome::Stripped {
            files: 1,
            before: 999_999,
            after: 1,
        },
        per_file: vec![StrippedFile {
            path: "lib/notify/ebin/notify.beam".to_owned(),
            before: 999_999,
            after: 1,
        }],
        before_total: 999_999,
        after_total: 1,
        warnings: Vec::new(),
    };

    let report = report::measure(&staged.root, &lying, staged.path()).expect("the tree reads");

    assert_eq!(report.total_before, staged.root.total_bytes());
    assert_eq!(report.total_after, staged.root.total_bytes());
}
