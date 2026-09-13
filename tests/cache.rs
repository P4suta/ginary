// SPDX-License-Identifier: MIT OR Apache-2.0
//! The cache: resolution, the ten extraction steps, the sweep and the clean.
//!
//! Everything here is driven through the library rather than through a
//! process, so a step can be interrupted, a directory can be made read-only
//! and a temporary tree can be planted with a chosen process id. The
//! same properties are asserted from outside, on real processes, in
//! `tests/launcher.rs`; both are needed, because a cache that is correct
//! in-process and wrong across processes is the failure mode this design
//! exists to prevent.

mod common;

use std::ffi::OsString;
// Only `CountingSink` implements it, and that is a `cfg(unix)` fixture.
#[cfg(unix)]
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use common::artifact::{APP, SyntheticArtifact};
use common::cachefs::{DAY, HeldLock, plant_entry};
use common::hostpath::same_path;
use common::payload::SharedSink;
use common::tools::require_flock;

use ginary::cache::{
    self, CacheDirs, DEFAULT_PRUNE_DAYS, Env, KeptReason, Origin, PRUNE_DAYS_VAR, PruneOptions,
    PruneReport,
};
// The two modes, and the sink the fallback warning is written to, belong to
// the `cfg(unix)` tests below and to nothing else.
#[cfg(unix)]
use ginary::cache::{APP_DIR_MODE, BIN_MODE};
use ginary::diag::Diag;
use ginary::trailer::Trailer;

/// A number outside the positive `i32` a `pid_t` occupies, so no process has
/// ever carried it and a tree naming it is a leftover by definition.
///
/// The sweep answers `dead` for this one before it asks the operating system
/// anything, which is why it is not the whole story:
/// [`a_reaped_process_s_temporary_tree_is_removed`] plants an id the process
/// table really held and really gave up, which is the only input that reaches
/// the "no such process" answer of the syscall underneath.
///
/// [`a_reaped_process_s_temporary_tree_is_removed`]: fn@a_reaped_process_s_temporary_tree_is_removed
const DEAD_PID: u32 = 4_000_000_000;

fn env(pairs: &[(&str, &str)]) -> Env {
    Env::from_pairs(
        pairs
            .iter()
            .map(|(key, value)| (OsString::from(*key), OsString::from(*value))),
    )
}

fn dirs(root: &Path) -> CacheDirs {
    CacheDirs {
        root: root.to_path_buf(),
        origin: Origin::GinaryCacheDir,
        is_fallback: false,
    }
}

fn tracing() -> (Diag, SharedSink) {
    let sink = SharedSink::new();
    (Diag::with_sinks(None, Some(Box::new(sink.clone()))), sink)
}

fn phases(sink: &SharedSink) -> Vec<String> {
    sink.lines()
        .iter()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter_map(|value| {
            value
                .get("phase")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
        .collect()
}

fn artifact(dir: &Path) -> (SyntheticArtifact, std::fs::File, Trailer) {
    let artifact = SyntheticArtifact::build(dir);
    let file = std::fs::File::open(artifact.path()).expect("open the artifact");
    let trailer = *artifact.trailer();
    (artifact, file, trailer)
}

fn names(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

// -------------------------------------------------------- creating a root --

// `cache::prepare` takes a uid and is `cfg(unix)`; `cache::prepare_windows`
// takes a user name and is the other one. `tests/windows.rs` holds that half.
#[cfg(unix)]
#[test]
fn prepare_creates_the_resolved_root() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("a/b/cache");
    let mut warnings = Vec::new();
    let resolved = cache::prepare(
        &env(&[("GINARY_CACHE_DIR", &root.to_string_lossy())]),
        1000,
        &mut warnings,
    )
    .expect("a writable root must be created");
    assert_eq!(resolved.root, root);
    assert!(root.is_dir(), "{} was not created", root.display());
    assert!(!resolved.is_fallback);
    assert!(
        warnings.is_empty(),
        "a root that worked must be silent, and it said {}",
        String::from_utf8_lossy(&warnings)
    );
}

// A directory nobody may write to is a mode bit, and a mode bit is a unix
// idea: on Windows a read-only directory still accepts a new child.
#[cfg(unix)]
#[test]
fn an_unwritable_root_falls_back_with_exactly_one_warning() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = tempfile::tempdir().expect("tempdir");
    let locked = dir.path().join("locked");
    std::fs::create_dir(&locked).expect("create the read-only parent");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o500))
        .expect("make it read-only");
    let tmpdir = dir.path().join("tmp");
    std::fs::create_dir(&tmpdir).expect("create TMPDIR");

    let mut warnings = Vec::new();
    let resolved = cache::prepare(
        &env(&[
            ("GINARY_CACHE_DIR", &locked.join("cache").to_string_lossy()),
            ("TMPDIR", &tmpdir.to_string_lossy()),
        ]),
        1000,
        &mut warnings,
    )
    .expect("an unwritable root must fall back rather than fail");

    assert_eq!(resolved.root, tmpdir.join("ginary-1000"));
    assert_eq!(resolved.origin, Origin::Fallback);
    assert!(resolved.is_fallback);
    assert!(resolved.root.is_dir());

    let text = String::from_utf8_lossy(&warnings).into_owned();
    let lines: Vec<&str> = text.lines().filter(|line| !line.is_empty()).collect();
    assert_eq!(lines.len(), 1, "expected one warning, got {lines:?}");
    let warning = lines[0];
    assert!(
        warning.starts_with("ginary: "),
        "the warning must be attributed to ginary, and it is `{warning}`"
    );
    assert!(
        warning.contains(&locked.join("cache").display().to_string())
            && warning.contains(&resolved.root.display().to_string()),
        "the warning must name both the root that failed and the one used: `{warning}`"
    );

    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700)).expect("restore");
}

#[cfg(unix)]
#[test]
fn reaching_the_fallback_because_nothing_was_set_is_silent() {
    let dir = tempfile::tempdir().expect("tempdir");
    let tmpdir = dir.path().join("tmp");
    std::fs::create_dir(&tmpdir).expect("create TMPDIR");
    let mut warnings = Vec::new();
    let resolved = cache::prepare(
        &env(&[("TMPDIR", &tmpdir.to_string_lossy())]),
        7,
        &mut warnings,
    )
    .expect("the fallback must be usable");
    assert_eq!(resolved.root, tmpdir.join("ginary-7"));
    assert!(
        warnings.is_empty(),
        "there was nothing to warn about, and it said {}",
        String::from_utf8_lossy(&warnings)
    );
}

// ---------------------------------------------------------- extracting --

#[test]
fn a_cold_cache_extracts_into_the_key_directory() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (artifact, file, trailer) = artifact(dir.path());
    let root = dir.path().join("cache");
    let (diag, sink) = tracing();

    let entry = cache::ensure_extracted(&file, &trailer, APP, &dirs(&root), &diag)
        .expect("a cold cache must extract");

    // `same_path` and not `==`: `ensure_extracted` answers with the verbatim
    // `\\?\` spelling on Windows — `ginary::winpath` says why — and the
    // directory this test built by hand holds the ordinary one. Both name one
    // directory, and the comparison is about which directory rather than
    // about which spelling.
    let expected = root.join(APP).join(artifact.key());
    assert!(
        same_path(&entry, &expected),
        "the entry is the key directory: {} is not {}",
        entry.display(),
        expected.display()
    );
    assert!(
        entry.join("ginary.json").is_file(),
        "the manifest is the completeness marker and must be a regular file"
    );
    assert!(entry.join("ginary.index.json").is_file());
    assert!(entry.join(format!("lib/{APP}/ebin/{APP}.beam")).is_file());
    assert!(
        phases(&sink).iter().any(|phase| phase == "extract"),
        "the extraction must be a recorded phase, and the trace holds {:?}",
        phases(&sink)
    );
}

