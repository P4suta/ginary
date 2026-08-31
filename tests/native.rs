// SPDX-License-Identifier: MIT OR Apache-2.0
//! Native code in a shipment, and what a build for one target does with it.
//!
//! Three groups, one per question `src/native.rs` answers. The scan reads a
//! shipment tree whose `priv` directories hold one of every shape — a real
//! ELF, a PE, a Mach-O, a shell script called `.so`, and two files that begin
//! like an object and are not one. The reconciliation is driven over
//! hand-built [`NativeArtifact`] values, because what it decides is a function
//! of the facts and not of the bytes they were read from, and the two
//! refusals it can raise are pinned as snapshots: they are the message a user
//! is expected to act on, and a table nobody reviewed is a table nobody can
//! follow.
//!
//! Every fixture is fabricated; see `tests/common/native.rs`. There is no
//! cross toolchain on this machine and none of these claims needs one.
// The build-side half of the tool: `native` is a `cli` module.
#![cfg(feature = "cli")]

mod common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use ginary::native::{
    self, HookCtx, NativeArtifact, NativeError, NativeKind, ObjectFacts, ObjectFormat,
    ReconcileCtx, Replacement, ReplacementSource, TargetNativeCfg, Verdict,
};
use ginary::target::{Arch, Libc, Linkage, Os, Target};

use crate::common::fake_otp::{FakeOtp, FakeOtpRoot, FakeShipment};
use crate::common::native::{
    MACHO_CPU_ARM64, MACHO_TYPE_DYLIB, SHELL_WRAPPER, dos_stub, elf_magic_only, host_interp,
    host_machine, macho_bytes, macho_magic_only, musl_interp, pe_bytes, plant, plant_executable,
    program, shared_object,
};
use crate::common::repack::{EM_AARCH64, EM_X86_64};
use crate::common::stubfile::PE_MACHINE_AMD64;

/// A temporary directory for one test.
fn tempdir() -> tempfile::TempDir {
    tempfile::tempdir().expect("a temporary directory")
}

/// The target every reconciliation test builds for.
///
/// `linux-aarch64-musl` is the interesting one: it is not this host, its
/// runtime is the static variant by default, and both of the refusals this
/// module owns are reachable from it.
fn cross_target() -> Target {
    Target::new(Os::Linux, Arch::Aarch64, Libc::Musl)
}

/// The target the shipment's own objects claim to be for.
fn shipped_target() -> Target {
    Target::new(Os::Linux, Arch::X86_64, Libc::Gnu)
}

/// What a glibc x86_64 object reads back as.
fn gnu_facts() -> ObjectFacts {
    ObjectFacts {
        format: ObjectFormat::Elf,
        machine: Arch::X86_64.as_str().to_owned(),
        target: Some(shipped_target()),
        linkage: Linkage::Dynamic,
    }
}

/// What an object already built for [`cross_target`] reads back as.
fn cross_facts() -> ObjectFacts {
    ObjectFacts {
        format: ObjectFormat::Elf,
        machine: Arch::Aarch64.as_str().to_owned(),
        target: Some(cross_target()),
        linkage: Linkage::Dynamic,
    }
}

/// One artifact, described rather than read off a disk.
fn artifact(rel_path: &str, kind: NativeKind, object: Option<ObjectFacts>) -> NativeArtifact {
    let package = rel_path
        .split('/')
        .next()
        .expect("a shipment path has a first component")
        .to_owned();
    NativeArtifact {
        package,
        rel_path: rel_path.to_owned(),
        kind,
        object,
        size: 4096,
        warning: None,
    }
}

/// An empty configuration: no override, no hook.
fn no_config() -> (BTreeMap<String, String>, BTreeMap<String, String>) {
    (BTreeMap::new(), BTreeMap::new())
}

/// The context a reconciliation runs in, over `dir` as the project.
fn ctx<'a>(
    target: &'a Target,
    nif_loading: bool,
    cfg: &'a TargetNativeCfg<'a>,
    dir: &'a Path,
    otp: &'a FakeOtpRoot,
    allow_mismatch: bool,
) -> ReconcileCtx<'a> {
    ReconcileCtx {
        target,
        erts_nif_loading: nif_loading,
        cfg,
        project_root: dir,
        work_dir: dir,
        erts_root: &otp.root,
        erts_version: crate::common::fake_otp::DEFAULT_ERTS_VSN,
        otp_version: crate::common::fake_otp::DEFAULT_OTP_VERSION,
        allow_mismatch,
    }
}

/// How deep under `priv` the deep-tree fixture buries its object.
///
/// Deeper than the walk goes, whatever that bound is: the claim is about what
/// the scan *says* when it stops, not about where it stops.
const DEEPER_THAN_THE_WALK: usize = 40;

/// A runtime root, which every reconciliation is given for its include paths.
fn fake_runtime(dir: &Path) -> FakeOtpRoot {
    FakeOtp::new().build_in(dir.join("otp"))
}

// ------------------------------------------------------------- scan --

/// The shipment the scan tests read: one shape of file per application.
fn scan_tree(dir: &Path) -> PathBuf {
    let host = shared_object(host_machine(), Some(&host_interp()));
    let shipment = FakeShipment::new()
        .app_with("esqlite", "1.0.0", |app| {
            app.priv_file("esqlite3_nif.so", &host)
                .priv_file("README.txt", b"not an object at all\n")
                .priv_file("lib/wrapper.so", SHELL_WRAPPER)
        })
        .app_with("winapp", "1.0.0", |app| {
            app.priv_file("lib/w.dll", &pe_bytes(PE_MACHINE_AMD64, true))
        })
        .app_with("macapp", "1.0.0", |app| {
            app.priv_file(
                "lib/m.dylib",
                &macho_bytes(MACHO_CPU_ARM64, MACHO_TYPE_DYLIB),
            )
            .priv_file("lib/half.dylib", &macho_magic_only())
        })
        .app_with("broken", "1.0.0", |app| {
            app.priv_file("lib/broken.so", &elf_magic_only())
        })
        .app_with("tooling", "1.0.0", |app| {
            app.priv_file("bin/helper", &program(host_machine(), Some(&host_interp())))
        })
        .build_in(dir.join("shipment"));

    // `ebin` holds the compiler's output. A scan that walked it would list
    // whatever a build system left there, and none of it is loaded as native
    // code by the runtime this artifact carries.
    plant(&shipment.root, "esqlite/ebin/stray.so", &host);
    shipment.root
}

