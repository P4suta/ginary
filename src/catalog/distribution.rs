// SPDX-License-Identifier: MIT OR Apache-2.0
//! Local runtime repacking and deterministic distribution assembly.

use super::*;
use std::collections::BTreeSet;

fn failure(path: &Path, message: impl ToString) -> RepackError {
    RepackError::Io {
        path: path.to_owned(),
        message: message.to_string(),
    }
}

fn require_output_outside(source: &Path, out: &Path) -> Result<(), RepackError> {
    let source = source
        .canonicalize()
        .map_err(|error| failure(source, error))?;
    if let Some(ancestor) = out.ancestors().find(|path| path.exists())
        && ancestor
            .canonicalize()
            .map_err(|error| failure(ancestor, error))?
            .starts_with(source)
    {
        return Err(failure(out, "output must be outside the input source"));
    }
    Ok(())
}

fn verify_distribution_binaries(
    dir: &Path,
    target: &Target,
    version: &str,
) -> Result<(), RepackError> {
    let suffix = if target.os == crate::target::Os::Windows {
        ".exe"
    } else {
        ""
    };
    for (prefix, flavor) in [
        ("ginary", crate::stubid::Flavor::Full),
        ("ginary-stub", crate::stubid::Flavor::Stub),
    ] {
        let binary = dir.join(format!("{prefix}-{version}-{}{suffix}", target.name()));
        let id = crate::stub::verify(&binary, target).map_err(|error| failure(&binary, error))?;
        if id.flavor != flavor {
            return Err(failure(
                &binary,
                "binary flavor disagrees with its distribution filename",
            ));
        }
    }
    Ok(())
}

fn publish_distribution(
    staging: tempfile::TempDir,
    out: &Path,
    assets: &[DistributionAsset],
) -> Result<(), RepackError> {
    let result: std::io::Result<()> = (|| {
        // Unlike rename, this cannot replace an empty directory that appeared
        // after input validation. Only this successful reservation owns output.
        std::fs::create_dir(out)?;
        for asset in assets {
            let source = staging.path().join(&asset.name);
            let mut temporary = tempfile::NamedTempFile::new_in(out)?;
            std::io::copy(&mut std::fs::File::open(source)?, &mut temporary)?;
            temporary.as_file().sync_all()?;
            let (digest, size) = digest_file(temporary.path())
                .map_err(|error| std::io::Error::other(error.to_string()))?;
            if digest != asset.sha256 || size != asset.size {
                return Err(std::io::Error::other(
                    "distribution bytes changed during publication",
                ));
            }
            temporary
                .persist_noclobber(out.join(&asset.name))
                .map_err(|error| error.error)?;
        }
        Ok(())
    })();
    if let Err(error) = result {
        let retained = staging.keep();
        return Err(failure(
            out,
            format!(
                "distribution publication failed: {error}; verified staging retained at {}",
                retained.display()
            ),
        ));
    }
    Ok(())
}

