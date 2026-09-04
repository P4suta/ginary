// SPDX-License-Identifier: MIT OR Apache-2.0
//! The application dependency closure: which applications an artifact needs,
//! where each one comes from, and what happens when one is missing.
//!
//! Every test but the last builds its two trees with [`FakeShipment`] and
//! [`FakeOtp`] in a temporary directory, so the whole file runs in
//! milliseconds with no Erlang installed. The last one is gated on the host
//! toolchain and runs the same computation over a real shipment.
//!
//! Two conventions hold throughout. Paths that reach a snapshot go through
//! `common::snapshot::scrub` first, because a `tempfile` directory name
//! changes on every run. And an assertion names a value, never a shape: the
//! closure's whole reason to exist is that its answer is exact and
//! reproducible.
// The command line half of the suite: every claim in this file is about a
// module the `cli` feature carries, so a `--no-default-features` build has
// nothing here to run. See `docs/dev/log/C2.md`.
#![cfg(feature = "cli")]

mod common;

use std::collections::BTreeMap;
use std::path::PathBuf;

use ginary::closure::{AppSet, AppSource, ClosureError, SeedKind, app_dependency_closure, explain};
use proptest::prelude::*;
use tempfile::TempDir;

use crate::common::fake_otp::{FakeOtp, FakeOtpRoot, FakeShipment, FakeShipmentRoot};
use crate::common::shipment::{SHIPMENT_VAR, ShipmentChoice, choose_shipment};
use crate::common::snapshot::scrub;
use crate::common::tools::{REQUIRE_VAR, require_tools};

/// A shipment and an OTP installation side by side in one temporary directory.
///
/// Both trees live under the same root so that one placeholder scrubs every
/// path a message can carry, and so that the directory is deleted when the
/// test ends.
struct Trees {
    dir: TempDir,
    shipment: FakeShipmentRoot,
    otp: FakeOtpRoot,
}

impl Trees {
    /// Writes both trees, the shipment at `<tmp>/shipment` and the OTP
    /// installation at `<tmp>/otp`.
    fn new(shipment: FakeShipment, otp: FakeOtp) -> Self {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let shipment = shipment.build_in(dir.path().join("shipment"));
        std::fs::create_dir_all(dir.path().join("otp")).expect("the otp root");
        let otp = otp.build_in(dir.path().join("otp"));
        Self { dir, shipment, otp }
    }

    /// The OTP `lib` directory the closure resolves against.
    fn lib(&self) -> PathBuf {
        self.otp.lib()
    }

    /// The closure over both trees.
    fn close(&self, roots: &[&str], extra: &[&str]) -> Result<AppSet, ClosureError> {
        app_dependency_closure(
            &self.shipment.root,
            &self.lib(),
            &owned(roots),
            &owned(extra),
        )
    }

    /// The closure, or a panic naming the error.
    fn closed(&self, roots: &[&str], extra: &[&str]) -> AppSet {
        match self.close(roots, extra) {
            Ok(set) => set,
            Err(error) => panic!("the closure of {roots:?} + {extra:?} should succeed: {error}"),
        }
    }

    /// The error, or a panic naming the closure that unexpectedly succeeded.
    fn failed(&self, roots: &[&str], extra: &[&str]) -> ClosureError {
        match self.close(roots, extra) {
            Ok(set) => panic!(
                "the closure of {roots:?} should fail, got {:?}",
                set.names()
            ),
            Err(error) => error,
        }
    }

    /// Replaces the temporary directory with `<tmp>` in `text`.
    fn scrub(&self, text: &str) -> String {
        scrub(text, &[(self.dir.path(), "<tmp>")])
    }
}