/// The `(package, rel_path)` pairs of a scan, in the order it returned them.
fn listed(artifacts: &[NativeArtifact]) -> Vec<(&str, &str)> {
    artifacts
        .iter()
        .map(|found| (found.package.as_str(), found.rel_path.as_str()))
        .collect()
}

/// The one artifact whose path is `rel_path`.
fn one<'a>(artifacts: &'a [NativeArtifact], rel_path: &str) -> &'a NativeArtifact {
    artifacts
        .iter()
        .find(|found| found.rel_path == rel_path)
        .unwrap_or_else(|| panic!("{rel_path} is not in {:?}", listed(artifacts)))
}

#[test]
fn every_object_under_priv_is_listed_once_in_path_order() {
    let dir = tempdir();
    let shipment = scan_tree(dir.path());

    let found = native::scan_shipment(&shipment).expect("the scan reads a whole shipment");

    assert_eq!(
        listed(&found),
        [
            ("broken", "broken/priv/lib/broken.so"),
            ("esqlite", "esqlite/priv/esqlite3_nif.so"),
            ("macapp", "macapp/priv/lib/half.dylib"),
            ("macapp", "macapp/priv/lib/m.dylib"),
            ("tooling", "tooling/priv/bin/helper"),
            ("winapp", "winapp/priv/lib/w.dll"),
        ],
        "the package is the first path component and the order is the path's"
    );
}

#[test]
fn an_object_is_found_by_its_magic_and_never_by_its_name() {
    let dir = tempdir();
    let shipment = scan_tree(dir.path());

    let found = native::scan_shipment(&shipment).expect("the scan reads a whole shipment");
    let paths: Vec<&str> = found.iter().map(|item| item.rel_path.as_str()).collect();

    assert!(
        !paths.contains(&"esqlite/priv/lib/wrapper.so"),
        "a shell script called `.so` is not native code: {paths:?}"
    );
    assert!(
        !paths.contains(&"esqlite/priv/README.txt"),
        "and neither is a text file: {paths:?}"
    );
    assert!(
        paths.contains(&"tooling/priv/bin/helper"),
        "a program with no extension at all is: {paths:?}"
    );
}

#[test]
fn native_code_outside_a_priv_directory_is_not_the_scans_business() {
    let dir = tempdir();
    let shipment = scan_tree(dir.path());

    let found = native::scan_shipment(&shipment).expect("the scan reads a whole shipment");
    let paths: Vec<&str> = found.iter().map(|item| item.rel_path.as_str()).collect();

    // The application's own NIF first: an assertion that something is *not*
    // in a list a bug could have left empty proves nothing on its own.
    assert!(
        paths.contains(&"esqlite/priv/esqlite3_nif.so"),
        "the same application's NIF is found: {paths:?}"
    );
    assert!(
        !paths.contains(&"esqlite/ebin/stray.so"),
        "an ELF under `ebin` is not loaded as native code: {paths:?}"
    );
}

#[test]
fn the_scan_reads_the_format_the_machine_and_the_target_of_each_object() {
    let dir = tempdir();
    let shipment = scan_tree(dir.path());

    let found = native::scan_shipment(&shipment).expect("the scan reads a whole shipment");

    assert_eq!(
        one(&found, "esqlite/priv/esqlite3_nif.so").object,
        Some(ObjectFacts {
            format: ObjectFormat::Elf,
            machine: Target::host().arch.as_str().to_owned(),
            target: Some(Target::host()),
            linkage: Linkage::Dynamic,
        }),
        "a dynamically linked ELF names its whole target through its interpreter"
    );
    assert_eq!(
        one(&found, "winapp/priv/lib/w.dll").object,
        Some(ObjectFacts {
            format: ObjectFormat::Pe,
            machine: Arch::X86_64.as_str().to_owned(),
            target: Some(Target::new(Os::Windows, Arch::X86_64, Libc::None)),
            linkage: Linkage::Dynamic,
        })
    );
    assert_eq!(
        one(&found, "macapp/priv/lib/m.dylib").object,
        Some(ObjectFacts {
            format: ObjectFormat::MachO,
            machine: Arch::Aarch64.as_str().to_owned(),
            target: Some(Target::new(Os::Macos, Arch::Aarch64, Libc::None)),
            linkage: Linkage::Dynamic,
        })
    );
}

#[test]
fn a_library_and_a_program_are_told_apart() {
    let dir = tempdir();
    let shipment = scan_tree(dir.path());

    let found = native::scan_shipment(&shipment).expect("the scan reads a whole shipment");

    assert_eq!(
        one(&found, "esqlite/priv/esqlite3_nif.so").kind,
        NativeKind::SharedObject,
        "a NIF is what the emulator dlopens, and it is the reason a static \
         runtime is refused"
    );
    assert_eq!(
        one(&found, "winapp/priv/lib/w.dll").kind,
        NativeKind::SharedObject
    );
    assert_eq!(
        one(&found, "macapp/priv/lib/m.dylib").kind,
        NativeKind::SharedObject
    );
    assert_eq!(
        one(&found, "tooling/priv/bin/helper").kind,
        NativeKind::Executable,
        "a program under `priv/bin` is run as a child process, not loaded"
    );
}

#[test]
fn a_file_that_begins_like_an_object_and_will_not_parse_is_listed_with_a_warning() {
    let dir = tempdir();
    let shipment = scan_tree(dir.path());

    let found = native::scan_shipment(&shipment).expect("a scan never fails over one file");

    for path in ["broken/priv/lib/broken.so", "macapp/priv/lib/half.dylib"] {
        let item = one(&found, path);
        assert_eq!(item.kind, NativeKind::Unknown, "{path}");
        assert_eq!(item.object, None, "{path}");
        let warning = item
            .warning
            .as_deref()
            .unwrap_or_else(|| panic!("{path} is listed without saying why it could not be read"));
        assert!(
            warning.contains(path),
            "the warning names the file it is about: {warning}"
        );
    }
}