/// Repacks one installed runtime without modifying the installation or using the network.
///
/// The source tree is copied and hashed before pruning; its provenance explicitly identifies
/// a local tree, rather than claiming verification against an upstream download digest.
///
/// # Errors
/// Refuses multiple selectors, a target/version/linkage mismatch, unsafe links, or failed I/O.
pub fn repack_from_root(
    options: &RepackOptions,
    root: &Path,
    diag: &Diag,
) -> Result<RepackReport, RepackError> {
    require_output_outside(root, &options.out)?;
    let [selector] = options.selectors.as_slice() else {
        return Err(failure(root, "--root requires exactly one target"));
    };
    if options.upstream_dir.is_some() {
        return Err(failure(
            root,
            "--root and --upstream-dir are mutually exclusive",
        ));
    }
    let requested: Target = selector
        .target
        .parse()
        .map_err(|error| failure(root, error))?;
    if !["default", "static", "dynamic"].contains(&selector.variant.as_str()) {
        return Err(failure(
            root,
            "runtime variant must be default, static, or dynamic",
        ));
    }
    let version = version_from_tag(&options.upstream_tag).ok_or_else(|| RepackError::BadTag {
        tag: options.upstream_tag.clone(),
    })?;
    let resolved = crate::erts_source::resolve(
        &crate::erts_source::ErtsSourceSpec::Dir(root.to_owned()),
        &requested,
    )
    .map_err(|error| failure(root, error))?;
    if resolved.otp.otp_version != version {
        return Err(failure(
            root,
            format!(
                "runtime is OTP {}, requested {version}",
                resolved.otp.otp_version
            ),
        ));
    }
    if resolved.linkage != claimed_linkage(&selector.variant) {
        return Err(RepackError::UpstreamLinkage {
            file: "local-runtime-root".into(),
            variant: selector.variant.clone(),
            claimed: claimed_linkage(&selector.variant).as_str(),
            actual: resolved.linkage.as_str(),
        });
    }
    let work = tempfile::tempdir().map_err(|error| failure(root, error))?;
    let copied = work.path().join("runtime");
    let canonical = root.canonicalize().map_err(|error| failure(root, error))?;
    copy_tree(&canonical, &canonical, &copied, &mut BTreeSet::new())?;
    let source_snapshot = work.path().join("source.tar.zst");
    pack_runtime(&copied, &source_snapshot)?;
    let (source_digest, _) = digest_file(&source_snapshot).map_err(|error| failure(root, error))?;
    let unpacked_bytes = tree_bytes(&copied)?;
    let prune = prune_tree(&copied)?;
    let deref = dereference_symlinks(&copied)?;
    assert_no_symlinks(&copied)?;
    let beam_strip = strip_beams(&copied, diag)?;
    let stamp = timestamp(options.source_date_epoch.unwrap_or_else(now_epoch));
    let name = format!(
        "otp-{version}-{}-{}.tar.zst",
        selector.target, selector.variant
    );
    let temporary = work.path().join(&name);
    let tarball_bytes = pack_runtime(&copied, &temporary)?;
    let (sha256, _) = digest_file(&temporary).map_err(|error| failure(&temporary, error))?;
    let emulator = crate::erts_source::emulator_path(&resolved.otp);
    let mut entry = Variant {
        url: name.clone(),
        sha256,
        size: tarball_bytes,
        linkage: resolved.linkage.as_str().into(),
        nif_loading: resolved.nif_loading,
        libc: LibcSpec {
            kind: resolved.libc_kind().unwrap_or("none").into(),
            min: resolved.libc_min,
            version: None,
        },
        openssl: openssl_version(&copied, &emulator),
        jit: has_jit(&emulator),
        excluded_apps: Vec::new(),
        upstream: Upstream {
            repo: "local-runtime-root".into(),
            tag: options.upstream_tag.clone(),
            file: "source.tar.zst".into(),
            sha256: source_digest,
        },
        built_at: stamp.clone(),
        extra: BTreeMap::new(),
    };
    let catalog_path = options.out.join(CATALOG_FILE);
    let existing = read_or_new(&catalog_path, &stamp)?;
    if let Some(previous) = existing
        .otp
        .get(&version)
        .and_then(|otp| otp.targets.get(&selector.target))
        .and_then(|target| target.variants.get(&selector.variant))
    {
        let mut comparable = previous.clone();
        comparable.built_at.clone_from(&entry.built_at);
        if comparable == entry {
            entry.built_at.clone_from(&previous.built_at);
        }
    }
    let mut fragment = Catalog::empty(&stamp);
    fragment.insert(
        &version,
        resolved.otp.release,
        &resolved.otp.erts_vsn,
        &selector.target,
        &selector.variant,
        entry.clone(),
    );
    let catalog = merge_catalogs(&[existing, fragment])?;
    std::fs::create_dir_all(&options.out).map_err(|error| failure(&options.out, error))?;
    let tarball = options.out.join(name);
    if tarball.exists() {
        let (existing, _) = digest_file(&tarball).map_err(|error| failure(&tarball, error))?;
        if existing != entry.sha256 {
            return Err(failure(
                &tarball,
                "refusing to replace different runtime bytes",
            ));
        }
    } else {
        let mut staged = tempfile::NamedTempFile::new_in(&options.out)
            .map_err(|error| failure(&options.out, error))?;
        std::io::copy(
            &mut std::fs::File::open(&temporary).map_err(|error| failure(&temporary, error))?,
            &mut staged,
        )
        .map_err(|error| failure(&tarball, error))?;
        staged
            .as_file()
            .sync_all()
            .map_err(|error| failure(&tarball, error))?;
        staged
            .persist_noclobber(&tarball)
            .map_err(|error| failure(&tarball, error))?;
    }
    write_catalog(&catalog_path, &catalog)?;
    Ok(RepackReport {
        catalog: catalog_path,
        outcomes: vec![RepackOutcome {
            target: selector.target.clone(),
            variant: selector.variant.clone(),
            upstream_file: "local-runtime-root".into(),
            unpacked_bytes,
            prune,
            deref,
            beam_strip,
            tarball,
            tarball_bytes,
            entry_release: resolved.otp.release,
            erts_vsn: resolved.otp.erts_vsn,
            entry,
        }],
    })
}

