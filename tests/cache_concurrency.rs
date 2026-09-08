// SPDX-License-Identifier: MIT OR Apache-2.0
//! Concurrent library calls own separate extraction trees and read positions.
mod common;

use std::io::Write;
#[cfg(unix)]
use std::io::{Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use common::artifact::{APP, SyntheticArtifact};
use ginary::cache::{self, CacheDirs, Origin};
use ginary::diag::Diag;

fn dirs(root: PathBuf) -> CacheDirs {
    CacheDirs {
        root,
        origin: Origin::GinaryCacheDir,
        is_fallback: false,
    }
}

#[test]
fn f1_failed_extraction_and_retry_preserve_preexisting_same_process_work() {
    let dir = tempfile::tempdir().unwrap();
    let artifact = SyntheticArtifact::build(dir.path());
    let file = std::fs::File::open(artifact.path()).unwrap();
    let dirs = dirs(dir.path().join("cache"));
    let app = dirs.root.join(APP);
    let original = app.join(format!(".{}.tmp-{}", artifact.key(), std::process::id()));
    std::fs::create_dir_all(&original).unwrap();
    std::fs::write(
        original.join("unrelated.txt"),
        b"owned by an earlier invocation",
    )
    .unwrap();
    let mut truncated = *artifact.trailer();
    truncated.payload_len -= 1;
    assert!(cache::ensure_extracted(&file, &truncated, APP, &dirs, &Diag::disabled()).is_err());
    assert_eq!(
        std::fs::read(original.join("unrelated.txt")).unwrap(),
        b"owned by an earlier invocation"
    );
    let entry =
        cache::ensure_extracted(&file, artifact.trailer(), APP, &dirs, &Diag::disabled()).unwrap();
    assert!(entry.join("ginary.json").is_file());
    assert_eq!(
        std::fs::read(original.join("unrelated.txt")).unwrap(),
        b"owned by an earlier invocation"
    );
    assert_eq!(
        std::fs::read_dir(app).unwrap().count(),
        2,
        "only this invocation's temporary trees are cleaned"
    );
}

struct ExtractionGate(Arc<(Mutex<Vec<PathBuf>>, Condvar)>);

impl Write for ExtractionGate {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if let Ok(record) = serde_json::from_slice::<serde_json::Value>(bytes)
            && record["phase"] == "cache_tmp"
        {
            let (state, ready) = &*self.0;
            let mut paths = state.lock().unwrap();
            paths.push(PathBuf::from(record["kv"]["path"].as_str().unwrap()));
            ready.notify_all();
            drop(
                ready
                    .wait_timeout_while(paths, Duration::from_secs(5), |paths| paths.len() < 2)
                    .unwrap(),
            );
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn f1_two_library_threads_extract_the_same_open_file_into_distinct_owned_trees() {
    let dir = tempfile::tempdir().unwrap();
    let artifact = SyntheticArtifact::build(dir.path());
    let file = std::fs::File::open(artifact.path()).unwrap();
    let dirs = dirs(dir.path().join("cache"));
    let gate = Arc::new((Mutex::new(Vec::new()), Condvar::new()));
    let results = std::thread::scope(|scope| {
        let handles = (0..2)
            .map(|_| {
                let diag =
                    Diag::with_sinks(None, Some(Box::new(ExtractionGate(Arc::clone(&gate)))))
                        .with_sensitive(true);
                let file = &file;
                let dirs = &dirs;
                let trailer = artifact.trailer();
                scope.spawn(move || cache::ensure_extracted(file, trailer, APP, dirs, &diag))
            })
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect::<Vec<_>>()
    });
    let paths = gate.0.lock().unwrap();
    assert_eq!(
        paths.len(),
        2,
        "both calls reached extraction before either published"
    );
    assert_ne!(paths[0], paths[1], "a PID is not an invocation identity");
    assert!(
        results.iter().all(Result::is_ok),
        "both calls must complete: {results:?}"
    );
    assert_eq!(results[0].as_ref().unwrap(), results[1].as_ref().unwrap());
    assert!(results[0].as_ref().unwrap().join("ginary.json").is_file());
    assert!(paths.iter().all(|path| !path.exists()));
    assert_eq!(std::fs::read_dir(dirs.root.join(APP)).unwrap().count(), 1);
}

#[cfg(unix)]
#[test]
fn f1_library_extraction_does_not_move_its_callers_file_cursor() {
    let dir = tempfile::tempdir().unwrap();
    let artifact = SyntheticArtifact::build(dir.path());
    let mut file = std::fs::File::open(artifact.path()).unwrap();
    file.seek(SeekFrom::Start(7)).unwrap();
    cache::ensure_extracted(
        &file,
        artifact.trailer(),
        APP,
        &dirs(dir.path().join("cache")),
        &Diag::disabled(),
    )
    .unwrap();
    assert_eq!(
        file.stream_position().unwrap(),
        7,
        "File::try_clone shares the cursor with concurrent calls"
    );
}

#[test]
fn f1_residue_maintenance_understands_unique_names_without_claiming_arbitrary_suffixes() {
    let dir = tempfile::tempdir().unwrap();
    let app = dir.path().join(APP);
    std::fs::create_dir(&app).unwrap();
    let key = "0123456789abcdef";
    let live = app.join(format!(".{key}.tmp-{}-aB0123cD4567", std::process::id()));
    let legacy = app.join(format!(".{key}.tmp-4000000000"));
    let dead = app.join(format!(".{key}.tmp-4000000000-aB0123cD4567"));
    let invalid = [
        format!(".{key}.tmp-4000000000-"),
        format!(".{key}.tmp-4000000000-not-ginary"),
        format!(".{key}.tmp-4000000000-short"),
        format!(".{key}.corrupt-4000000000-aB0123cD4567"),
    ];
    for path in [&live, &legacy, &dead] {
        std::fs::create_dir(path).unwrap();
        std::fs::write(path.join("partial"), b"work").unwrap();
    }
    for name in &invalid {
        std::fs::create_dir(app.join(name)).unwrap();
    }
    let report = cache::sweep(&app, std::process::id(), &Diag::disabled()).unwrap();
    assert!(report.removed.contains(&legacy) && report.removed.contains(&dead));
    assert!(report.kept.contains(&live));
    assert!(live.join("partial").is_file());
    assert!(invalid.iter().all(|name| app.join(name).is_dir()));
    let clean = cache::uninstall(&app);
    assert!(
        clean
            .kept
            .contains(&(live.clone(), cache::KeptReason::Active))
    );
    assert!(live.join("partial").is_file());
    assert!(invalid.iter().all(|name| app.join(name).is_dir()));
}