#[test]
fn each_artifact_carries_its_length_on_disk() {
    let dir = tempdir();
    let shipment = scan_tree(dir.path());

    let found = native::scan_shipment(&shipment).expect("the scan reads a whole shipment");

    assert_eq!(found.len(), 6, "the whole tree, or the loop below is empty");
    for item in &found {
        let path = shipment.join(&item.rel_path);
        let size = std::fs::metadata(&path)
            .unwrap_or_else(|error| panic!("cannot stat {}: {error}", path.display()))
            .len();
        assert_eq!(item.size, size, "{}", item.rel_path);
    }
}

#[test]
fn a_file_that_begins_like_a_dos_program_and_carries_no_pe_header_is_listed_too() {
    let dir = tempdir();
    let shipment = FakeShipment::new()
        .app_with("winapp", "1.0.0", |app| {
            app.priv_file("lib/dos.dll", &dos_stub())
        })
        .build_in(dir.path().join("shipment"));

    let found = native::scan_shipment(&shipment.root).expect("a scan never fails over one file");

    let item = one(&found, "winapp/priv/lib/dos.dll");
    assert_eq!(
        item.kind,
        NativeKind::Unknown,
        "`MZ` and no `PE\\0\\0` behind it is a file that begins like an object \
         and is not one, which is the case the scan reports rather than drops"
    );
    assert_eq!(item.object, None);
    let warning = item
        .warning
        .as_deref()
        .expect("a file nobody could read is listed with the reason");
    assert!(
        warning.contains("winapp/priv/lib/dos.dll") && warning.contains("PE"),
        "the warning names the file and what was missing from it: {warning}"
    );
}

#[test]
fn an_object_too_large_to_read_is_listed_with_its_length_rather_than_dropped() {
    let dir = tempdir();
    let shipment = FakeShipment::new()
        .app_with("huge", "1.0.0", |app| app.priv_file("lib/huge.so", &[]))
        .build_in(dir.path().join("shipment"));
    // Sparse: the bound is on the length the header sits in, and writing a
    // hundred megabytes to prove it would be a hundred megabytes of disk.
    let path = shipment.root.join("huge/priv/lib/huge.so");
    std::fs::write(&path, shared_object(host_machine(), None)).expect("the header");
    std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .expect("the object")
        .set_len(native::MAX_OBJECT_BYTES.saturating_add(1))
        .expect("a sparse file the length of the bound plus one");

    let found = native::scan_shipment(&shipment.root).expect("a scan never fails over one file");

    let item = one(&found, "huge/priv/lib/huge.so");
    assert_eq!(item.kind, NativeKind::Unknown);
    let warning = item
        .warning
        .as_deref()
        .expect("an object nobody read is listed with the reason");
    assert!(
        warning.contains(&native::MAX_OBJECT_BYTES.to_string()),
        "the warning says what the bound is: {warning}"
    );
}

#[test]
fn a_priv_tree_deeper_than_the_walk_goes_says_where_it_stopped() {
    let dir = tempdir();
    let shipment = FakeShipment::new()
        .app("deep", "1.0.0", &[])
        .build_in(dir.path().join("shipment"));
    let mut relative = "deep/priv".to_owned();
    for level in 1..=DEEPER_THAN_THE_WALK {
        relative.push_str(&format!("/d{level}"));
    }
    plant(
        &shipment.root,
        &format!("{relative}/buried.so"),
        &shared_object(host_machine(), None),
    );

    let found = native::scan_shipment(&shipment.root).expect("the scan reads a whole shipment");

    let warnings: Vec<&str> = found
        .iter()
        .filter_map(|artifact| artifact.warning.as_deref())
        .collect();
    assert!(
        warnings
            .iter()
            .any(|warning| warning.contains("deep/priv/d1/d2") && warning.contains("depth")),
        "a walk that stopped names the directory it stopped at rather than \
         dropping whatever is under it: {warnings:?}"
    );
}

// ------------------------------------------------------ reconcile --

#[test]
fn an_object_already_built_for_the_target_is_kept() {
    let dir = tempdir();
    let otp = fake_runtime(dir.path());
    let (overrides, hooks) = no_config();
    let cfg = TargetNativeCfg {
        overrides: &overrides,
        hooks: &hooks,
    };
    let target = cross_target();
    let artifacts = [artifact(
        "esqlite/priv/esqlite3_nif.so",
        NativeKind::SharedObject,
        Some(cross_facts()),
    )];

    let done = native::reconcile(
        &artifacts,
        &ctx(&target, true, &cfg, dir.path(), &otp, false),
    )
    .expect("an object for the target is nothing to decide");

    assert_eq!(done.replacements, Vec::new(), "nothing is replaced");
    assert_eq!(done.warnings, Vec::<String>::new(), "and nothing is said");
}

#[test]
fn an_override_replaces_the_artifact_it_names() {
    let dir = tempdir();
    let otp = fake_runtime(dir.path());
    let replacement = plant(
        dir.path(),
        "native/aarch64-musl/esqlite3_nif.so",
        &shared_object(EM_AARCH64, Some(&musl_interp(EM_AARCH64))),
    );
    let overrides = BTreeMap::from([(
        "esqlite/priv/esqlite3_nif.so".to_owned(),
        "native/aarch64-musl/esqlite3_nif.so".to_owned(),
    )]);
    let hooks = BTreeMap::new();
    let cfg = TargetNativeCfg {
        overrides: &overrides,
        hooks: &hooks,
    };
    let target = cross_target();
    let artifacts = [artifact(
        "esqlite/priv/esqlite3_nif.so",
        NativeKind::SharedObject,
        Some(gnu_facts()),
    )];

    let done = native::reconcile(
        &artifacts,
        &ctx(&target, true, &cfg, dir.path(), &otp, false),
    )
    .expect("an override answers the mismatch");

    assert_eq!(
        done.replacements,
        vec![Replacement {
            artifact_rel_path: "esqlite/priv/esqlite3_nif.so".to_owned(),
            source: ReplacementSource::Override(replacement),
        }],
        "the override's path is resolved against the project"
    );
}

