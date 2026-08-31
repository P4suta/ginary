// SPDX-License-Identifier: MIT OR Apache-2.0
//! Where a cross-target build gets its stub, and what it refuses.
//!
//! Two halves, and they are deliberately separate. [`ginary::stub::locate`] is
//! a search: it answers *which file*, from a fixed order of sources, and it
//! reads nothing. [`ginary::stub::verify`] is the proof: this ginary's
//! version, this payload format, this target, and a file whose own object
//! header agrees with the marker printed inside it. Splitting them is what
//! lets a wrong answer to the second question name the file it is about.
//!
//! The fixtures are copies of this test run's own `ginary` binary with the
//! marker rewritten — see `tests/common/stubfile.rs`. Nothing here needs a
//! cross toolchain, with one exception at the end: proving that a real
//! cross-built stub gets a cross build all the way to the runtime is a claim
//! about a real ELF, and that test is gated on one being present.
// The command line half of the suite: every claim in this file is about a
// module the `cli` feature carries, so a `--no-default-features` build has
// nothing here to run. See `docs/dev/log/C2.md`.
#![cfg(feature = "cli")]

mod common;

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use ginary::manifest::FORMAT_VERSION;
use ginary::stub::{self, StubError, StubOpts, StubSource};
use ginary::stubid::{Flavor, StubId, StubIdError};
use ginary::target::{Arch, Target};

use crate::common::artifact::SyntheticArtifact;
use crate::common::fixture::FixtureProject;
use crate::common::project::TempProject;
use crate::common::snapshot::scrub;
use crate::common::stubfile::{
    self, Marker, VERSION, cache_stub_path, ginary_bin, plain_file_name, stub_copy,
    stub_copy_without_marker, stub_file_name, text_with_marker,
};
use crate::common::tools::require_tools;

/// A temporary root with an empty `stubs` directory and an empty cache.
struct Search {
    _dir: tempfile::TempDir,
    env_dir: PathBuf,
    cache_dir: PathBuf,
}

impl Search {
    /// Builds the two directories, both empty.
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let env_dir = dir.path().join("stubs");
        let cache_dir = dir.path().join("cache");
        std::fs::create_dir_all(&env_dir).expect("the stub directory");
        std::fs::create_dir_all(&cache_dir).expect("the cache root");
        Self {
            _dir: dir,
            env_dir,
            cache_dir,
        }
    }

    /// The options a build with `GINARY_STUB_DIR` set would search with.
    fn opts(&self) -> StubOpts {
        StubOpts {
            explicit: None,
            env_dir: Some(self.env_dir.clone()),
            cache_dir: self.cache_dir.clone(),
        }
    }
}

/// A Linux gnu target whose architecture is not this machine's.
///
/// The one target that is certainly foreign and certainly an ELF, which is
/// what the header gate needs: a marker can be rewritten to say anything, and
/// the machine field of the file cannot.
fn other_arch_target() -> Target {
    let arch = if Target::host().arch == Arch::X86_64 {
        Arch::Aarch64
    } else {
        Arch::X86_64
    };
    Target::new(ginary::target::Os::Linux, arch, ginary::target::Libc::Gnu)
}

/// The target the search snapshot is written for.
///
/// Windows rather than a Linux target, for two reasons: it is not the host on
/// any machine this suite runs on, so the self-executable source never appears
/// and the snapshot is the same everywhere, and its `.exe` suffix is part of
/// every name the search builds.
fn windows() -> Target {
    "windows-x86_64".parse().expect("a target name")
}

/// The identity a stub of this ginary for `target` reports.
fn expected_id(target: Target, flavor: Flavor) -> StubId {
    StubId {
        version: VERSION.to_owned(),
        target,
        format_version: FORMAT_VERSION,
        flavor,
    }
}

// ------------------------------------------------------------- the search --

#[test]
fn the_search_list_is_the_four_sources_in_the_documented_order() {
    let search = Search::new();
    let host = Target::host();
    let (_file, self_exe) = ginary::selfexe::open_self().expect("the test binary is open");

    let candidates = stub::candidate_paths(&host, &search.opts());

    assert_eq!(
        candidates,
        vec![
            (
                search.env_dir.join(stub_file_name(VERSION, &host)),
                StubSource::EnvDir(search.env_dir.clone()),
            ),
            (
                search.env_dir.join(plain_file_name(VERSION, &host)),
                StubSource::EnvDir(search.env_dir.clone()),
            ),
            (self_exe, StubSource::SelfExe),
            (
                cache_stub_path(&search.cache_dir, VERSION, &host),
                StubSource::Cache(search.cache_dir.clone()),
            ),
        ]
    );
}