/// Copies a slice of borrowed names.
fn owned(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

/// The `requested_by` of one application, or a panic naming the closure.
fn requested_by(set: &AppSet, name: &str) -> Vec<String> {
    app(set, name).requested_by.clone()
}

/// One application, or a panic naming what the closure did hold.
fn app<'a>(set: &'a AppSet, name: &str) -> &'a ginary::closure::ResolvedApp {
    match set.get(name) {
        Some(app) => app,
        None => panic!("`{name}` is not in the closure; it holds {:?}", set.names()),
    }
}

/// The six-application scenario the `explain` snapshot and the CLI both use.
///
/// It covers all four [`SeedKind`]s and both sources: `notify` is the root,
/// `sasl` an extra, `kernel` and `stdlib` are always there, and `crypto` is
/// reached two edges deep through a shipment application.
fn six_app_scenario() -> Trees {
    Trees::new(
        FakeShipment::new()
            .app("notify", "1.0.0", &["gleam_crypto"])
            .app("gleam_crypto", "0.4.0", &["crypto"]),
        FakeOtp::new()
            .app("crypto", "5.9.2", &["kernel", "stdlib"])
            .app("sasl", "4.3.1", &["kernel", "stdlib"]),
    )
}

#[test]
fn kernel_and_stdlib_are_seeds_even_when_nothing_lists_them() {
    let trees = Trees::new(
        FakeShipment::new().app("solo", "1.0.0", &[]),
        FakeOtp::new(),
    );

    let set = trees.closed(&["solo"], &[]);

    assert_eq!(set.names(), ["kernel", "solo", "stdlib"]);
    assert_eq!(app(&set, "kernel").seed, SeedKind::Always);
    assert_eq!(app(&set, "stdlib").seed, SeedKind::Always);
    assert_eq!(app(&set, "solo").seed, SeedKind::Root);
}

#[test]
fn a_seed_records_no_requesters_even_when_another_application_lists_it() {
    let trees = Trees::new(
        FakeShipment::new().app("solo", "1.0.0", &[]),
        FakeOtp::new(),
    );

    let set = trees.closed(&["solo"], &[]);

    let stdlib = ginary::appfile::parse_app_file(
        &trees.otp.app_dir("stdlib").join("ebin").join("stdlib.app"),
    )
    .expect("the fake stdlib.app parses");
    assert!(
        stdlib.applications.contains(&"kernel".to_owned()),
        "the fixture must have `stdlib` list `kernel` for this test to mean anything"
    );
    assert_eq!(
        requested_by(&set, "kernel"),
        Vec::<String>::new(),
        "a seed is in the closure on its own account"
    );
}

#[test]
fn a_root_is_a_root_and_an_extra_is_an_extra() {
    let trees = Trees::new(
        FakeShipment::new().app("solo", "1.0.0", &[]),
        FakeOtp::new().app("sasl", "4.3.1", &["kernel"]),
    );

    let set = trees.closed(&["solo"], &["sasl"]);

    assert_eq!(app(&set, "solo").seed, SeedKind::Root);
    assert_eq!(app(&set, "sasl").seed, SeedKind::Extra);
    assert_eq!(requested_by(&set, "sasl"), Vec::<String>::new());
}

#[test]
fn a_name_that_is_both_a_root_and_an_extra_stays_a_root() {
    let trees = Trees::new(
        FakeShipment::new().app("solo", "1.0.0", &[]),
        FakeOtp::new(),
    );

    let set = trees.closed(&["solo"], &["solo"]);

    assert_eq!(app(&set, "solo").seed, SeedKind::Root);
}

#[test]
fn kernel_named_as_a_root_is_marked_root() {
    let trees = Trees::new(FakeShipment::new(), FakeOtp::new());

    let set = trees.closed(&["kernel"], &[]);

    assert_eq!(
        app(&set, "kernel").seed,
        SeedKind::Root,
        "an explicit root wins over the implicit seed"
    );
    assert_eq!(app(&set, "stdlib").seed, SeedKind::Always);
}