#[test]
fn the_extraction_records_the_entries_and_the_bytes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (_artifact, file, trailer) = artifact(dir.path());
    let (diag, sink) = tracing();
    cache::ensure_extracted(
        &file,
        &trailer,
        APP,
        &dirs(&dir.path().join("cache")),
        &diag,
    )
    .expect("a cold cache must extract");

    let record = sink
        .lines()
        .into_iter()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(&line).ok())
        .find(|value| value.get("phase").and_then(serde_json::Value::as_str) == Some("extract"))
        .expect("an `extract` phase");
    let kv = record.get("kv").expect("a kv object");
    assert_eq!(
        kv.get("entries").and_then(serde_json::Value::as_str),
        Some("11"),
        "nine staged files plus the manifest and the index"
    );
    assert!(
        kv.get("bytes")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|bytes| bytes.parse::<u64>().is_ok_and(|bytes| bytes > 0)),
        "the extraction must record how many bytes it wrote"
    );
}

#[test]
fn a_second_call_is_a_hit_and_writes_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (_artifact, file, trailer) = artifact(dir.path());
    let root = dir.path().join("cache");
    let first = cache::ensure_extracted(&file, &trailer, APP, &dirs(&root), &Diag::disabled())
        .expect("a cold cache must extract");
    let marker = first.join("ginary.json");
    let before = std::fs::metadata(&marker).expect("stat").modified().ok();

    let (diag, sink) = tracing();
    let second = cache::ensure_extracted(&file, &trailer, APP, &dirs(&root), &diag)
        .expect("a warm cache must hit");

    assert_eq!(second, first);
    assert_eq!(
        std::fs::metadata(&marker).expect("stat").modified().ok(),
        before,
        "a hit must not rewrite the entry"
    );
    assert_eq!(
        phases(&sink),
        vec!["cache_hit".to_owned()],
        "a hit is one phase and no extraction"
    );
}

#[test]
fn a_key_directory_without_a_manifest_is_moved_aside_and_extracted_again() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (artifact, file, trailer) = artifact(dir.path());
    let root = dir.path().join("cache");
    let app_dir = root.join(APP);
    let entry = app_dir.join(artifact.key());
    std::fs::create_dir_all(entry.join("lib")).expect("plant an incomplete entry");
    std::fs::write(entry.join("lib/leftover"), b"x").expect("plant a file");

    let (diag, _sink) = tracing();
    let extracted = cache::ensure_extracted(&file, &trailer, APP, &dirs(&root), &diag)
        .expect("an incomplete entry must be replaced");

    assert!(
        same_path(&extracted, &entry),
        "the incomplete entry is replaced in place: {} is not {}",
        extracted.display(),
        entry.display()
    );
    assert!(entry.join("ginary.json").is_file());
    assert!(
        !entry.join("lib/leftover").exists(),
        "the incomplete entry's files must not survive into the complete one"
    );
    assert_eq!(
        names(&app_dir),
        vec![artifact.key()],
        "the moved-aside tree is removed, not left behind"
    );
}

#[cfg(unix)]
#[test]
fn the_application_directory_is_private_and_the_bindir_is_executable() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = tempfile::tempdir().expect("tempdir");
    let (_artifact, file, trailer) = artifact(dir.path());
    let root = dir.path().join("cache");
    let entry = cache::ensure_extracted(&file, &trailer, APP, &dirs(&root), &Diag::disabled())
        .expect("a cold cache must extract");

    let app_mode = std::fs::metadata(root.join(APP))
        .expect("stat the application directory")
        .permissions()
        .mode()
        & 0o7777;
    assert_eq!(
        app_mode, APP_DIR_MODE,
        "the cache may live in a shared /tmp, so nobody else may add a file to it"
    );

    for name in ["erlexec", "beam.smp", "erl_child_setup", "inet_gethost"] {
        let program = entry.join("erts-17.0.5/bin").join(name);
        let mode = std::fs::metadata(&program)
            .expect("stat a program")
            .permissions()
            .mode()
            & 0o7777;
        assert_eq!(
            mode,
            BIN_MODE,
            "{} must be executable whatever the tar said",
            program.display()
        );
    }
}

#[test]
fn a_corrupt_payload_leaves_no_key_directory() {
    let dir = tempfile::tempdir().expect("tempdir");
    let artifact = SyntheticArtifact::build(dir.path());
    artifact.break_payload();
    let file = std::fs::File::open(artifact.path()).expect("open the artifact");
    let root = dir.path().join("cache");

    let error = cache::ensure_extracted(
        &file,
        artifact.trailer(),
        APP,
        &dirs(&root),
        &Diag::disabled(),
    )
    .expect_err("a payload that does not hash must be refused");

    assert_eq!(error.exit_code(), 123);
    assert!(
        !root.join(APP).join(artifact.key()).exists(),
        "a failed extraction must leave no entry a later run would trust"
    );
    assert_eq!(
        names(&root.join(APP)),
        Vec::<String>::new(),
        "and no temporary tree either"
    );
}

// ------------------------------------------------------------ sweeping --

fn plant(app_dir: &Path, key: &str, kind: &str, pid: u32) -> PathBuf {
    let path = app_dir.join(format!(".{key}.{kind}-{pid}"));
    std::fs::create_dir_all(path.join("lib")).expect("plant a tree");
    path
}

#[test]
fn a_dead_process_s_temporary_tree_is_removed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app_dir = dir.path().join(APP);
    std::fs::create_dir_all(&app_dir).expect("create the application directory");
    let tmp = plant(&app_dir, "0123456789abcdef", "tmp", DEAD_PID);
    let corrupt = plant(&app_dir, "0123456789abcdef", "corrupt", DEAD_PID);

    let report =
        cache::sweep(&app_dir, std::process::id(), &Diag::disabled()).expect("the sweep must run");

    assert_eq!(report.removed, vec![corrupt.clone(), tmp.clone()]);
    assert!(report.kept.is_empty());
    assert_eq!(names(&app_dir), Vec::<String>::new());
}

/// How many reaped process ids the test below will ask about before it
/// insists on an answer.
///
/// One attempt is enough on any machine that is not handing the id straight
/// back; see the test's own comment for the window this closes.
const REAPED_PID_ATTEMPTS: usize = 5;

