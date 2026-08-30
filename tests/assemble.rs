// SPDX-License-Identifier: MIT OR Apache-2.0
//! The staging root: the exact tree an artifact is made of.
//!
//! Every test here builds its two inputs with [`FakeShipment`] and [`FakeOtp`]
//! in a temporary directory, so the whole file runs in milliseconds with no
//! Erlang installed. `tests/stage_run.rs` is the other half: it stages a real
//! shipment against the host runtime and then boots it.
//!
//! Assembly is the first module whose output is a *tree* rather than a value,
//! and the assertions are written accordingly. A test names the paths it
//! expects, in full and in order, rather than checking that a few of them are
//! there: the whole point of an allowlist is that what is absent is as
//! deliberate as what is present, and an assertion that only looks at what is
//! present cannot see a file that should not have been copied.

mod common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use ginary::assemble::{
    self, AssembleError, Category, LISTING_NAME, StageListing, StageOptions, StagedRoot,
    StagedSource,
};
use ginary::closure::{AppSet, app_dependency_closure};
use ginary::otp::OtpInfo;
use tempfile::TempDir;

use crate::common::fake_otp::{
    DEFAULT_ERTS_VSN, DEFAULT_KERNEL_VSN, DEFAULT_OTP_VERSION, DEFAULT_RELEASE, DEFAULT_STDLIB_VSN,
    FakeOtp, FakeOtpRoot, FakeShipment, FakeShipmentRoot, boot_bytes_for, make_executable,
};

/// The six-application scenario the whole file is written against.
///
/// Three applications come from the shipment and three from the OTP library,
/// which is the smallest scenario that shows both staged layouts, both `.beam`
/// categories, a `priv` on each side, and every kind of file assembly refuses
/// to copy.
const EXPECTED_TREE: [&str; 22] = [
    "bin/no_dot_erlang.boot",
    "erts-17.0.5/bin/beam.smp",
    "erts-17.0.5/bin/erl_child_setup",
    "erts-17.0.5/bin/erlexec",
    "erts-17.0.5/bin/inet_gethost",
    "ginary.stage.json",
    "lib/crypto-5.9.2/ebin/crypto.app",
    "lib/crypto-5.9.2/ebin/crypto.beam",
    "lib/crypto-5.9.2/priv/lib/crypto.so",
    "lib/gleam_crypto/ebin/gleam_crypto.app",
    "lib/gleam_crypto/ebin/gleam_crypto.beam",
    "lib/gleam_stdlib/ebin/gleam@list.beam",
    "lib/gleam_stdlib/ebin/gleam_stdlib.app",
    "lib/gleam_stdlib/ebin/gleam_stdlib.beam",
    "lib/kernel-11.0.3/ebin/kernel.app",
    "lib/kernel-11.0.3/ebin/kernel.beam",
    "lib/notify/ebin/notify.app",
    "lib/notify/ebin/notify.beam",
    "lib/notify/ebin/notify@@main.beam",
    "lib/notify/priv/greeting.txt",
    "lib/stdlib-8.0.3/ebin/stdlib.app",
    "lib/stdlib-8.0.3/ebin/stdlib.beam",
];

/// The programs the scenario's runtime holds beyond the required four.
const SPARE_ERTS_BINS: [&str; 6] = ["epmd", "erl", "erlc", "escript", "heart", "run_erl"];

/// The contents of the junk files, whose lengths the removal report names.
const TEST_ENGINE: &[u8] = b"a fake test engine";
const STATIC_ARCHIVE: &[u8] = b"a fake static archive";
const OBJECT_FILE: &[u8] = b"a fake object file";
const GREETING: &[u8] = b"hello from priv\n";
const FAKE_NIF: &[u8] = b"a fake NIF";

/// A shipment and a runtime side by side in one temporary directory.
///
/// The same shape `tests/closure.rs` uses, plus an output directory: all three
/// live under one root so that a single `TempDir` deletes everything the test
/// wrote, including a staging root that a failing assertion left behind.
struct Trees {
    dir: TempDir,
    shipment: FakeShipmentRoot,
    otp: FakeOtpRoot,
}