#[test]
fn applications_are_followed_transitively() {
    let trees = Trees::new(
        FakeShipment::new()
            .app("top", "1.0.0", &["middle"])
            .app("middle", "0.2.0", &["crypto"]),
        FakeOtp::new().app("crypto", "5.9.2", &["kernel", "stdlib"]),
    );

    let set = trees.closed(&["top"], &[]);

    assert_eq!(set.names(), ["crypto", "kernel", "middle", "stdlib", "top"]);
    assert_eq!(requested_by(&set, "middle"), ["top"]);
    assert_eq!(requested_by(&set, "crypto"), ["middle"]);
}

#[test]
fn included_applications_are_bundled_too() {
    let trees = Trees::new(
        FakeShipment::new()
            .app_with("host", "1.0.0", |app| app.included(&["helper"]))
            .app("helper", "0.1.0", &[]),
        FakeOtp::new(),
    );

    let set = trees.closed(&["host"], &[]);

    assert!(set.get("helper").is_some(), "{:?}", set.names());
    assert_eq!(requested_by(&set, "helper"), ["host"]);
}

#[test]
fn an_optional_application_that_resolves_is_bundled() {
    let trees = Trees::new(
        FakeShipment::new().app_with("app", "1.0.0", |app| app.optional(&["crypto"])),
        FakeOtp::new().app("crypto", "5.9.2", &["kernel", "stdlib"]),
    );

    let set = trees.closed(&["app"], &[]);

    assert!(set.get("crypto").is_some(), "{:?}", set.names());
    assert_eq!(requested_by(&set, "crypto"), ["app"]);
    assert_eq!(set.skipped_optional, Vec::<(String, String)>::new());
}

#[test]
fn an_optional_application_that_is_absent_is_skipped_and_is_not_an_error() {
    let trees = Trees::new(
        FakeShipment::new().app_with("app", "1.0.0", |app| app.optional(&["observer"])),
        FakeOtp::new(),
    );

    let set = trees.closed(&["app"], &[]);

    assert_eq!(set.names(), ["app", "kernel", "stdlib"]);
    assert_eq!(
        set.skipped_optional,
        vec![("observer".to_owned(), "app".to_owned())]
    );
    assert_eq!(set.warnings, Vec::<String>::new());
}

#[test]
fn a_required_application_that_is_absent_is_an_error() {
    let trees = Trees::new(
        FakeShipment::new().app("app", "1.0.0", &["observer"]),
        FakeOtp::new(),
    );

    assert!(
        matches!(trees.failed(&["app"], &[]), ClosureError::AppNotFound { name, .. } if name == "observer"),
        "an application is optional only when `optional_applications` says so"
    );
}

#[test]
fn permuting_the_roots_and_the_extras_does_not_change_the_app_set() {
    let trees = Trees::new(
        FakeShipment::new()
            .app("one", "1.0.0", &["shared"])
            .app("two", "2.0.0", &["shared"])
            .app("shared", "0.1.0", &["crypto"]),
        FakeOtp::new()
            .app("crypto", "5.9.2", &["kernel", "stdlib"])
            .app("sasl", "4.3.1", &["kernel"])
            .app("runtime_tools", "2.1.0", &["kernel"]),
    );

    let reference = trees.closed(&["one", "two"], &["sasl", "runtime_tools"]);

    for (roots, extra) in [
        (["two", "one"], ["sasl", "runtime_tools"]),
        (["one", "two"], ["runtime_tools", "sasl"]),
        (["two", "one"], ["runtime_tools", "sasl"]),
    ] {
        let other = trees.closed(&roots, &extra);
        assert_eq!(
            other, reference,
            "roots {roots:?} and extras {extra:?} produced a different closure"
        );
    }
}

#[test]
fn requested_by_is_sorted_and_deduplicated() {
    let trees = Trees::new(
        FakeShipment::new()
            .app_with("two", "2.0.0", |app| {
                app.applications(&["shared"]).included(&["shared"])
            })
            .app("one", "1.0.0", &["shared"])
            .app("shared", "0.1.0", &[]),
        FakeOtp::new(),
    );

    let set = trees.closed(&["two", "one"], &[]);

    assert_eq!(
        requested_by(&set, "shared"),
        ["one", "two"],
        "sorted by name, and `two` counted once even though it has two edges"
    );
}

