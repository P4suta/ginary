// SPDX-License-Identifier: MIT OR Apache-2.0
//! A manifest's `launch.env` could append a second value for a variable the
//! launcher derives, and the second one wins.
//!
//! **What went wrong.** `launch::plan` applies `launch.env` after its own
//! `ROOTDIR`, `BINDIR`, `EMU`, `PROGNAME`, `HOME` and `ERL_CRASH_DUMP`, and
//! after `HEART_COMMAND`. It skipped a name the caller had set and a name the
//! scrub had just removed, and nothing else — so a manifest naming `ROOTDIR`
//! pushed a *second* `("ROOTDIR", ...)` pair onto the same list, and the
//! runtime was given the manifest's value rather than the entry it was
//! extracted into. `HEART_COMMAND` was worse: `launch.env` is applied before
//! the `heart` block, so the artifact's own restart command silently replaced
//! a value the project had deliberately set.
//!
//! `config::REJECTED_ENV_NAMES` refuses those names at build time, so this is
//! reachable only from a hand-written or older `ginary.json` — which is
//! exactly the input the launcher may not trust.
//!
//! **The input.** A manifest under `heart` whose `launch.env` names `ROOTDIR`,
//! `ERL_CRASH_DUMP` and `HEART_COMMAND`.
//!
//! **The correct behaviour.** A name the launcher has already decided is not
//! decided twice: each of the three keeps the launcher's own value, and each
//! appears in the plan exactly once.

use std::ffi::{OsStr, OsString};
use std::path::Path;

use ginary::cache::Env;
use ginary::launch::{self, CRASH_DUMP_NAME, HEART_COMMAND_VAR, LaunchPlan};

use crate::common::artifact::canonical_manifest;
use crate::common::hostpath::joined_for;

/// The entry the plan is built against.
const ROOT: &str = "/cache/hello/0123456789abcdef";

/// The application directory, where a crash dump goes.
const DUMPS: &str = "/cache/hello";

/// The running artifact, which is what `HEART_COMMAND` names.
const SELF_EXE: &str = "/opt/bin/hello";

/// What a manifest that tried to take the launcher's names over would say.
const HIJACK: &str = "/tmp/somewhere-else";

fn plan() -> LaunchPlan {
    let mut manifest = canonical_manifest();
    manifest.launch.heart = true;
    for name in ["ROOTDIR", "ERL_CRASH_DUMP", HEART_COMMAND_VAR] {
        manifest
            .launch
            .env
            .insert(name.to_owned(), HIJACK.to_owned());
    }
    // One name nothing else claims, so that a plan which ignored the whole
    // table would fail the last assertion rather than pass the first three.
    manifest
        .launch
        .env
        .insert("GINARY_OWN".to_owned(), "applied".to_owned());

    launch::plan(
        Path::new(ROOT),
        &manifest,
        &[OsString::from("--name"), OsString::from("world")],
        &Env::from_pairs(std::iter::empty()),
        Path::new(DUMPS),
        Path::new(SELF_EXE),
    )
    .expect("the manifest plans")
}

/// Every value the plan sets for `name`, in order.
fn values(plan: &LaunchPlan, name: &str) -> Vec<String> {
    plan.set
        .iter()
        .filter(|(key, _)| key == OsStr::new(name))
        .map(|(_, value)| value.to_string_lossy().into_owned())
        .collect()
}

#[test]
fn a_manifest_env_may_not_take_over_a_name_the_launcher_derives() {
    let plan = plan();

    assert_eq!(
        values(&plan, "ROOTDIR"),
        vec![ROOT.to_owned()],
        "the runtime's root is the entry it was extracted into, and a manifest does not get \
         a second say"
    );
    // The launcher joins the dump directory and the file name with
    // `Path::join`, so the separator between them is the host's: writing `/`
    // down asserted that this host spells one that way. `hostpath::joined_for`
    // is the rule; see `e11_a_listing_path_was_joined_the_way_the_host_spells_one`.
    assert_eq!(
        values(&plan, "ERL_CRASH_DUMP"),
        vec![joined_for(ginary::platform::HOST, DUMPS, CRASH_DUMP_NAME)],
        "the dump goes where the launcher put it"
    );
    assert_eq!(
        values(&plan, HEART_COMMAND_VAR),
        vec![format!("{SELF_EXE} --name world")],
        "only this artifact knows how to restart this application"
    );
    assert_eq!(
        values(&plan, "GINARY_OWN"),
        vec!["applied".to_owned()],
        "a name nothing else claims is still the manifest's to set, so the three above are \
         the rule rather than the table being ignored"
    );
}
