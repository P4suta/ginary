// SPDX-License-Identifier: MIT OR Apache-2.0
//! Public API failures which must keep both caller data and diagnostic context.
#![cfg(feature = "cli")]

mod common;

use std::path::{Path, PathBuf};

use common::fake_otp::{FakeOtp, FakeShipment};
use ginary::assemble::{self, StageOptions};

fn stage_inputs(dir: &Path) -> (ginary::closure::AppSet, ginary::otp::OtpInfo) {
    let shipment = FakeShipment::new()
        .app("hello", "1.0.0", &[])
        .build_in(dir.join("shipment"));
    let otp = FakeOtp::new().build_in(dir.join("otp"));
    let info = ginary::otp::inspect_root(&otp.root).expect("inspect fixture OTP");
    let apps = ginary::closure::app_dependency_closure(
        &shipment.root,
        &otp.lib(),
        &["hello".to_owned()],
        &[],
    )
    .expect("close fixture applications");
    (apps, info)
}

fn legacy_stage_temp(out: &Path) -> PathBuf {
    out.with_file_name(format!("out.tmp-{}", std::process::id()))
}

#[test]
fn stage_success_preserves_a_preexisting_same_pid_tree() {
    let dir = tempfile::tempdir().expect("temporary workspace");
    let (apps, otp) = stage_inputs(dir.path());
    let out = dir.path().join("out");
    let residue = legacy_stage_temp(&out);
    std::fs::create_dir(&residue).unwrap();
    std::fs::write(
        residue.join("caller-data"),
        b"belongs to another invocation",
    )
    .unwrap();

    let staged = assemble::stage(&apps, &otp, &StageOptions::default(), &out)
        .expect("the unrelated tree does not prevent staging");

    assert!(staged.root().join(assemble::LISTING_NAME).is_file());
    assert_eq!(
        std::fs::read(residue.join("caller-data")).expect("unowned data must survive success"),
        b"belongs to another invocation"
    );
}

#[test]
fn stage_failure_preserves_a_preexisting_same_pid_tree() {
    let dir = tempfile::tempdir().expect("temporary workspace");
    let (apps, otp) = stage_inputs(dir.path());
    let out = dir.path().join("out");
    let residue = legacy_stage_temp(&out);
    std::fs::create_dir(&residue).unwrap();
    std::fs::write(
        residue.join("caller-data"),
        b"belongs to another invocation",
    )
    .unwrap();
    let options = StageOptions {
        extra_bins: vec!["missing-extra-program".to_owned()],
        ..StageOptions::default()
    };

    assert!(matches!(
        assemble::stage(&apps, &otp, &options, &out),
        Err(assemble::AssembleError::MissingExtraBinary { .. })
    ));
    assert!(!out.exists(), "a failed staging must not publish");
    assert_eq!(
        std::fs::read(residue.join("caller-data")).expect("unowned data must survive failure"),
        b"belongs to another invocation"
    );
}

#[test]
fn concurrent_stage_calls_publish_one_complete_tree_without_deleting_the_winner() {
    let dir = tempfile::tempdir().unwrap();
    let (apps, otp) = stage_inputs(dir.path());
    let out = dir.path().join("out");
    let barrier = std::sync::Barrier::new(2);
    let results = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..2)
            .map(|_| {
                scope.spawn(|| {
                    barrier.wait();
                    assemble::stage(&apps, &otp, &StageOptions::default(), &out)
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    let winner = results.into_iter().find_map(Result::ok).unwrap();
    let refreshed = winner
        .refresh()
        .expect("the losing invocation cannot remove the winner's files");
    assert_eq!(refreshed.files(), winner.files());
    assert!(out.join("lib/hello/ebin/hello.app").is_file());
    assert!(!std::fs::read_dir(dir.path()).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("out.tmp-")
    }));
}

#[test]
fn updating_a_staged_file_replaces_its_listing_entry_and_refresh_reports_a_missing_file() {
    let dir = tempfile::tempdir().unwrap();
    let (apps, otp) = stage_inputs(dir.path());
    let out = dir.path().join("out");
    let mut staged = assemble::stage(&apps, &otp, &StageOptions::default(), &out).unwrap();
    assert_eq!(staged.otp_release(), otp.release);
    staged
        .add_file(
            "config/vm.args",
            b"+S 1\n",
            0o644,
            assemble::Category::Other,
        )
        .unwrap();
    staged
        .add_file(
            "config/vm.args",
            b"+S 2:2\n",
            0o644,
            assemble::Category::Other,
        )
        .unwrap();
    let rows: Vec<_> = staged
        .files()
        .iter()
        .filter(|file| file.path == "config/vm.args")
        .collect();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].size, 7);
    assert_eq!(rows[0].category.to_string(), "other");
    let listing: assemble::StageListing =
        serde_json::from_slice(&std::fs::read(out.join(assemble::LISTING_NAME)).unwrap()).unwrap();
    assert_eq!(listing.files, staged.files());
    std::fs::remove_file(out.join("config/vm.args")).unwrap();
    assert!(
        matches!(staged.refresh(), Err(assemble::AssembleError::Io { path, .. }) if path == out.join("config/vm.args"))
    );
}