#[test]
fn a_reaped_process_s_temporary_tree_is_removed() {
    // The other half of "dead", and the half `DEAD_PID` cannot state: a
    // number that is a plausible process id, that the process table held, and
    // that it no longer holds. That is the input the sweep's liveness rule
    // answers with a *syscall* rather than with a range check — `ESRCH` from
    // `kill(pid, 0)` on unix, `ERROR_INVALID_PARAMETER` from `OpenProcess` on
    // Windows — and without it a rule that read every error as "alive" would
    // never sweep a real leftover and nothing here would notice.
    //
    // The window this cannot close, only shrink: an id becomes eligible for
    // reuse at the moment the last handle to the process goes, which is the
    // `drop` below, and Windows hands ids back out of a recycled pool rather
    // than climbing to a `pid_max` the way Linux does. This suite spawns many
    // processes and CI runs its targets in parallel, so the id can be another
    // process's before the sweep asks about it — and then keeping the tree is
    // the *correct* answer, not a defect. A retry with a fresh reaped id is
    // therefore a re-run rather than a failure; only the last attempt asserts.
    let dir = tempfile::tempdir().expect("tempdir");
    for attempt in 1..=REAPED_PID_ATTEMPTS {
        let app_dir = dir.path().join(format!("{APP}-{attempt}"));
        std::fs::create_dir_all(&app_dir).expect("create the application directory");

        let mut child = crate::common::script::live_process(dir.path(), 0);
        let pid = child.id();
        child.wait().expect("the planted program runs and exits");
        // Reaped by the wait on unix; on Windows the id is the process
        // object's and is not free until the last handle to it is closed,
        // which is what dropping the child does.
        drop(child);
        let tmp = plant(&app_dir, "0123456789abcdef", "tmp", pid);

        let report = cache::sweep(&app_dir, std::process::id(), &Diag::disabled())
            .expect("the sweep must run");

        if report.removed.is_empty() && attempt < REAPED_PID_ATTEMPTS {
            // The id was handed out again between the drop and the sweep.
            continue;
        }

        assert_eq!(
            report.removed,
            vec![tmp],
            "the launcher that owned this tree has gone, so the tree it was extracting into is \
             a leftover and the next launcher may have the space back"
        );
        assert!(report.kept.is_empty());
        assert_eq!(names(&app_dir), Vec::<String>::new());
        return;
    }
}

/// How long the planted live process stays alive for.
///
/// Thirty seconds, which is what the `/bin/sh -c 'sleep 30'` this replaced
/// asked for: long enough that the sweep below certainly runs while the
/// process is up, and finite so a leaked child cannot outlive the suite.
const LIVE_MILLISECONDS: u64 = 30_000;

#[test]
fn a_live_process_s_temporary_tree_is_kept() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app_dir = dir.path().join(APP);
    std::fs::create_dir_all(&app_dir).expect("create the application directory");

    // A planted program that sleeps, and not `/bin/sh -c 'sleep 30'`: the
    // sweep's rule is "a process that is still alive", and a host with no
    // POSIX shell has no way to make one that way — the spawn failed with
    // `The system cannot find the path specified.` before the sweep ran at
    // all. `script::live_process` renders the same behaviour twice, as a
    // shell script and as the compiled shim.
    let mut child = crate::common::script::live_process(dir.path(), LIVE_MILLISECONDS);
    let live = plant(&app_dir, "0123456789abcdef", "tmp", child.id());

    let report =
        cache::sweep(&app_dir, std::process::id(), &Diag::disabled()).expect("the sweep must run");
    let _ = child.kill();
    let _ = child.wait();

    assert_eq!(
        report.kept,
        vec![live.clone()],
        "another launcher's extraction in progress must be left alone"
    );
    assert!(report.removed.is_empty());
    assert!(live.is_dir());
}

#[test]
fn our_own_temporary_tree_is_kept_while_we_are_alive() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app_dir = dir.path().join(APP);
    std::fs::create_dir_all(&app_dir).expect("create the application directory");
    let mine = plant(&app_dir, "0123456789abcdef", "tmp", std::process::id());

    let report =
        cache::sweep(&app_dir, std::process::id(), &Diag::disabled()).expect("the sweep must run");

    assert_eq!(
        report.kept,
        vec![mine],
        "another thread can still be extracting into this process's temporary tree"
    );
    assert!(report.removed.is_empty());
}

#[test]
fn the_sweep_leaves_complete_entries_and_unrecognised_names_alone() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app_dir = dir.path().join(APP);
    std::fs::create_dir_all(app_dir.join("0123456789abcdef")).expect("a complete entry");
    std::fs::create_dir_all(app_dir.join(".not-a-tree")).expect("something else");
    plant(&app_dir, "0123456789abcdef", "tmp", DEAD_PID);

    let report =
        cache::sweep(&app_dir, std::process::id(), &Diag::disabled()).expect("the sweep must run");

    assert_eq!(report.removed.len(), 1);
    assert_eq!(
        names(&app_dir),
        vec![".not-a-tree".to_owned(), "0123456789abcdef".to_owned()]
    );
}

#[test]
fn a_sweep_of_a_directory_that_is_not_there_is_an_empty_report() {
    let dir = tempfile::tempdir().expect("tempdir");
    let report = cache::sweep(&dir.path().join("absent"), 1, &Diag::disabled())
        .expect("a cache that was never created is not an error");
    assert_eq!(report, ginary::cache::SweepReport::default());
}

// ------------------------------------------------------------ pruning --

/// Everything a prune needs but the age: the cache root, the application
/// directory under it, and `now`.
fn prune_tree(dir: &Path) -> (PathBuf, PathBuf) {
    let root = dir.join("cache");
    let app_dir = root.join(APP);
    std::fs::create_dir_all(&app_dir).expect("create the application directory");
    (root, app_dir)
}

/// The default options with a chosen age.
fn after(days: u64) -> PruneOptions {
    PruneOptions { days, all: false }
}

#[test]
fn the_prune_age_defaults_to_a_fortnight() {
    assert_eq!(DEFAULT_PRUNE_DAYS, 14);
    assert_eq!(cache::prune_days(&env(&[])), DEFAULT_PRUNE_DAYS);
    assert_eq!(cache::prune_days(&env(&[(PRUNE_DAYS_VAR, "3")])), 3);
    assert_eq!(
        cache::prune_days(&env(&[(PRUNE_DAYS_VAR, "0")])),
        0,
        "zero is the documented way to turn pruning off"
    );
}

#[test]
fn a_prune_age_that_is_not_a_count_of_days_falls_back_to_the_default() {
    // A misspelt housekeeping preference must not stop an application from
    // starting, so the launcher reads what it can and carries on.
    for value in ["", "  ", "fourteen", "-3", "3d", "99999999999999999999999"] {
        assert_eq!(
            cache::prune_days(&env(&[(PRUNE_DAYS_VAR, value)])),
            DEFAULT_PRUNE_DAYS,
            "`{PRUNE_DAYS_VAR}={value}` must fall back rather than fail a launch"
        );
    }
}

#[test]
fn an_old_unlocked_sibling_is_removed_and_the_entry_being_launched_is_not() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (_root, app_dir) = prune_tree(dir.path());
    let ours = plant_entry(&app_dir, "0000000000000000", DAY * 90);
    let old = plant_entry(&app_dir, "1111111111111111", DAY * 30);

    let report = cache::prune_app(
        &app_dir,
        Some("0000000000000000"),
        after(14),
        std::time::SystemTime::now(),
        &Diag::disabled(),
    );

    assert_eq!(report.removed, vec![old.clone()]);
    assert!(report.kept.is_empty());
    assert!(!old.exists(), "an old sibling must actually be gone");
    assert!(
        ours.join("ginary.json").is_file(),
        "the entry this launch is about must never be a candidate, whatever its age"
    );
}

#[test]
fn a_sibling_younger_than_the_age_is_kept_and_says_so() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (_root, app_dir) = prune_tree(dir.path());
    let fresh = plant_entry(&app_dir, "1111111111111111", DAY * 3);

    let report = cache::prune_app(
        &app_dir,
        None,
        after(14),
        std::time::SystemTime::now(),
        &Diag::disabled(),
    );

    assert_eq!(report.removed, Vec::<PathBuf>::new());
    assert_eq!(report.kept, vec![(fresh.clone(), KeptReason::Fresh)]);
    assert!(fresh.is_dir());
}

