// SPDX-License-Identifier: MIT OR Apache-2.0
//! A Windows artifact was assembled without `inet_gethost.exe`, and the whole
//! Windows staging path — the flavour dispatch, the copy, the `erl.ini`
//! removal and its junk accounting — was reached by no test at all.
//!
//! Two defects, one path.
//!
//! `windows_required_bins` kept the launch program and every `*.dll` beside
//! it, and nothing else. `inet_gethost` is in
//! `otp::REQUIRED_ERTS_BINARIES` on unix — "every one of them must exist under
//! `erts-<vsn>/bin`" — because without it the runtime resolves no host name,
//! and a Windows tree ships the same port program as `inet_gethost.exe`. The
//! probe left it behind and a unit test pinned that answer as correct.
//!
//! And `stage_erts_bins`' flavour dispatch, the `remove_windows_erl_ini` call
//! and the `junk_removed` push were exercised only through their two leaf
//! helpers, against the *source* tree. Deleting the dispatch, or the removal,
//! or the accounting broke nothing. This test drives `assemble::stage` itself,
//! which is where all three live.
//!
//! The `erl.ini` bookkeeping was wrong in the same place: `--extra-bin
//! erl.ini` copied the file into `staged_bins`, the excluded list was computed
//! as the complement *before* the removal, and the removal then deleted it. A
//! user saw a file reported as staged, absent from the index, and present in
//! `junk_removed`.
#![cfg(feature = "cli")]

use std::path::{Path, PathBuf};

use ginary::assemble::{
    self, AssembleError, StageOptions, StagedRoot, WINDOWS_EMULATOR_DLL, WINDOWS_ERL_INI,
    WINDOWS_LAUNCH_BINARY,
};
use ginary::closure::AppSet;
use ginary::otp::OtpInfo;

use crate::common::fake_otp::{
    DEFAULT_ERTS_VSN, DEFAULT_OTP_VERSION, DEFAULT_RELEASE, FakeOtp, FakeOtpRoot, FakeShipment,
};

/// The name a Windows runtime ships the resolver port program under.
///
/// Spelled here rather than taken from the crate, so that the test states the
/// name it is about rather than agreeing with whatever the code says.
const RESOLVER: &str = "inet_gethost.exe";

/// Both trees, and the directory assembly writes into.
struct Trees {
    dir: tempfile::TempDir,
    shipment: PathBuf,
    otp: FakeOtpRoot,
}

impl Trees {
    /// Writes a runtime of the given flavour and an empty shipment beside it.
    fn new(otp: FakeOtp) -> Self {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let shipment = dir.path().join("shipment");
        FakeShipment::new().build_in(&shipment);
        std::fs::create_dir_all(dir.path().join("otp")).expect("the otp root");
        let otp = otp.build_in(dir.path().join("otp"));
        std::fs::create_dir_all(dir.path().join("work")).expect("the work directory");
        Self { dir, shipment, otp }
    }

    /// Where the staging root goes.
    fn out(&self) -> PathBuf {
        self.dir.path().join("work").join("out")
    }

    /// The runtime as assembly reads it.
    ///
    /// Built by hand rather than through `otp::inspect_root`, because a
    /// Windows root is one no unix tool can produce and `inspect_root` would
    /// refuse it for the four unix programs it does not hold. Everything
    /// assembly reads off an `OtpInfo` is a path or a version, and every one of
    /// them is what the builder wrote.
    fn otp_info(&self) -> OtpInfo {
        OtpInfo {
            root: self.otp.root.clone(),
            release: DEFAULT_RELEASE,
            erts_vsn: DEFAULT_ERTS_VSN.to_owned(),
            otp_version: DEFAULT_OTP_VERSION.to_owned(),
            erts_bin: self.otp.erts_bin(),
            lib: self.otp.lib(),
        }
    }

    /// The closure `stdlib` seeds, which is `kernel` and `stdlib`.
    fn set(&self) -> AppSet {
        ginary::closure::app_dependency_closure(
            &self.shipment,
            &self.otp.lib(),
            &["stdlib".to_owned()],
            &[],
        )
        .expect("the seeded root resolves")
    }

    /// Stages that closure with the given extra programs.
    fn stage(&self, extra_bins: &[&str]) -> Result<StagedRoot, AssembleError> {
        assemble::stage(
            &self.set(),
            &self.otp_info(),
            &StageOptions {
                extra_bins: extra_bins.iter().map(|name| (*name).to_owned()).collect(),
                remove_junk: true,
                force: true,
            },
            &self.out(),
        )
    }
}