#[test]
fn an_application_in_both_trees_comes_from_the_shipment() {
    let trees = Trees::new(
        FakeShipment::new()
            .app("app", "1.0.0", &["crypto"])
            .app("crypto", "9.9.9", &[]),
        FakeOtp::new().app("crypto", "5.9.2", &["kernel", "stdlib"]),
    );

    let set = trees.closed(&["app"], &[]);

    let crypto = app(&set, "crypto");
    assert_eq!(crypto.source, AppSource::Shipment);
    assert_eq!(crypto.vsn, "9.9.9");
    assert_eq!(crypto.ebin, trees.shipment.app_dir("crypto").join("ebin"));
    assert_eq!(set.warnings.len(), 1, "{:?}", set.warnings);
}

#[test]
fn the_shadowing_warning_names_both_directories() {
    let trees = Trees::new(
        FakeShipment::new()
            .app("app", "1.0.0", &["crypto"])
            .app("crypto", "9.9.9", &[]),
        FakeOtp::new().app("crypto", "5.9.2", &["kernel", "stdlib"]),
    );

    let set = trees.closed(&["app"], &[]);
    let rendered = trees.scrub(&set.warnings.join("\n"));

    insta::assert_snapshot!("shadowed_otp_application_warning", rendered);
}

#[test]
fn two_version_directories_for_one_otp_application_are_ambiguous() {
    let trees = Trees::new(
        FakeShipment::new().app("app", "1.0.0", &["crypto"]),
        FakeOtp::new().app("crypto", "5.9.2", &["kernel", "stdlib"]),
    );
    copy_app_dir(&trees, "crypto-5.9.2", "crypto-5.9.3");

    match trees.failed(&["app"], &[]) {
        ClosureError::AmbiguousOtpApp { name, candidates } => {
            assert_eq!(name, "crypto");
            assert_eq!(candidates, ["crypto-5.9.2", "crypto-5.9.3"]);
        }
        other => panic!("expected AmbiguousOtpApp, got {other:?}"),
    }
}

#[test]
fn only_a_digits_and_dots_suffix_makes_an_otp_directory_a_match() {
    let trees = Trees::new(
        FakeShipment::new().app("app", "1.0.0", &["crypto"]),
        FakeOtp::new().app("crypto", "5.9.2", &["kernel", "stdlib"]),
    );
    for decoy in ["crypto-doc", "crypto-5.9.2.bak", "crypto-latest", "crypto-"] {
        copy_app_dir(&trees, "crypto-5.9.2", decoy);
    }

    let set = trees.closed(&["app"], &[]);

    assert_eq!(
        app(&set, "crypto").source,
        AppSource::Otp {
            vsn: "5.9.2".to_owned()
        }
    );
    assert_eq!(
        app(&set, "crypto").ebin,
        trees.lib().join("crypto-5.9.2").join("ebin")
    );
}

#[test]
fn a_single_number_is_a_version() {
    let trees = Trees::new(
        FakeShipment::new().app("app", "1.0.0", &["odbc"]),
        FakeOtp::new().app("odbc", "3", &["kernel"]),
    );

    let set = trees.closed(&["app"], &[]);

    assert_eq!(
        app(&set, "odbc").source,
        AppSource::Otp {
            vsn: "3".to_owned()
        }
    );
}