#[test]
fn an_age_of_zero_prunes_nothing_at_all() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (_root, app_dir) = prune_tree(dir.path());
    let ancient = plant_entry(&app_dir, "1111111111111111", DAY * 400);

    let report = cache::prune_app(
        &app_dir,
        None,
        after(0),
        std::time::SystemTime::now(),
        &Diag::disabled(),
    );

    assert_eq!(
        report,
        PruneReport::default(),
        "zero days disables pruning, and a disabled prune reports nothing rather than \
         everything"
    );
    assert!(ancient.is_dir());

    // The same tree against a real age, so that the emptiness above is the
    // setting doing its work rather than the entry being unprunable.
    let report = cache::prune_app(
        &app_dir,
        None,
        after(1),
        std::time::SystemTime::now(),
        &Diag::disabled(),
    );
    assert_eq!(report.removed, vec![ancient.clone()]);
    assert!(!ancient.exists());
}

#[test]
fn a_locked_sibling_is_kept_however_old_it_is() {
    let Some(tools) = require_flock() else {
        return;
    };
    let dir = tempfile::tempdir().expect("tempdir");
    let (_root, app_dir) = prune_tree(dir.path());
    let held = plant_entry(&app_dir, "1111111111111111", DAY * 365);
    let lock = HeldLock::take(tools.path("flock"), &held);

    let report = cache::prune_app(
        &app_dir,
        None,
        after(14),
        std::time::SystemTime::now(),
        &Diag::disabled(),
    );

    assert_eq!(report.removed, Vec::<PathBuf>::new());
    assert_eq!(report.kept, vec![(held.clone(), KeptReason::Locked)]);
    assert!(
        held.join("ginary.json").is_file(),
        "an entry a running application holds must survive its own age"
    );
    lock.release(tools.path("flock"));
}

#[test]
fn all_ignores_the_age_and_still_honours_the_lock() {
    let Some(tools) = require_flock() else {
        return;
    };
    let dir = tempfile::tempdir().expect("tempdir");
    let (_root, app_dir) = prune_tree(dir.path());
    let fresh = plant_entry(&app_dir, "1111111111111111", DAY);
    let busy = plant_entry(&app_dir, "2222222222222222", DAY);
    let lock = HeldLock::take(tools.path("flock"), &busy);

    let report = cache::prune_app(
        &app_dir,
        None,
        PruneOptions {
            days: DEFAULT_PRUNE_DAYS,
            all: true,
        },
        std::time::SystemTime::now(),
        &Diag::disabled(),
    );

    assert_eq!(
        report.removed,
        vec![fresh.clone()],
        "`--all` is `whatever its age`, not `whatever is using it`"
    );
    assert_eq!(report.kept, vec![(busy.clone(), KeptReason::Locked)]);
    assert!(!fresh.exists());
    assert!(busy.is_dir());
    lock.release(tools.path("flock"));
}

#[test]
fn pruning_leaves_temporary_corrupt_and_unrecognised_names_alone() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (_root, app_dir) = prune_tree(dir.path());
    plant(&app_dir, "abc", "tmp", DEAD_PID);
    std::fs::create_dir_all(app_dir.join(".not-an-entry")).expect("something else");
    let old = plant_entry(&app_dir, "1111111111111111", DAY * 30);

    let report = cache::prune_app(
        &app_dir,
        None,
        after(14),
        std::time::SystemTime::now(),
        &Diag::disabled(),
    );

    assert_eq!(
        report.removed,
        vec![old],
        "pruning owns complete entries; the sweep owns the rest"
    );
    assert!(app_dir.join(".not-an-entry").is_dir());
    assert!(names(&app_dir).iter().any(|name| name.starts_with(".abc.")));
}

#[test]
fn a_directory_without_a_manifest_has_no_age_and_is_not_pruned() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (_root, app_dir) = prune_tree(dir.path());
    let half = app_dir.join("1111111111111111");
    std::fs::create_dir_all(&half).expect("a key directory with no manifest");

    let complete = plant_entry(&app_dir, "2222222222222222", DAY * 30);

    let report = cache::prune_app(
        &app_dir,
        None,
        PruneOptions {
            days: DEFAULT_PRUNE_DAYS,
            all: true,
        },
        std::time::SystemTime::now(),
        &Diag::disabled(),
    );

    assert_eq!(
        report.removed,
        vec![complete],
        "a complete entry is prunable and a half-extracted one is the sweep's business"
    );
    assert!(half.is_dir());
}

#[test]
fn pruning_an_application_directory_that_is_not_there_reports_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let absent = cache::prune_app(
        &dir.path().join("absent"),
        None,
        after(14),
        std::time::SystemTime::now(),
        &Diag::disabled(),
    );
    assert_eq!(
        absent,
        PruneReport::default(),
        "an application nobody has ever run has nothing to prune"
    );

    // And a directory that *is* there is not silently the same answer.
    let (_root, app_dir) = prune_tree(dir.path());
    let old = plant_entry(&app_dir, "1111111111111111", DAY * 30);
    let present = cache::prune_app(
        &app_dir,
        None,
        after(14),
        std::time::SystemTime::now(),
        &Diag::disabled(),
    );
    assert_eq!(present.removed, vec![old]);
}

#[test]
fn pruning_the_whole_root_visits_every_application() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("cache");
    let hello = plant_entry(&root.join("hello"), "1111111111111111", DAY * 30);
    let other = plant_entry(&root.join("other"), "2222222222222222", DAY * 30);
    let fresh = plant_entry(&root.join("other"), "3333333333333333", DAY);

    let report = cache::prune(&root, None, after(14), std::time::SystemTime::now())
        .expect("pruning must run over a root that exists");

    assert_eq!(report.removed, vec![hello, other]);
    assert_eq!(report.kept, vec![(fresh.clone(), KeptReason::Fresh)]);
    assert!(fresh.is_dir());
}

#[test]
fn pruning_one_application_leaves_the_others_untouched() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("cache");
    let hello = plant_entry(&root.join("hello"), "1111111111111111", DAY * 30);
    let other = plant_entry(&root.join("other"), "2222222222222222", DAY * 30);

    let report = cache::prune(
        &root,
        Some("hello"),
        after(14),
        std::time::SystemTime::now(),
    )
    .expect("pruning one application must run");

    assert_eq!(report.removed, vec![hello]);
    assert!(other.is_dir(), "`--app` must not reach another application");
}

#[test]
fn pruning_an_application_that_is_not_a_name_is_refused_before_anything_is_joined() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("cache");
    std::fs::create_dir_all(&root).expect("create the root");

    for app in ["..", "/etc", "a/b", ""] {
        let error = cache::prune(&root, Some(app), after(14), std::time::SystemTime::now())
            .expect_err("what pruning does to a directory is remove it");
        assert_eq!(
            error.exit_code(),
            124,
            "`--app {app}` must be refused as a cache failure"
        );
    }
}

#[test]
fn pruning_a_cache_that_was_never_created_is_an_empty_report() {
    let dir = tempfile::tempdir().expect("tempdir");
    let absent = cache::prune(
        &dir.path().join("absent"),
        None,
        after(14),
        std::time::SystemTime::now(),
    )
    .expect("pruning nothing is not an error");
    assert_eq!(absent, PruneReport::default());

    let root = dir.path().join("cache");
    let old = plant_entry(&root.join("hello"), "1111111111111111", DAY * 30);
    let present = cache::prune(&root, None, after(14), std::time::SystemTime::now())
        .expect("pruning a root that exists must run");
    assert_eq!(
        present.removed,
        vec![old],
        "a root that was never created and one with nothing prunable in it are different \
         answers to the same question"
    );
}