#[test]
fn staged_generated_file_collisions_leave_existing_bytes_and_listing_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let (apps, otp) = stage_inputs(dir.path());
    let out = dir.path().join("out");
    let mut staged = assemble::stage(&apps, &otp, &StageOptions::default(), &out).unwrap();
    staged
        .add_file(
            "settings",
            b"caller settings",
            0o644,
            assemble::Category::Other,
        )
        .unwrap();
    let listing = std::fs::read(out.join(assemble::LISTING_NAME)).unwrap();
    assert!(matches!(
        staged.add_file("settings/nested", b"new", 0o644, assemble::Category::Other),
        Err(assemble::AssembleError::Io { .. })
    ));
    assert_eq!(
        std::fs::read(out.join("settings")).unwrap(),
        b"caller settings"
    );
    std::fs::create_dir(out.join("directory")).unwrap();
    assert!(matches!(
        staged.add_file("directory", b"new", 0o644, assemble::Category::Other),
        Err(assemble::AssembleError::Io { .. })
    ));
    assert_eq!(
        std::fs::read(out.join(assemble::LISTING_NAME)).unwrap(),
        listing
    );
}

#[cfg(windows)]
fn deny_shared_access(path: &Path) -> std::fs::File {
    use std::os::windows::fs::OpenOptionsExt as _;
    std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(path)
        .unwrap()
}

#[cfg(windows)]
#[test]
fn staging_reports_the_locked_source_and_cleans_only_its_own_unpublished_tree() {
    let dir = tempfile::tempdir().unwrap();
    let (apps, otp) = stage_inputs(dir.path());
    for relative in ["erts-17.0.5/bin/beam.smp", "bin/no_dot_erlang.boot"] {
        let source = otp.root.join(relative);
        let original = std::fs::read(&source).unwrap();
        let locked = deny_shared_access(&source);
        let out = dir.path().join("out");
        let error = assemble::stage(&apps, &otp, &StageOptions::default(), &out).unwrap_err();
        match error {
            assemble::AssembleError::Copy { from, .. } => assert_eq!(from, source),
            assemble::AssembleError::Io { path, .. } => assert_eq!(path, source),
            other => panic!("locked source must be identified: {other}"),
        }
        assert!(!out.exists());
        assert!(!std::fs::read_dir(dir.path()).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("out.tmp-")
        }));
        drop(locked);
        assert_eq!(std::fs::read(source).unwrap(), original);
    }
    assemble::stage(
        &apps,
        &otp,
        &StageOptions::default(),
        &dir.path().join("out"),
    )
    .expect("retry after the external lock is released");
}

#[cfg(windows)]
#[test]
fn locked_generated_files_and_listings_report_the_actual_path_and_can_be_retried() {
    let dir = tempfile::tempdir().unwrap();
    let (apps, otp) = stage_inputs(dir.path());
    let out = dir.path().join("out");
    let mut staged = assemble::stage(&apps, &otp, &StageOptions::default(), &out).unwrap();
    staged
        .add_file("vm.args", b"old", 0o644, assemble::Category::Other)
        .unwrap();
    let locked = deny_shared_access(&out.join("vm.args"));
    assert!(
        matches!(staged.add_file("vm.args", b"new", 0o644, assemble::Category::Other), Err(assemble::AssembleError::Io { path, .. }) if path == out.join("vm.args"))
    );
    drop(locked);
    assert_eq!(std::fs::read(out.join("vm.args")).unwrap(), b"old");
    let locked = deny_shared_access(&out.join(assemble::LISTING_NAME));
    assert!(
        matches!(staged.refresh(), Err(assemble::AssembleError::Io { path, .. }) if path == out.join(assemble::LISTING_NAME))
    );
    drop(locked);
    staged
        .refresh()
        .expect("listing retry after releasing the outside reader");
}
