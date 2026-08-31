// SPDX-License-Identifier: MIT OR Apache-2.0
//! `doctor` printed `ok` for exactly the NIF a build refuses with the one
//! error no flag lifts.
//!
//! `doctor::fill_verdicts` derived a target's `nif_loading` from its
//! `otp_variant` alone:
//!
//! ```text
//! let nif_loading = config
//!     .and_then(|config| config.otp_variant.as_deref())
//!     .is_none_or(|variant| variant != STATIC_VARIANT);
//! ```
//!
//! `None` — a target with no sub-table, or one with `erts = "catalog"` and no
//! `otp_variant` — answered `true`. But the catalogue's *own* default for a
//! musl target is the static runtime, which publishes `nif_loading: false`, so
//! the ordinary cross-compiling setup
//!
//! ```toml
//! [tools.ginary.target.linux-x86_64-musl]
//! erts = "catalog"
//! ```
//!
//! had `doctor` answer `ok` for a NIF and the build stop with
//! `NativeError::StaticRuntime` — the refusal `--allow-native-mismatch`
//! deliberately does not lift, because a static emulator cannot `dlopen`
//! anything. `README.md` promises the opposite: "so the answer a build gives
//! with an error is one you can read before starting it".
//!
//! The right behaviour: apply the catalogue's own default rule. A named
//! `otp_variant` still decides, and a musl target reading its runtime out of
//! the catalogue is one whose default variant is `static`.
#![cfg(feature = "cli")]

use std::time::SystemTime;

use ginary::catalog::{self, Catalog};
use ginary::doctor;
use ginary::native::Verdict;

use crate::common::native::{plant, shared_object};
use crate::common::project::TempProject;
use crate::common::repack::EM_X86_64;

/// The target whose catalogue runtime cannot load a NIF.
const TARGET: &str = "linux-x86_64-musl";

/// Where the NIF is planted, and the row the verdict is read off.
const NIF: &str = "esqlite/priv/esqlite3_nif.so";

/// A project that names one musl target, with `otp_variant` spelled or not.
fn project(variant: Option<&str>) -> TempProject {
    let variant = variant.map_or_else(String::new, |name| format!("otp_variant = \"{name}\"\n"));
    let project = TempProject::new(&format!(
        "name = \"notify\"\nversion = \"0.1.0\"\n\n\
         [tools.ginary]\ntargets = [\"{TARGET}\"]\n\n\
         [tools.ginary.target.{TARGET}]\nerts = \"catalog\"\n{variant}"
    ));
    // A NIF for this very machine's architecture and with no interpreter,
    // which is what a musl object is: nothing but the runtime's own linkage
    // can be what decides the verdict below.
    plant(
        &project.empty_shipment(),
        NIF,
        &shared_object(EM_X86_64, None),
    );
    project
}

/// The verdict `doctor` reaches for [`TARGET`] over the planted NIF.
fn verdict(variant: Option<&str>) -> Verdict {
    let project = project(variant);
    let report = doctor::project_context(project.root(), SystemTime::now()).expect("a project");
    let row = report
        .native
        .iter()
        .find(|row| row.path == NIF)
        .unwrap_or_else(|| panic!("the planted NIF was not listed: {:?}", report.native));
    *row.verdicts
        .get(TARGET)
        .unwrap_or_else(|| panic!("no verdict for {TARGET}: {:?}", row.verdicts))
}

#[test]
fn a_catalog_musl_target_with_no_variant_named_says_the_runtime_loads_no_nif() {
    assert_eq!(
        verdict(None),
        Verdict::StaticRuntime,
        "the catalogue's default for a musl target is the static runtime, and \
         a build stops on this NIF with the one error no flag lifts"
    );
}

#[test]
fn a_target_that_names_a_dynamic_variant_is_still_ok() {
    // The other half of the same rule: a project that asks for the runtime
    // that *can* load a NIF has a NIF that is fine, or the fix above would be
    // "every musl target is refused".
    assert_eq!(
        verdict(Some("dynamic")),
        Verdict::Ok,
        "a dynamic runtime opens a NIF, whatever the catalogue's default is"
    );
}

#[test]
fn the_catalog_this_repository_ships_defaults_a_musl_target_to_a_static_runtime() {
    // What the rule above is derived from, held against the document itself:
    // if a future catalogue published a dynamic default for a musl target, the
    // verdict would be a guess about a runtime nobody selected.
    let text = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("dist/otp/catalog.json"),
    )
    .expect("the committed catalog");
    let parsed = Catalog::parse(&text, "dist/otp/catalog.json").expect("it parses");

    let mut seen = 0usize;
    for (version, entry) in &parsed.otp {
        // The rule `catalog::default_variant` states, spelled the same way.
        for name in entry.targets.keys().filter(|name| name.ends_with("-musl")) {
            seen = seen.saturating_add(1);
            let selected = parsed
                .lookup(version, name, None, "dist/otp/catalog.json")
                .expect("the default variant of a musl target");
            assert_eq!(
                selected.variant,
                catalog::DEFAULT_MUSL_VARIANT,
                "{name} of {version} defaults to something else"
            );
            assert!(
                !selected.entry.nif_loading,
                "{name} of {version} publishes a default variant that loads NIFs"
            );
        }
    }
    assert!(seen > 0, "the catalog holds no musl target to check");
}