#[test]
fn the_running_executable_is_not_a_candidate_for_another_target() {
    let search = Search::new();
    let target = other_arch_target();

    let candidates = stub::candidate_paths(&target, &search.opts());

    assert!(
        !candidates
            .iter()
            .any(|(_, source)| *source == StubSource::SelfExe),
        "this binary runs on {} and nothing else: {candidates:?}",
        Target::host()
    );
    assert_eq!(candidates.len(), 3);
}

#[test]
fn a_search_with_no_stub_directory_still_looks_in_the_cache() {
    let search = Search::new();
    let opts = StubOpts {
        env_dir: None,
        ..search.opts()
    };
    let target = other_arch_target();

    let candidates = stub::candidate_paths(&target, &opts);

    assert_eq!(
        candidates,
        vec![(
            cache_stub_path(&search.cache_dir, VERSION, &target),
            StubSource::Cache(search.cache_dir.clone()),
        )]
    );
}

#[test]
fn an_explicit_stub_wins_over_every_other_source() {
    let search = Search::new();
    let target = other_arch_target();
    // Every other source is populated, so a search that ignored `--stub` would
    // still find something and would still look like it worked.
    let named = stub_copy(
        &search.env_dir,
        "chosen",
        &Marker::for_target(&target).bytes(),
    );
    stub_copy(
        &search.env_dir,
        &stub_file_name(VERSION, &target),
        &Marker::for_target(&target).bytes(),
    );
    let opts = StubOpts {
        explicit: Some(named.clone()),
        ..search.opts()
    };

    let (path, source) = stub::locate(&target, &opts).expect("`--stub` names the file");

    assert_eq!(path, named);
    assert_eq!(source, StubSource::Explicit(named));
}

#[test]
fn an_explicit_stub_that_is_not_there_is_refused_rather_than_searched_past() {
    let search = Search::new();
    let target = other_arch_target();
    stub_copy(
        &search.env_dir,
        &stub_file_name(VERSION, &target),
        &Marker::for_target(&target).bytes(),
    );
    let missing = search.env_dir.join("no-such-stub");
    let opts = StubOpts {
        explicit: Some(missing.clone()),
        ..search.opts()
    };

    let error = stub::locate(&target, &opts).expect_err("an instruction is not a hint");

    assert!(
        matches!(&error, StubError::Missing { path } if *path == missing),
        "expected StubError::Missing, got {error:?}"
    );
}

#[test]
fn the_stub_spelling_wins_over_the_plain_one_in_the_same_directory() {
    let search = Search::new();
    let target = other_arch_target();
    let marker = Marker::for_target(&target).bytes();
    let preferred = stub_copy(&search.env_dir, &stub_file_name(VERSION, &target), &marker);
    stub_copy(&search.env_dir, &plain_file_name(VERSION, &target), &marker);

    let (path, source) = stub::locate(&target, &search.opts()).expect("the directory holds one");

    assert_eq!(path, preferred);
    assert_eq!(source, StubSource::EnvDir(search.env_dir.clone()));
}

#[test]
fn the_plain_spelling_is_used_when_it_is_the_only_one() {
    let search = Search::new();
    let target = other_arch_target();
    let only = stub_copy(
        &search.env_dir,
        &plain_file_name(VERSION, &target),
        &Marker::for_target(&target).bytes(),
    );

    let (path, source) = stub::locate(&target, &search.opts()).expect("the directory holds one");

    assert_eq!(path, only);
    assert_eq!(source, StubSource::EnvDir(search.env_dir.clone()));
}

#[test]
fn the_stub_directory_is_searched_before_the_cache() {
    let search = Search::new();
    let target = other_arch_target();
    let marker = Marker::for_target(&target).bytes();
    let cached = cache_stub_path(&search.cache_dir, VERSION, &target);
    stub_copy(
        cached.parent().expect("the cache has a directory"),
        &cached
            .file_name()
            .expect("the cache entry has a name")
            .to_string_lossy(),
        &marker,
    );
    let in_env = stub_copy(&search.env_dir, &stub_file_name(VERSION, &target), &marker);

    let (path, source) = stub::locate(&target, &search.opts()).expect("both sources hold one");

    assert_eq!(path, in_env);
    assert_eq!(source, StubSource::EnvDir(search.env_dir.clone()));
}