#[test]
fn a_kept_entry_names_its_reason_in_one_word() {
    assert_eq!(KeptReason::Locked.describe(), "locked");
    assert_eq!(KeptReason::Fresh.describe(), "fresh");

    // And the word is the one a report actually carries.
    let dir = tempfile::tempdir().expect("tempdir");
    let (_root, app_dir) = prune_tree(dir.path());
    plant_entry(&app_dir, "1111111111111111", DAY);
    let report = cache::prune_app(
        &app_dir,
        None,
        after(14),
        std::time::SystemTime::now(),
        &Diag::disabled(),
    );
    assert_eq!(
        report
            .kept
            .iter()
            .map(|(_, reason)| reason.describe())
            .collect::<Vec<&str>>(),
        ["fresh"]
    );
}

// ------------------------------------------------------------ cleaning --

#[test]
fn clean_removes_one_application_and_leaves_the_others() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("cache");
    for app in ["hello", "other"] {
        plant_entry(&root.join(app), "0123456789abcdef", DAY);
    }
    let bytes = std::fs::metadata(root.join("hello/0123456789abcdef/ginary.json"))
        .expect("manifest size")
        .len();

    let report = cache::clean(&root, Some("hello")).expect("clean must run");

    assert_eq!(report.removed, vec![root.join("hello/0123456789abcdef")]);
    assert_eq!(report.bytes, bytes);
    assert_eq!(names(&root), vec!["other".to_owned()]);
}

#[test]
fn clean_without_an_application_empties_the_root_and_keeps_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("cache");
    for app in ["hello", "other"] {
        plant_entry(&root.join(app), "0123456789abcdef", DAY);
    }

    let report = cache::clean(&root, None).expect("clean must run");

    assert_eq!(
        report.removed,
        vec![
            root.join("hello/0123456789abcdef"),
            root.join("other/0123456789abcdef")
        ]
    );
    assert!(root.is_dir(), "the root itself stays");
    assert_eq!(names(&root), Vec::<String>::new());
}

#[test]
fn clean_removes_temporary_and_corrupt_trees_too() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("cache");
    let app_dir = root.join(APP);
    std::fs::create_dir_all(&app_dir).expect("create the application directory");
    plant(&app_dir, "0123456789abcdef", "tmp", DEAD_PID);
    plant(&app_dir, "0123456789abcdef", "corrupt", DEAD_PID);

    cache::clean(&root, Some(APP)).expect("clean must run");

    assert_eq!(names(&root), Vec::<String>::new());
}

#[test]
fn cleaning_a_cache_that_was_never_created_is_an_empty_report() {
    let dir = tempfile::tempdir().expect("tempdir");
    let report = cache::clean(&dir.path().join("absent"), None)
        .expect("cleaning nothing is what the caller asked for");
    assert_eq!(report, ginary::cache::CleanReport::default());
}

/// A sink that remembers what was written to it and how often it was flushed.
///
/// `prepare` takes a `dyn Write` so that the launcher can pass standard error
/// and a test can pass a buffer. The warning it writes is the last thing a
/// user sees before the artifact goes on to run, and standard error is not
/// line buffered when it is a pipe, so the flush is part of the contract
/// rather than a detail: a warning still sitting in a buffer when `execve`
/// replaces the process is a warning nobody was given.
#[cfg(unix)]
#[derive(Debug, Default)]
struct CountingSink {
    written: Vec<u8>,
    flushes: usize,
}

#[cfg(unix)]
impl Write for CountingSink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.written.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.flushes += 1;
        Ok(())
    }
}

#[cfg(unix)]
#[test]
fn a_warning_sink_is_written_through_and_flushed() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = tempfile::tempdir().expect("tempdir");
    let locked = dir.path().join("locked");
    std::fs::create_dir(&locked).expect("create the read-only parent");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o500))
        .expect("make it read-only");
    let tmpdir = dir.path().join("tmp");
    std::fs::create_dir(&tmpdir).expect("create TMPDIR");

    let mut sink = CountingSink::default();
    let resolved = cache::prepare(
        &env(&[
            ("GINARY_CACHE_DIR", &locked.join("cache").to_string_lossy()),
            ("TMPDIR", &tmpdir.to_string_lossy()),
        ]),
        cache::current_uid(),
        &mut sink,
    )
    .expect("an unwritable root falls back");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700)).expect("restore");

    assert!(resolved.is_fallback);
    assert!(
        String::from_utf8_lossy(&sink.written).starts_with("ginary: "),
        "the warning must reach the sink, and it wrote {:?}",
        String::from_utf8_lossy(&sink.written)
    );
    assert_eq!(
        sink.flushes, 1,
        "the warning must be flushed before `prepare` returns, and it was flushed {} times",
        sink.flushes
    );
}

// ------------------------------------------------- trusting the fallback --

#[cfg(unix)]
#[test]
fn a_fallback_root_somebody_else_may_write_to_is_refused() {
    use std::os::unix::fs::PermissionsExt as _;
    // `/tmp` is shared, and `prepare` creates the fallback root with
    // `create_dir_all`, which succeeds on a directory that is already there
    // whatever its mode. A root another user can write to is the parent of the
    // directory this launcher extracts programs into and then executes them
    // from, so it is refused rather than used.
    let dir = tempfile::tempdir().expect("tempdir");
    let tmpdir = dir.path().join("tmp");
    let uid = cache::current_uid();
    let planted = tmpdir.join(format!("ginary-{uid}"));
    std::fs::create_dir_all(&planted).expect("plant the fallback root");
    std::fs::set_permissions(&planted, std::fs::Permissions::from_mode(0o777))
        .expect("make it world-writable");

    let mut warnings = Vec::new();
    let error = cache::prepare(
        &env(&[("TMPDIR", &tmpdir.to_string_lossy())]),
        uid,
        &mut warnings,
    )
    .expect_err("a world-writable fallback root must be refused");

    assert_eq!(error.exit_code(), 124);
    let message = error.to_string();
    assert!(
        message.contains(&planted.display().to_string()),
        "the refusal must name the directory, and it said `{message}`"
    );
}

#[cfg(unix)]
#[test]
fn a_symlink_in_the_place_of_the_fallback_root_is_refused() {
    // `create_dir_all` follows a symlink and reports success, so an attacker
    // who wins the race to create `/tmp/ginary-<uid>` as a link gets to choose
    // the directory the launcher extracts into.
    let dir = tempfile::tempdir().expect("tempdir");
    let tmpdir = dir.path().join("tmp");
    std::fs::create_dir_all(&tmpdir).expect("create TMPDIR");
    let elsewhere = dir.path().join("elsewhere");
    std::fs::create_dir(&elsewhere).expect("create the directory the link points at");
    let uid = cache::current_uid();
    std::os::unix::fs::symlink(&elsewhere, tmpdir.join(format!("ginary-{uid}")))
        .expect("plant the symlink");

    let mut warnings = Vec::new();
    let error = cache::prepare(
        &env(&[("TMPDIR", &tmpdir.to_string_lossy())]),
        uid,
        &mut warnings,
    )
    .expect_err("a symlinked fallback root must be refused");

    assert_eq!(error.exit_code(), 124);
    assert!(
        names(&elsewhere).is_empty(),
        "nothing may be written through the link"
    );
}

