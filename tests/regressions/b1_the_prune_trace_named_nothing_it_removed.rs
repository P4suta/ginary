// SPDX-License-Identifier: MIT OR Apache-2.0
//! The trace recorded how many entries a prune removed and never which ones.
//!
//! **What went wrong.** `cache::prune_app` recorded `removed=<n> kept=<m>` and
//! stopped there, while `launcher::prune_siblings` documented the opposite —
//! "what was pruned goes to the trace, because an entry that vanished is a
//! thing a bug report has to be able to explain". A user whose cache had lost
//! a directory could read from the trace that *something* had gone and never
//! what, which is precisely the question a bug report asks. `launch::record`
//! already had the shape for it: a JSON array of strings under one key.
//!
//! **The input.** An application directory holding one stale entry and one
//! fresh one, pruned with a trace sink attached.
//!
//! **The correct behaviour.** The `prune` record carries the path of every
//! entry it removed and of every entry it kept, so the trace answers "where
//! did that directory go" on its own.

use ginary::cache::{self, PruneOptions};
use ginary::diag::Diag;

use crate::common::cachefs::{DAY, plant_entry};
use crate::common::payload::SharedSink;

/// The stale entry, which pruning is for.
const OLD: &str = "0000000000000000";

/// The one beside it that nobody has finished with.
const FRESH: &str = "1111111111111111";

#[test]
fn the_prune_record_names_the_entry_that_vanished() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let app_dir = dir.path().join("hello");
    let old = plant_entry(&app_dir, OLD, DAY * 30);
    let fresh = plant_entry(&app_dir, FRESH, DAY);
    let sink = SharedSink::new();
    let diag = Diag::with_sinks(None, Some(Box::new(sink.clone())));

    let report = cache::prune_app(
        &app_dir,
        None,
        PruneOptions {
            days: 14,
            all: false,
        },
        std::time::SystemTime::now(),
        &diag,
    );

    assert_eq!(report.removed, vec![old.clone()], "the stale one goes");
    let trace = sink.text();
    let record = trace
        .lines()
        .find(|line| line.contains("\"phase\":\"prune\""))
        .unwrap_or_else(|| panic!("no prune record in the trace:\n{trace}"));
    assert!(
        record.contains(&old.display().to_string()),
        "the record must name the entry that vanished, and it is:\n{record}"
    );
    assert!(
        record.contains(&fresh.display().to_string()),
        "and the one it left, so that a trace explains a cache rather than counting it:\n{record}"
    );
}