fn copy_tree(
    root: &Path,
    source: &Path,
    dest: &Path,
    ancestors: &mut BTreeSet<PathBuf>,
) -> Result<(), RepackError> {
    let actual = source
        .canonicalize()
        .map_err(|error| failure(source, error))?;
    if !actual.starts_with(root) {
        return Err(RepackError::UnsafeSymlink {
            path: source.to_owned(),
            target: actual,
        });
    }
    let metadata = actual.metadata().map_err(|error| failure(source, error))?;
    if metadata.is_dir() {
        if !ancestors.insert(actual.clone()) {
            return Err(failure(source, "runtime contains a symlink cycle"));
        }
        std::fs::create_dir_all(dest).map_err(|error| failure(dest, error))?;
        let mut entries = std::fs::read_dir(&actual)
            .map_err(|error| failure(&actual, error))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| failure(&actual, error))?;
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries {
            copy_tree(
                root,
                &entry.path(),
                &dest.join(entry.file_name()),
                ancestors,
            )?;
        }
        ancestors.remove(&actual);
    } else if metadata.is_file() {
        std::fs::copy(&actual, dest).map_err(|error| failure(dest, error))?;
    } else {
        return Err(failure(source, "runtime contains an unsupported file type"));
    }
    Ok(())
}

/// Merges catalogue fragments without silently replacing a conflicting claim.
///
/// Order does not affect output; the newest fragment timestamp is retained.
/// # Errors
/// Refuses empty input, unsupported schemas and conflicting metadata or variants.
pub fn merge_catalogs(inputs: &[Catalog]) -> Result<Catalog, RepackError> {
    let first = inputs
        .first()
        .ok_or_else(|| failure(Path::new(CATALOG_FILE), "no catalog fragments"))?;
    let mut result = Catalog::empty(&first.generated_at);
    for input in inputs {
        if input.schema_version != SCHEMA_VERSION {
            return Err(failure(
                Path::new(CATALOG_FILE),
                "unsupported catalog schema",
            ));
        }
        if input.generated_at > result.generated_at {
            result.generated_at.clone_from(&input.generated_at);
        }
        merge_extra(&mut result.extra, &input.extra, "catalog")?;
        for (version, incoming) in &input.otp {
            let entry = result
                .otp
                .entry(version.clone())
                .or_insert_with(|| OtpVersionEntry {
                    targets: BTreeMap::new(),
                    ..incoming.clone()
                });
            if entry.erts_vsn != incoming.erts_vsn || entry.otp_release != incoming.otp_release {
                return Err(failure(
                    Path::new(CATALOG_FILE),
                    format!("conflicting OTP metadata for {version}"),
                ));
            }
            merge_extra(&mut entry.extra, &incoming.extra, version)?;
            for (target, target_entry) in &incoming.targets {
                let merged = entry.targets.entry(target.clone()).or_default();
                merge_extra(&mut merged.extra, &target_entry.extra, target)?;
                for (variant, value) in &target_entry.variants {
                    if let Some(previous) = merged.variants.get(variant) {
                        if previous != value {
                            return Err(failure(
                                Path::new(CATALOG_FILE),
                                format!("conflicting runtime {version}/{target}/{variant}"),
                            ));
                        }
                    } else {
                        merged.variants.insert(variant.clone(), value.clone());
                    }
                }
            }
        }
    }
    Ok(result)
}

fn merge_extra(
    output: &mut BTreeMap<String, serde_json::Value>,
    input: &BTreeMap<String, serde_json::Value>,
    context: &str,
) -> Result<(), RepackError> {
    for (key, value) in input {
        if output.get(key).is_some_and(|previous| previous != value) {
            return Err(failure(
                Path::new(CATALOG_FILE),
                format!("conflicting metadata {context}/{key}"),
            ));
        }
        output.insert(key.clone(), value.clone());
    }
    Ok(())
}

/// One checked file in a distribution inventory.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DistributionAsset {
    /// The single filename in the distribution directory.
    pub name: String,
    /// The SHA-256 of the exact bytes to distribute.
    pub sha256: String,
    /// The exact length in bytes.
    pub size: u64,
}

/// The inventory written by a completed local distribution assembly.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DistributionReport {
    /// Inventory wire version.
    pub schema_version: u32,
    /// The ginary version shared by every binary and stub filename.
    pub version: String,
    /// Every payload asset, in filename order; the inventory and checksum file describe these.
    pub assets: Vec<DistributionAsset>,
}