#[test]
fn junk_that_is_not_a_directory_in_the_otp_library_is_ignored() {
    let trees = Trees::new(
        FakeShipment::new().app("app", "1.0.0", &["crypto"]),
        FakeOtp::new().app("crypto", "5.9.2", &["kernel", "stdlib"]),
    );
    std::fs::write(trees.lib().join("crypto-9.9.9"), b"not a directory").expect("the decoy file");
    std::fs::write(trees.lib().join("README"), b"junk").expect("the junk file");

    let set = trees.closed(&["app"], &[]);

    assert_eq!(
        app(&set, "crypto").source,
        AppSource::Otp {
            vsn: "5.9.2".to_owned()
        },
        "a regular file called `crypto-9.9.9` is neither a match nor an ambiguity"
    );
}

#[test]
fn the_ebin_and_priv_directories_are_the_ones_on_disk() {
    let trees = Trees::new(
        FakeShipment::new()
            .app_with("app", "1.0.0", |app| {
                app.applications(&["bare"]).priv_file("greeting.txt", b"hi")
            })
            .app("bare", "0.1.0", &[]),
        FakeOtp::new(),
    );

    let set = trees.closed(&["app"], &[]);

    let with_priv = app(&set, "app");
    assert_eq!(with_priv.ebin, trees.shipment.app_dir("app").join("ebin"));
    assert_eq!(
        with_priv.priv_dir,
        Some(trees.shipment.app_dir("app").join("priv"))
    );
    assert_eq!(
        app(&set, "bare").priv_dir,
        None,
        "an application with no `priv` directory records none"
    );
}

#[test]
fn a_missing_application_names_the_chain_from_the_seed() {
    let trees = Trees::new(
        FakeShipment::new()
            .app("app", "1.0.0", &["gleam_crypto"])
            .app("gleam_crypto", "0.4.0", &["crypto"]),
        FakeOtp::new(),
    );

    match trees.failed(&["app"], &[]) {
        ClosureError::AppNotFound {
            name,
            requested_by,
            searched,
        } => {
            assert_eq!(name, "crypto");
            assert_eq!(requested_by, ["app", "gleam_crypto", "crypto"]);
            assert_eq!(
                searched,
                [
                    trees
                        .shipment
                        .root
                        .join("crypto")
                        .join("ebin")
                        .join("crypto.app"),
                    trees.lib().join("crypto-<vsn>"),
                ]
            );
        }
        other => panic!("expected AppNotFound, got {other:?}"),
    }
}

#[test]
fn a_missing_root_is_reported_with_a_one_element_chain() {
    let trees = Trees::new(FakeShipment::new(), FakeOtp::new());

    match trees.failed(&["nope"], &[]) {
        ClosureError::AppNotFound {
            name,
            requested_by,
            searched,
        } => {
            assert_eq!(name, "nope");
            assert_eq!(requested_by, ["nope"]);
            assert_eq!(searched.len(), 2, "{searched:?}");
        }
        other => panic!("expected AppNotFound, got {other:?}"),
    }
    assert!(
        trees
            .failed(&["nope"], &[])
            .to_string()
            .contains("required by: nothing; it was asked for directly"),
        "a name nothing asked for says so, rather than naming itself as its own requester"
    );
}

#[test]
fn the_missing_application_message_ends_with_the_gleam_toml_hint() {
    let trees = Trees::new(
        FakeShipment::new()
            .app("app", "1.0.0", &["gleam_crypto"])
            .app("gleam_crypto", "0.4.0", &["crypto"]),
        FakeOtp::new(),
    );

    let rendered = trees.scrub(&trees.failed(&["app"], &[]).to_string());

    assert!(
        rendered.ends_with("check the dependency's .app file."),
        "the hint must be the last thing a reader sees:\n{rendered}"
    );
    insta::assert_snapshot!("app_not_found_message", rendered);
}

#[test]
fn a_cycle_between_two_applications_terminates() {
    let trees = Trees::new(
        FakeShipment::new()
            .app("a", "1.0.0", &["b"])
            .app("b", "1.0.0", &["a"]),
        FakeOtp::new(),
    );

    let set = trees.closed(&["a"], &[]);

    assert_eq!(set.names(), ["a", "b", "kernel", "stdlib"]);
    assert_eq!(requested_by(&set, "b"), ["a"]);
    assert_eq!(
        requested_by(&set, "a"),
        Vec::<String>::new(),
        "`a` is a root, so it records no requester even though `b` lists it"
    );
}

