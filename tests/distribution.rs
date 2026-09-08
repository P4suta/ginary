// SPDX-License-Identifier: MIT OR Apache-2.0
//! Distribution is assembled and verified locally before any hosted operation.
#![cfg(feature = "cli")]

mod common;

use ginary::catalog::{self, Catalog, RepackOptions, RepackSelector, Variant};
use ginary::target::{Arch, Os};
use std::path::Path;

fn native_options(out: &Path, target: &str) -> RepackOptions {
    RepackOptions {
        upstream_tag: "OTP-29.0.5".into(),
        selectors: vec![RepackSelector::parse(target).unwrap()],
        out: out.into(),
        upstream_dir: None,
        source_date_epoch: Some(0),
    }
}

#[test]
fn installed_windows_and_macos_roots_repack_without_modifying_the_source() {
    for (target, fake) in [
        ("windows-x86_64", common::fake_otp::FakeOtp::new().windows()),
        ("macos-aarch64", common::fake_otp::FakeOtp::new().macos()),
    ] {
        let work = tempfile::tempdir().unwrap();
        let root = fake
            .otp_version("29.0.5")
            .build_in(work.path().join("source"));
        let source_file = root.root.join("doc/keep-in-source.txt");
        std::fs::create_dir_all(source_file.parent().unwrap()).unwrap();
        std::fs::write(&source_file, "source remains unchanged").unwrap();
        let first = catalog::repack_from_root(
            &native_options(&work.path().join("a"), target),
            &root.root,
            &ginary::diag::Diag::disabled(),
        )
        .unwrap();
        let second = catalog::repack_from_root(
            &native_options(&work.path().join("b"), target),
            &root.root,
            &ginary::diag::Diag::disabled(),
        )
        .unwrap();
        assert_eq!(
            std::fs::read(&first.outcomes[0].tarball).unwrap(),
            std::fs::read(&second.outcomes[0].tarball).unwrap()
        );
        assert_eq!(
            std::fs::read(first.catalog).unwrap(),
            std::fs::read(second.catalog).unwrap()
        );
        assert_eq!(
            std::fs::read_to_string(source_file).unwrap(),
            "source remains unchanged"
        );
        assert_eq!(first.outcomes[0].entry.upstream.repo, "local-runtime-root");
        assert_eq!(first.outcomes[0].entry.libc.kind, "none");
        assert_eq!(first.outcomes[0].entry.upstream.sha256.len(), 64);
    }
}

#[test]
fn a_mislabelled_native_runtime_creates_no_output() {
    let work = tempfile::tempdir().unwrap();
    let root = common::fake_otp::FakeOtp::new()
        .windows()
        .otp_version("29.0.5")
        .build_in(work.path().join("source"));
    let out = work.path().join("out");
    let error = catalog::repack_from_root(
        &native_options(&out, "macos-aarch64"),
        &root.root,
        &ginary::diag::Diag::disabled(),
    )
    .unwrap_err();
    assert!(error.to_string().contains("windows-x86_64"));
    assert!(!out.exists());
    let mut options = native_options(&out, "windows-x86_64");
    options.upstream_tag = "OTP-29.0.4".into();
    assert!(
        catalog::repack_from_root(&options, &root.root, &ginary::diag::Diag::disabled())
            .unwrap_err()
            .to_string()
            .contains("29.0.5")
    );
    assert!(!out.exists());
}

#[test]
fn native_repacking_refuses_an_output_inside_the_source_installation() {
    let work = tempfile::tempdir().unwrap();
    let root = common::fake_otp::FakeOtp::new()
        .windows()
        .otp_version("29.0.5")
        .build_in(work.path().join("source"));
    let out = root.root.join("repacked");
    let result = catalog::repack_from_root(
        &native_options(&out, "windows-x86_64"),
        &root.root,
        &ginary::diag::Diag::disabled(),
    );
    assert!(
        result.is_err(),
        "repacking must never write into its source installation"
    );
    assert!(!out.exists());
}

fn fragment(target: &str, bytes: &[u8]) -> Catalog {
    use sha2::{Digest as _, Sha256};
    let mut catalog = Catalog::empty("1970-01-01T00:00:00Z");
    catalog.insert(
        "29.0.5",
        29,
        "17.0.5",
        target,
        "default",
        Variant {
            url: format!("otp-29.0.5-{target}-default.tar.zst"),
            sha256: hex::encode(Sha256::digest(bytes)),
            size: bytes.len() as u64,
            ..Variant::default()
        },
    );
    catalog
}