#[cfg(unix)]
#[test]
fn a_fallback_root_this_process_owns_is_created_private() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = tempfile::tempdir().expect("tempdir");
    let tmpdir = dir.path().join("tmp");
    let uid = cache::current_uid();

    let mut warnings = Vec::new();
    let resolved = cache::prepare(
        &env(&[("TMPDIR", &tmpdir.to_string_lossy())]),
        uid,
        &mut warnings,
    )
    .expect("the fallback must be usable");

    assert_eq!(resolved.root, tmpdir.join(format!("ginary-{uid}")));
    let mode = std::fs::symlink_metadata(&resolved.root)
        .expect("stat the fallback root")
        .permissions()
        .mode()
        & 0o7777;
    assert_eq!(
        mode, APP_DIR_MODE,
        "the shared-directory fallback must be private to its owner"
    );

    // And a second call finds the directory it made and accepts it.
    let mut again = Vec::new();
    cache::prepare(
        &env(&[("TMPDIR", &tmpdir.to_string_lossy())]),
        uid,
        &mut again,
    )
    .expect("the root this process created must be trusted");
}

// ----------------------------------- the bound on a completeness marker --
//
// `maintenance_owns` decides whether a key-shaped directory is this
// application's before anything removes it, and it reads the marker under two
// bounds: `symlink_metadata` refuses one that is not a plain file, and
// `MAX_FRONT_ENTRY_BYTES` refuses one too large to be a manifest. Both bounds
// are *off-by-one sensitive* and neither edge was asserted, so the nightly
// mutation campaign left eight survivors across four lines of one function:
// `>` traded for `>=` and for `==`, `+ 1` traded for `- 1` and for `* 1`, and
// two `||` traded for `&&`. Each test below is one edge of one bound, written
// so that the two sides of the edge have *different outcomes* — an entry that
// is removed or an entry that is kept and reported `Unowned` — because an edge
// whose two sides look the same proves nothing about where it is.

/// A manifest for `app`, padded with trailing spaces to exactly `bytes`.
///
/// The padding is after the closing brace, so every prefix of the file down to
/// the JSON itself still parses. That is deliberate: a reader that took fewer
/// bytes than it should would otherwise fail for the wrong reason — a truncated
/// object — and a test that cannot tell "read too little" from "read garbage"
/// is not measuring the bound.
fn manifest_padded_to(app: &str, bytes: usize) -> Vec<u8> {
    let mut manifest = common::artifact::canonical_manifest();
    manifest.app = app.to_owned();
    let mut out = serde_json::to_vec(&manifest).expect("manifest JSON");
    assert!(
        out.len() < bytes,
        "the manifest is {} bytes and cannot be padded down to {bytes}",
        out.len()
    );
    out.resize(bytes, b' ');
    out
}

/// An application directory holding one key-shaped entry whose marker is
/// `marker`, and the entry's path.
fn app_with_marker(dir: &Path, marker: &[u8]) -> (PathBuf, PathBuf) {
    let app = dir.join(APP);
    let entry = app.join("0123456789abcdef");
    std::fs::create_dir_all(&entry).expect("the entry directory");
    std::fs::write(entry.join("ginary.json"), marker).expect("the marker");
    (app, entry)
}

#[test]
fn a_marker_exactly_at_the_front_entry_bound_is_this_applications() {
    let dir = tempfile::tempdir().expect("tempdir");
    let bound = usize::try_from(ginary::payload::MAX_FRONT_ENTRY_BYTES).expect("a usize bound");
    let (app, entry) = app_with_marker(dir.path(), &manifest_padded_to(APP, bound));

    let report = cache::uninstall(&app);

    assert_eq!(
        report.removed,
        vec![entry.clone()],
        "`MAX_FRONT_ENTRY_BYTES` is the largest marker there is, not the first one too large: a \
         manifest of exactly that many bytes is one this application wrote, and refusing it \
         leaves an entry nothing can ever reclaim. kept: {:?}",
        report.kept
    );
    assert!(!entry.exists(), "the entry it reported removed is gone");
}

#[test]
fn a_marker_one_byte_over_the_bound_is_refused_however_well_its_first_bytes_parse() {
    let dir = tempfile::tempdir().expect("tempdir");
    let over = usize::try_from(ginary::payload::MAX_FRONT_ENTRY_BYTES).expect("a usize bound") + 1;
    let (app, entry) = app_with_marker(dir.path(), &manifest_padded_to(APP, over));

    let report = cache::uninstall(&app);

    assert_eq!(
        report.kept,
        vec![(entry.clone(), KeptReason::Unowned)],
        "one byte over the bound is over the bound. The padding means the first
         `MAX_FRONT_ENTRY_BYTES` of this file are a complete, valid manifest, so a reader that
         stopped one byte early would accept it and delete the entry — which is exactly what a
         bound that is read as `take(MAX)` rather than `take(MAX + 1)` does. removed: {:?}",
        report.removed
    );
    assert!(entry.is_dir(), "the entry it kept is still there");
}

#[cfg(unix)]
#[test]
fn a_marker_that_is_a_symlink_is_not_read_through() {
    // A symlink is not a plain file, and `symlink_metadata` is what says so.
    // The link points at a manifest that is valid in every other way, so a
    // check that followed it — or that lost the `is_symlink` term — would call
    // the entry this application's and delete it. Planting the target *outside*
    // the application directory is the point: it is a file the cache does not
    // own, reached by a name inside a directory it does.
    let dir = tempfile::tempdir().expect("tempdir");
    let real = dir.path().join("elsewhere.json");
    std::fs::write(&real, manifest_padded_to(APP, 1024)).expect("the real manifest");
    let app = dir.path().join(APP);
    let entry = app.join("0123456789abcdef");
    std::fs::create_dir_all(&entry).expect("the entry directory");
    std::os::unix::fs::symlink(&real, entry.join("ginary.json")).expect("the symlinked marker");

    let report = cache::uninstall(&app);

    assert_eq!(
        report.kept,
        vec![(entry.clone(), KeptReason::Unowned)],
        "a marker that is a link is not a marker this application wrote. removed: {:?}",
        report.removed
    );
    assert!(entry.is_dir(), "the entry it kept is still there");
    assert!(
        real.is_file(),
        "and the file the link pointed at is untouched"
    );
}

#[cfg(unix)]
#[test]
fn an_entry_whose_marker_cannot_be_stated_is_not_this_applications() {
    // The third arm of the same match: a marker that is *absent* means "not
    // finished yet", which a residue name is allowed to be, and a marker that
    // cannot be looked at at all means nothing at all — so it is not ours.
    // Directory mode `0o000` is how a real filesystem says so; running as root
    // defeats it, so the test checks the condition it needs before asserting on
    // it rather than passing for the wrong reason.
    use std::os::unix::fs::PermissionsExt as _;

    let dir = tempfile::tempdir().expect("tempdir");
    let app = dir.path().join(APP);
    let entry = app.join(format!(".0123456789abcdef.tmp-{}", 4_000_000_000_u32));
    std::fs::create_dir_all(&entry).expect("the residue directory");
    std::fs::set_permissions(&entry, std::fs::Permissions::from_mode(0o000))
        .expect("an unreadable residue");
    let unreadable = std::fs::symlink_metadata(entry.join("ginary.json"))
        .err()
        .is_some_and(|error| error.kind() != std::io::ErrorKind::NotFound);
    if !unreadable {
        std::fs::set_permissions(&entry, std::fs::Permissions::from_mode(0o755))
            .expect("restore the mode");
        eprintln!("skipping: this user can read a directory with mode 000");
        return;
    }

    let report = cache::uninstall(&app);
    std::fs::set_permissions(&entry, std::fs::Permissions::from_mode(0o755))
        .expect("restore the mode so the temporary directory can be cleaned up");

    assert_eq!(
        report.kept,
        vec![(entry.clone(), KeptReason::Unowned)],
        "an unfinished residue is allowed to have *no* marker, and this one has a marker nothing \
         can look at — a different thing, and not a licence to delete the tree. removed: {:?}",
        report.removed
    );
    assert!(entry.is_dir(), "the entry it kept is still there");
}