/// Assembles all seven `dist-<target>` fragments into one verified local distribution.
///
/// Output must not exist. After validating staged bytes, the output directory is reserved
/// exclusively and files are published without replacement; `SHA256SUMS` is the final
/// completion marker. A publication failure retains and names staging for diagnosis.
/// No hosted operation is performed.
/// # Errors
/// Refuses missing/extra/duplicate assets, nonlocal catalog URLs, changed runtime bytes,
/// conflicting catalog metadata or an existing output directory.
pub fn assemble_distribution(
    inputs: &Path,
    out: &Path,
    version: &str,
) -> Result<DistributionReport, RepackError> {
    if version != env!("CARGO_PKG_VERSION") {
        return Err(failure(
            inputs,
            "distribution version must match this ginary build",
        ));
    }
    if out.exists() {
        return Err(failure(out, "distribution output already exists"));
    }
    require_output_outside(inputs, out)?;
    let mut catalogs = Vec::new();
    let mut files = BTreeMap::new();
    let expected_dirs: BTreeSet<String> = crate::target::ALL
        .iter()
        .map(|target| format!("dist-{}", target.name()))
        .collect();
    let found_dirs: BTreeSet<String> = std::fs::read_dir(inputs)
        .map_err(|error| failure(inputs, error))?
        .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
        .collect::<Result<_, _>>()
        .map_err(|error| failure(inputs, error))?;
    if found_dirs != expected_dirs {
        return Err(failure(
            inputs,
            format!("expected target directories {expected_dirs:?}; found {found_dirs:?}"),
        ));
    }
    for target in crate::target::ALL {
        let target_name = target.name();
        let dir = inputs.join(format!("dist-{target_name}"));
        if !std::fs::symlink_metadata(&dir)
            .map_err(|error| failure(&dir, error))?
            .is_dir()
        {
            return Err(failure(
                &dir,
                "distribution fragments must be real directories",
            ));
        }
        let fragment = read_or_new(&dir.join(CATALOG_FILE), "")?;
        if fragment.otp.is_empty() {
            return Err(failure(&dir, "missing runtime catalog fragment"));
        }
        let suffix = if target.os == crate::target::Os::Windows {
            ".exe"
        } else {
            ""
        };
        verify_distribution_binaries(&dir, &target, version)?;
        let mut expected = BTreeSet::from([
            CATALOG_FILE.to_owned(),
            format!("ginary-{version}-{target_name}{suffix}"),
            format!("ginary-stub-{version}-{target_name}{suffix}"),
        ]);
        for otp in fragment.otp.values() {
            if otp.targets.len() != 1 || !otp.targets.contains_key(&target_name) {
                return Err(failure(
                    &dir,
                    "catalog fragment must describe its own target only",
                ));
            }
            for runtime in otp.targets[&target_name].variants.values() {
                let url = &runtime.url;
                if url.is_empty()
                    || url.contains(['/', '\\', ':'])
                    || !url.starts_with("otp-")
                    || !url.ends_with(".tar.zst")
                {
                    return Err(failure(
                        &dir,
                        "runtime URL must be one local otp-*.tar.zst filename",
                    ));
                }
                let archive = dir.join(url);
                let (digest, size) =
                    digest_file(&archive).map_err(|error| failure(&archive, error))?;
                if digest != runtime.sha256 || size != runtime.size {
                    return Err(failure(
                        &archive,
                        "runtime bytes disagree with catalog digest or size",
                    ));
                }
                expected.insert(url.clone());
            }
            if otp.targets[&target_name].variants.is_empty() {
                return Err(failure(&dir, "catalog target has no runtime variants"));
            }
        }
        let mut actual = BTreeSet::new();
        for entry in std::fs::read_dir(&dir).map_err(|error| failure(&dir, error))? {
            let entry = entry.map_err(|error| failure(&dir, error))?;
            if !entry
                .file_type()
                .map_err(|error| failure(&entry.path(), error))?
                .is_file()
            {
                return Err(failure(
                    &entry.path(),
                    "distribution assets must be regular files",
                ));
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            actual.insert(name.clone());
            if name != CATALOG_FILE && files.insert(name, entry.path()).is_some() {
                return Err(failure(&dir, "duplicate distribution asset filename"));
            }
        }
        if actual != expected {
            return Err(failure(
                &dir,
                format!("asset set mismatch: expected {expected:?}, found {actual:?}"),
            ));
        }
        catalogs.push(fragment);
    }
    let catalog = merge_catalogs(&catalogs)?;
    if catalog.otp.len() != 1
        || catalog
            .otp
            .values()
            .any(|entry| entry.targets.len() != crate::target::ALL.len())
    {
        return Err(failure(
            inputs,
            "every distribution target must provide the same OTP version",
        ));
    }
    let parent = out
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent).map_err(|error| failure(parent, error))?;
    let staging = tempfile::tempdir_in(parent).map_err(|error| failure(parent, error))?;
    for (name, path) in &files {
        std::fs::copy(path, staging.path().join(name)).map_err(|error| failure(path, error))?;
    }
    for target in crate::target::ALL {
        verify_distribution_binaries(staging.path(), &target, version)?;
    }
    // Verify the bytes actually copied, so a changed input cannot leave a catalog
    // claiming the digest checked before the copy began.
    for otp in catalog.otp.values() {
        for target in otp.targets.values() {
            for runtime in target.variants.values() {
                let path = staging.path().join(&runtime.url);
                let (digest, size) = digest_file(&path).map_err(|error| failure(&path, error))?;
                if digest != runtime.sha256 || size != runtime.size {
                    return Err(failure(
                        &path,
                        "runtime changed while assembling distribution",
                    ));
                }
            }
        }
    }
    write_catalog(&staging.path().join(CATALOG_FILE), &catalog)?;
    files.insert(CATALOG_FILE.into(), staging.path().join(CATALOG_FILE));
    let mut assets = Vec::new();
    for name in files.keys() {
        let path = staging.path().join(name);
        let (sha256, size) = digest_file(&path).map_err(|error| failure(&path, error))?;
        assets.push(DistributionAsset {
            name: name.clone(),
            sha256,
            size,
        });
    }
    let report = DistributionReport {
        schema_version: 1,
        version: version.into(),
        assets,
    };
    let inventory = serde_json::to_vec_pretty(&report).map_err(|error| failure(out, error))?;
    std::fs::write(staging.path().join("inventory.json"), inventory)
        .map_err(|error| failure(out, error))?;
    let mut sums = report
        .assets
        .iter()
        .map(|asset| format!("{}  {}\n", asset.sha256, asset.name))
        .collect::<String>();
    let (digest, inventory_size) =
        digest_file(&staging.path().join("inventory.json")).map_err(|error| failure(out, error))?;
    sums.push_str(&format!("{digest}  inventory.json\n"));
    std::fs::write(staging.path().join("SHA256SUMS"), sums).map_err(|error| failure(out, error))?;
    let mut publication = report.assets.clone();
    publication.push(DistributionAsset {
        name: "inventory.json".into(),
        sha256: digest,
        size: inventory_size,
    });
    let (digest, size) =
        digest_file(&staging.path().join("SHA256SUMS")).map_err(|error| failure(out, error))?;
    publication.push(DistributionAsset {
        name: "SHA256SUMS".into(),
        sha256: digest,
        size,
    });
    publish_distribution(staging, out, &publication)?;
    Ok(report)
}