#[test]
fn the_cache_is_the_last_source_before_the_error() {
    let search = Search::new();
    let target = other_arch_target();
    let cached = cache_stub_path(&search.cache_dir, VERSION, &target);
    stub_copy(
        cached.parent().expect("the cache has a directory"),
        &cached
            .file_name()
            .expect("the cache entry has a name")
            .to_string_lossy(),
        &Marker::for_target(&target).bytes(),
    );

    let (path, source) = stub::locate(&target, &search.opts()).expect("the cache holds one");

    assert_eq!(path, cached);
    assert_eq!(source, StubSource::Cache(search.cache_dir.clone()));
}

#[test]
fn the_host_falls_back_to_the_running_executable() {
    let search = Search::new();
    let (_file, self_exe) = ginary::selfexe::open_self().expect("the test binary is open");

    let (path, source) =
        stub::locate(&Target::host(), &search.opts()).expect("the host always has a stub");

    assert_eq!(source, StubSource::SelfExe);
    assert_eq!(path, self_exe);
}

#[test]
fn a_windows_stub_is_looked_for_with_its_exe_suffix() {
    let search = Search::new();
    let target = windows();
    let name = stub_file_name(VERSION, &target);
    assert_eq!(name, format!("ginary-stub-{VERSION}-windows-x86_64.exe"));
    let expected = stub_copy(&search.env_dir, &name, &Marker::for_target(&target).bytes());

    let (path, _source) = stub::locate(&target, &search.opts()).expect("the directory holds one");

    assert_eq!(path, expected);
}

#[test]
fn nothing_found_names_every_path_that_was_searched() {
    let search = Search::new();

    let error = stub::locate(&windows(), &search.opts()).expect_err("the directories are empty");

    let StubError::NotFound {
        target,
        version,
        searched,
    } = &error
    else {
        panic!("expected StubError::NotFound, got {error:?}");
    };
    assert_eq!(*target, windows());
    assert_eq!(version, VERSION);
    assert_eq!(searched.len(), 3, "{searched:?}");

    let rendered = scrub(
        &error.to_string(),
        &[
            (&search.env_dir, "<env>"),
            (&search.cache_dir, "<cache>"),
            (Path::new(VERSION), "<ver>"),
        ],
    );
    insta::assert_snapshot!("stub_not_found_message", rendered);
}

// -------------------------------------------------------------- the proof --

#[test]
fn the_running_ginary_verifies_as_a_host_stub() {
    let id = stub::verify(&ginary_bin(), &Target::host()).expect("this ginary is its own stub");

    assert_eq!(
        id,
        expected_id(
            Target::host(),
            if cfg!(feature = "cli") {
                Flavor::Full
            } else {
                Flavor::Stub
            }
        )
    );
}

#[test]
fn a_stub_built_by_another_ginary_is_refused_by_version() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let host = Target::host();
    let path = stub_copy(dir.path(), "old", &Marker::host().version("0.0.1").bytes());

    let error = stub::verify(&path, &host).expect_err("stubs are version-locked");

    assert!(
        matches!(
            &error,
            StubError::VersionMismatch { stub, ginary, .. }
                if stub == "0.0.1" && ginary == VERSION
        ),
        "expected StubError::VersionMismatch, got {error:?}"
    );
    let message = error.to_string();
    assert!(
        message.contains("0.0.1")
            && message.contains(VERSION)
            && message.contains("version-locked"),
        "the message names both versions and the rule: {message}"
    );
}

#[test]
fn a_stub_that_reads_another_payload_format_is_refused() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = stub_copy(dir.path(), "future", &Marker::host().format("9").bytes());

    let error = stub::verify(&path, &Target::host()).expect_err("the format is part of the lock");

    assert!(
        matches!(
            &error,
            StubError::FormatMismatch { stub, supported, .. }
                if *stub == 9 && *supported == FORMAT_VERSION
        ),
        "expected StubError::FormatMismatch, got {error:?}"
    );
}

#[test]
fn a_stub_whose_marker_names_another_target_is_refused() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let other = other_arch_target();
    let path = stub_copy(dir.path(), "cross", &Marker::for_target(&other).bytes());

    let error = stub::verify(&path, &Target::host()).expect_err("a stub is for one target");

    assert!(
        matches!(
            &error,
            StubError::TargetMismatch { stub, want, .. }
                if *stub == other && *want == Target::host()
        ),
        "expected StubError::TargetMismatch, got {error:?}"
    );
}