impl Trees {
    /// Writes both trees, the shipment at `<tmp>/shipment` and the runtime at
    /// `<tmp>/otp`. The output directory is `<tmp>/work/out` and is not
    /// created: assembly is what creates it.
    fn new(shipment: FakeShipment, otp: FakeOtp) -> Self {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let shipment = shipment.build_in(dir.path().join("shipment"));
        std::fs::create_dir_all(dir.path().join("otp")).expect("the otp root");
        let otp = otp.build_in(dir.path().join("otp"));
        std::fs::create_dir_all(dir.path().join("work")).expect("the work directory");
        Self { dir, shipment, otp }
    }

    /// The directory `out` and any temporary staging directory live in.
    fn work(&self) -> PathBuf {
        self.dir.path().join("work")
    }

    /// Where the staging root goes.
    fn out(&self) -> PathBuf {
        self.work().join("out")
    }

    /// The runtime as `ginary::otp` sees it.
    fn otp_info(&self) -> OtpInfo {
        ginary::otp::inspect_root(&self.otp.root).expect("the fake root is a usable OTP root")
    }

    /// The closure over both trees, or a panic naming the error.
    fn closed(&self, roots: &[&str], extra: &[&str]) -> AppSet {
        let roots: Vec<String> = roots.iter().map(|name| (*name).to_owned()).collect();
        let extra: Vec<String> = extra.iter().map(|name| (*name).to_owned()).collect();
        match app_dependency_closure(&self.shipment.root, &self.otp.lib(), &roots, &extra) {
            Ok(set) => set,
            Err(error) => panic!("the closure should resolve: {error}"),
        }
    }

    /// Stages `notify` into `<tmp>/work/out` with the given options.
    fn stage(&self, opts: &StageOptions) -> Result<StagedRoot, AssembleError> {
        assemble::stage(
            &self.closed(&["notify"], &[]),
            &self.otp_info(),
            opts,
            &self.out(),
        )
    }

    /// Stages `notify` with the default options, or panics naming the error.
    fn staged(&self) -> StagedRoot {
        self.staged_with(&StageOptions::default())
    }

    /// Stages `notify`, or panics naming the error.
    fn staged_with(&self, opts: &StageOptions) -> StagedRoot {
        match self.stage(opts) {
            Ok(root) => root,
            Err(error) => panic!("staging should succeed: {error}"),
        }
    }
}

/// The six-application scenario.
///
/// `notify` is the root and pulls `gleam_stdlib` and `gleam_crypto` from the
/// shipment; `gleam_crypto` pulls `crypto` from the runtime; `kernel` and
/// `stdlib` are always there. The runtime carries six spare programs in its
/// `bin`, and `crypto` carries one real NIF and three pieces of junk.
///
/// Everything assembly must refuse to copy is written afterwards, by hand: a
/// `.appup` beside an `.app`, and one directory of each excluded name. The
/// builders deliberately cannot write those, because a test that needs an
/// unusual tree should be visibly building one.
fn six_apps() -> Trees {
    let shipment = FakeShipment::new()
        .app_with("notify", "1.0.0", |app| {
            app.applications(&["gleam_stdlib", "gleam_crypto"])
                .modules(&["notify", "notify@@main"])
                .priv_file("greeting.txt", GREETING)
        })
        .app_with("gleam_stdlib", "0.62.0", |app| {
            app.modules(&["gleam_stdlib", "gleam@list"])
        })
        .app("gleam_crypto", "1.4.0", &["crypto"]);

    let otp = FakeOtp::new()
        .extra_erts_bins(&SPARE_ERTS_BINS)
        .app_with("crypto", "5.9.2", |app| {
            app.applications(&["kernel", "stdlib"])
                .priv_file("lib/crypto.so", FAKE_NIF)
                .priv_file("lib/otp_test_engine.so", TEST_ENGINE)
                .priv_file("lib/libcrypto_static.a", STATIC_ARCHIVE)
                .priv_file("obj/crypto_callback.o", OBJECT_FILE)
        });

    let trees = Trees::new(shipment, otp);

    let crypto = trees.otp.app_dir("crypto");
    make_executable(&crypto.join("priv/lib/crypto.so"));
    write(&crypto.join("ebin/crypto.appup"), b"{\"5.9.2\", [], []}.\n");
    for (dir, file) in [
        ("src", "crypto.erl"),
        ("include", "crypto.hrl"),
        ("doc", "crypto.html"),
        ("examples", "demo.erl"),
        ("c_src", "crypto.c"),
        ("mibs", "OTP-CRYPTO.mib"),
    ] {
        write(&crypto.join(dir).join(file), b"not for the artifact\n");
    }
    write(
        &trees.shipment.app_dir("notify").join("src/notify.gleam"),
        b"pub fn main() { Nil }\n",
    );

    trees
}