// ------------------------------------------------ what the sweep owns --
//
// `sweep` reclaims the residue of interrupted extractions, and every decision
// it makes is a refusal to delete something: a live owner's work, a tree that
// is not ours, a name that is not a residue at all. The campaign's survivors
// here are the refusals that had no fixture — a well-formed residue name whose
// *marker* disowns it, a residue that is a link to an entry that would
// otherwise qualify, and an application directory the sweep cannot read at all.

/// A residue name for `key` owned by a process id nothing will ever have.
fn dead_residue(key: &str) -> String {
    format!(".{key}.tmp-4000000000")
}

#[test]
fn a_dead_owners_residue_whose_marker_disowns_it_is_kept() {
    // The name is a residue, the owner is gone, and the tree is a directory:
    // everything the sweep looks at agrees except the marker, which is valid
    // JSON that is not a manifest. `owned_sweep_tree` is the term that says so,
    // and a sweep that dropped it — or that read the two conditions as `&&`
    // instead of `||` — would delete a directory it cannot prove it wrote.
    let dir = tempfile::tempdir().expect("tempdir");
    let app = dir.path().join(APP);
    let residue = app.join(dead_residue("0123456789abcdef"));
    std::fs::create_dir_all(&residue).expect("the residue directory");
    std::fs::write(residue.join("ginary.json"), b"{}").expect("a marker that is not a manifest");
    std::fs::write(residue.join("partial"), b"an interrupted extraction").expect("some content");

    let report = cache::sweep(&app, std::process::id(), &Diag::disabled()).expect("the sweep");

    assert_eq!(
        report.removed,
        Vec::<PathBuf>::new(),
        "a residue name is not ownership; the marker is"
    );
    assert!(
        residue.join("partial").is_file(),
        "the tree the sweep kept is still whole"
    );
}

#[cfg(unix)]
#[test]
fn a_residue_that_is_a_link_to_an_entry_this_application_owns_is_still_a_link() {
    // The nastiest shape the sweep can be handed: a link whose *target* would
    // pass every ownership check, so that a reader which asked about the target
    // instead of the name would delete somebody else's directory and leave the
    // link behind. `owned_sweep_tree` asks `symlink_metadata` first and only
    // then asks about the marker, and both halves have to hold.
    let dir = tempfile::tempdir().expect("tempdir");
    let app = dir.path().join(APP);
    std::fs::create_dir_all(&app).expect("the application directory");
    let elsewhere = dir.path().join("elsewhere");
    std::fs::create_dir_all(&elsewhere).expect("a directory outside the application");
    // The manifest names *this* application, so the only thing standing
    // between the sweep and this tree is that the residue is a link.
    std::fs::write(elsewhere.join("ginary.json"), manifest_padded_to(APP, 1024))
        .expect("a manifest for this application, outside it");
    let link = app.join(dead_residue("0123456789abcdef"));
    std::os::unix::fs::symlink(&elsewhere, &link).expect("the residue link");

    let report = cache::sweep(&app, std::process::id(), &Diag::disabled()).expect("the sweep");

    assert_eq!(
        report.removed,
        Vec::<PathBuf>::new(),
        "a link is not a tree the sweep extracted, whatever the marker at the end of it says"
    );
    assert!(
        std::fs::symlink_metadata(&link)
            .expect("the link is still there")
            .file_type()
            .is_symlink(),
        "and it is still a link rather than a directory"
    );
    assert!(
        elsewhere.join("ginary.json").is_file(),
        "the directory it pointed at is untouched"
    );
}

#[cfg(unix)]
#[test]
fn an_application_directory_the_sweep_cannot_read_is_an_error_and_not_an_empty_answer() {
    // `Ok(SweepReport::default())` means "there was nothing to sweep", and a
    // directory this process may not look inside is not that. The launcher acts
    // on the difference: an empty report is a clean cache and an error is a
    // numbered exit code with a hint. Mode `0o000` is how a real filesystem
    // says it, and running as root defeats it, so the condition is checked
    // before it is asserted on.
    use std::os::unix::fs::PermissionsExt as _;

    let dir = tempfile::tempdir().expect("tempdir");
    let app = dir.path().join(APP);
    std::fs::create_dir_all(&app).expect("the application directory");
    std::fs::write(app.join("marker"), b"something to hide").expect("some content");
    std::fs::set_permissions(&app, std::fs::Permissions::from_mode(0o000))
        .expect("an unreadable application directory");
    let unreadable = std::fs::read_dir(&app).is_err();

    let outcome = cache::sweep(&app, std::process::id(), &Diag::disabled());
    std::fs::set_permissions(&app, std::fs::Permissions::from_mode(0o755))
        .expect("restore the mode so the temporary directory can be cleaned up");

    if !unreadable {
        eprintln!("skipping: this user can read a directory with mode 000");
        return;
    }
    let error = outcome.expect_err(
        "a directory the sweep cannot read is not a directory with nothing in it, and answering \
         `Ok` for it tells a launcher its cache is clean",
    );
    let rendered = error.to_string();
    assert!(
        rendered.contains(&app.display().to_string()),
        "the error names the directory it could not read: {rendered}"
    );
}

// ------------------------------------------- the edges of what is pruned --

/// The instant a planted entry's marker is stamped with, so that the age a
/// prune computes is exact rather than however long the test took to get here.
///
/// `plant_entry` sets the mtime to `now - age`, which makes `age.as_secs()`
/// depend on the milliseconds between planting and the call. The boundary this
/// pins is a single second wide, so the stamp is fixed and `now` is derived
/// from it: every run computes the same age.
fn stamped(app: &Path, key: &str) -> (PathBuf, SystemTime) {
    let entry = plant_entry(app, key, Duration::ZERO);
    let stamp = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    common::cachefs::set_mtime(&entry.join("ginary.json"), stamp);
    (entry, stamp)
}

#[test]
fn an_entry_exactly_as_old_as_the_prune_asks_for_is_old_enough() {
    // `age < days` and not `age <= days`: `--days 7` prunes what is seven days
    // old, because "keep what is younger than seven days" is what the flag
    // means. One second either side of that decides, so both are here.
    for (extra, removed) in [(0u64, true), (1, true), (-1i64 as u64, false)] {
        let dir = tempfile::tempdir().expect("tempdir");
        let app = dir.path().join(APP);
        let (entry, stamp) = stamped(&app, "0123456789abcdef");
        let seconds = 7 * 86_400_u64;
        let now = stamp + Duration::from_secs(seconds.wrapping_add(extra));

        let report = cache::prune_app(
            &app,
            None,
            PruneOptions {
                all: false,
                days: 7,
            },
            now,
            &Diag::disabled(),
        );

        assert_eq!(
            report.removed.contains(&entry),
            removed,
            "an entry {} seconds past seven days: {report:?}",
            extra as i64
        );
    }
}