#[cfg(test)]
mod publication_tests {
    use super::*;

    fn staged_asset(staging: &Path, name: &str, bytes: &[u8]) -> DistributionAsset {
        std::fs::write(staging.join(name), bytes).unwrap();
        let (sha256, size) = digest_file(&staging.join(name)).unwrap();
        DistributionAsset {
            name: name.into(),
            sha256,
            size,
        }
    }

    #[test]
    fn a_directory_created_after_validation_is_never_replaced_or_removed() {
        let work = tempfile::tempdir().unwrap();
        let staging = tempfile::tempdir_in(work.path()).unwrap();
        let retained = staging.path().to_owned();
        let asset = staged_asset(staging.path(), "asset", b"verified");
        let out = work.path().join("out");
        std::fs::create_dir(&out).unwrap();
        let error = publish_distribution(staging, &out, &[asset]).unwrap_err();
        assert!(error.to_string().contains("verified staging retained"));
        assert!(out.is_dir());
        assert_eq!(std::fs::read_dir(&out).unwrap().count(), 0);
        assert_eq!(std::fs::read(retained.join("asset")).unwrap(), b"verified");
    }

    #[test]
    fn partial_publication_retains_evidence_without_a_completion_marker() {
        let work = tempfile::tempdir().unwrap();
        let staging = tempfile::tempdir_in(work.path()).unwrap();
        let retained = staging.path().to_owned();
        let first = staged_asset(staging.path(), "first", b"verified");
        let mut missing = first.clone();
        missing.name = "missing".into();
        let out = work.path().join("out");
        assert!(publish_distribution(staging, &out, &[first, missing]).is_err());
        assert_eq!(std::fs::read(out.join("first")).unwrap(), b"verified");
        assert!(retained.is_dir());
        assert!(!out.join("SHA256SUMS").exists());
    }
}