#[test]
fn an_override_with_no_interpreter_is_accepted_and_said_so() {
    let dir = tempdir();
    let otp = fake_runtime(dir.path());
    // No `PT_INTERP` at all: the machine is written down and the C library is
    // not, which is what a statically linked object is and what every musl
    // NIF built `-static` looks like. Refusing it would refuse the ordinary
    // case; pretending to have read a libc would be a guess in a manifest.
    plant(
        dir.path(),
        "native/nif.so",
        &shared_object(EM_AARCH64, None),
    );
    let overrides = BTreeMap::from([(
        "esqlite/priv/esqlite3_nif.so".to_owned(),
        "native/nif.so".to_owned(),
    )]);
    let hooks = BTreeMap::new();
    let cfg = TargetNativeCfg {
        overrides: &overrides,
        hooks: &hooks,
    };
    let target = cross_target();
    let artifacts = [artifact(
        "esqlite/priv/esqlite3_nif.so",
        NativeKind::SharedObject,
        Some(gnu_facts()),
    )];

    let done = native::reconcile(
        &artifacts,
        &ctx(&target, true, &cfg, dir.path(), &otp, false),
    )
    .expect("a static object is for whichever libc asked for it");

    assert_eq!(done.replacements.len(), 1, "{:?}", done.replacements);
    assert_eq!(
        done.warnings.len(),
        1,
        "an accepted file whose C library nobody read is worth one line: {:?}",
        done.warnings
    );
    assert!(
        done.warnings[0].contains("native/nif.so") && done.warnings[0].contains("interpreter"),
        "the note names the file and why it was accepted: {}",
        done.warnings[0]
    );
}

#[test]
fn an_override_built_for_another_machine_is_refused_and_names_the_file() {
    let dir = tempdir();
    let otp = fake_runtime(dir.path());
    let wrong = plant(
        dir.path(),
        "native/wrong.so",
        &shared_object(EM_X86_64, Some(&musl_interp(EM_X86_64))),
    );
    let overrides = BTreeMap::from([(
        "esqlite/priv/esqlite3_nif.so".to_owned(),
        "native/wrong.so".to_owned(),
    )]);
    let hooks = BTreeMap::new();
    let cfg = TargetNativeCfg {
        overrides: &overrides,
        hooks: &hooks,
    };
    let target = cross_target();
    let artifacts = [artifact(
        "esqlite/priv/esqlite3_nif.so",
        NativeKind::SharedObject,
        Some(gnu_facts()),
    )];

    let error = native::reconcile(
        &artifacts,
        &ctx(&target, true, &cfg, dir.path(), &otp, false),
    )
    .expect_err("an override for the wrong machine is not an answer");

    match error {
        NativeError::OverrideMismatch {
            rel_path,
            path,
            found,
            target: refused,
        } => {
            assert_eq!(rel_path, "esqlite/priv/esqlite3_nif.so");
            assert_eq!(path, wrong);
            assert!(found.contains("x86_64"), "{found}");
            assert_eq!(refused, cross_target());
        }
        other => panic!("expected OverrideMismatch, got {other:?}"),
    }
}

#[test]
fn an_override_that_is_not_there_is_refused_before_anything_is_built() {
    let dir = tempdir();
    let otp = fake_runtime(dir.path());
    let overrides = BTreeMap::from([(
        "esqlite/priv/esqlite3_nif.so".to_owned(),
        "native/absent.so".to_owned(),
    )]);
    let hooks = BTreeMap::new();
    let cfg = TargetNativeCfg {
        overrides: &overrides,
        hooks: &hooks,
    };
    let target = cross_target();
    let artifacts = [artifact(
        "esqlite/priv/esqlite3_nif.so",
        NativeKind::SharedObject,
        Some(gnu_facts()),
    )];

    let error = native::reconcile(
        &artifacts,
        &ctx(&target, true, &cfg, dir.path(), &otp, false),
    )
    .expect_err("an override nobody can read is not an answer either");

    match error {
        NativeError::OverrideMissing { rel_path, path } => {
            assert_eq!(rel_path, "esqlite/priv/esqlite3_nif.so");
            assert_eq!(path, dir.path().join("native/absent.so"));
        }
        other => panic!("expected OverrideMissing, got {other:?}"),
    }
}

/// A hook script that records its environment and writes the artifact.
///
/// Everything the contract names is written to `$OUT_DIR/env.txt`, one
/// `NAME=VALUE` line each, with `<unset>` for a variable that is not set — so
/// that an absent `ERL_INTERFACE_INCLUDE_DIR` cannot be confused with a script
/// that never looked.
fn hook_script(project: &Path, produces: Option<&str>) -> String {
    let body = produces.map_or_else(String::new, |rel_path| {
        format!(
            "mkdir -p \"$OUT_DIR/$(dirname {rel_path})\"\ncp \"$PWD/replacement.so\" \
             \"$OUT_DIR/{rel_path}\"\n"
        )
    });
    let script = format!(
        "#!/bin/sh\n{{\n  echo \"argv1=$1\"\n  echo \"argv2=$2\"\n  echo \
         \"GINARY_TARGET=${{GINARY_TARGET-<unset>}}\"\n  echo \
         \"GINARY_TARGET_TRIPLE=${{GINARY_TARGET_TRIPLE-<unset>}}\"\n  echo \
         \"OUT_DIR=${{OUT_DIR-<unset>}}\"\n  echo \
         \"ERTS_INCLUDE_DIR=${{ERTS_INCLUDE_DIR-<unset>}}\"\n  echo \
         \"ERL_INTERFACE_INCLUDE_DIR=${{ERL_INTERFACE_INCLUDE_DIR-<unset>}}\"\n  echo \
         \"OTP_VERSION=${{OTP_VERSION-<unset>}}\"\n  echo \"PWD=$PWD\"\n}} > \
         \"$OUT_DIR/env.txt\"\n{body}"
    );
    plant(project, "build_nif.sh", script.as_bytes());
    "sh build_nif.sh {target} {out_dir}".to_owned()
}

/// A hook script that writes the artifact on its first run and never again.
///
/// A `make`-style hook: the second run decides its output is up to date and
/// exits zero having written nothing. Whether that is caught is the whole
/// claim of the test that uses it.
fn once_only_hook_script(project: &Path, rel_path: &str) -> String {
    let script = format!(
        "#!/bin/sh\nif [ -e \"$PWD/already-built\" ]; then exit 0; fi\ntouch \
         \"$PWD/already-built\"\nmkdir -p \"$OUT_DIR/$(dirname {rel_path})\"\ncp \
         \"$PWD/replacement.so\" \"$OUT_DIR/{rel_path}\"\n"
    );
    plant(project, "build_once.sh", script.as_bytes());
    "sh build_once.sh {target} {out_dir}".to_owned()
}