#[test]
fn an_application_that_lists_itself_terminates() {
    let trees = Trees::new(
        FakeShipment::new()
            .app("top", "1.0.0", &["loop"])
            .app("loop", "1.0.0", &["loop"]),
        FakeOtp::new(),
    );

    let set = trees.closed(&["top"], &[]);

    assert_eq!(set.names(), ["kernel", "loop", "stdlib", "top"]);
    assert_eq!(
        requested_by(&set, "loop"),
        ["top"],
        "an application is never its own requester"
    );
}

#[test]
fn a_malformed_app_file_in_a_dependency_names_the_path() {
    let trees = Trees::new(
        FakeShipment::new()
            .app("app", "1.0.0", &["gleam_crypto"])
            .app("gleam_crypto", "0.4.0", &[]),
        FakeOtp::new(),
    );
    let broken = trees.shipment.app_file("gleam_crypto");
    std::fs::write(&broken, b"{application, gleam_crypto,\n").expect("the broken file");

    match trees.failed(&["app"], &[]) {
        ClosureError::AppFile { path, .. } => assert_eq!(path, broken),
        other => panic!("expected AppFile, got {other:?}"),
    }
    assert!(
        trees
            .failed(&["app"], &[])
            .to_string()
            .contains(&broken.display().to_string()),
        "the message must name the file that could not be read"
    );
}

#[test]
fn explain_renders_name_version_source_and_origin() {
    let trees = six_app_scenario();

    let set = trees.closed(&["notify"], &["sasl"]);
    let rendered = explain(&set);

    assert_eq!(set.len(), 6, "{:?}", set.names());
    insta::assert_snapshot!("explain_table", rendered);
}

#[test]
fn chain_returns_one_shortest_path_from_a_seed() {
    let trees = Trees::new(
        FakeShipment::new()
            .app("app", "1.0.0", &["middle", "leaf"])
            .app("middle", "1.0.0", &["leaf"])
            .app("leaf", "1.0.0", &[]),
        FakeOtp::new(),
    );

    let set = trees.closed(&["app"], &[]);

    assert_eq!(
        requested_by(&set, "leaf"),
        ["app", "middle"],
        "the fixture must give `leaf` two requesters for `chain` to have a choice"
    );
    assert_eq!(set.chain("leaf"), ["app", "leaf"]);
}

#[test]
fn the_chain_of_a_seed_is_the_seed_alone() {
    let trees = Trees::new(FakeShipment::new().app("app", "1.0.0", &[]), FakeOtp::new());

    let set = trees.closed(&["app"], &[]);

    assert_eq!(set.chain("app"), ["app"]);
    assert_eq!(set.chain("kernel"), ["kernel"]);
}

#[test]
fn the_chain_of_a_name_that_is_not_in_the_closure_is_empty() {
    let trees = Trees::new(FakeShipment::new().app("app", "1.0.0", &[]), FakeOtp::new());

    let set = trees.closed(&["app"], &[]);

    assert_eq!(set.chain("observer"), Vec::<String>::new());
}

#[test]
fn otp_apps_and_shipment_apps_partition_the_closure() {
    let trees = six_app_scenario();

    let set = trees.closed(&["notify"], &["sasl"]);

    let otp: Vec<&str> = set.otp_apps().iter().map(|app| app.name.as_str()).collect();
    let shipment: Vec<&str> = set
        .shipment_apps()
        .iter()
        .map(|app| app.name.as_str())
        .collect();
    assert_eq!(otp, ["crypto", "kernel", "sasl", "stdlib"]);
    assert_eq!(shipment, ["gleam_crypto", "notify"]);
    assert_eq!(otp.len() + shipment.len(), set.len());

    let borrowed: Vec<&str> = (&set).into_iter().map(|app| app.name.as_str()).collect();
    assert_eq!(
        borrowed,
        set.names(),
        "iterating a borrowed set yields the same applications, in name order"
    );
}

