// SPDX-License-Identifier: MIT OR Apache-2.0
//! The manifest's `app` was interpolated into filesystem paths unchecked.
//!
//! **What went wrong.** `LaunchSpec::validate` checked `launch.program`,
//! `launch.bindir`, `launch.boot` and every `launch.pa[i]`, but nothing
//! checked `Manifest::app` — which the launcher uses exactly the same way.
//! `CacheDirs::app_dir` is `root.join(app)`, and `ensure_extracted` then
//! creates that directory, chmods it 0700, extracts into a temporary tree
//! beside it and renames the entry into it. An artifact whose manifest said
//! `app: "../escape"` therefore had the launcher build its cache *outside* the
//! cache root; `app: "/etc"` made the application directory `/etc` and chmod'd
//! it 0700; `app: ""` made it the cache root itself.
//!
//! **The input.** A hand-assembled artifact whose manifest carries
//! `app: "../escape"`, run with a scrubbed environment and its own cache root.
//!
//! **The correct behaviour.** The application name is one path component or
//! the manifest is refused. The artifact exits 122 — the manifest is
//! well-formed bytes and the *format* is what is wrong, the same fault the
//! trailer's version byte reports — with one `ginary: ` line naming the field,
//! and it creates nothing at all.

use crate::common::artifact::{ArtifactOptions, SyntheticArtifact};

/// The name that walks out of the cache root.
const ESCAPING_APP: &str = "../escape";

#[test]
fn an_application_name_that_is_not_one_component_exits_122_and_creates_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = SyntheticArtifact::build_with(
        dir.path(),
        &ArtifactOptions {
            app: Some(ESCAPING_APP.to_owned()),
            ..ArtifactOptions::default()
        },
    );

    let run = artifact.run().output();

    assert_eq!(
        run.code(),
        122,
        "a manifest this build must not act on is a format failure\n--- stderr ---\n{}",
        run.stderr_text()
    );
    let lines: Vec<String> = run
        .stderr_text()
        .lines()
        .filter(|line| line.starts_with("ginary:"))
        .map(str::to_owned)
        .collect();
    assert_eq!(lines.len(), 1, "expected one diagnostic, got {lines:?}");
    assert!(
        lines[0].contains("app") && lines[0].contains(ESCAPING_APP),
        "the diagnostic must name the field and the value: `{}`",
        lines[0]
    );

    // Nothing at all: not the escaped directory beside the cache root, and not
    // the cache root's own contents.
    assert!(
        !artifact.cache_root().join(ESCAPING_APP).exists(),
        "the launcher created a directory outside its own cache root"
    );
    assert!(
        !artifact.cache_root().join("escape").exists(),
        "the launcher created the escaped directory"
    );
}