/// The `NAME=VALUE` lines a hook script recorded.
fn recorded(out_dir: &Path) -> BTreeMap<String, String> {
    let path = out_dir.join("env.txt");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("the hook wrote no {}: {error}", path.display()));
    text.lines()
        .filter_map(|line| line.split_once('='))
        .map(|(name, value)| (name.to_owned(), value.to_owned()))
        .collect()
}

#[test]
fn a_hook_runs_in_the_project_with_the_environment_the_contract_names() {
    let dir = tempdir();
    let otp = fake_runtime(dir.path());
    std::fs::create_dir_all(otp.root.join("lib/erl_interface-5.5.2/include"))
        .expect("the erl_interface include directory");
    plant(
        dir.path(),
        "replacement.so",
        &shared_object(EM_AARCH64, Some(&musl_interp(EM_AARCH64))),
    );
    let command = hook_script(dir.path(), Some("esqlite/priv/esqlite3_nif.so"));
    let out_dir = dir.path().join("out");
    std::fs::create_dir_all(&out_dir).expect("the output directory");
    let target = cross_target();

    let written = native::run_hook(
        "esqlite",
        &command,
        &HookCtx {
            target: &target,
            out_dir: &out_dir,
            project_root: dir.path(),
            erts_root: &otp.root,
            erts_version: crate::common::fake_otp::DEFAULT_ERTS_VSN,
            otp_version: crate::common::fake_otp::DEFAULT_OTP_VERSION,
        },
    )
    .expect("the hook runs");

    assert_eq!(
        written, out_dir,
        "a hook writes into the directory it is given"
    );
    let env = recorded(&out_dir);
    assert_eq!(
        env.get("argv1").map(String::as_str),
        Some("linux-aarch64-musl")
    );
    assert_eq!(
        env.get("argv2").map(String::as_str),
        Some(out_dir.to_string_lossy().as_ref()),
        "{{target}} and {{out_dir}} are substituted in the command line"
    );
    assert_eq!(
        env.get("GINARY_TARGET").map(String::as_str),
        Some("linux-aarch64-musl")
    );
    assert_eq!(
        env.get("GINARY_TARGET_TRIPLE").map(String::as_str),
        Some(cross_target().rust_triple())
    );
    assert_eq!(
        env.get("OUT_DIR").map(String::as_str),
        Some(out_dir.to_string_lossy().as_ref())
    );
    assert_eq!(
        env.get("ERTS_INCLUDE_DIR").map(String::as_str),
        Some(
            otp.root
                .join(format!(
                    "erts-{}/include",
                    crate::common::fake_otp::DEFAULT_ERTS_VSN
                ))
                .to_string_lossy()
                .as_ref()
        )
    );
    assert_eq!(
        env.get("ERL_INTERFACE_INCLUDE_DIR").map(String::as_str),
        Some(
            otp.root
                .join("lib/erl_interface-5.5.2/include")
                .to_string_lossy()
                .as_ref()
        )
    );
    assert_eq!(
        env.get("OTP_VERSION").map(String::as_str),
        Some(crate::common::fake_otp::DEFAULT_OTP_VERSION)
    );
    assert_eq!(
        env.get("PWD").map(String::as_str),
        Some(dir.path().to_string_lossy().as_ref()),
        "a hook runs in the project, which is what its relative paths mean"
    );
}

#[test]
fn a_runtime_with_no_erl_interface_leaves_that_variable_unset() {
    let dir = tempdir();
    let otp = fake_runtime(dir.path());
    let command = hook_script(dir.path(), None);
    let out_dir = dir.path().join("out");
    std::fs::create_dir_all(&out_dir).expect("the output directory");
    let target = cross_target();

    let _ = native::run_hook(
        "esqlite",
        &command,
        &HookCtx {
            target: &target,
            out_dir: &out_dir,
            project_root: dir.path(),
            erts_root: &otp.root,
            erts_version: crate::common::fake_otp::DEFAULT_ERTS_VSN,
            otp_version: crate::common::fake_otp::DEFAULT_OTP_VERSION,
        },
    )
    .expect("the hook runs");

    assert_eq!(
        recorded(&out_dir)
            .get("ERL_INTERFACE_INCLUDE_DIR")
            .map(String::as_str),
        Some("<unset>"),
        "naming a directory that is not there would be worse than saying nothing"
    );
}

#[test]
fn a_hooks_output_replaces_the_artifact_it_belongs_to() {
    let dir = tempdir();
    let otp = fake_runtime(dir.path());
    plant(
        dir.path(),
        "replacement.so",
        &shared_object(EM_AARCH64, Some(&musl_interp(EM_AARCH64))),
    );
    let command = hook_script(dir.path(), Some("esqlite/priv/esqlite3_nif.so"));
    let overrides = BTreeMap::new();
    let hooks = BTreeMap::from([("esqlite".to_owned(), command)]);
    let cfg = TargetNativeCfg {
        overrides: &overrides,
        hooks: &hooks,
    };
    let target = cross_target();
    let artifacts = [artifact(
        "esqlite/priv/esqlite3_nif.so",
        NativeKind::SharedObject,
        Some(gnu_facts()),
    )];

    let done = native::reconcile(
        &artifacts,
        &ctx(&target, true, &cfg, dir.path(), &otp, false),
    )
    .expect("a hook answers the mismatch");

    assert_eq!(
        done.replacements,
        vec![Replacement {
            artifact_rel_path: "esqlite/priv/esqlite3_nif.so".to_owned(),
            source: ReplacementSource::Hook {
                package: "esqlite".to_owned(),
                out_path: dir
                    .path()
                    .join("native/linux-aarch64-musl/esqlite/esqlite/priv/esqlite3_nif.so"),
            },
        }],
        "a hook writes under <work>/native/<target>/<package>/ and the artifact's own path"
    );
}

#[test]
fn a_hook_that_writes_nothing_where_the_artifact_belongs_is_refused() {
    let dir = tempdir();
    let otp = fake_runtime(dir.path());
    let command = hook_script(dir.path(), None);
    let overrides = BTreeMap::new();
    let hooks = BTreeMap::from([("esqlite".to_owned(), command)]);
    let cfg = TargetNativeCfg {
        overrides: &overrides,
        hooks: &hooks,
    };
    let target = cross_target();
    let artifacts = [artifact(
        "esqlite/priv/esqlite3_nif.so",
        NativeKind::SharedObject,
        Some(gnu_facts()),
    )];

    let error = native::reconcile(
        &artifacts,
        &ctx(&target, true, &cfg, dir.path(), &otp, false),
    )
    .expect_err("a hook that succeeded and produced nothing is a failed build");

    match error {
        NativeError::HookOutputMissing { package, expected } => {
            assert_eq!(package, "esqlite");
            assert_eq!(
                expected,
                dir.path()
                    .join("native/linux-aarch64-musl/esqlite/esqlite/priv/esqlite3_nif.so")
            );
        }
        other => panic!("expected HookOutputMissing, got {other:?}"),
    }
}

