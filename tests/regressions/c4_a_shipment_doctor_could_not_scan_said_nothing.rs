// SPDX-License-Identifier: MIT OR Apache-2.0
//! A shipment `doctor` could not scan left every per-target column blank and
//! said nothing about why.
//!
//! `fill_verdicts` opened with
//!
//! ```text
//! let Ok(artifacts) = crate::native::scan_shipment(shipment) else {
//!     return;
//! };
//! ```
//!
//! so a `priv` directory the walk could not read — a mode the exporter left
//! behind, a stale mount — produced a table whose target columns were all `-`,
//! which is the same thing the table prints for "this object has no row in the
//! scan". The reader had a report that looked complete and was not, and
//! `CLAUDE.md` is explicit: "Do not silently skip a tar entry, a missing tool,
//! or a failed verification. Skipping is a reported decision or an error,
//! never a default."
//!
//! The right behaviour is the one every bound in `native.rs` already follows:
//! the depth bound and the size bound each produce a named warning so that
//! nothing vanishes. A scan that failed is a line beside the table naming the
//! path and what the system said.
#![cfg(feature = "cli")]
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt as _;
use std::time::SystemTime;

use ginary::doctor;

use crate::common::native::{host_machine, plant, shared_object};
use crate::common::project::TempProject;

/// The NIF the walk does find, so that there is a table at all.
const NIF: &str = "notify/priv/lib/nif.so";

#[test]
fn a_priv_directory_that_cannot_be_read_is_reported_beside_the_table() {
    let project = TempProject::new(concat!(
        "name = \"notify\"\n",
        "version = \"0.1.0\"\n\n",
        "[tools.ginary]\n",
        "targets = [\"host\"]\n",
    ));
    let shipment = project.empty_shipment();
    plant(&shipment, NIF, &shared_object(host_machine(), None));
    let locked = shipment.join("notify/priv/locked");
    std::fs::create_dir_all(&locked).expect("the directory that will be closed");
    // Mode 000: the walk cannot list it, which is what a scan reports rather
    // than passes over. Restored below, because a `TempDir` cannot delete what
    // it cannot search.
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000))
        .expect("the directory is closed");

    let report = doctor::project_context(project.root(), SystemTime::now());

    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755))
        .expect("the directory is reopened");
    let report = report.expect("a project");
    let text = report.render();
    assert!(
        text.contains(&locked.display().to_string()),
        "the directory nobody could read is named:\n{text}"
    );
    assert!(
        report
            .native
            .iter()
            .any(|row| row.path == NIF && row.verdicts.is_empty()),
        "the column stays blank, and now says why: {:?}",
        report.native
    );
}