#[test]
fn a_marker_that_disagrees_with_the_file_is_refused_by_the_header() {
    // The whole point of the second gate: the marker is text and copies, and
    // the ELF machine field is what the linker wrote. This file says it is for
    // the other architecture and is this machine's own binary.
    let dir = tempfile::tempdir().expect("a temporary directory");
    let other = other_arch_target();
    let path = stub_copy(dir.path(), "liar", &Marker::for_target(&other).bytes());

    let error = stub::verify(&path, &other).expect_err("the header is believed, not the marker");

    assert!(
        matches!(&error, StubError::ObjectMismatch { want, .. } if *want == other),
        "expected StubError::ObjectMismatch, got {error:?}"
    );
    let message = error.to_string();
    assert!(
        message.contains(&Target::host().arch.as_str().to_owned()),
        "the message says what the file really is: {message}"
    );
}

#[test]
fn a_file_that_is_not_an_object_is_refused_after_its_marker_passes() {
    // A shell script with a perfectly good marker in it. Everything the marker
    // claims is true, which is what makes this the object gate's test and not
    // the scanner's.
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = text_with_marker(dir.path(), "script", &Marker::host().bytes());

    let error = stub::verify(&path, &Target::host()).expect_err("a script is not a stub");

    assert!(
        matches!(&error, StubError::NotAnObject { path: named, .. } if *named == path),
        "expected StubError::NotAnObject, got {error:?}"
    );
}

#[test]
fn a_binary_with_no_marker_is_refused_by_the_scanner() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = stub_copy_without_marker(dir.path(), "anonymous");

    let error = stub::verify(&path, &Target::host()).expect_err("an unmarked binary is not a stub");

    assert!(
        matches!(
            &error,
            StubError::Marker {
                source: StubIdError::NotAStub,
                ..
            }
        ),
        "expected StubError::Marker(NotAStub), got {error:?}"
    );
}

#[test]
fn a_packaged_application_is_not_a_stub() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let artifact = SyntheticArtifact::build(dir.path());

    let error = stub::verify(artifact.path(), &Target::host())
        .expect_err("a payload may not be appended twice");

    assert!(
        matches!(&error, StubError::Trailered { path } if path == artifact.path()),
        "expected StubError::Trailered, got {error:?}"
    );
}

#[test]
fn a_darwin_stub_cannot_be_checked_here_yet() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let macos: Target = "macos-aarch64".parse().expect("a target name");
    let path = stub_copy(dir.path(), "darwin", &Marker::for_target(&macos).bytes());

    let error = stub::verify(&path, &macos).expect_err("Mach-O is not read yet");

    assert!(
        matches!(&error, StubError::NotYetSupported { target } if *target == macos),
        "expected StubError::NotYetSupported, got {error:?}"
    );
    assert!(
        error.to_string().contains("release build"),
        "the message says where a darwin stub comes from: {error}"
    );
}

#[test]
fn a_file_past_the_cap_is_refused_without_reading_it() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("enormous");
    let file = std::fs::File::create(&path).expect("the sparse file");
    let len = stub::MAX_STUB_BYTES + 1;
    file.set_len(len).expect("a sparse file of the right size");
    drop(file);
    stubfile::set_executable(&path);

    let error = stub::verify(&path, &Target::host()).expect_err("a stub has a size bound");

    assert!(
        matches!(
            &error,
            StubError::TooLarge { len: actual, cap, .. }
                if *actual == len && *cap == stub::MAX_STUB_BYTES
        ),
        "expected StubError::TooLarge, got {error:?}"
    );
}

