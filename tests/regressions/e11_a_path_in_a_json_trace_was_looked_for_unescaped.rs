// SPDX-License-Identifier: MIT OR Apache-2.0
//! A test looked for a raw path inside a JSON trace record and said the path
//! was missing when it was there, twice escaped.
//!
//! **What went wrong.** `cache::prune_app` records what it removed and what it
//! kept in a `GINARY_TRACE` line. The record is a JSON document whose
//! `removed_paths` value is itself a JSON document, so each separator of a
//! Windows path appears as four characters. The assertion looked for the one
//! character a person types:
//!
//! ```text
//! ---- the_prune_record_names_the_entry_that_vanished ----
//! the record must name the entry that vanished, and it is:
//! {"t_us":2122,"phase":"prune","kv":{"removed":"1","kept":"1",
//!  "removed_paths":"[\"C:\\\\Users\\\\RUNNER~1\\\\...\\\\hello\\\\0000000000000000\"]",
//!  "kept_paths":"[\"C:\\\\Users\\\\RUNNER~1\\\\...\\\\hello\\\\1111111111111111 (fresh)\"]"}}
//! ```
//!
//! (`Windows build and exit-code propagation`
//! <https://github.com/P4suta/ginary/actions/runs/33751715516/job/100636537290>,
//! `tests/regressions/b1_the_prune_trace_named_nothing_it_removed.rs:57`.)
//!
//! Nothing is wrong with the record: it names both entries, which is the whole
//! claim the test carries.
//!
//! **The input.** Any path holding a character JSON escapes — a backslash, a
//! quote, a control character. A unix path holds none of them, which is why
//! the assertion was green here and red there.
//!
//! **The correct behaviour.** A test that searches rendered JSON for a path
//! searches for the path *as JSON spells it*. The escaping is JSON's rather
//! than the platform's, so the rule runs on every host and is the identity on
//! one whose paths need no escaping.

use crate::common::hostpath::json_escaped;

#[test]
fn a_windows_path_inside_a_json_string_is_the_escaped_spelling() {
    assert_eq!(
        json_escaped(r"C:\Users\RUNNER~1\hello\0000000000000000"),
        r"C:\\Users\\RUNNER~1\\hello\\0000000000000000",
    );
}

#[test]
fn a_unix_path_is_its_own_escaping() {
    for path in ["/tmp/ginary-1000/hello/0000000000000000", "/", "relative"] {
        assert_eq!(json_escaped(path), path);
    }
}

#[test]
fn the_escaping_is_the_one_serde_json_produces_and_not_a_hand_rolled_replace() {
    // The doubly-nested record is what the trace really carries, so the rule
    // has to survive being applied to its own output.
    let path = r#"C:\a "quoted" \dir"#;
    let once = json_escaped(path);
    let rendered =
        serde_json::to_string(&serde_json::json!({ "removed_paths": [path] })).expect("json");
    assert!(
        rendered.contains(&once),
        "the escaped spelling has to be findable in the rendered record:\n{rendered}\n{once}"
    );
    assert!(
        !rendered.contains(path),
        "and the raw one must not be, or this test proves nothing:\n{rendered}"
    );
}
