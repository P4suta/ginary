// SPDX-License-Identifier: MIT OR Apache-2.0
//! The prune record escapes a path twice and the test that reads it back
//! escaped it once.
//!
//! **What went wrong.** E11 read the record right — a `prune` line is a JSON
//! document whose `removed_paths` value is *itself* a JSON document — wrote
//! that down in `hostpath::json_escaped`'s own documentation, and then
//! implemented one level of escaping:
//!
//! ```rust,ignore
//! /// so a Windows path inside it is escaped twice: the four characters
//! /// `\\\\` stand for the one separator a person typed.
//! pub fn json_escaped(text: &str) -> String {
//!     let rendered = serde_json::to_string(text)…   // one level
//! ```
//!
//! `cache::record_prune` renders the list with `launch::json_array` and hands
//! the result to `Diag::kv`, which renders the whole record; the path is
//! escaped once on the way into the array and once on the way into the record.
//! So the needle was `C:\\Users` and the haystack said `C:\\\\Users`:
//!
//! ```text
//! ---- b1_the_prune_trace_named_nothing_it_removed::the_prune_record_names_the_entry_that_vanished ----
//! the record must name the entry that vanished, and it is:
//! {"t_us":2609,"phase":"prune","kv":{"removed":"1","kept":"1",
//!  "removed_paths":"[\"C:\\\\Users\\\\RUNNER~1\\\\...\\\\hello\\\\0000000000000000\"]",
//!  "kept_paths":"[\"C:\\\\Users\\\\RUNNER~1\\\\...\\\\1111111111111111 (fresh)\"]"}}
//! ```
//!
//! (`Windows build and exit-code propagation`
//! <https://github.com/P4suta/ginary/actions/runs/33823103540/job/100869848230>,
//! `tests/regressions/b1_the_prune_trace_named_nothing_it_removed.rs:63`.)
//! Nothing is wrong with the record, and nothing was wrong with E11's reading
//! of it either — only with the number of times the rule was applied.
//!
//! **The input.** Any path holding a character JSON escapes. A Windows
//! separator is one; so is a backslash in a unix directory name, which is what
//! makes this reproducible here rather than only on a runner. E11's own
//! regression test asserted the single escaping against a *string*, which is a
//! true statement about a string and the wrong needle for this haystack, so it
//! passed on every host and the defect it was written for survived.
//!
//! **The correct behaviour.** A test that searches a *nested* rendered
//! document for a path escapes the path as many times as the document nests
//! it, and the rule is asserted against a record the production code really
//! wrote rather than against one the test composed.

#![cfg(feature = "cli")]

use ginary::cache::{self, PruneOptions};
use ginary::diag::Diag;

use crate::common::cachefs::{DAY, plant_entry};
use crate::common::hostpath::{json_escaped, nested_json_escaped};
use crate::common::payload::SharedSink;

/// An application directory whose own name holds the one character a unix
/// path can carry and JSON has to escape.
///
/// This is the whole instrument: on Windows every separator is such a
/// character, and on Linux none of them is unless a test puts one there. With
/// it, the runner's failure is reproduced on the machine ginary is developed
/// on.
const AWKWARD_APP: &str = r"back\slash";

/// How many JSON documents a path in a `prune` record is written into.
///
/// The array `ginary::launch::json_array` renders, and the record `Diag::kv`
/// renders around it.
const RECORD_DEPTH: usize = 2;

/// The stale entry, which pruning is for.
const OLD: &str = "0000000000000000";

/// The one beside it that nobody has finished with.
const FRESH: &str = "1111111111111111";

/// Prunes an application directory named [`AWKWARD_APP`] and returns the
/// `prune` record, with the removed and kept paths.
fn pruned_record() -> (String, std::path::PathBuf, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let app_dir = dir.path().join(AWKWARD_APP);
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
        .unwrap_or_else(|| panic!("no prune record in the trace:\n{trace}"))
        .to_owned();
    (record, old, fresh)
}

#[test]
fn the_needle_a_nested_record_is_searched_with_is_escaped_as_often_as_it_nests() {
    let (record, old, fresh) = pruned_record();

    for (which, path) in [("removed", &old), ("kept", &fresh)] {
        let needle = nested_json_escaped(&path.display().to_string(), RECORD_DEPTH);
        assert!(
            record.contains(&needle),
            "the {which} entry has to be findable in the record that names it, and the record \
             escapes it once per document it nests inside:\n{record}\nlooked for: {needle}"
        );
    }
}

#[test]
fn the_rule_is_the_one_the_two_renderers_really_apply() {
    // The instrument, held against the record the production renderers really
    // wrote rather than against a hand-written expectation: the array is
    // `cache::record_prune`'s `launch::json_array` and the record is
    // `Diag::kv`, and the needle has to survive both.
    let (record, old, _) = pruned_record();
    let path = old.display().to_string();

    assert!(
        record.contains(&nested_json_escaped(&path, RECORD_DEPTH)),
        "the rule has to name the path as the rendered record spells it:\n{record}"
    );
    assert!(
        !record.contains(&path),
        "and the raw spelling must not be in it, or this test proves nothing:\n{record}"
    );
    assert_eq!(
        nested_json_escaped(&path, 1),
        json_escaped(&path),
        "one document deep is the rule E11 wrote, unchanged: the depth is what the caller knows \
         and the escaping is the same escaping"
    );
}

#[test]
fn a_path_with_nothing_to_escape_is_its_own_needle() {
    for path in ["/tmp/ginary-1000/hello/0000000000000000", "/", "relative"] {
        assert_eq!(
            nested_json_escaped(path, RECORD_DEPTH),
            path,
            "escaping is JSON's and not the platform's, so it does nothing to a path that holds \
             none of its characters, however many times it is applied: {path}"
        );
    }
}
