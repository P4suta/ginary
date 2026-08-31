// SPDX-License-Identifier: MIT OR Apache-2.0
//! A prune that took the entry while the launcher was on its way to the lock
//! left the launcher `execve`ing into a directory that no longer existed.
//!
//! **What went wrong.** The launcher extracts, preflights, prunes and only
//! then locks. Nothing between the preflight and the lock re-checked the
//! entry, and a concurrent pruner — another `ginary cache prune --all`, or a
//! sibling key of the same application launching while this entry was over
//! `GINARY_PRUNE_DAYS` old — holds the exclusive lock only across its rename.
//! A launcher that arrived after that rename found no lock file, recorded
//! "no lock", and then executed a program out of a tree `remove_dir_all` had
//! taken: exit 125, `ENOENT`, on an artifact that was perfectly good.
//!
//! **The input.** `GINARY_FAULT=before-lock:on`, which removes the entry at
//! exactly that point — what a pruner that won the race leaves behind.
//!
//! **The correct behaviour.** The lock is followed by one re-check of the
//! entry's `ginary.json`. An entry that has gone is extracted once more and
//! locked again, and the application starts.

#![cfg(feature = "fault-injection")]

use crate::common::artifact::{STUB_EXIT, SyntheticArtifact, read_trace};

#[test]
fn an_entry_removed_under_the_launcher_is_extracted_again_rather_than_executed() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let artifact = SyntheticArtifact::build(dir.path());
    let trace = dir.path().join("vanish.jsonl");

    let run = artifact
        .run()
        .env("GINARY_FAULT", "before-lock:on")
        .env("GINARY_TRACE", &trace)
        .output();

    assert_eq!(
        run.code(),
        STUB_EXIT,
        "an entry that vanished under the launcher is a cache to rebuild, not a failure to \
         report\n--- stderr ---\n{}",
        run.stderr_text()
    );
    assert!(
        artifact.key_dir().join("ginary.json").is_file(),
        "and the entry the application is running out of is on disk again"
    );

    let extracts = read_trace(&trace)
        .into_iter()
        .filter(|record| record.phase == "extract")
        .count();
    assert_eq!(
        extracts, 2,
        "exactly one re-extraction: the first is the one the fault removed"
    );
}