/// Writes a file and every directory above it, failing the test if it cannot.
fn write(path: &Path, bytes: &[u8]) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .unwrap_or_else(|error| panic!("cannot create {}: {error}", parent.display()));
    }
    std::fs::write(path, bytes)
        .unwrap_or_else(|error| panic!("cannot write {}: {error}", path.display()));
}

/// Every file under `root`, as sorted `/`-separated relative paths.
///
/// Directories are not listed: an empty directory carries no bytes and is not
/// part of what the payload will hold.
fn walk(root: &Path) -> Vec<String> {
    let mut found = Vec::new();
    collect(root, root, &mut found);
    found.sort();
    found
}

/// The recursive half of [`walk`].
fn collect(root: &Path, dir: &Path, found: &mut Vec<String>) {
    let entries = std::fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", dir.display()));
    for entry in entries {
        let path = entry
            .unwrap_or_else(|error| panic!("cannot read an entry of {}: {error}", dir.display()))
            .path();
        if path.is_dir() {
            collect(root, &path, found);
        } else {
            let relative = path
                .strip_prefix(root)
                .unwrap_or_else(|_| panic!("{} is not under {}", path.display(), root.display()));
            found.push(relative.to_string_lossy().replace('\\', "/"));
        }
    }
}

/// The names of a directory's entries, sorted.
fn read_dir_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", dir.display()))
        .map(|entry| {
            entry
                .unwrap_or_else(|error| {
                    panic!("cannot read an entry of {}: {error}", dir.display())
                })
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

/// The Unix permission bits of a file.
#[cfg(unix)]
fn mode_of(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::metadata(path)
        .unwrap_or_else(|error| panic!("cannot stat {}: {error}", path.display()))
        .permissions()
        .mode()
        & 0o7777
}

/// The staged file listing, keyed by path.
fn by_path(staged: &StagedRoot) -> BTreeMap<&str, &ginary::assemble::StagedFile> {
    staged
        .files()
        .iter()
        .map(|file| (file.path.as_str(), file))
        .collect()
}

/// Junk removal as `(path, bytes)` strings, for a readable assertion.
fn junk(staged: &StagedRoot) -> Vec<(String, u64)> {
    staged
        .junk_removed()
        .iter()
        .map(|(path, bytes)| (path.display().to_string(), *bytes))
        .collect()
}

#[test]
fn the_staged_tree_holds_exactly_the_expected_paths() {
    let trees = six_apps();
    let staged = trees.staged();

    assert_eq!(walk(staged.root()), EXPECTED_TREE);
}

#[test]
fn the_staged_root_is_the_directory_that_was_asked_for() {
    let trees = six_apps();
    let staged = trees.staged();

    assert_eq!(staged.root(), trees.out());
    assert!(trees.out().is_dir());
}

#[test]
fn an_appup_beside_an_app_file_is_not_staged() {
    let trees = six_apps();
    let staged = trees.staged();

    assert!(
        trees
            .otp
            .app_dir("crypto")
            .join("ebin/crypto.appup")
            .is_file(),
        "the source tree should hold the .appup this test is about"
    );
    assert!(
        !staged
            .root()
            .join("lib/crypto-5.9.2/ebin/crypto.appup")
            .exists(),
        "an .appup is a release upgrade instruction and is never read at run time"
    );
}

#[test]
fn the_source_include_doc_examples_c_src_and_mibs_directories_are_not_staged() {
    let trees = six_apps();
    let staged = trees.staged();

    // The exclusion is structural and it applies at the top level of an
    // application, which is where those directories live: `ebin` and `priv`
    // are copied and nothing else is. Asserting the *names* at any depth would
    // pin a rule the module deliberately does not have — `priv/mibs/*.bin` is
    // runtime data and is staged; the test below that one says so.
    for app in staged.apps() {
        for excluded in assemble::EXCLUDED_APP_DIRS {
            let path = staged.root().join(&app.dir).join(excluded);
            assert!(
                !path.exists(),
                "{} was staged, and `{excluded}` never is",
                path.display()
            );
        }
        assert_eq!(
            read_dir_names(&staged.root().join(&app.dir)),
            ["ebin", "priv"]
                .iter()
                .filter(|name| staged.root().join(&app.dir).join(name).exists())
                .map(|name| (*name).to_owned())
                .collect::<Vec<String>>(),
            "an application holds `ebin` and `priv` and nothing else"
        );
    }
}

#[test]
fn a_mibs_directory_inside_priv_is_staged_because_the_exclusion_is_structural() {
    let trees = six_apps();
    // What `snmp` ships: compiled MIBs under `priv`, read at run time. The
    // application's own top-level `mibs/` is the source form and stays behind;
    // this one is data the application loads and must travel.
    write(
        &trees.otp.app_dir("crypto").join("priv/mibs/OTP-CRYPTO.bin"),
        b"a compiled mib\n",
    );

    let staged = trees.staged();

    assert!(
        staged
            .root()
            .join("lib/crypto-5.9.2/priv/mibs/OTP-CRYPTO.bin")
            .is_file(),
        "nothing inside `priv` is pruned by name"
    );
    assert!(
        !staged.root().join("lib/crypto-5.9.2/mibs").exists(),
        "the application's own top-level `mibs` is still left behind"
    );
}

#[test]
fn the_junk_files_are_removed_and_recorded_with_their_sizes() {
    let trees = six_apps();
    let staged = trees.staged();

    assert_eq!(
        junk(&staged),
        vec![
            (
                "lib/crypto-5.9.2/priv/lib/libcrypto_static.a".to_owned(),
                STATIC_ARCHIVE.len() as u64
            ),
            (
                "lib/crypto-5.9.2/priv/lib/otp_test_engine.so".to_owned(),
                TEST_ENGINE.len() as u64
            ),
            (
                "lib/crypto-5.9.2/priv/obj".to_owned(),
                OBJECT_FILE.len() as u64
            ),
        ]
    );
    for gone in [
        "lib/crypto-5.9.2/priv/lib/libcrypto_static.a",
        "lib/crypto-5.9.2/priv/lib/otp_test_engine.so",
        "lib/crypto-5.9.2/priv/obj/crypto_callback.o",
    ] {
        assert!(!staged.root().join(gone).exists(), "{gone} should be gone");
    }
    assert!(
        staged
            .root()
            .join("lib/crypto-5.9.2/priv/lib/crypto.so")
            .is_file(),
        "the real NIF beside the junk stays"
    );
}

#[test]
fn keep_junk_leaves_the_junk_in_place_and_records_nothing() {
    let trees = six_apps();
    let staged = trees.staged_with(&StageOptions {
        remove_junk: false,
        ..StageOptions::default()
    });

    assert_eq!(junk(&staged), Vec::new());
    for kept in [
        "lib/crypto-5.9.2/priv/lib/libcrypto_static.a",
        "lib/crypto-5.9.2/priv/lib/otp_test_engine.so",
        "lib/crypto-5.9.2/priv/obj/crypto_callback.o",
    ] {
        assert!(staged.root().join(kept).is_file(), "{kept} should be kept");
    }
}

#[cfg(unix)]
#[test]
fn a_priv_file_keeps_the_mode_it_had_in_the_source_tree() {
    let trees = six_apps();
    let staged = trees.staged();

    for (source, relative) in [
        (
            trees.otp.app_dir("crypto").join("priv/lib/crypto.so"),
            "lib/crypto-5.9.2/priv/lib/crypto.so",
        ),
        (
            trees.shipment.app_dir("notify").join("priv/greeting.txt"),
            "lib/notify/priv/greeting.txt",
        ),
    ] {
        assert_eq!(
            mode_of(&staged.root().join(relative)),
            mode_of(&source),
            "{relative} lost the mode of {}",
            source.display()
        );
    }

    assert_ne!(
        mode_of(&staged.root().join("lib/crypto-5.9.2/priv/lib/crypto.so")) & 0o111,
        0,
        "a NIF that is not executable cannot be loaded"
    );
    assert_eq!(
        mode_of(&staged.root().join("lib/notify/priv/greeting.txt")) & 0o111,
        0,
        "a data file must not gain an execute bit on the way in"
    );
}

#[cfg(unix)]
#[test]
fn every_staged_erts_binary_stays_executable() {
    let trees = six_apps();
    let staged = trees.staged();

    for name in ginary::otp::REQUIRED_ERTS_BINARIES {
        let path = staged.root().join(format!("erts-17.0.5/bin/{name}"));
        assert_eq!(
            mode_of(&path),
            mode_of(&trees.otp.erts_bin().join(name)),
            "{name} lost its mode"
        );
        assert_ne!(mode_of(&path) & 0o111, 0, "{name} is not executable");
    }
}

#[cfg(unix)]
#[test]
fn the_listing_records_the_mode_of_every_file_it_wrote() {
    let trees = six_apps();
    let staged = trees.staged();

    for file in staged.files() {
        assert_eq!(
            file.mode,
            mode_of(&staged.root().join(&file.path)),
            "the listing disagrees with the tree about {}",
            file.path
        );
    }
}

#[test]
fn a_boot_file_naming_a_version_that_is_not_staged_is_an_error() {
    let trees = six_apps();
    // The runtime's own boot file names `kernel-11.0.3`. This one names a
    // version nothing in the library has, which is what a boot file carried
    // over from another OTP installation looks like.
    write(
        &trees.otp.boot_file(),
        &boot_bytes_for(&["kernel-1.0", "stdlib-8.0.3"]),
    );

    let error = trees
        .stage(&StageOptions::default())
        .expect_err("a boot file that names a missing application is refused");

    match &error {
        AssembleError::BootReferencesMissingApp { dir, staged, boot } => {
            assert_eq!(dir, "kernel-1.0");
            assert_eq!(staged, &vec![format!("kernel-{DEFAULT_KERNEL_VSN}")]);
            assert_eq!(boot, &trees.otp.boot_file());
        }
        other => panic!("expected BootReferencesMissingApp, got {other:?}"),
    }

    let message = error.to_string();
    assert!(message.contains("kernel-1.0"), "{message}");
    assert!(
        message.contains(&format!("kernel-{DEFAULT_KERNEL_VSN}")),
        "the message has to name both halves of the mismatch: {message}"
    );
}

#[test]
fn a_missing_required_erts_binary_is_an_error() {
    let trees = six_apps();
    let otp = trees.otp_info();
    // Removed after inspection, because `otp::inspect_root` refuses a runtime
    // without it: this is the runtime changing under the build, not a runtime
    // that was never usable.
    let removed = trees.otp.erts_bin().join("inet_gethost");
    std::fs::remove_file(&removed).expect("the fake binary is removable");

    let error = assemble::stage(
        &trees.closed(&["notify"], &[]),
        &otp,
        &StageOptions::default(),
        &trees.out(),
    )
    .expect_err("a runtime missing a required binary cannot be staged");

    match &error {
        AssembleError::MissingErtsBinary { name, searched } => {
            assert_eq!(name, "inet_gethost");
            assert_eq!(searched, &removed);
        }
        other => panic!("expected MissingErtsBinary, got {other:?}"),
    }
}

#[test]
fn the_extra_binaries_are_staged_beside_the_required_four() {
    let trees = six_apps();
    let staged = trees.staged_with(&StageOptions {
        extra_bins: vec!["heart".to_owned(), "epmd".to_owned()],
        ..StageOptions::default()
    });

    let bins: Vec<String> = walk(&staged.root().join("erts-17.0.5/bin"));
    assert_eq!(
        bins,
        [
            "beam.smp",
            "epmd",
            "erl_child_setup",
            "erlexec",
            "heart",
            "inet_gethost"
        ]
    );
}

#[test]
fn an_extra_binary_the_runtime_does_not_have_is_an_error() {
    let trees = six_apps();

    let error = trees
        .stage(&StageOptions {
            extra_bins: vec!["zephyr".to_owned()],
            ..StageOptions::default()
        })
        .expect_err("an extra binary that is not there is refused rather than skipped");

    match &error {
        AssembleError::MissingExtraBinary { name } => assert_eq!(name, "zephyr"),
        other => panic!("expected MissingExtraBinary, got {other:?}"),
    }
}

#[test]
fn every_erts_binary_that_was_not_staged_is_listed_with_a_reason() {
    let trees = six_apps();
    let staged = trees.staged();

    let names: Vec<&str> = staged
        .excluded_erts_bins()
        .iter()
        .map(|bin| bin.name.as_str())
        .collect();
    assert_eq!(names, SPARE_ERTS_BINS);

    for bin in staged.excluded_erts_bins() {
        assert_eq!(
            bin.reason,
            assemble::excluded_reason(&bin.name),
            "the reason for `{}` is not the one the policy gives",
            bin.name
        );
        assert!(!bin.reason.is_empty());
    }
}

#[test]
fn an_extra_binary_is_not_also_reported_as_excluded() {
    let trees = six_apps();
    let staged = trees.staged_with(&StageOptions {
        extra_bins: vec!["heart".to_owned()],
        ..StageOptions::default()
    });

    let names: Vec<&str> = staged
        .excluded_erts_bins()
        .iter()
        .map(|bin| bin.name.as_str())
        .collect();
    assert_eq!(names, ["epmd", "erl", "erlc", "escript", "run_erl"]);
}

#[test]
fn a_non_empty_output_directory_is_an_error() {
    let trees = six_apps();
    write(&trees.out().join("something.txt"), b"not mine\n");

    let error = trees
        .stage(&StageOptions::default())
        .expect_err("an occupied output directory is refused");

    match &error {
        AssembleError::OutputNotEmpty { path } => assert_eq!(path, &trees.out()),
        other => panic!("expected OutputNotEmpty, got {other:?}"),
    }
    assert!(
        trees.out().join("something.txt").is_file(),
        "a refused staging must not have touched the directory"
    );
}

#[test]
fn an_empty_output_directory_is_accepted() {
    let trees = six_apps();
    std::fs::create_dir_all(trees.out()).expect("the output directory");

    let staged = trees.staged();

    assert_eq!(walk(staged.root()), EXPECTED_TREE);
}

#[test]
fn force_replaces_a_non_empty_output_directory() {
    let trees = six_apps();
    write(
        &trees.out().join("stale/leftover.txt"),
        b"from a previous run\n",
    );

    let staged = trees.staged_with(&StageOptions {
        force: true,
        ..StageOptions::default()
    });

    assert_eq!(walk(staged.root()), EXPECTED_TREE);
    assert!(
        !trees.out().join("stale").exists(),
        "--force replaces the directory rather than merging into it"
    );
}

#[cfg(unix)]
#[test]
fn a_symlink_inside_the_application_directory_is_copied_as_a_file() {
    let trees = six_apps();
    let notify = trees.shipment.app_dir("notify");
    std::os::unix::fs::symlink("greeting.txt", notify.join("priv/greeting-link.txt"))
        .expect("the symlink");

    let staged = trees.staged();

    let link = staged.root().join("lib/notify/priv/greeting-link.txt");
    assert!(
        !link
            .symlink_metadata()
            .expect("the staged entry")
            .is_symlink(),
        "a staged tree holds no symlinks; the payload format cannot carry them"
    );
    assert_eq!(std::fs::read(&link).expect("readable"), GREETING);
}

#[cfg(unix)]
#[test]
fn a_symlink_pointing_out_of_the_application_directory_is_refused() {
    let trees = six_apps();
    write(
        &trees.dir.path().join("outside.txt"),
        b"not part of the app\n",
    );
    let notify = trees.shipment.app_dir("notify");
    std::os::unix::fs::symlink("../../../outside.txt", notify.join("priv/escape.txt"))
        .expect("the symlink");

    let error = trees
        .stage(&StageOptions::default())
        .expect_err("a link out of the application is refused rather than followed");

    match &error {
        AssembleError::UnsafeSymlink { path, .. } => {
            assert_eq!(path, &notify.join("priv/escape.txt"));
        }
        other => panic!("expected UnsafeSymlink, got {other:?}"),
    }
}

#[cfg(unix)]
#[test]
fn a_dangling_symlink_is_refused() {
    let trees = six_apps();
    let notify = trees.shipment.app_dir("notify");
    std::os::unix::fs::symlink("nowhere.txt", notify.join("priv/dangling.txt"))
        .expect("the symlink");

    let error = trees
        .stage(&StageOptions::default())
        .expect_err("a dangling link is refused rather than skipped");

    match &error {
        AssembleError::UnsafeSymlink { path, .. } => {
            assert_eq!(path, &notify.join("priv/dangling.txt"));
        }
        other => panic!("expected UnsafeSymlink, got {other:?}"),
    }
}

#[test]
fn a_failed_staging_leaves_neither_an_output_nor_a_temporary_directory() {
    let trees = six_apps();
    let otp = trees.otp_info();
    // `inet_gethost` is the last of `otp::REQUIRED_ERTS_BINARIES` to be
    // copied, so the failure happens after the temporary tree already holds
    // files: this is the case where a careless implementation leaves
    // `<out>.tmp-<pid>` behind.
    std::fs::remove_file(trees.otp.erts_bin().join("inet_gethost"))
        .expect("the fake binary is removable");

    let error = assemble::stage(
        &trees.closed(&["notify"], &[]),
        &otp,
        &StageOptions::default(),
        &trees.out(),
    )
    .expect_err("a runtime missing a required binary cannot be staged");

    assert!(
        matches!(error, AssembleError::MissingErtsBinary { .. }),
        "expected MissingErtsBinary, got {error:?}"
    );
    assert!(!trees.out().exists(), "no half-built output directory");
    assert_eq!(
        walk(&trees.work()),
        Vec::<String>::new(),
        "no file is left behind under the work directory"
    );
    assert_eq!(
        read_dir_names(&trees.work()),
        Vec::<String>::new(),
        "and no `<out>.tmp-<pid>` directory either, empty or not"
    );
}

#[test]
fn the_category_totals_sum_to_the_total_bytes() {
    let trees = six_apps();
    let staged = trees.staged();

    let totals = staged.bytes_by_category();
    let bytes: u64 = totals.values().map(|(bytes, _)| bytes).sum();
    let files: usize = totals.values().map(|(_, files)| files).sum();

    assert_eq!(bytes, staged.total_bytes());
    assert_eq!(files, staged.files().len());
}

#[test]
fn the_total_bytes_are_the_size_of_the_tree_the_listing_aside() {
    let trees = six_apps();
    let staged = trees.staged();

    let counted: u64 = walk(staged.root())
        .into_iter()
        .filter(|path| path != LISTING_NAME)
        .map(|path| {
            std::fs::metadata(staged.root().join(&path))
                .unwrap_or_else(|error| panic!("cannot stat {path}: {error}"))
                .len()
        })
        .sum();

    assert_eq!(staged.total_bytes(), counted);
    assert_eq!(staged.files().len(), EXPECTED_TREE.len() - 1);
}

#[test]
fn every_file_is_put_in_the_category_the_report_will_add_up() {
    let trees = six_apps();
    let staged = trees.staged();
    let files = by_path(&staged);

    for (path, expected) in [
        ("bin/no_dot_erlang.boot", Category::Boot),
        ("erts-17.0.5/bin/beam.smp", Category::ErtsBinary),
        ("lib/crypto-5.9.2/ebin/crypto.app", Category::AppResource),
        ("lib/crypto-5.9.2/ebin/crypto.beam", Category::OtpBeam),
        ("lib/crypto-5.9.2/priv/lib/crypto.so", Category::Priv),
        (
            "lib/gleam_stdlib/ebin/gleam_stdlib.app",
            Category::AppResource,
        ),
        ("lib/gleam_stdlib/ebin/gleam@list.beam", Category::GleamBeam),
        ("lib/notify/ebin/notify@@main.beam", Category::GleamBeam),
        ("lib/notify/priv/greeting.txt", Category::Priv),
    ] {
        let file = files
            .get(path)
            .unwrap_or_else(|| panic!("`{path}` is not in the listing"));
        assert_eq!(file.category, expected, "{path}");
    }
}

#[test]
fn the_listing_lists_every_file_sorted_by_path_and_never_itself() {
    let trees = six_apps();
    let staged = trees.staged();

    let listed: Vec<&str> = staged
        .files()
        .iter()
        .map(|file| file.path.as_str())
        .collect();
    let expected: Vec<&str> = EXPECTED_TREE
        .iter()
        .copied()
        .filter(|path| *path != LISTING_NAME)
        .collect();
    assert_eq!(listed, expected);
}

#[test]
fn the_listing_names_the_erts_version_the_release_and_the_otp_version() {
    let trees = six_apps();
    let staged = trees.staged();
    let listing = staged.listing();

    assert_eq!(listing.erts_vsn, DEFAULT_ERTS_VSN);
    assert_eq!(listing.otp_release, DEFAULT_RELEASE);
    assert_eq!(listing.otp_version, DEFAULT_OTP_VERSION);
    assert_eq!(staged.erts_vsn(), DEFAULT_ERTS_VSN);
}

#[test]
fn the_listing_on_disk_round_trips_through_serde() {
    let trees = six_apps();
    let staged = trees.staged();

    let bytes = std::fs::read(staged.root().join(LISTING_NAME)).expect("the listing is written");
    let parsed: StageListing = serde_json::from_slice(&bytes).expect("the listing is valid JSON");

    assert_eq!(parsed, staged.listing());
}

#[test]
fn the_staged_applications_name_their_version_source_and_directory() {
    let trees = six_apps();
    let staged = trees.staged();

    let apps: Vec<(&str, &str, StagedSource, &str)> = staged
        .apps()
        .iter()
        .map(|app| {
            (
                app.name.as_str(),
                app.vsn.as_str(),
                app.source,
                app.dir.as_str(),
            )
        })
        .collect();

    assert_eq!(
        apps,
        vec![
            ("crypto", "5.9.2", StagedSource::Otp, "lib/crypto-5.9.2"),
            (
                "gleam_crypto",
                "1.4.0",
                StagedSource::Shipment,
                "lib/gleam_crypto"
            ),
            (
                "gleam_stdlib",
                "0.62.0",
                StagedSource::Shipment,
                "lib/gleam_stdlib"
            ),
            (
                "kernel",
                DEFAULT_KERNEL_VSN,
                StagedSource::Otp,
                "lib/kernel-11.0.3"
            ),
            ("notify", "1.0.0", StagedSource::Shipment, "lib/notify"),
            (
                "stdlib",
                DEFAULT_STDLIB_VSN,
                StagedSource::Otp,
                "lib/stdlib-8.0.3"
            ),
        ]
    );
}

#[test]
fn the_boot_references_that_were_checked_are_reported() {
    let trees = six_apps();
    let staged = trees.staged();

    assert_eq!(
        staged.boot_refs(),
        [
            format!("kernel-{DEFAULT_KERNEL_VSN}"),
            format!("stdlib-{DEFAULT_STDLIB_VSN}")
        ]
    );
}

#[test]
fn staging_the_same_inputs_twice_produces_identical_trees() {
    let trees = six_apps();
    let first = trees.staged();
    let second = assemble::stage(
        &trees.closed(&["notify"], &[]),
        &trees.otp_info(),
        &StageOptions::default(),
        &trees.work().join("out2"),
    )
    .expect("the second staging should succeed");

    assert_eq!(walk(first.root()), walk(second.root()));
    for path in walk(first.root()) {
        let left = first.root().join(&path);
        let right = second.root().join(&path);
        assert_eq!(
            std::fs::read(&left).expect("readable"),
            std::fs::read(&right).expect("readable"),
            "`{path}` differs between two stagings of the same input"
        );
        #[cfg(unix)]
        assert_eq!(mode_of(&left), mode_of(&right), "`{path}` differs in mode");
    }
}

#[test]
fn staging_the_same_inputs_twice_produces_an_identical_listing() {
    let trees = six_apps();
    let first = trees.staged();
    let second = assemble::stage(
        &trees.closed(&["notify"], &[]),
        &trees.otp_info(),
        &StageOptions::default(),
        &trees.work().join("out2"),
    )
    .expect("the second staging should succeed");

    assert_eq!(first.listing(), second.listing());
    assert_eq!(first.total_bytes(), second.total_bytes());
    assert_eq!(first.bytes_by_category(), second.bytes_by_category());
}

#[test]
fn explain_reports_the_sizes_the_applications_the_exclusions_and_the_junk() {
    let trees = six_apps();
    let staged = trees.staged();

    insta::assert_snapshot!("stage_explain_table", staged.explain());
}
