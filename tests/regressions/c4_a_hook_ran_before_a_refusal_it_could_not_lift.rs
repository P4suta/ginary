// SPDX-License-Identifier: MIT OR Apache-2.0
//! `reconcile` spent a project's compiler on output it threw away one line
//! later.
//!
//! The static-runtime refusal is decided by the scan and the runtime alone: a
//! shared object under `priv` and a target whose emulator cannot `dlopen` one.
//! Nothing a replacement does changes it — a NIF an override or a hook
//! answered for is still a NIF — and `--allow-native-mismatch` deliberately
//! does not lift it. But the rows were collected *after* the per-artifact
//! loop, so every configured `[tools.ginary.native.<package>] build` ran
//! first, each under `native::HOOK_TIMEOUT` (600 s), before the build stopped
//! with an error none of them could have prevented.
//!
//! The right behaviour: refuse first. The answer is knowable before the first
//! hook starts, and a user who has to wait ten minutes for an error the flag
//! cannot waive is a user who waited for nothing.
#![cfg(feature = "cli")]

use std::collections::BTreeMap;

use ginary::native::{self, NativeError, ReconcileCtx, TargetNativeCfg};
use ginary::target::Target;

use crate::common::fake_otp::{DEFAULT_ERTS_VSN, DEFAULT_OTP_VERSION, FakeOtp, FakeShipment};
use crate::common::native::{host_machine, plant, shared_object};

/// The NIF a static runtime could never open.
const NIF: &str = "esqlite/priv/esqlite3_nif.so";

/// What the hook leaves behind when it runs at all.
const MARKER: &str = "the-hook-ran";

#[test]
fn a_hook_does_not_run_for_a_build_a_static_runtime_has_already_refused() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let project = dir.path().join("project");
    std::fs::create_dir_all(&project).expect("the project directory");
    let shipment = FakeShipment::new()
        .app("esqlite", "1.0.0", &[])
        .build_in(dir.path().join("shipment"));
    let object = shared_object(host_machine(), None);
    plant(&shipment.root, NIF, &object);
    // A hook that succeeds and writes exactly what is asked of it, so that the
    // only thing left to stop the build is the runtime. It also records that
    // it ran, which is the whole assertion.
    std::fs::write(project.join("nif.so"), &object).expect("the object the hook copies");
    let otp = FakeOtp::new().build_in(dir.path().join("otp"));
    let artifacts = native::scan_shipment(&shipment.root).expect("the scan reads the shipment");
    let overrides = BTreeMap::new();
    let hooks = BTreeMap::from([(
        "esqlite".to_owned(),
        format!(
            "touch {MARKER} && mkdir -p \"$OUT_DIR/esqlite/priv\" && \
             cp nif.so \"$OUT_DIR/{NIF}\""
        ),
    )]);
    let cfg = TargetNativeCfg {
        overrides: &overrides,
        hooks: &hooks,
    };
    let target = Target::host();

    let error = native::reconcile(
        &artifacts,
        &ReconcileCtx {
            target: &target,
            erts_nif_loading: false,
            cfg: &cfg,
            project_root: &project,
            work_dir: dir.path(),
            erts_root: &otp.root,
            erts_version: DEFAULT_ERTS_VSN,
            otp_version: DEFAULT_OTP_VERSION,
            allow_mismatch: false,
        },
    )
    .expect_err("a static emulator cannot open a NIF, whoever built it");

    assert!(
        matches!(error, NativeError::StaticRuntime { .. }),
        "expected StaticRuntime, got {error:?}"
    );
    assert!(
        !project.join(MARKER).exists(),
        "the hook was run for an answer that was already decided"
    );
}
