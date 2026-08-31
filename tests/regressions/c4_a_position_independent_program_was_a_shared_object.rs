// SPDX-License-Identifier: MIT OR Apache-2.0
//! A port program under `priv/bin` was classified as a shared object, and made
//! every static-runtime cross build impossible.
//!
//! `native::describe_elf` mapped `e_type` straight through: `ET_DYN` became
//! [`NativeKind::SharedObject`] and nothing else was consulted. Every program a
//! modern toolchain links is an `ET_DYN` — `readelf -h` calls it
//! `DYN (Position-Independent Executable file)` — so a `tooling/priv/bin/helper`
//! compiled today read as a library the emulator would `dlopen`.
//!
//! The consequence was a refusal with no way out. `reconcile`'s static-runtime
//! rule fires for any shared object when the target's runtime cannot load one,
//! `--allow-native-mismatch` deliberately does not lift it, and the remedy it
//! prints — `otp_variant = "dynamic"` — is one the shipped catalogue cannot
//! satisfy for a musl target, which publishes a single static variant. A
//! project shipping one port program could not be built for musl at all.
//!
//! The right behaviour, and what `README.md` already promised: a program is run
//! as a child process rather than loaded, so a static runtime is no trouble for
//! one. `DT_FLAGS_1`'s `DF_1_PIE` is what separates the two `ET_DYN` shapes —
//! glibc's own `libc.so.6` carries a `PT_INTERP` and does *not* carry that flag,
//! so the interpreter cannot be the discriminator — and this test uses the one
//! genuine position-independent executable every run has to hand: itself.
#![cfg(feature = "cli")]

use std::collections::BTreeMap;
use std::path::Path;

use ginary::native::{
    self, NativeArtifact, NativeError, NativeKind, ReconcileCtx, TargetNativeCfg,
};
use ginary::target::Target;

use crate::common::fake_otp::{
    DEFAULT_ERTS_VSN, DEFAULT_OTP_VERSION, FakeOtp, FakeOtpRoot, FakeShipment,
};
use crate::common::native::{host_machine, plant, shared_object};
use crate::common::repack::test_binary;

/// The program: this test binary, which `cargo` links `-pie`.
const PROGRAM: &str = "tooling/priv/bin/helper";

/// The library: an `ET_DYN` with neither an interpreter nor `DF_1_PIE`, which
/// is what a NIF built `-shared` is.
const LIBRARY: &str = "esqlite/priv/esqlite3_nif.so";

/// A shipment holding one of each, and the runtime a reconciliation reads.
fn shipment(dir: &Path) -> (std::path::PathBuf, FakeOtpRoot) {
    let root = FakeShipment::new()
        .app("tooling", "1.0.0", &[])
        .app("esqlite", "1.0.0", &[])
        .build_in(dir.join("shipment"));
    plant(&root.root, PROGRAM, &test_binary());
    plant(&root.root, LIBRARY, &shared_object(host_machine(), None));
    (root.root, FakeOtp::new().build_in(dir.join("otp")))
}

/// A reconciliation for this host whose runtime cannot load a NIF.
fn refuse_nifs<'a>(
    target: &'a Target,
    cfg: &'a TargetNativeCfg<'a>,
    dir: &'a Path,
    otp: &'a FakeOtpRoot,
) -> ReconcileCtx<'a> {
    ReconcileCtx {
        target,
        erts_nif_loading: false,
        cfg,
        project_root: dir,
        work_dir: dir,
        erts_root: &otp.root,
        erts_version: DEFAULT_ERTS_VSN,
        otp_version: DEFAULT_OTP_VERSION,
        allow_mismatch: false,
    }
}

/// The one artifact whose path is `rel_path`.
fn one<'a>(found: &'a [NativeArtifact], rel_path: &str) -> &'a NativeArtifact {
    found
        .iter()
        .find(|artifact| artifact.rel_path == rel_path)
        .unwrap_or_else(|| panic!("{rel_path} was not scanned"))
}

#[test]
fn a_position_independent_program_is_a_program_and_not_a_shared_object() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let (shipment, _otp) = shipment(dir.path());

    let found = native::scan_shipment(&shipment).expect("the scan reads a whole shipment");

    assert_eq!(
        one(&found, PROGRAM).kind,
        NativeKind::Executable,
        "every program a modern toolchain links is an `ET_DYN`, and a program \
         is run rather than loaded"
    );
    assert_eq!(
        one(&found, LIBRARY).kind,
        NativeKind::SharedObject,
        "and a real shared object is still one, or the rule below proves nothing"
    );
}

#[test]
fn a_runtime_that_cannot_load_a_nif_does_not_refuse_a_port_program() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let (shipment, otp) = shipment(dir.path());
    let found = native::scan_shipment(&shipment).expect("the scan reads a whole shipment");
    let programs: Vec<NativeArtifact> = found
        .iter()
        .filter(|artifact| artifact.rel_path == PROGRAM)
        .cloned()
        .collect();
    let (overrides, hooks) = (BTreeMap::new(), BTreeMap::new());
    let cfg = TargetNativeCfg {
        overrides: &overrides,
        hooks: &hooks,
    };
    let target = Target::host();

    let done = native::reconcile(&programs, &refuse_nifs(&target, &cfg, dir.path(), &otp))
        .expect("a static runtime never has to open a program");

    assert!(
        done.replacements.is_empty(),
        "nothing was configured, so nothing is replaced: {:?}",
        done.replacements
    );
}

#[test]
fn a_runtime_that_cannot_load_a_nif_still_refuses_the_shared_object_beside_it() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let (shipment, otp) = shipment(dir.path());
    let found = native::scan_shipment(&shipment).expect("the scan reads a whole shipment");
    let (overrides, hooks) = (BTreeMap::new(), BTreeMap::new());
    let cfg = TargetNativeCfg {
        overrides: &overrides,
        hooks: &hooks,
    };
    let target = Target::host();

    let error = native::reconcile(&found, &refuse_nifs(&target, &cfg, dir.path(), &otp))
        .expect_err("a static emulator cannot open a NIF");

    match error {
        NativeError::StaticRuntime { rows, .. } => {
            let named: Vec<&str> = rows.iter().map(|row| row.rel_path.as_str()).collect();
            assert_eq!(
                named,
                [LIBRARY],
                "the table names the file that would never load, and only it"
            );
        }
        other => panic!("expected StaticRuntime, got {other:?}"),
    }
}
