// SPDX-License-Identifier: MIT OR Apache-2.0
//! Cache maintenance deleted crash evidence, unrelated files and live work.
//!
//! `clean` removed whole application directories, and `uninstall` treated any
//! directory holding `ginary.json` as owned cache data. Neither protected a
//! temporary extraction by its live owner. These fixtures distinguish owned,
//! inactive runtime data from evidence and work that a concurrent user needs.

use std::path::{Path, PathBuf};

use ginary::cache;

const KEY: &str = "0123456789abcdef";
const DEAD_PID: u32 = 4_000_000_000;

#[test]
fn f1_sweep_preserves_unowned_names_and_current_process_work() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app = dir.path().join("hello");
    let foreign = entry(&app, &format!(".notes.tmp-{DEAD_PID}"));
    let live = entry(&app, &format!(".{KEY}.tmp-{}", std::process::id()));
    let dead = app.join(format!(".{KEY}.corrupt-{DEAD_PID}"));
    std::fs::create_dir(&dead).expect("unfinished owned residue");
    std::fs::write(dead.join("partial-data"), b"interrupted extraction").expect("partial file");

    let report =
        cache::sweep(&app, std::process::id(), &ginary::diag::Diag::disabled()).expect("sweep");

    assert!(
        foreign.join("ginary.json").is_file(),
        "user directory was removed"
    );
    assert!(
        live.join("ginary.json").is_file(),
        "concurrent library extraction was removed"
    );
    assert_eq!(report.removed, [dead]);
    assert!(report.kept.contains(&live));
}

#[test]
fn f1_sweep_requires_valid_ownership_and_respects_a_residue_lock() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app = dir.path().join("hello");
    let invalid = entry(&app, &format!(".{KEY}.tmp-{DEAD_PID}"));
    std::fs::write(invalid.join("ginary.json"), b"{}").expect("foreign marker");
    let locked = entry(&app, &format!(".{KEY}.corrupt-{DEAD_PID}"));
    let lock = ginary::cache_lock::SharedLock::acquire(&locked).expect("hold residue lock");

    let report =
        cache::sweep(&app, std::process::id(), &ginary::diag::Diag::disabled()).expect("sweep");
    assert!(report.removed.is_empty(), "{report:?}");
    assert!(
        invalid.is_dir() && locked.is_dir(),
        "unowned or locked residue was removed"
    );
    assert!(report.kept.contains(&invalid) && report.kept.contains(&locked));
    drop(lock);
    let report = cache::sweep(&app, std::process::id(), &ginary::diag::Diag::disabled())
        .expect("sweep after release");
    assert_eq!(report.removed, [locked]);
    assert!(invalid.is_dir());
}

#[cfg(unix)]
#[test]
fn f1_sweep_does_not_follow_application_or_residue_symlinks() {
    let dir = tempfile::tempdir().expect("tempdir");
    let outside = dir.path().join("outside");
    let owned = entry(&outside, &format!(".{KEY}.tmp-{DEAD_PID}"));
    let app = dir.path().join("hello");
    std::os::unix::fs::symlink(&outside, &app).expect("application symlink");
    let report = cache::sweep(&app, std::process::id(), &ginary::diag::Diag::disabled())
        .expect("sweep symlink");
    assert!(report.removed.is_empty());
    assert!(owned.join("ginary.json").is_file());
    std::fs::remove_file(&app).expect("remove test symlink");
    std::fs::create_dir(&app).expect("actual application directory");
    let link = app.join(format!(".{KEY}.tmp-{DEAD_PID}"));
    std::os::unix::fs::symlink(&owned, &link).expect("residue symlink");
    let report = cache::sweep(&app, std::process::id(), &ginary::diag::Diag::disabled())
        .expect("sweep residue symlink");
    assert!(report.removed.is_empty());
    assert!(
        std::fs::symlink_metadata(&link)
            .expect("link survives")
            .file_type()
            .is_symlink()
    );
    assert!(owned.join("ginary.json").is_file());
}

#[test]
fn f1_prune_preserves_foreign_directories_even_with_a_completion_marker() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app = dir.path().join("hello");
    let foreign = entry(&app, "notes");
    let invalid = entry(&app, KEY);
    std::fs::write(invalid.join("ginary.json"), b"{}").expect("invalid marker");
    let report = cache::prune_app(
        &app,
        None,
        cache::PruneOptions { all: true, days: 0 },
        std::time::SystemTime::now(),
        &ginary::diag::Diag::disabled(),
    );
    assert!(
        foreign.is_dir() && invalid.is_dir(),
        "prune removed unrelated directories: {report:?}"
    );
    assert_eq!(report.kept.len(), 2);
}

