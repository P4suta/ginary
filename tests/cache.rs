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
use std::io::Write;
use std::path::{Path, PathBuf};

use common::artifact::{APP, SyntheticArtifact};
use common::payload::SharedSink;

use ginary::cache::{self, APP_DIR_MODE, BIN_MODE, CacheDirs, Env, Origin};
use ginary::diag::Diag;
use ginary::trailer::Trailer;

/// A process id no machine has: `/proc/<pid>` cannot exist for it, so a tree
/// carrying it is a leftover by definition.
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

    assert_eq!(entry, root.join(APP).join(artifact.key()));
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

    assert_eq!(extracted, entry);
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
    let tmp = plant(&app_dir, "abc", "tmp", DEAD_PID);
    let corrupt = plant(&app_dir, "abc", "corrupt", DEAD_PID);

    let report =
        cache::sweep(&app_dir, std::process::id(), &Diag::disabled()).expect("the sweep must run");

    assert_eq!(report.removed, vec![corrupt.clone(), tmp.clone()]);
    assert!(report.kept.is_empty());
    assert_eq!(names(&app_dir), Vec::<String>::new());
}

#[test]
fn a_live_process_s_temporary_tree_is_kept() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app_dir = dir.path().join(APP);
    std::fs::create_dir_all(&app_dir).expect("create the application directory");

    let mut child = std::process::Command::new("/bin/sh")
        .args(["-c", "sleep 30"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn a live process");
    let live = plant(&app_dir, "abc", "tmp", child.id());

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
fn our_own_leftovers_are_swept_even_though_we_are_alive() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app_dir = dir.path().join(APP);
    std::fs::create_dir_all(&app_dir).expect("create the application directory");
    let mine = plant(&app_dir, "abc", "tmp", std::process::id());

    let report =
        cache::sweep(&app_dir, std::process::id(), &Diag::disabled()).expect("the sweep must run");

    assert_eq!(
        report.removed,
        vec![mine],
        "a tree carrying this process's own id is a leftover of a previous run of that id"
    );
}

#[test]
fn the_sweep_leaves_complete_entries_and_unrecognised_names_alone() {
    let dir = tempfile::tempdir().expect("tempdir");
    let app_dir = dir.path().join(APP);
    std::fs::create_dir_all(app_dir.join("0123456789abcdef")).expect("a complete entry");
    std::fs::create_dir_all(app_dir.join(".not-a-tree")).expect("something else");
    plant(&app_dir, "abc", "tmp", DEAD_PID);

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

// ------------------------------------------------------------ cleaning --

#[test]
fn clean_removes_one_application_and_leaves_the_others() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("cache");
    for app in ["hello", "other"] {
        let entry = root.join(app).join("0123456789abcdef");
        std::fs::create_dir_all(&entry).expect("create an entry");
        std::fs::write(entry.join("ginary.json"), b"{}").expect("write a marker");
    }

    let report = cache::clean(&root, Some("hello")).expect("clean must run");

    assert_eq!(report.removed, vec![root.join("hello")]);
    assert_eq!(report.bytes, 2);
    assert_eq!(names(&root), vec!["other".to_owned()]);
}

#[test]
fn clean_without_an_application_empties_the_root_and_keeps_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("cache");
    for app in ["hello", "other"] {
        std::fs::create_dir_all(root.join(app).join("key")).expect("create an entry");
    }

    let report = cache::clean(&root, None).expect("clean must run");

    assert_eq!(report.removed, vec![root.join("hello"), root.join("other")]);
    assert!(root.is_dir(), "the root itself stays");
    assert_eq!(names(&root), Vec::<String>::new());
}

#[test]
fn clean_removes_temporary_and_corrupt_trees_too() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("cache");
    let app_dir = root.join(APP);
    std::fs::create_dir_all(&app_dir).expect("create the application directory");
    plant(&app_dir, "abc", "tmp", DEAD_PID);
    plant(&app_dir, "abc", "corrupt", std::process::id());

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
#[derive(Debug, Default)]
struct CountingSink {
    written: Vec<u8>,
    flushes: usize,
}

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