#[test]
fn a_hook_that_fails_is_refused_with_everything_it_wrote_to_stderr() {
    let dir = tempdir();
    let otp = fake_runtime(dir.path());
    let overrides = BTreeMap::new();
    let hooks = BTreeMap::from([(
        "esqlite".to_owned(),
        "echo 'no compiler for {target}' >&2; exit 3".to_owned(),
    )]);
    let cfg = TargetNativeCfg {
        overrides: &overrides,
        hooks: &hooks,
    };
    let target = cross_target();
    let artifacts = [artifact(
        "esqlite/priv/esqlite3_nif.so",
        NativeKind::SharedObject,
        Some(gnu_facts()),
    )];

    let error = native::reconcile(
        &artifacts,
        &ctx(&target, true, &cfg, dir.path(), &otp, false),
    )
    .expect_err("a hook that exits non-zero stops the build");

    match error {
        NativeError::HookFailed {
            package,
            command,
            stderr,
        } => {
            assert_eq!(package, "esqlite");
            assert!(
                command.contains("linux-aarch64-musl"),
                "the command is quoted after substitution: {command}"
            );
            assert!(
                stderr.contains("no compiler for linux-aarch64-musl"),
                "everything the hook said travels: {stderr}"
            );
        }
        other => panic!("expected HookFailed, got {other:?}"),
    }
}

#[test]
fn an_override_answers_before_a_hook_is_run() {
    let dir = tempdir();
    let otp = fake_runtime(dir.path());
    let replacement = plant(
        dir.path(),
        "native/nif.so",
        &shared_object(EM_AARCH64, Some(&musl_interp(EM_AARCH64))),
    );
    let overrides = BTreeMap::from([(
        "esqlite/priv/esqlite3_nif.so".to_owned(),
        "native/nif.so".to_owned(),
    )]);
    let hooks = BTreeMap::from([("esqlite".to_owned(), "touch ran-anyway; exit 1".to_owned())]);
    let cfg = TargetNativeCfg {
        overrides: &overrides,
        hooks: &hooks,
    };
    let target = cross_target();
    let artifacts = [artifact(
        "esqlite/priv/esqlite3_nif.so",
        NativeKind::SharedObject,
        Some(gnu_facts()),
    )];

    let done = native::reconcile(
        &artifacts,
        &ctx(&target, true, &cfg, dir.path(), &otp, false),
    )
    .expect("the override answers");

    assert_eq!(
        done.replacements[0].source,
        ReplacementSource::Override(replacement)
    );
    assert!(
        !dir.path().join("ran-anyway").exists(),
        "a hook whose artifact was already answered for is not run"
    );
}

#[test]
fn a_hook_that_builds_for_another_machine_is_refused_and_names_what_it_wrote() {
    let dir = tempdir();
    let otp = fake_runtime(dir.path());
    // The hook is a compiler somebody else wrote, and one that quietly builds
    // for the host on a cross build is the failure this module exists to stop:
    // its output is checked exactly as an override's is.
    plant(
        dir.path(),
        "replacement.so",
        &shared_object(EM_X86_64, Some(&musl_interp(EM_X86_64))),
    );
    let command = hook_script(dir.path(), Some("esqlite/priv/esqlite3_nif.so"));
    let overrides = BTreeMap::new();
    let hooks = BTreeMap::from([("esqlite".to_owned(), command)]);
    let cfg = TargetNativeCfg {
        overrides: &overrides,
        hooks: &hooks,
    };
    let target = cross_target();
    let artifacts = [artifact(
        "esqlite/priv/esqlite3_nif.so",
        NativeKind::SharedObject,
        Some(gnu_facts()),
    )];

    let error = native::reconcile(
        &artifacts,
        &ctx(&target, true, &cfg, dir.path(), &otp, false),
    )
    .expect_err("what a hook wrote for another machine is not an answer either");

    match error {
        NativeError::HookMismatch {
            package,
            path,
            found,
            target: refused,
        } => {
            assert_eq!(package, "esqlite");
            assert!(
                path.ends_with("esqlite/priv/esqlite3_nif.so"),
                "the error names the file the hook wrote: {}",
                path.display()
            );
            assert!(found.contains("x86_64"), "{found}");
            assert_eq!(refused, cross_target());
        }
        other => panic!("expected HookMismatch, got {other:?}"),
    }
}

#[test]
fn a_hook_that_writes_once_does_not_answer_for_a_second_target() {
    let dir = tempdir();
    let otp = fake_runtime(dir.path());
    // An object with no interpreter, which is accepted for any target of its
    // machine: if the second target read the first one's directory, nothing
    // downstream would ever notice.
    plant(
        dir.path(),
        "replacement.so",
        &shared_object(EM_AARCH64, None),
    );
    let command = once_only_hook_script(dir.path(), "esqlite/priv/esqlite3_nif.so");
    let overrides = BTreeMap::new();
    let hooks = BTreeMap::from([("esqlite".to_owned(), command)]);
    let cfg = TargetNativeCfg {
        overrides: &overrides,
        hooks: &hooks,
    };
    let artifacts = [artifact(
        "esqlite/priv/esqlite3_nif.so",
        NativeKind::SharedObject,
        Some(gnu_facts()),
    )];
    let first = cross_target();
    let second = Target::new(Os::Linux, Arch::Aarch64, Libc::Gnu);

    let done = native::reconcile(
        &artifacts,
        &ctx(&first, true, &cfg, dir.path(), &otp, false),
    )
    .expect("the hook writes on its first run");
    let error = native::reconcile(
        &artifacts,
        &ctx(&second, true, &cfg, dir.path(), &otp, false),
    )
    .expect_err("a hook that wrote nothing for this target answered for nothing");

    match done.replacements.first().map(|one| &one.source) {
        Some(ReplacementSource::Hook { out_path, .. }) => assert!(
            out_path.starts_with(dir.path().join("native").join(first.name())),
            "one target's hook output is its own: {}",
            out_path.display()
        ),
        other => panic!("expected a hook replacement, got {other:?}"),
    }
    match error {
        NativeError::HookOutputMissing { package, expected } => {
            assert_eq!(package, "esqlite");
            assert!(
                expected.starts_with(dir.path().join("native").join(second.name())),
                "and the second target looked in its own: {}",
                expected.display()
            );
        }
        other => panic!("expected HookOutputMissing, got {other:?}"),
    }
}