#[test]
fn the_app_set_serialises_paths_as_strings_and_tags_the_source() {
    let trees = six_app_scenario();

    let set = trees.closed(&["notify"], &["sasl"]);
    let value = serde_json::to_value(&set).expect("the app set serialises");

    assert_eq!(value["apps"]["crypto"]["source"]["kind"], "otp");
    assert_eq!(value["apps"]["crypto"]["source"]["vsn"], "5.9.2");
    assert_eq!(value["apps"]["notify"]["source"]["kind"], "shipment");
    assert_eq!(value["apps"]["notify"]["seed"], "root");
    assert_eq!(value["apps"]["sasl"]["seed"], "extra");
    assert_eq!(value["apps"]["kernel"]["seed"], "always");
    assert_eq!(value["apps"]["gleam_crypto"]["seed"], "none");
    assert_eq!(
        value["apps"]["notify"]["ebin"],
        serde_json::Value::from(trees.shipment.app_dir("notify").join("ebin").to_str()),
        "a path is a string, not an array of components"
    );
    assert_eq!(value["apps"]["notify"]["priv_dir"], serde_json::Value::Null);
    assert!(value["warnings"].is_array(), "{value}");
    assert!(value["skipped_optional"].is_array(), "{value}");
}

/// Copies `<lib>/<from>` to `<lib>/<to>`, `.app` file and all.
///
/// The builders deliberately cannot write a broken tree, so a test that needs
/// two versions of one application builds a whole one and duplicates a
/// directory, exactly as `tests/otp.rs` does for a second `erts-*`.
fn copy_app_dir(trees: &Trees, from: &str, to: &str) {
    let source = trees.lib().join(from);
    let target = trees.lib().join(to);
    let ebin = target.join("ebin");
    std::fs::create_dir_all(&ebin).expect("the copied ebin");
    for entry in std::fs::read_dir(source.join("ebin")).expect("the source ebin") {
        let entry = entry.expect("a readable entry");
        std::fs::copy(entry.path(), ebin.join(entry.file_name())).expect("the copied file");
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    /// Adding a name to `extra` can add applications and can never remove one.
    #[test]
    fn the_closure_only_grows_when_extra_grows(dag in small_dag()) {
        let trees = dag.build();
        let without = trees.closed(&["a0"], &[]);
        let with = trees.closed(&["a0"], &[dag.extra_name()]);

        for name in without.names() {
            prop_assert!(
                with.get(&name).is_some(),
                "`{name}` was dropped when `{}` joined the extras: {:?} -> {:?}",
                dag.extra_name(),
                without.names(),
                with.names()
            );
        }
    }

    /// Feeding a closure its own names back as extras changes nothing.
    #[test]
    fn taking_the_closures_own_names_as_extras_changes_nothing(dag in small_dag()) {
        let trees = dag.build();
        let once = trees.closed(&["a0"], &[]);
        let names = once.names();
        let borrowed: Vec<&str> = names.iter().map(String::as_str).collect();
        let twice = trees.closed(&["a0"], &borrowed);

        prop_assert_eq!(twice.names(), once.names());
    }
}

/// A small directed acyclic graph of shipment applications.
///
/// Edges only ever point from a lower index to a higher one, so the graph is
/// acyclic by construction and `a0` reaches whatever is reachable at all.
/// Cycles have their own test; what this generator is for is the shape of the
/// closure over many graphs, not one interesting graph.
#[derive(Clone, Debug)]
struct SmallDag {
    size: usize,
    edges: BTreeMap<usize, Vec<usize>>,
}

impl SmallDag {
    /// The shipment and OTP trees this graph describes.
    fn build(&self) -> Trees {
        let mut shipment = FakeShipment::new();
        for index in 0..self.size {
            let names: Vec<String> = self
                .edges
                .get(&index)
                .into_iter()
                .flatten()
                .map(|target| format!("a{target}"))
                .collect();
            let borrowed: Vec<&str> = names.iter().map(String::as_str).collect();
            shipment = shipment.app(&format!("a{index}"), "1.0.0", &borrowed);
        }
        Trees::new(shipment, FakeOtp::new().app("sasl", "4.3.1", &["kernel"]))
    }

    /// The name the monotonicity test adds to `extra`.
    fn extra_name(&self) -> &'static str {
        "sasl"
    }
}

