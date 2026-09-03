// SPDX-License-Identifier: MIT OR Apache-2.0
//! The build half of Windows support: what a Windows ERTS tree has to hold,
//! what assembly deletes out of it, which runtime sources may produce one, and
//! the scaffolding around all of it.
//!
//! The tree these tests read is a `FakeOtp::windows()`, which writes `erl.exe`,
//! `beam.smp.dll`, `inet_gethost.exe` and `erl.ini` as plain files. That is the honest fixture: there is no
//! Windows runtime on this machine and no way to make one, and every claim
//! here is about *which names are in a directory* rather than about what any
//! of them does. The one claim that needs a real `otp_win64_<version>.zip` —
//! that the names in it are these names — is the GitHub Actions milestone, and
//! `docs/dev/log/D2.md` records it as an open question rather than as a fact.
//!
//! The last five tests are the same shape as `tests/smoke_matrix.rs`: a task
//! and four documents that nothing else would notice going stale.
// Every claim is about the build side, which a launcher-only stub does not
// carry.
#![cfg(feature = "cli")]

mod common;

use std::path::PathBuf;

use common::fake_otp::FakeOtp;

use ginary::assemble::{
    self, AssembleError, WINDOWS_EMULATOR_DLL, WINDOWS_ERL_INI, WINDOWS_LAUNCH_BINARY,
    WINDOWS_RESOLVER_PROGRAM,
};
use ginary::bundle::{self, BundleError, WINDOWS_ERTS_FROM_CATALOG};
use ginary::erts_source::ErtsSourceSpec;
use ginary::target::{Arch, Libc, Os, Target};

/// The target every refusal in this file is about.
fn windows() -> Target {
    Target::new(Os::Windows, Arch::X86_64, Libc::None)
}

/// The repository root.
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Reads a repository file as text.
///
/// # Panics
///
/// If the file is not there, which is what these tests are about.
fn read(relative: &str) -> String {
    let path = root().join(relative);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()))
}

// ------------------------------------------ the Windows ERTS bin --

#[test]
fn a_windows_erts_bin_contributes_erl_exe_and_every_dll_beside_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let otp = FakeOtp::new()
        .windows()
        .extra_erts_bins(&["ei.dll", "erlsrv.exe", "werl.exe"])
        .build_in(dir.path());

    let bins = assemble::windows_required_bins(&otp.erts_bin()).expect("a whole Windows tree");
    assert_eq!(
        bins,
        vec![
            WINDOWS_EMULATOR_DLL.to_owned(),
            "ei.dll".to_owned(),
            WINDOWS_LAUNCH_BINARY.to_owned(),
            WINDOWS_RESOLVER_PROGRAM.to_owned(),
        ],
        "the three required names and every DLL travel, sorted; `erl.ini`, `erlsrv.exe` and \
         `werl.exe` are left behind the way the unix tree's spare programs are"
    );
}

#[test]
fn a_windows_erts_bin_without_erl_exe_is_refused_by_name() {
    let dir = tempfile::tempdir().expect("tempdir");
    let otp = FakeOtp::new().windows().build_in(dir.path());
    let missing = otp.erts_bin().join(WINDOWS_LAUNCH_BINARY);
    std::fs::remove_file(&missing).expect("remove erl.exe");

    match assemble::windows_required_bins(&otp.erts_bin()) {
        Err(AssembleError::MissingErtsBinary { name, searched }) => {
            assert_eq!(name, WINDOWS_LAUNCH_BINARY);
            assert_eq!(
                searched, missing,
                "the error names the path that was looked at"
            );
        }
        other => panic!("a tree with no erl.exe is not a runtime, and this answered {other:?}"),
    }
}

#[test]
fn a_windows_erts_bin_without_the_emulator_dll_is_refused_by_name() {
    let dir = tempfile::tempdir().expect("tempdir");
    let otp = FakeOtp::new().windows().build_in(dir.path());
    let missing = otp.erts_bin().join(WINDOWS_EMULATOR_DLL);
    std::fs::remove_file(&missing).expect("remove beam.smp.dll");

    match assemble::windows_required_bins(&otp.erts_bin()) {
        Err(AssembleError::MissingErtsBinary { name, searched }) => {
            assert_eq!(
                name, WINDOWS_EMULATOR_DLL,
                "`erl.exe` loads the emulator in process, so a tree without it starts nothing"
            );
            assert_eq!(searched, missing);
        }
        other => panic!("a tree with no emulator is not a runtime, and this answered {other:?}"),
    }
}

#[test]
fn a_windows_erts_bin_without_the_resolver_is_refused_by_name() {
    let dir = tempfile::tempdir().expect("tempdir");
    let otp = FakeOtp::new().windows().build_in(dir.path());
    let missing = otp.erts_bin().join(WINDOWS_RESOLVER_PROGRAM);
    std::fs::remove_file(&missing).expect("remove inet_gethost.exe");

    match assemble::windows_required_bins(&otp.erts_bin()) {
        Err(AssembleError::MissingErtsBinary { name, searched }) => {
            assert_eq!(
                name, WINDOWS_RESOLVER_PROGRAM,
                "`inet_gethost` is required on unix because a runtime without it resolves no \
                 host name, and a Windows tree ships the same port program"
            );
            assert_eq!(searched, missing);
        }
        other => panic!(
            "a tree with no resolver ships a runtime that cannot resolve, and this \
                         answered {other:?}"
        ),
    }
}