#[test]
fn the_stub_errors_name_the_file_and_the_remedy() {
    let path = PathBuf::from("/stubs/ginary-stub-1.2.3-linux-aarch64-musl");
    let host: Target = "linux-x86_64-gnu".parse().expect("a target name");
    let want: Target = "linux-aarch64-musl".parse().expect("a target name");
    let cases: Vec<StubError> = vec![
        StubError::Missing { path: path.clone() },
        StubError::NotAFile {
            path: path.clone(),
            found: "a directory".to_owned(),
        },
        StubError::TooLarge {
            path: path.clone(),
            len: 1_073_741_824,
            cap: stub::MAX_STUB_BYTES,
        },
        StubError::Marker {
            path: path.clone(),
            source: StubIdError::NotAStub,
        },
        StubError::VersionMismatch {
            path: path.clone(),
            stub: "1.2.3".to_owned(),
            ginary: "1.3.0".to_owned(),
        },
        StubError::FormatMismatch {
            path: path.clone(),
            stub: 2,
            supported: 1,
        },
        StubError::TargetMismatch {
            path: path.clone(),
            stub: host,
            want,
        },
        StubError::NotAnObject {
            path: path.clone(),
            reason: "the file is 26 bytes and begins `#!`".to_owned(),
        },
        StubError::ObjectMismatch {
            path: path.clone(),
            want,
            found: "an ELF for x86_64 with a glibc interpreter".to_owned(),
        },
        StubError::NotYetSupported {
            target: "macos-aarch64".parse().expect("a target name"),
        },
        StubError::Trailered { path },
    ];

    let rendered = cases
        .iter()
        .map(std::string::ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n");

    insta::assert_snapshot!("stub_error_messages", rendered);
}

// --------------------------------------------------------- the build path --

/// A `ginary build` in `project`, with an empty stub directory and cache.
///
/// The environment is what makes the assertion honest: a developer with
/// `GINARY_STUB_DIR` set, or with a stub already in `~/.cache`, would
/// otherwise get a different error from the same command.
fn build_in(project: &Path, empty: &Path, args: &[&str]) -> assert_cmd::assert::Assert {
    let stubs = empty.join("stubs");
    let cache = empty.join("cache");
    std::fs::create_dir_all(&stubs).expect("an empty stub directory");
    std::fs::create_dir_all(&cache).expect("an empty cache");
    let mut command = Command::cargo_bin("ginary").expect("the `ginary` binary is built for tests");
    command
        .current_dir(project)
        .env(stub::STUB_DIR_VAR, &stubs)
        .env("GINARY_CACHE_DIR", &cache)
        .args(args)
        .assert()
}

#[test]
fn a_cross_build_with_no_stub_names_every_path_it_searched() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let project = TempProject::named("hello");
    let target = other_arch_target();

    let assert = build_in(
        project.root(),
        dir.path(),
        &["build", "--skip-export", "--target", &target.name()],
    )
    .failure();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf-8");

    assert!(
        stderr.contains(&format!("no stub found for {target}")),
        "the cross build says what is missing rather than that it is impossible: {stderr}"
    );
    assert!(
        stderr.contains(&stub_file_name(VERSION, &target)),
        "the searched names are in the message: {stderr}"
    );
    assert!(
        stderr.contains("stubs:build"),
        "the message says how to make one: {stderr}"
    );
    assert!(
        !stderr.contains("cannot obtain the Gleam shipment"),
        "the stub is looked for before the project is exported: {stderr}"
    );
}

#[test]
fn the_build_command_takes_the_stub_it_is_given() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let project = TempProject::named("hello");
    let target = other_arch_target();
    let missing = dir.path().join("no-such-stub");

    let assert = build_in(
        project.root(),
        dir.path(),
        &[
            "build",
            "--skip-export",
            "--target",
            &target.name(),
            "--stub",
            &missing.display().to_string(),
        ],
    )
    .failure();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf-8");

    assert!(
        stderr.contains(&missing.display().to_string()) && stderr.contains("is not there"),
        "`--stub` is a path the build reports on rather than an unknown flag: {stderr}"
    );
}

#[test]
fn a_cross_build_with_a_real_stub_stops_at_the_runtime_and_says_why() {
    // The whole chain, and the honest end of it on this machine: the stub is a
    // real cross-built ELF and passes every gate, and then the runtime does
    // not — the host's ERTS is a glibc build, and a musl artifact may not
    // carry it. A complete cross artifact needs a musl runtime, which arrives
    // with the catalogue milestone.
    let target: Target = "linux-x86_64-musl".parse().expect("a target name");
    if require_tools(&["gleam", "erl"]).is_none() {
        return;
    }
    let Some(stub_path) = stubfile::cross_stub(&target) else {
        return;
    };

    let dir = tempfile::tempdir().expect("a temporary directory");
    let project = FixtureProject::copy("hello_ffi", dir.path());
    project.export_shipment();
    let otp = ginary::otp::discover(None).expect("this machine has an Erlang");
    let manifest = project.dir().join("gleam.toml");
    let text = std::fs::read_to_string(&manifest).expect("the fixture manifest");
    std::fs::write(
        &manifest,
        format!(
            "{text}\n[tools.ginary.target.{}]\nerts = \"dir:{}\"\n",
            target.name(),
            otp.root.display()
        ),
    )
    .expect("the target sub-table is written");

    let assert = build_in(
        project.dir(),
        dir.path(),
        &[
            "build",
            "--skip-export",
            "--target",
            &target.name(),
            "--stub",
            &stub_path.display().to_string(),
        ],
    )
    .failure();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf-8");

    assert!(
        stderr.contains(&format!("and this build is for {target}")),
        "the stub was accepted and the runtime was refused: {stderr}"
    );
    assert!(
        !stderr.contains("no stub found"),
        "the stub passed every gate: {stderr}"
    );
}