#[test]
fn f1_clean_preserves_a_key_shaped_directory_with_an_invalid_or_foreign_manifest() {
    for foreign_app in [false, true] {
        let dir = tempfile::tempdir().expect("tempdir");
        let app = dir.path().join("hello");
        let path = entry(&app, KEY);
        std::fs::write(path.join("ginary.json"), b"{}").expect("invalid marker");
        if foreign_app {
            let mut manifest = crate::common::artifact::canonical_manifest();
            manifest.app = "different_application".into();
            std::fs::write(
                path.join("ginary.json"),
                serde_json::to_vec(&manifest).expect("manifest JSON"),
            )
            .expect("manifest");
        }
        std::fs::write(path.join("important.txt"), b"user data").expect("user file");
        let report = cache::clean_detailed(dir.path(), Some("hello")).expect("clean");
        assert!(
            path.join("important.txt").is_file(),
            "key shape alone established ownership"
        );
        assert!(
            report
                .kept
                .iter()
                .any(|(kept, reason)| kept == &path && *reason == cache::KeptReason::Unowned)
        );
    }
}

fn entry(app: &Path, name: &str) -> PathBuf {
    let path = app.join(name);
    std::fs::create_dir_all(&path).expect("create entry");
    let mut manifest = crate::common::artifact::canonical_manifest();
    manifest.app = app
        .file_name()
        .expect("application name")
        .to_string_lossy()
        .into_owned();
    std::fs::write(
        path.join("ginary.json"),
        serde_json::to_vec(&manifest).expect("manifest JSON"),
    )
    .expect("write marker");
    path
}

#[test]
fn f1_clean_preserves_crash_evidence_and_unowned_files() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app = dir.path().join("hello");
    let owned = entry(&app, KEY);
    let foreign = entry(&app, "my-notes");
    let dump = app.join("erl_crash.dump");
    std::fs::write(&dump, b"evidence").expect("write dump");

    cache::clean(dir.path(), Some("hello")).expect("clean");

    assert!(!owned.exists(), "unused owned data should be removed");
    assert_eq!(
        std::fs::read(&dump).ok().as_deref(),
        Some(b"evidence".as_slice())
    );
    assert!(
        foreign.join("ginary.json").is_file(),
        "a marker alone proves no ownership"
    );
}

#[test]
fn f1_uninstall_preserves_unowned_directories_with_a_manifest() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app = dir.path().join("hello");
    let owned = entry(&app, KEY);
    let foreign = entry(&app, "my-notes");

    cache::uninstall(&app);

    assert!(!owned.exists());
    assert!(
        foreign.join("ginary.json").is_file(),
        "unrelated data must survive uninstall"
    );
}

#[test]
fn f1_maintenance_preserves_live_residue_even_when_its_manifest_is_complete() {
    for clean in [false, true] {
        let dir = tempfile::tempdir().expect("tempdir");
        let app = dir.path().join("hello");
        let live = entry(&app, &format!(".{KEY}.tmp-{}", std::process::id()));
        let dead = entry(&app, &format!(".{KEY}.tmp-{DEAD_PID}"));

        if clean {
            cache::clean(dir.path(), Some("hello")).expect("clean");
        } else {
            cache::uninstall(&app);
        }

        assert!(
            live.join("ginary.json").is_file(),
            "active extraction was removed; clean={clean}"
        );
        assert!(
            !dead.exists(),
            "dead extraction should be reclaimed; clean={clean}"
        );
    }
}

#[test]
fn f1_clean_respects_a_running_entry_lock() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app = dir.path().join("hello");
    let held = entry(&app, KEY);
    let lock = ginary::cache_lock::SharedLock::acquire(&held).expect("hold runtime lock");

    cache::clean(dir.path(), Some("hello")).expect("clean while running");

    assert!(
        held.join("ginary.json").is_file(),
        "running runtime was removed"
    );
    drop(lock);
    cache::clean(dir.path(), Some("hello")).expect("clean after exit");
    assert!(!held.exists(), "the released entry should be reclaimed");
}

#[test]
fn f1_detailed_clean_accounts_for_every_retained_path() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app = dir.path().join("hello");
    let owned = entry(&app, KEY);
    let live = entry(
        &app,
        &format!(".fedcba9876543210.trash-{}", std::process::id()),
    );
    let note = app.join("notes.txt");
    std::fs::write(&note, b"keep me").expect("write note");
    let owned_bytes = std::fs::metadata(owned.join("ginary.json"))
        .expect("manifest size")
        .len();
    let report = cache::clean_detailed(dir.path(), None).expect("detailed clean");

    assert_eq!(report.removed, [owned]);
    assert_eq!(
        report.bytes, owned_bytes,
        "retained evidence is not reported as freed space"
    );
    assert_eq!(
        report.kept,
        [
            (live, cache::KeptReason::Active),
            (note, cache::KeptReason::Unowned),
        ]
    );
}