/// The names in the staged `erts-<vsn>/bin`, sorted.
fn staged_bin_names(root: &Path) -> Vec<String> {
    let bin = root.join(format!("erts-{DEFAULT_ERTS_VSN}")).join("bin");
    let mut names: Vec<String> = std::fs::read_dir(&bin)
        .unwrap_or_else(|error| panic!("cannot list {}: {error}", bin.display()))
        .map(|entry| entry.expect("a directory entry").file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn a_staged_windows_runtime_carries_the_resolver_the_unix_list_requires() {
    let trees = Trees::new(
        FakeOtp::new()
            .windows()
            .extra_erts_bins(&["ei.dll", "werl.exe"]),
    );
    let staged = trees.stage(&[]).expect("a whole Windows tree stages");

    assert_eq!(
        staged_bin_names(staged.root()),
        vec![
            WINDOWS_EMULATOR_DLL.to_owned(),
            "ei.dll".to_owned(),
            WINDOWS_LAUNCH_BINARY.to_owned(),
            RESOLVER.to_owned(),
        ],
        "`inet_gethost` is required on unix because without it the runtime \
         resolves no host name, and a Windows tree ships the same port program"
    );
    assert!(
        !staged
            .root()
            .join(format!("erts-{DEFAULT_ERTS_VSN}/bin/{WINDOWS_ERL_INI}"))
            .exists(),
        "an `erl.ini` in the artifact points `erl.exe` at the build machine's Rootdir"
    );
    assert!(
        !staged_bin_names(staged.root()).contains(&"werl.exe".to_owned()),
        "and a program nothing needs is left behind the way the unix tree's are"
    );
}

#[test]
fn a_windows_tree_without_the_resolver_is_refused_by_name() {
    let trees = Trees::new(FakeOtp::new().windows());
    std::fs::remove_file(trees.otp.erts_bin().join(RESOLVER)).expect("remove the resolver");

    match trees.stage(&[]) {
        Err(AssembleError::MissingErtsBinary { name, searched }) => {
            assert_eq!(name, RESOLVER);
            assert_eq!(
                searched,
                trees.otp.erts_bin().join(RESOLVER),
                "the error names the path that was looked at"
            );
        }
        other => panic!(
            "a Windows tree with no resolver is an artifact that cannot resolve \
             a host name, and this answered {other:?}"
        ),
    }
}

#[test]
fn the_erl_ini_an_extra_bin_asked_for_is_removed_and_not_reported_as_staged() {
    let trees = Trees::new(FakeOtp::new().windows());
    let ini = trees.otp.erts_bin().join(WINDOWS_ERL_INI);
    let size = std::fs::metadata(&ini).expect("the fixture erl.ini").len();

    let staged = trees
        .stage(&[WINDOWS_ERL_INI])
        .expect("the build does not stop over it");

    assert!(
        !staged_bin_names(staged.root()).contains(&WINDOWS_ERL_INI.to_owned()),
        "the file is deleted from the staged tree whatever asked for it"
    );
    assert!(
        staged.junk_removed().contains(&(
            PathBuf::from(format!("erts-{DEFAULT_ERTS_VSN}"))
                .join("bin")
                .join(WINDOWS_ERL_INI),
            size,
        )),
        "the removal is in the account, with its size and the relative path \
         every other entry there carries: {:?}",
        staged.junk_removed()
    );
    assert!(
        staged
            .excluded_erts_bins()
            .iter()
            .any(|excluded| excluded.name == WINDOWS_ERL_INI),
        "and the complement of what was staged names it, rather than reporting \
         a file that is in neither the tree nor the index as staged"
    );
}

#[test]
fn a_unix_tree_is_staged_as_a_unix_tree_however_it_was_asked_for() {
    // The other half of the flavour dispatch: it is read off the directory,
    // not off the requested target, so a unix runtime never stages the Windows
    // list and a Windows one never stages the unix list.
    let trees = Trees::new(FakeOtp::new());
    let staged = trees.stage(&[]).expect("a whole unix tree stages");

    assert_eq!(
        staged_bin_names(staged.root()),
        vec![
            "beam.smp".to_owned(),
            "erl_child_setup".to_owned(),
            "erlexec".to_owned(),
            "inet_gethost".to_owned(),
        ],
        "the unix required list, unchanged by anything the Windows arm added"
    );
}