/// Two to five applications with a random subset of the forward edges.
fn small_dag() -> impl Strategy<Value = SmallDag> {
    (2usize..=5, prop::collection::vec(any::<bool>(), 25)).prop_map(|(size, bits)| {
        let mut edges: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for from in 0..size {
            for to in (from + 1)..size {
                if bits[from * 5 + to] {
                    edges.entry(from).or_default().push(to);
                }
            }
        }
        SmallDag { size, edges }
    })
}

/// The real shipment to close over, or a reported skip.
///
/// There is no default. A shipment is not a program on `PATH`: it is a
/// directory somebody produced with `gleam export erlang-shipment`, and a path
/// that exists on one machine is not a fallback for the others. So an unset
/// [`SHIPMENT_VAR`] is a loud skip however `GINARY_REQUIRE_TOOLCHAIN` is set,
/// and a value that is not a directory is a failure however it is set — the
/// caller asked for a run and mistyped the path. The rule itself lives in
/// [`crate::common::shipment`], where it can be asserted without a filesystem;
/// see `tests/regressions/e5_a_gated_test_defaulted_to_one_developers_machine.rs`.
fn real_shipment() -> Option<PathBuf> {
    let required = std::env::var_os(REQUIRE_VAR).is_some_and(|value| value == "1");
    match choose_shipment(
        std::env::var_os(SHIPMENT_VAR).as_deref(),
        required,
        &|path| path.is_dir(),
    ) {
        ShipmentChoice::Run(path) => Some(path),
        ShipmentChoice::Skip(reason) => {
            eprintln!("skipping: {reason}");
            None
        }
        ShipmentChoice::Fail(message) => panic!("{message}"),
    }
}

#[test]
fn the_real_notify_shipment_closes_over_the_host_otp() {
    let Some(_tools) = require_tools(&["erl"]) else {
        return;
    };
    let Some(shipment) = real_shipment() else {
        return;
    };
    let shipment = shipment.as_path();

    let otp = ginary::otp::discover(None).expect("the host OTP installation");
    let set = match app_dependency_closure(shipment, &otp.lib, &owned(&["notify"]), &[]) {
        Ok(set) => set,
        Err(error) => panic!("the real shipment should close: {error}"),
    };

    let crypto = app(&set, "crypto");
    match &crypto.source {
        AppSource::Otp { vsn } => assert!(
            otp.lib.join(format!("crypto-{vsn}")).is_dir(),
            "crypto-{vsn} is not in {}",
            otp.lib.display()
        ),
        AppSource::Shipment => panic!("`crypto` must come from OTP, not the shipment"),
    }

    for app in set.otp_apps() {
        assert!(
            app.ebin.is_dir(),
            "{} is not a directory",
            app.ebin.display()
        );
        assert!(
            app.ebin.starts_with(&otp.lib),
            "{} is not under {}",
            app.ebin.display(),
            otp.lib.display()
        );
    }
    for app in set.shipment_apps() {
        assert!(
            shipment.join(&app.name).is_dir(),
            "{}/{} is not a directory",
            shipment.display(),
            app.name
        );
    }

    eprintln!("{}", explain(&set));
    eprintln!("skipped optional applications: {:?}", set.skipped_optional);
}