#[test]
fn f1_cleaning_a_missing_cache_creates_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let absent = dir.path().join("not-created");
    assert_eq!(
        cache::clean_detailed(&absent, None).expect("missing cache"),
        cache::DetailedCleanReport::default()
    );
    assert!(!absent.exists());
}

#[test]
fn f1_clean_preserves_live_trash_that_already_owns_the_rename_destination() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app = dir.path().join("hello");
    let owned = entry(&app, KEY);
    let live = entry(&app, &format!(".{KEY}.trash-{}", std::process::id()));
    let report = cache::clean_detailed(dir.path(), Some("hello")).expect("clean");
    assert!(report.removed.is_empty());
    assert!(owned.join("ginary.json").is_file());
    assert!(live.join("ginary.json").is_file());
    assert!(report.kept.contains(&(live, cache::KeptReason::Active)));
    assert!(
        report
            .kept
            .contains(&(owned, cache::KeptReason::Unremovable))
    );
}

#[test]
fn f1_clean_preserves_even_an_empty_live_trash_directory() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app = dir.path().join("hello");
    let owned = entry(&app, KEY);
    let live = app.join(format!(".{KEY}.trash-{}", std::process::id()));
    std::fs::create_dir(&live).expect("empty live trash");
    let report = cache::clean_detailed(dir.path(), Some("hello")).expect("clean");
    assert!(report.removed.is_empty());
    assert!(owned.join("ginary.json").is_file());
    assert!(live.is_dir());
    assert_eq!(
        std::fs::read_dir(&live).expect("read live trash").count(),
        0
    );
}

#[cfg(feature = "fault-injection")]
#[test]
fn f1_uninstall_during_extraction_preserves_the_live_launch() {
    use crate::common::artifact::{STUB_EXIT, SyntheticArtifact};
    use crate::common::bounded::wait_bounded;
    use crate::common::script::ShimStep;
    use std::time::{Duration, Instant};

    let dir = tempfile::tempdir().expect("tempdir");
    let artifact =
        SyntheticArtifact::build_with_runtime_steps(dir.path(), &[ShimStep::Exit(STUB_EXIT)]);
    let mut child = artifact
        .run()
        .env("GINARY_CACHE_DIR", artifact.cache_root())
        .env("GINARY_FAULT", "after-extract:pause")
        .spawn();
    let prefix = format!(".{}.tmp-{}-", artifact.key(), child.id());
    let deadline = Instant::now() + Duration::from_secs(5);
    let live = loop {
        let found = std::fs::read_dir(artifact.app_dir())
            .ok()
            .and_then(|entries| {
                entries
                    .filter_map(Result::ok)
                    .map(|entry| entry.path())
                    .find(|path| {
                        path.file_name()
                            .is_some_and(|name| name.to_string_lossy().starts_with(&prefix))
                            && path.join("ginary.json").is_file()
                    })
            });
        if found.is_some() {
            break found;
        }
        if Instant::now() >= deadline || child.try_wait().expect("child state").is_some() {
            break None;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let report = cache::uninstall(&artifact.app_dir());
    let preserved = live
        .as_ref()
        .is_some_and(|path| path.join("ginary.json").is_file());
    let output = wait_bounded(child, Duration::from_secs(20), "paused extraction");

    assert!(
        preserved,
        "uninstall removed the extraction in progress: {report:?}"
    );
    let live = live.expect("the child published a recognizable invocation-specific temporary tree");
    assert!(
        report.kept.contains(&(live, cache::KeptReason::Active)),
        "live work was not explained: {report:?}"
    );
    assert_eq!(
        output.status.code(),
        Some(STUB_EXIT),
        "launch failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(artifact.key_dir().join("ginary.json").is_file());
}

#[cfg(unix)]
#[test]
fn f1_maintenance_does_not_follow_a_cache_entry_or_application_symlink() {
    let dir = tempfile::tempdir().expect("tempdir");
    let outside = entry(dir.path(), "outside");
    let original = std::fs::read(outside.join("ginary.json")).expect("original manifest");
    let root = dir.path().join("cache");
    let app = root.join("hello");
    std::fs::create_dir_all(&app).expect("application directory");
    let linked_entry = app.join(KEY);
    std::os::unix::fs::symlink(&outside, &linked_entry).expect("entry symlink");
    let linked_app = root.join("linked-app");
    std::os::unix::fs::symlink(&outside, &linked_app).expect("application symlink");

    let report = cache::clean_detailed(&root, None).expect("clean");
    assert!(report.removed.is_empty());
    assert_eq!(
        report.kept,
        [
            (linked_entry, cache::KeptReason::Unowned),
            (linked_app, cache::KeptReason::Unowned),
        ]
    );
    assert_eq!(
        std::fs::read(outside.join("ginary.json")).expect("outside intact"),
        original
    );
}