#[test]
fn catalog_merge_is_order_independent_and_refuses_conflicting_claims() {
    let one = fragment("windows-x86_64", b"one");
    let two = fragment("macos-aarch64", b"two");
    assert_eq!(
        catalog::merge_catalogs(&[one.clone(), two.clone()]).unwrap(),
        catalog::merge_catalogs(&[two, one.clone()]).unwrap()
    );
    let conflict = fragment("windows-x86_64", b"other");
    assert!(
        catalog::merge_catalogs(&[one, conflict])
            .unwrap_err()
            .to_string()
            .contains("conflicting runtime")
    );
    assert!(catalog::merge_catalogs(&[]).is_err());
}

fn distribution_inputs(root: &Path) {
    for target in ginary::target::ALL {
        let name = target.name();
        let dir = root.join(format!("dist-{name}"));
        std::fs::create_dir_all(&dir).unwrap();
        let fragment = fragment(&name, b"runtime fixture");
        std::fs::write(dir.join("catalog.json"), fragment.to_json()).unwrap();
        std::fs::write(
            dir.join(&fragment.otp["29.0.5"].targets[&name].variants["default"].url),
            b"runtime fixture",
        )
        .unwrap();
        for (prefix, flavor) in [("ginary", "full"), ("ginary-stub", "stub")] {
            let mut marker = common::stubfile::Marker::for_target(&target);
            marker.flavor = flavor.into();
            let marker = marker.bytes();
            let machine = if target.arch == Arch::X86_64 { 62 } else { 183 };
            let mut bytes = match target.os {
                Os::Linux => common::native::program(machine, None),
                Os::Macos => common::macho::thin_header(
                    if target.arch == Arch::X86_64 {
                        0x0100_0007
                    } else {
                        0x0100_000c
                    },
                    2,
                ),
                Os::Windows => common::stubfile::pe_bytes(0x8664, &marker),
            };
            if target.os != Os::Windows {
                bytes.extend_from_slice(&marker);
            }
            let suffix = if target.os == Os::Windows { ".exe" } else { "" };
            std::fs::write(
                dir.join(format!(
                    "{prefix}-{}-{name}{suffix}",
                    env!("CARGO_PKG_VERSION")
                )),
                bytes,
            )
            .unwrap();
        }
    }
}

#[test]
fn distribution_inventory_contains_all_targets_and_verifies_runtime_bytes() {
    let work = tempfile::tempdir().unwrap();
    let inputs = work.path().join("inputs");
    distribution_inputs(&inputs);
    let out = work.path().join("out");
    let report = catalog::assemble_distribution(&inputs, &out, env!("CARGO_PKG_VERSION")).unwrap();
    assert_eq!(
        report.assets.len(),
        22,
        "seven full binaries, seven stubs, seven runtimes, one catalog"
    );
    let merged = Catalog::parse(
        &std::fs::read_to_string(out.join("catalog.json")).unwrap(),
        "merged",
    )
    .unwrap();
    assert_eq!(merged.otp["29.0.5"].targets.len(), 7);
    let sums = std::fs::read_to_string(out.join("SHA256SUMS")).unwrap();
    assert_eq!(sums.lines().count(), 23, "the inventory is checksummed too");
    let again = work.path().join("again");
    catalog::assemble_distribution(&inputs, &again, env!("CARGO_PKG_VERSION")).unwrap();
    assert_eq!(
        std::fs::read(out.join("inventory.json")).unwrap(),
        std::fs::read(again.join("inventory.json")).unwrap()
    );
    assert!(catalog::assemble_distribution(&inputs, &out, env!("CARGO_PKG_VERSION")).is_err());
    std::fs::write(
        inputs.join("dist-windows-x86_64/otp-29.0.5-windows-x86_64-default.tar.zst"),
        "broken",
    )
    .unwrap();
    let refused = work.path().join("refused");
    assert!(
        catalog::assemble_distribution(&inputs, &refused, env!("CARGO_PKG_VERSION"))
            .unwrap_err()
            .to_string()
            .contains("runtime bytes disagree")
    );
    assert!(!refused.exists());
}