#[test]
fn the_erl_ini_beside_erl_exe_is_removed_and_its_size_accounted_for() {
    let dir = tempfile::tempdir().expect("tempdir");
    let otp = FakeOtp::new().windows().build_in(dir.path());
    let ini = otp.erts_bin().join(WINDOWS_ERL_INI);
    let size = std::fs::metadata(&ini).expect("the fake erl.ini").len();
    assert!(size > 0, "the fixture has to have written one");

    assert_eq!(
        assemble::remove_windows_erl_ini(&otp.erts_bin()).expect("the removal"),
        Some(size),
        "the size is the junk account's: a removal nobody can see the cost of is a removal \
         nobody can explain"
    );
    assert!(
        !ini.exists(),
        "an `erl.ini` in the artifact points `erl.exe` at the build machine's Rootdir"
    );
    assert_eq!(
        assemble::remove_windows_erl_ini(&otp.erts_bin()).expect("the second removal"),
        None,
        "a tree that never had one is the ordinary case, not a failure"
    );
}

// ------------------------------------------ where a Windows runtime comes from --

#[test]
fn a_windows_build_may_only_take_its_runtime_from_a_directory() {
    let unpacked = ErtsSourceSpec::Dir(PathBuf::from("/srv/otp_win64_29.0.5"));
    assert!(
        bundle::check_windows_erts(windows(), &unpacked, Os::Linux).is_ok(),
        "a tree somebody unpacked from the upstream zip is the one source that can hold one"
    );

    for spec in [
        ErtsSourceSpec::Host,
        ErtsSourceSpec::Catalog,
        ErtsSourceSpec::Tarball(PathBuf::from("/srv/otp-29.0.5-linux-x86_64-gnu.tar.zst")),
        ErtsSourceSpec::Docker("erlang:29".to_owned()),
    ] {
        let label = spec.label();
        match bundle::check_windows_erts(windows(), &spec, Os::Linux) {
            Err(BundleError::WindowsErtsUnavailable {
                target,
                spec: named,
            }) => {
                assert_eq!(target, windows());
                assert_eq!(
                    named, label,
                    "the refusal quotes the source as it was spelled"
                );
                let message = BundleError::WindowsErtsUnavailable {
                    target,
                    spec: named,
                }
                .to_string();
                assert!(
                    message.contains(WINDOWS_ERTS_FROM_CATALOG),
                    "the refusal says where a Windows runtime comes from; it said: {message}"
                );
            }
            other => panic!("`{label}` cannot hold a Windows runtime, and this answered {other:?}"),
        }
    }

    // The target is named rather than asked of the machine: on a Windows
    // runner `Target::host()` *is* the Windows target, and the claim this
    // line makes would be the opposite of the one it is written to make.
    let linux = Target::new(Os::Linux, Arch::X86_64, Libc::Gnu);
    assert!(
        bundle::check_windows_erts(linux, &ErtsSourceSpec::Host, Os::Linux).is_ok(),
        "this check has nothing to say about a target that is not Windows"
    );
}

// ------------------------------------------ the scaffolding --

#[test]
fn the_windows_build_task_builds_both_flavors_for_the_windows_triple() {
    let mise = read("mise.toml");
    let header = "[tasks.\"build:windows\"]";
    let start = mise
        .find(header)
        .unwrap_or_else(|| panic!("mise.toml has to hold {header}"));
    let rest = &mise[start + header.len()..];
    let task = rest.find("\n[tasks.").map_or(rest, |end| &rest[..end]);

    for needle in [
        "x86_64-pc-windows-gnu",
        "--no-default-features",
        "--release",
    ] {
        assert!(
            task.contains(needle),
            "`build:windows` has to run the cross build with {needle}; it holds: {task}"
        );
    }
    assert_eq!(
        task.matches("cross build").count(),
        2,
        "both flavors are built: the launcher-only stub and the full command line tool, because \
         a cfg split that only compiles one of them is half a split"
    );
}

#[test]
fn the_readme_records_what_windows_support_covers_and_what_is_untested() {
    let readme = read("README.md");
    let start = readme
        .find("\n## Windows")
        .expect("README.md needs a `## Windows` section stating where support stands");
    let rest = &readme[start + 1..];
    let section = rest.find("\n## ").map_or(rest, |end| &rest[..end]);

    for needle in ["cross", "stub", "erl.exe", "GitHub Actions"] {
        assert!(
            section.contains(needle),
            "the Windows section has to say what works and what has never been run: it mentions \
             no `{needle}`"
        );
    }
}

#[test]
fn the_debugging_guide_documents_the_windows_cache_root_and_lock_semantics() {
    let doc = read("docs/dev/debugging.md");
    for needle in ["LOCALAPPDATA", "%TEMP%", "FILE_SHARE_READ"] {
        assert!(
            doc.contains(needle),
            "docs/dev/debugging.md has to say where a Windows cache is and what holds it: it \
             mentions no `{needle}`"
        );
    }
}

#[test]
fn the_windows_launcher_adr_is_committed() {
    let adr = read("docs/adr/0015-windows-launcher-stays-resident.md");
    for needle in [
        "execve",
        "job object",
        "SetConsoleCtrlHandler",
        "FILE_SHARE_READ",
    ] {
        assert!(
            adr.contains(needle),
            "the ADR has to record why the launcher stays alive: it mentions no `{needle}`"
        );
    }
}

#[test]
fn the_windows_launcher_adr_is_in_the_index() {
    let index = read("docs/adr/README.md");
    assert!(
        index.contains("0015-windows-launcher-stays-resident.md"),
        "an ADR nobody links to is an ADR nobody reads"
    );
}