/// Two packages, neither for the target and neither configured.
fn two_mismatches() -> [NativeArtifact; 2] {
    [
        artifact(
            "esqlite/priv/esqlite3_nif.so",
            NativeKind::SharedObject,
            Some(gnu_facts()),
        ),
        artifact(
            "bcrypt/priv/bcrypt_nif.so",
            NativeKind::SharedObject,
            Some(gnu_facts()),
        ),
    ]
}

#[test]
fn every_unaccounted_object_is_one_refusal_naming_the_keys_that_fix_it() {
    let dir = tempdir();
    let otp = fake_runtime(dir.path());
    let (overrides, hooks) = no_config();
    let cfg = TargetNativeCfg {
        overrides: &overrides,
        hooks: &hooks,
    };
    let target = cross_target();

    let error = native::reconcile(
        &two_mismatches(),
        &ctx(&target, true, &cfg, dir.path(), &otp, false),
    )
    .expect_err("native code for another machine stops a build");

    match &error {
        NativeError::Mismatch {
            target: refused,
            rows,
        } => {
            assert_eq!(*refused, cross_target());
            assert_eq!(
                rows.iter()
                    .map(|row| row.rel_path.as_str())
                    .collect::<Vec<_>>(),
                ["bcrypt/priv/bcrypt_nif.so", "esqlite/priv/esqlite3_nif.so"],
                "one table, in path order, rather than one build per file"
            );
        }
        other => panic!("expected Mismatch, got {other:?}"),
    }
    insta::assert_snapshot!("native_mismatch_message", error.to_string());
}

#[test]
fn allowing_the_mismatch_keeps_the_objects_and_says_the_same_thing_as_a_warning() {
    let dir = tempdir();
    let otp = fake_runtime(dir.path());
    let (overrides, hooks) = no_config();
    let cfg = TargetNativeCfg {
        overrides: &overrides,
        hooks: &hooks,
    };
    let target = cross_target();

    let done = native::reconcile(
        &two_mismatches(),
        &ctx(&target, true, &cfg, dir.path(), &otp, true),
    )
    .expect("--allow-native-mismatch is the user taking the decision");

    assert_eq!(
        done.replacements,
        Vec::new(),
        "the shipment's own objects are kept, as they are"
    );
    assert_eq!(done.warnings.len(), 1, "{:?}", done.warnings);
    insta::assert_snapshot!("native_mismatch_warning", done.warnings[0].clone());
}

#[test]
fn a_static_runtime_refuses_a_shared_object_it_could_never_load() {
    let dir = tempdir();
    let otp = fake_runtime(dir.path());
    let (overrides, hooks) = no_config();
    let cfg = TargetNativeCfg {
        overrides: &overrides,
        hooks: &hooks,
    };
    let target = cross_target();
    let artifacts = [artifact(
        "esqlite/priv/esqlite3_nif.so",
        NativeKind::SharedObject,
        Some(cross_facts()),
    )];

    let error = native::reconcile(
        &artifacts,
        &ctx(&target, false, &cfg, dir.path(), &otp, false),
    )
    .expect_err("a static emulator has no dynamic loader in it");

    match &error {
        NativeError::StaticRuntime {
            target: refused,
            rows,
        } => {
            assert_eq!(*refused, cross_target());
            assert_eq!(
                rows.iter()
                    .map(|row| row.rel_path.as_str())
                    .collect::<Vec<_>>(),
                ["esqlite/priv/esqlite3_nif.so"]
            );
        }
        other => panic!("expected StaticRuntime, got {other:?}"),
    }
    insta::assert_snapshot!("native_static_runtime_message", error.to_string());
}

#[test]
fn allowing_the_mismatch_does_not_lift_the_static_runtime_refusal() {
    let dir = tempdir();
    let otp = fake_runtime(dir.path());
    let (overrides, hooks) = no_config();
    let cfg = TargetNativeCfg {
        overrides: &overrides,
        hooks: &hooks,
    };
    let target = cross_target();
    let artifacts = [artifact(
        "esqlite/priv/esqlite3_nif.so",
        NativeKind::SharedObject,
        Some(cross_facts()),
    )];

    let error = native::reconcile(
        &artifacts,
        &ctx(&target, false, &cfg, dir.path(), &otp, true),
    )
    .expect_err("the flag says `ship it anyway`, and this one cannot be shipped");

    assert!(
        matches!(error, NativeError::StaticRuntime { .. }),
        "expected StaticRuntime, got {error:?}"
    );
}

#[test]
fn an_object_a_replacement_answered_for_is_still_a_shared_object() {
    let dir = tempdir();
    let otp = fake_runtime(dir.path());
    plant(
        dir.path(),
        "native/nif.so",
        &shared_object(EM_AARCH64, Some(&musl_interp(EM_AARCH64))),
    );
    let overrides = BTreeMap::from([(
        "esqlite/priv/esqlite3_nif.so".to_owned(),
        "native/nif.so".to_owned(),
    )]);
    let hooks = BTreeMap::new();
    let cfg = TargetNativeCfg {
        overrides: &overrides,
        hooks: &hooks,
    };
    let target = cross_target();
    let artifacts = [artifact(
        "esqlite/priv/esqlite3_nif.so",
        NativeKind::SharedObject,
        Some(gnu_facts()),
    )];

    let error = native::reconcile(
        &artifacts,
        &ctx(&target, false, &cfg, dir.path(), &otp, false),
    )
    .expect_err("replacing a NIF does not give the runtime a loader");

    assert!(
        matches!(error, NativeError::StaticRuntime { .. }),
        "the check is made after the reconciliation, over what is left: {error:?}"
    );
}