#[test]
fn a_prune_keeps_a_name_that_is_a_file_and_a_name_that_is_a_link() {
    // The ownership chain is three terms — the name is a cache key, the path is
    // a real directory, and the marker says this application wrote it — and a
    // reading that lost the middle one would hand a file or a symlink to the
    // removal below it. Both are planted beside an entry that *is* prunable, so
    // the prune has something to do and the report is not empty for the wrong
    // reason.
    let dir = tempfile::tempdir().expect("tempdir");
    let app = dir.path().join(APP);
    let (prunable, stamp) = stamped(&app, "0123456789abcdef");
    let file = app.join("fedcba9876543210");
    std::fs::write(&file, b"a file wearing a key's name").expect("the file");
    #[cfg(unix)]
    let link = {
        let link = app.join("aaaabbbbccccdddd");
        std::os::unix::fs::symlink(&prunable, &link).expect("the link");
        link
    };

    let report = cache::prune_app(
        &app,
        None,
        PruneOptions { all: true, days: 0 },
        stamp + Duration::from_secs(1),
        &Diag::disabled(),
    );

    assert_eq!(report.removed, vec![prunable], "{report:?}");
    assert!(
        file.is_file(),
        "a file is not an entry, whatever it is called"
    );
    #[cfg(unix)]
    assert!(
        std::fs::symlink_metadata(&link)
            .expect("the link is still there")
            .file_type()
            .is_symlink(),
        "a link is not an entry either"
    );
}

#[test]
fn an_application_directory_that_still_holds_something_is_not_removed() {
    // `uninstall` removes the application directory only when it emptied it:
    // an entry it kept is an application somebody is still using, and the crash
    // dumps beside it are still worth reading. Both halves have to be true at
    // once, so this plants one entry it will take and one file it will not.
    let dir = tempfile::tempdir().expect("tempdir");
    let app = dir.path().join(APP);
    let entry = plant_entry(&app, "0123456789abcdef", Duration::ZERO);
    let evidence = app.join("erl_crash.dump");
    std::fs::write(&evidence, b"a dump a user has not read yet").expect("the evidence");

    let report = cache::uninstall(&app);

    assert_eq!(report.removed, vec![entry], "{report:?}");
    assert!(
        app.is_dir(),
        "the application directory survives its evidence"
    );
    assert!(evidence.is_file(), "and so does the evidence");
}

#[cfg(unix)]
#[test]
fn an_application_directory_that_cannot_be_stated_is_unremovable_and_not_absent() {
    // The first thing `clean_app` does is ask what `app_dir` is, and the two
    // failures mean opposite things: a directory that is not there has already
    // been uninstalled, and one this process may not look at is a directory
    // whose contents are unknown. Reporting the second as the first tells a
    // user their cache is clean.
    use std::os::unix::fs::PermissionsExt as _;

    let dir = tempfile::tempdir().expect("tempdir");
    let closed = dir.path().join("closed");
    let app = closed.join(APP);
    std::fs::create_dir_all(&app).expect("the application directory");
    std::fs::set_permissions(&closed, std::fs::Permissions::from_mode(0o000))
        .expect("close the parent");
    let unstatable = std::fs::symlink_metadata(&app).is_err();

    let report = cache::uninstall(&app);
    std::fs::set_permissions(&closed, std::fs::Permissions::from_mode(0o755))
        .expect("restore the mode so the temporary directory can be cleaned up");

    if !unstatable {
        eprintln!("skipping: this user can stat through a directory with mode 000");
        return;
    }
    assert!(
        report.removed.is_empty() && !report.kept.is_empty(),
        "a directory nothing can look at is reported, not passed over: {report:?}"
    );
}

#[cfg(unix)]
#[test]
fn a_link_wearing_a_residue_name_is_not_followed_to_the_tree_it_points_at() {
    // The nastiest shape `clean_app` can be handed, and the one its
    // `symlink_metadata` arm exists for: a link whose *target* would pass every
    // ownership check, so a reader that asked about the target instead of the
    // name would delete somebody else's tree and leave the link. The marker at
    // the end of the link names this application, so the only thing standing
    // between the removal and that directory is that the entry is a link.
    let dir = tempfile::tempdir().expect("tempdir");
    let app = dir.path().join(APP);
    std::fs::create_dir_all(&app).expect("the application directory");
    let elsewhere = dir.path().join("elsewhere");
    std::fs::create_dir_all(&elsewhere).expect("a directory outside the application");
    std::fs::write(elsewhere.join("ginary.json"), manifest_padded_to(APP, 1024))
        .expect("a manifest for this application, outside it");
    let link = app.join(dead_residue("0123456789abcdef"));
    std::os::unix::fs::symlink(&elsewhere, &link).expect("the residue link");

    let report = cache::uninstall(&app);

    assert!(
        report.removed.is_empty(),
        "a link is not a tree this application extracted: {report:?}"
    );
    assert!(
        std::fs::symlink_metadata(&link)
            .expect("the link is still there")
            .file_type()
            .is_symlink()
    );
    assert!(
        elsewhere.join("ginary.json").is_file(),
        "and the directory it pointed at is untouched"
    );
}

#[test]
fn an_empty_application_directory_is_left_where_it_is() {
    // `uninstall` removes the application directory only when it *emptied* it.
    // A directory that was already empty is one it did nothing to, and removing
    // it would be this command deleting a thing it never owned — the same
    // reasoning that keeps a directory holding evidence. Both halves of that
    // condition decide, and this is the half the other test cannot reach.
    let dir = tempfile::tempdir().expect("tempdir");
    let app = dir.path().join(APP);
    std::fs::create_dir_all(&app).expect("an application directory with nothing in it");

    let report = cache::uninstall(&app);

    assert!(
        report.removed.is_empty() && report.kept.is_empty(),
        "{report:?}"
    );
    assert!(
        app.is_dir(),
        "a directory this call did not empty is a directory it does not remove"
    );
}

#[cfg(unix)]
#[test]
fn an_application_directory_the_sweep_cannot_stat_is_an_error() {
    // The companion of `…cannot_read_is_an_error_and_not_an_empty_answer`,
    // which closes the directory itself and so fails at `read_dir`. This closes
    // the *parent*, so the failure is in the `symlink_metadata` above it — a
    // different match with the same two meanings to tell apart, and reporting
    // "there was nothing to sweep" for either is telling a launcher its cache
    // is clean.
    use std::os::unix::fs::PermissionsExt as _;

    let dir = tempfile::tempdir().expect("tempdir");
    let closed = dir.path().join("closed");
    let app = closed.join(APP);
    std::fs::create_dir_all(&app).expect("the application directory");
    std::fs::set_permissions(&closed, std::fs::Permissions::from_mode(0o000))
        .expect("close the parent");
    let unstatable = std::fs::symlink_metadata(&app).is_err();

    let outcome = cache::sweep(&app, std::process::id(), &Diag::disabled());
    std::fs::set_permissions(&closed, std::fs::Permissions::from_mode(0o755))
        .expect("restore the mode so the temporary directory can be cleaned up");

    if !unstatable {
        eprintln!("skipping: this user can stat through a directory with mode 000");
        return;
    }
    outcome.expect_err("a directory nothing can look at is not a directory with nothing in it");
}