#[test]
fn publication_transaction_is_rehearsed_with_a_readonly_mock_and_no_hosted_operations() {
    let git_bash = std::env::var_os("ProgramFiles")
        .map(std::path::PathBuf::from)
        .map(|path| path.join("Git/bin/bash.exe"))
        .filter(|path| path.is_file());
    let shell = git_bash.or_else(|| {
        common::tools::require_tools(&["bash"]).map(|tools| tools.path("bash").to_owned())
    });
    let Some(shell) = shell else {
        return;
    };
    let root = common::repo::root();
    for scenario in ["good", "fail", "extra", "already-public"] {
        let work = tempfile::tempdir().unwrap();
        let mut command = std::process::Command::new(&shell);
        command
            .arg(root.join("tests/fixtures/release/publish_rehearsal.sh"))
            .arg(root.join("scripts/ci/publish-distribution.sh"))
            .arg(work.path())
            .arg(scenario);
        let output = common::bounded::run_bounded(
            &mut command,
            std::time::Duration::from_secs(30),
            "local publication rehearsal with a readonly mock gh",
        );
        assert_eq!(
            output.status.success(),
            scenario == "good",
            "{scenario}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(work.path().join("published").exists(), scenario == "good");
        let commands = std::fs::read_to_string(work.path().join("commands.log")).unwrap();
        assert!(!commands.contains("release create"));
        if scenario == "already-public" {
            assert!(!commands.contains("release upload"));
        }
    }
}

#[test]
fn distribution_does_not_require_publishing_an_unverified_release() {
    let workflow = common::repo::yaml(".github/workflows/distribute.yml");
    let events = &workflow["on"];
    assert!(
        events.as_mapping_get("release").is_none(),
        "a published release is too late to begin verification"
    );
    assert!(
        events["workflow_dispatch"]["inputs"]["tag"]
            .as_mapping()
            .is_some(),
        "a rehearsal names its tag explicitly"
    );
    assert_eq!(
        events["workflow_dispatch"]["inputs"]["publish"]["default"].as_bool(),
        Some(false),
        "publishing requires an explicit choice"
    );
}

#[test]
fn distribution_keeps_catalog_fragments_separate_until_they_are_merged() {
    let steps = common::repo::workflow_steps(".github/workflows/distribute.yml");
    for step in &steps {
        for command in step.commands() {
            assert!(
                !command.contains("gh release create"),
                "release-please already owns the release; distribution must never create a second one"
            );
        }
    }
    let text = common::repo::read(".github/workflows/distribute.yml");
    assert!(
        !text.contains("merge-multiple: true"),
        "catalog.json fragments overwrite each other when artifacts are flattened"
    );
    assert!(
        steps
            .iter()
            .flat_map(common::repo::WorkflowStep::commands)
            .any(|line| line.contains("otp merge")),
        "the catalog and inventory must be assembled by the checked local command"
    );
}

fn edit_fragment(inputs: &Path, target: &str, edit: impl FnOnce(&mut Catalog)) {
    let path = inputs.join(format!("dist-{target}/catalog.json"));
    let mut value = Catalog::parse(&std::fs::read_to_string(&path).unwrap(), "test fragment")
        .expect("the original catalog");
    edit(&mut value);
    std::fs::write(path, value.to_json()).unwrap();
}

fn refuses_distribution_input(mutate: impl FnOnce(&Path), expected: &str) {
    let work = tempfile::tempdir().unwrap();
    let inputs = work.path().join("inputs");
    distribution_inputs(&inputs);
    mutate(&inputs);
    let out = work.path().join("distribution");
    let error = catalog::assemble_distribution(&inputs, &out, env!("CARGO_PKG_VERSION"))
        .expect_err("invalid fragments must never become a distributable release");
    assert!(error.to_string().contains(expected), "{error}");
    assert!(!out.exists(), "refusal must precede output reservation");
    assert!(
        inputs.is_dir(),
        "the rejected inputs remain available for repair"
    );
}

macro_rules! distribution_refusal {
    ($name:ident, $edit:expr, $message:expr) => {
        #[test]
        fn $name() {
            refuses_distribution_input($edit, $message);
        }
    };
}

distribution_refusal!(
    missing_target_is_refused_before_publication,
    |inputs| {
        std::fs::remove_dir_all(inputs.join("dist-windows-x86_64")).unwrap();
    },
    "expected target directories"
);

distribution_refusal!(
    unexpected_target_is_refused_before_publication,
    |inputs| {
        std::fs::create_dir(inputs.join("dist-unsupported-target")).unwrap();
    },
    "expected target directories"
);

distribution_refusal!(
    a_fragment_cannot_be_a_regular_file,
    |inputs| {
        let path = inputs.join("dist-windows-x86_64");
        std::fs::remove_dir_all(&path).unwrap();
        std::fs::write(path, "not a directory").unwrap();
    },
    "fragments must be real directories"
);

distribution_refusal!(
    missing_catalog_does_not_become_an_empty_fragment,
    |inputs| {
        std::fs::remove_file(inputs.join("dist-windows-x86_64/catalog.json")).unwrap();
    },
    "missing runtime catalog fragment"
);

distribution_refusal!(
    a_fragment_cannot_claim_a_different_target,
    |inputs| {
        edit_fragment(inputs, "windows-x86_64", |catalog| {
            let entry = catalog.otp.get_mut("29.0.5").unwrap();
            let target = entry.targets.remove("windows-x86_64").unwrap();
            entry.targets.insert("macos-aarch64".into(), target);
        });
    },
    "must describe its own target only"
);

distribution_refusal!(
    empty_runtime_variants_are_refused,
    |inputs| {
        edit_fragment(inputs, "windows-x86_64", |catalog| {
            catalog
                .otp
                .get_mut("29.0.5")
                .unwrap()
                .targets
                .get_mut("windows-x86_64")
                .unwrap()
                .variants
                .clear();
        });
    },
    "has no runtime variants"
);

distribution_refusal!(
    remote_runtime_urls_are_refused_by_local_assembly,
    |inputs| {
        edit_fragment(inputs, "windows-x86_64", |catalog| {
            catalog
                .otp
                .get_mut("29.0.5")
                .unwrap()
                .targets
                .get_mut("windows-x86_64")
                .unwrap()
                .variants
                .get_mut("default")
                .unwrap()
                .url = "https://example.invalid/otp-runtime.tar.zst".into();
        });
    },
    "must be one local otp-*.tar.zst filename"
);

distribution_refusal!(
    runtime_urls_cannot_escape_their_fragment,
    |inputs| {
        edit_fragment(inputs, "windows-x86_64", |catalog| {
            catalog
                .otp
                .get_mut("29.0.5")
                .unwrap()
                .targets
                .get_mut("windows-x86_64")
                .unwrap()
                .variants
                .get_mut("default")
                .unwrap()
                .url = "../otp-runtime.tar.zst".into();
        });
    },
    "must be one local otp-*.tar.zst filename"
);

distribution_refusal!(
    runtime_size_must_match_even_when_the_digest_matches,
    |inputs| {
        edit_fragment(inputs, "windows-x86_64", |catalog| {
            catalog
                .otp
                .get_mut("29.0.5")
                .unwrap()
                .targets
                .get_mut("windows-x86_64")
                .unwrap()
                .variants
                .get_mut("default")
                .unwrap()
                .size += 1;
        });
    },
    "runtime bytes disagree with catalog digest or size"
);

distribution_refusal!(
    unlisted_assets_cannot_enter_the_inventory,
    |inputs| {
        std::fs::write(
            inputs.join("dist-windows-x86_64/unlisted.txt"),
            "user notes",
        )
        .unwrap();
    },
    "asset set mismatch"
);

distribution_refusal!(
    subdirectories_cannot_enter_the_asset_inventory,
    |inputs| {
        std::fs::create_dir(inputs.join("dist-windows-x86_64/unlisted-directory")).unwrap();
    },
    "assets must be regular files"
);

distribution_refusal!(
    cli_and_stub_names_cannot_hide_a_flavor_swap,
    |inputs| {
        let dir = inputs.join("dist-windows-x86_64");
        let full = dir.join(format!(
            "ginary-{}-windows-x86_64.exe",
            env!("CARGO_PKG_VERSION")
        ));
        let stub = dir.join(format!(
            "ginary-stub-{}-windows-x86_64.exe",
            env!("CARGO_PKG_VERSION")
        ));
        let full_bytes = std::fs::read(&full).unwrap();
        std::fs::write(full, std::fs::read(&stub).unwrap()).unwrap();
        std::fs::write(stub, full_bytes).unwrap();
    },
    "binary flavor disagrees"
);

distribution_refusal!(
    all_target_fragments_must_supply_the_same_otp_version,
    |inputs| {
        edit_fragment(inputs, "windows-x86_64", |catalog| {
            let entry = catalog.otp.remove("29.0.5").unwrap();
            catalog.otp.insert("29.0.4".into(), entry);
        });
    },
    "must provide the same OTP version"
);

#[test]
fn different_targets_cannot_publish_one_runtime_filename_twice() {
    refuses_distribution_input(
        |inputs| {
            for target in ["windows-x86_64", "macos-aarch64"] {
                let dir = inputs.join(format!("dist-{target}"));
                edit_fragment(inputs, target, |catalog| {
                    let runtime = catalog
                        .otp
                        .get_mut("29.0.5")
                        .unwrap()
                        .targets
                        .get_mut(target)
                        .unwrap()
                        .variants
                        .get_mut("default")
                        .unwrap();
                    std::fs::rename(dir.join(&runtime.url), dir.join("otp-shared.tar.zst"))
                        .unwrap();
                    runtime.url = "otp-shared.tar.zst".into();
                });
            }
        },
        "duplicate distribution asset filename",
    );
}

#[test]
fn assembly_refuses_a_version_other_than_its_binaries() {
    let work = tempfile::tempdir().unwrap();
    let inputs = work.path().join("inputs");
    distribution_inputs(&inputs);
    let out = work.path().join("out");
    let error = catalog::assemble_distribution(&inputs, &out, "unrelated-version").unwrap_err();
    assert!(error.to_string().contains("must match this ginary build"));
    assert!(!out.exists());
}

#[test]
fn catalog_merge_keeps_extensions_and_latest_time_independent_of_order() {
    let mut first = fragment("windows-x86_64", b"runtime");
    first.extra.insert(
        "provenance-policy".into(),
        serde_json::json!({"checked": true}),
    );
    first
        .otp
        .get_mut("29.0.5")
        .unwrap()
        .extra
        .insert("support".into(), serde_json::json!("verified"));
    first
        .otp
        .get_mut("29.0.5")
        .unwrap()
        .targets
        .get_mut("windows-x86_64")
        .unwrap()
        .extra
        .insert("runner".into(), serde_json::json!("windows"));
    let mut later = first.clone();
    later.generated_at = "2026-09-08T00:00:00Z".into();
    let forward = catalog::merge_catalogs(&[first.clone(), later.clone()]).unwrap();
    let reverse = catalog::merge_catalogs(&[later.clone(), first]).unwrap();
    assert_eq!(forward, reverse);
    assert_eq!(
        forward, later,
        "duplicate identical claims preserve every extension exactly"
    );
}

#[test]
fn catalog_merge_refuses_conflicting_extensions_at_each_scope() {
    for scope in ["catalog", "otp", "target"] {
        let mut first = fragment("windows-x86_64", b"runtime");
        let extras = match scope {
            "catalog" => &mut first.extra,
            "otp" => &mut first.otp.get_mut("29.0.5").unwrap().extra,
            _ => {
                &mut first
                    .otp
                    .get_mut("29.0.5")
                    .unwrap()
                    .targets
                    .get_mut("windows-x86_64")
                    .unwrap()
                    .extra
            }
        };
        extras.insert("origin".into(), serde_json::json!("one"));
        let mut second = first.clone();
        let extras = match scope {
            "catalog" => &mut second.extra,
            "otp" => &mut second.otp.get_mut("29.0.5").unwrap().extra,
            _ => {
                &mut second
                    .otp
                    .get_mut("29.0.5")
                    .unwrap()
                    .targets
                    .get_mut("windows-x86_64")
                    .unwrap()
                    .extra
            }
        };
        extras.insert("origin".into(), serde_json::json!("different"));
        let error = catalog::merge_catalogs(&[first, second]).unwrap_err();
        assert!(
            error.to_string().contains("conflicting metadata"),
            "{scope}: {error}"
        );
    }
}

#[test]
fn catalog_merge_refuses_schema_and_otp_identity_conflicts() {
    let original = fragment("windows-x86_64", b"runtime");
    let mut unsupported = original.clone();
    unsupported.schema_version += 1;
    assert!(
        catalog::merge_catalogs(&[unsupported])
            .unwrap_err()
            .to_string()
            .contains("unsupported catalog schema")
    );
    for field in ["release", "erts"] {
        let mut different = original.clone();
        let otp = different.otp.get_mut("29.0.5").unwrap();
        if field == "release" {
            otp.otp_release += 1;
        } else {
            otp.erts_vsn = "different".into();
        }
        assert!(
            catalog::merge_catalogs(&[original.clone(), different])
                .unwrap_err()
                .to_string()
                .contains("conflicting OTP metadata")
        );
    }
}