#[test]
fn a_static_runtime_with_nothing_to_load_is_no_trouble_at_all() {
    let dir = tempdir();
    let otp = fake_runtime(dir.path());
    let (overrides, hooks) = no_config();
    let cfg = TargetNativeCfg {
        overrides: &overrides,
        hooks: &hooks,
    };
    let target = cross_target();
    let artifacts = [artifact(
        "tooling/priv/bin/helper",
        NativeKind::Executable,
        Some(cross_facts()),
    )];

    let done = native::reconcile(
        &artifacts,
        &ctx(&target, false, &cfg, dir.path(), &otp, false),
    )
    .expect("a program is executed, never dlopened");

    assert_eq!(done.replacements, Vec::new());
    assert_eq!(done.warnings, Vec::<String>::new());
}

#[test]
fn a_program_for_the_wrong_machine_is_the_same_mismatch_a_library_is() {
    let dir = tempdir();
    let otp = fake_runtime(dir.path());
    let (overrides, hooks) = no_config();
    let cfg = TargetNativeCfg {
        overrides: &overrides,
        hooks: &hooks,
    };
    let target = cross_target();
    let artifacts = [artifact(
        "tooling/priv/bin/helper",
        NativeKind::Executable,
        Some(gnu_facts()),
    )];

    let error = native::reconcile(
        &artifacts,
        &ctx(&target, true, &cfg, dir.path(), &otp, false),
    )
    .expect_err("a program the artifact carries has to run on the target too");

    match &error {
        NativeError::Mismatch { rows, .. } => assert_eq!(
            rows.iter()
                .map(|row| row.rel_path.as_str())
                .collect::<Vec<_>>(),
            ["tooling/priv/bin/helper"]
        ),
        other => panic!("expected Mismatch, got {other:?}"),
    }
}

// ---------------------------------------------------------- apply --

#[test]
fn a_replacement_is_copied_over_the_staged_file_it_answers_for() {
    let dir = tempdir();
    let staged = dir.path().join("root");
    plant_executable(
        &staged,
        "lib/esqlite/priv/esqlite3_nif.so",
        b"the old bytes",
    );
    let replacement = plant(
        dir.path(),
        "native/nif.so",
        &shared_object(EM_AARCH64, Some(&musl_interp(EM_AARCH64))),
    );

    native::apply(
        &[Replacement {
            artifact_rel_path: "esqlite/priv/esqlite3_nif.so".to_owned(),
            source: ReplacementSource::Override(replacement.clone()),
        }],
        &staged,
    )
    .expect("the staged tree holds the file");

    let landed = staged.join("lib/esqlite/priv/esqlite3_nif.so");
    assert_eq!(
        std::fs::read(&landed).expect("the staged file"),
        std::fs::read(&replacement).expect("the replacement"),
        "the shipment application is staged at lib/<name>"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(&landed)
            .expect("the staged file")
            .permissions()
            .mode();
        assert_eq!(
            mode & 0o111,
            0o111,
            "a NIF that arrived executable stays executable"
        );
    }
}

#[test]
fn a_replacement_for_a_file_the_staged_tree_does_not_hold_is_an_error() {
    let dir = tempdir();
    let staged = dir.path().join("root");
    std::fs::create_dir_all(&staged).expect("the staging root");
    let replacement = plant(
        dir.path(),
        "native/nif.so",
        &shared_object(EM_AARCH64, Some(&musl_interp(EM_AARCH64))),
    );

    let error = native::apply(
        &[Replacement {
            artifact_rel_path: "esqlite/priv/esqlite3_nif.so".to_owned(),
            source: ReplacementSource::Override(replacement),
        }],
        &staged,
    )
    .expect_err("a replacement that lands nowhere is a silently unpatched artifact");

    match error {
        NativeError::StagedMissing { path, package } => {
            assert_eq!(path, "lib/esqlite/priv/esqlite3_nif.so");
            assert_eq!(
                package, "esqlite",
                "the sentence names the application, which is the reason a \
                 staged path can be missing at all"
            );
        }
        other => panic!("expected StagedMissing, got {other:?}"),
    }
}

// -------------------------------------------------------- verdicts --

#[test]
fn every_artifact_gets_the_verdict_the_build_would_reach() {
    let overrides = BTreeMap::from([(
        "esqlite/priv/esqlite3_nif.so".to_owned(),
        "native/nif.so".to_owned(),
    )]);
    let hooks = BTreeMap::from([("bcrypt".to_owned(), "make nif".to_owned())]);
    let cfg = TargetNativeCfg {
        overrides: &overrides,
        hooks: &hooks,
    };
    let target = cross_target();
    let artifacts = [
        artifact(
            "esqlite/priv/esqlite3_nif.so",
            NativeKind::SharedObject,
            Some(gnu_facts()),
        ),
        artifact(
            "bcrypt/priv/bcrypt_nif.so",
            NativeKind::SharedObject,
            Some(gnu_facts()),
        ),
        artifact(
            "exile/priv/exile_nif.so",
            NativeKind::SharedObject,
            Some(gnu_facts()),
        ),
        artifact(
            "tooling/priv/bin/helper",
            NativeKind::Executable,
            Some(cross_facts()),
        ),
    ];

    assert_eq!(
        native::verdicts_for_target(&artifacts, &target, true, &cfg),
        vec![
            Verdict::Override,
            Verdict::Hook,
            Verdict::Mismatch,
            Verdict::Ok,
        ],
        "one verdict per artifact, in the order they were given"
    );
}

#[test]
fn a_runtime_that_cannot_load_a_nif_outranks_every_other_verdict() {
    let overrides = BTreeMap::from([(
        "esqlite/priv/esqlite3_nif.so".to_owned(),
        "native/nif.so".to_owned(),
    )]);
    let hooks = BTreeMap::new();
    let cfg = TargetNativeCfg {
        overrides: &overrides,
        hooks: &hooks,
    };
    let target = cross_target();
    let artifacts = [
        artifact(
            "esqlite/priv/esqlite3_nif.so",
            NativeKind::SharedObject,
            Some(cross_facts()),
        ),
        artifact(
            "tooling/priv/bin/helper",
            NativeKind::Executable,
            Some(cross_facts()),
        ),
    ];

    assert_eq!(
        native::verdicts_for_target(&artifacts, &target, false, &cfg),
        vec![Verdict::StaticRuntime, Verdict::Ok],
        "a shared object nothing can load is the finding, override or no override"
    );
}
