// SPDX-License-Identifier: MIT OR Apache-2.0
//! Planning distinct output paths and publishing complete files atomically.
//!
//! Existing files are compared by identity, including hard links. Paths that
//! do not yet exist are resolved through their existing ancestors before they
//! are compared, so alternate spellings cannot hide an output collision.

use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};

/// Resolves an output through existing ancestors, retaining its missing tail.
///
/// Symbolic links are resolved before a following `..`, matching filesystem
/// traversal rather than normalising away a link before it can be followed.
///
/// # Errors
///
/// An inaccessible ancestor, an invalid component, or an unreadable current
/// directory. A missing final path is an ordinary planned output.
pub fn resolve(path: &Path) -> io::Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    // Start with the deepest path: a sandbox may permit a project while
    // refusing inspection of a parent home directory. Walking from the
    // filesystem root would need permissions the output itself does not.
    let mut ancestor = absolute.as_path();
    let mut missing = Vec::new();
    loop {
        match ancestor.canonicalize() {
            Ok(mut resolved) => {
                for part in missing.iter().rev() {
                    match Path::new(part).components().next() {
                        Some(Component::ParentDir) => {
                            resolved.pop();
                        }
                        Some(Component::CurDir) | None => {}
                        _ => resolved.push(part),
                    }
                }
                return Ok(resolved);
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let Some(part) = ancestor.components().next_back() else {
                    return Err(error);
                };
                let Some(parent) = ancestor.parent() else {
                    return Err(error);
                };
                missing.push(part.as_os_str().to_os_string());
                ancestor = parent;
            }
            Err(error) => return Err(error),
        }
    }
}

/// The comparison spelling for the host's ordinary path case rules.
fn comparison_path(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        PathBuf::from(path.as_os_str().to_string_lossy().to_lowercase())
    }
    #[cfg(not(windows))]
    {
        path.to_path_buf()
    }
}

/// Whether two paths name the same existing file or planned destination.
///
/// # Errors
///
/// An existing file or an ancestor cannot be inspected. Missing final paths
/// are compared by their resolved spelling.
pub fn aliases(left: &Path, right: &Path) -> io::Result<bool> {
    let a = resolve(left)?;
    let b = resolve(right)?;
    if comparison_path(&a) == comparison_path(&b) {
        return Ok(true);
    }
    for path in [&a, &b] {
        check_identity_kind(path)?;
    }
    match same_file::is_same_file(&a, &b) {
        Ok(same) => Ok(same),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

/// Refuses special files before an identity handle could block opening a FIFO.
fn check_identity_kind(path: &Path) -> io::Result<()> {
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_file() || metadata.is_dir() => Ok(()),
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("cannot validate special file {}", path.display()),
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// Whether `path` is `directory` or lies within its resolved tree.
///
/// # Errors
///
/// Either path has an ancestor that cannot be inspected.
pub fn within(path: &Path, directory: &Path) -> io::Result<bool> {
    Ok(comparison_path(&resolve(path)?).starts_with(comparison_path(&resolve(directory)?)))
}

/// Refuses outputs that alias one another or need one another as directories.
///
/// # Errors
///
/// [`io::ErrorKind::InvalidInput`] naming both conflicting outputs, or an
/// error inspecting their existing ancestors. Windows destinations with
/// ambiguous components, alternate streams or device names are refused.
pub fn validate_distinct(paths: &[PathBuf]) -> io::Result<()> {
    #[cfg(windows)]
    for path in paths {
        validate_windows_destination(path)?;
    }
    for (index, path) in paths.iter().enumerate() {
        for other in &paths[..index] {
            if aliases(path, other)? || within(path, other)? || within(other, path)? {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!(
                        "output paths {} and {} conflict",
                        path.display(),
                        other.display()
                    ),
                ));
            }
        }
    }
    Ok(())
}

/// Windows path spelling must agree between canonical inspection and Win32
/// publication. In particular, a missing `name.` canonicalizes as a verbatim
/// tail, but MoveFileExW publishes an ordinary path onto `name`.
///
/// Refuse ambiguous components, streams and devices before opening any file.
/// Ordinary verbatim disk/UNC paths remain supported; unusual verbatim names
/// follow the same conservative rule, so callers get one output-file contract.
#[cfg(windows)]
fn validate_windows_destination(path: &Path) -> io::Result<()> {
    use std::path::Prefix;

    fn component_reason(name: &std::ffi::OsStr) -> Option<&'static str> {
        let name = name.to_string_lossy();
        if name.ends_with(['.', ' ']) {
            return Some("a component ends with a dot or space");
        }
        if name
            .chars()
            .any(|ch| ch < ' ' || matches!(ch, '<' | '>' | ':' | '"' | '|' | '?' | '*' | '/'))
        {
            return Some("a component names a stream or contains a reserved character");
        }
        let stem = name
            .split('.')
            .next()
            .unwrap_or_default()
            .trim_end_matches(' ')
            .to_uppercase();
        let port = stem
            .strip_prefix("COM")
            .or_else(|| stem.strip_prefix("LPT"));
        if matches!(
            stem.as_str(),
            "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
        ) || matches!(
            port,
            Some("1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³")
        ) {
            return Some("a component names a reserved Windows device");
        }
        None
    }

    for component in path.components() {
        let reason = match component {
            Component::Prefix(prefix) => match prefix.kind() {
                Prefix::DeviceNS(_) | Prefix::Verbatim(_) => {
                    Some("device namespaces cannot be output files")
                }
                Prefix::Disk(_) if !path.has_root() => {
                    Some("a drive-relative path has an ambiguous working directory")
                }
                Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => {
                    component_reason(server).or_else(|| component_reason(share))
                }
                _ => None,
            },
            Component::Normal(name) => component_reason(name),
            _ => None,
        };
        if let Some(reason) = reason {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "unsafe Windows output destination {}: {reason}",
                    path.display()
                ),
            ));
        }
    }
    Ok(())
}

/// Maximum directory entries inspected by one input-identity validation.
const INPUT_ENTRY_LIMIT: usize = 1_000_000;

/// Refuses existing output hard links to files below any protected input tree.
///
/// Missing trees are allowed because export may not have created the shipment
/// yet. Existing trees must be fully readable; cyclic directory aliases are
/// visited once, and exceeding one million entries is a reported refusal.
///
/// # Errors
///
/// An output aliases an input, a tree cannot be inspected, or the scan exceeds
/// its explicit resource bound. No unknown input is treated as safe.
pub fn validate_input_trees(outputs: &[PathBuf], trees: &[PathBuf]) -> io::Result<()> {
    input_tree_identities(outputs, trees, INPUT_ENTRY_LIMIT)
}

/// The bounded scan, with a small bound available to unit tests.
fn input_tree_identities(outputs: &[PathBuf], trees: &[PathBuf], limit: usize) -> io::Result<()> {
    let mut identities = std::collections::HashMap::new();
    for output in outputs {
        check_identity_kind(output)?;
        match same_file::Handle::from_path(output) {
            Ok(handle) => {
                identities.insert(handle, output);
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    if identities.is_empty() {
        return Ok(());
    }
    let mut pending = Vec::new();
    for tree in trees {
        match tree.canonicalize() {
            Ok(path) => pending.push(path),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    let mut visited = std::collections::BTreeSet::new();
    let mut count = 0usize;
    while let Some(directory) = pending.pop() {
        if !visited.insert(directory.clone()) {
            continue;
        }
        for entry in std::fs::read_dir(&directory)? {
            let entry = entry?;
            count = count.saturating_add(1);
            if count > limit {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "input identity scan exceeded {limit} entries at {}",
                        directory.display()
                    ),
                ));
            }
            let path = entry.path();
            let metadata = std::fs::metadata(&path)?;
            if metadata.is_dir() {
                pending.push(path.canonicalize()?);
            } else if metadata.is_file() {
                let identity = same_file::Handle::from_path(&path)?;
                if let Some(output) = identities.get(&identity) {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!(
                            "output {} aliases input {}",
                            output.display(),
                            path.display()
                        ),
                    ));
                }
            } else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("cannot validate special input entry {}", path.display()),
                ));
            }
        }
    }
    Ok(())
}

/// Writes and flushes a complete file before replacing the destination name.
///
/// The temporary file is in the destination directory. A failed write leaves
/// the previous destination intact; a hard link or symlink at the destination
/// is replaced without truncating the file reached through another name.
/// The parent must already exist, so this does not create unexpected trees.
///
/// # Errors
///
/// Creating, writing, syncing, or publishing the temporary file failed, or a
/// Windows destination names an ambiguous path, alternate stream or device.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    #[cfg(windows)]
    validate_windows_destination(path)?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    if matches!(
        crate::fault::point("output-write"),
        Some("fail" | "fail-document")
    ) {
        temporary.write_all(&bytes[..bytes.len() / 2])?;
        return Err(io::Error::other(
            "injected output-write failure after a partial temporary write",
        ));
    }
    temporary.write_all(bytes)?;
    temporary.as_file().sync_all()?;
    if matches!(
        crate::fault::point("output-persist"),
        Some("fail" | "fail-document")
    ) {
        return Err(io::Error::other(
            "injected output-persist failure before replacement",
        ));
    }
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_outputs_are_compared_through_the_existing_parent() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let direct = dir.path().join("new/output.json");
        let dotted = dir.path().join("new/./output.json");
        assert!(aliases(&direct, &dotted).expect("resolve missing outputs"));
        assert!(validate_distinct(&[direct, dotted]).is_err());
    }

    #[test]
    fn a_hardlink_is_a_collision_even_with_a_different_name() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let first = dir.path().join("first");
        let other = dir.path().join("other");
        std::fs::write(&first, b"artifact").expect("artifact");
        std::fs::hard_link(&first, &other).expect("hardlink");
        assert!(aliases(&first, &other).expect("file identity"));
        assert!(validate_distinct(&[first, other]).is_err());
    }

    #[test]
    fn an_input_scan_refuses_to_guess_after_reaching_its_bound() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let input = dir.path().join("inputs");
        std::fs::create_dir(&input).expect("input directory");
        std::fs::write(input.join("one"), b"input").expect("input file");
        let output = dir.path().join("output");
        std::fs::write(&output, b"previous artifact").expect("output file");
        let error = input_tree_identities(&[output], &[input], 0).expect_err("zero scan budget");
        assert!(error.to_string().contains("exceeded 0 entries"));
    }

    #[test]
    fn an_output_cannot_also_be_another_outputs_parent() {
        let dir = tempfile::tempdir().expect("temporary directory");
        assert!(
            validate_distinct(&[dir.path().join("new"), dir.path().join("new/nested.json"),])
                .is_err()
        );
        assert!(validate_distinct(&[dir.path().join("new"), dir.path().join("newer"),]).is_ok());
    }

    #[test]
    fn a_failed_publication_keeps_the_destination_directory_and_removes_the_temporary() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let destination = dir.path().join("existing");
        std::fs::create_dir(&destination).expect("destination directory");
        std::fs::write(destination.join("kept"), b"old").expect("existing data");
        assert!(atomic_write(&destination, b"replacement").is_err());
        assert_eq!(
            std::fs::read(destination.join("kept")).expect("retained data"),
            b"old"
        );
        assert_eq!(std::fs::read_dir(dir.path()).expect("directory").count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_parent_is_resolved_before_parent_components() {
        let dir = tempfile::tempdir().expect("temporary directory");
        std::fs::create_dir_all(dir.path().join("actual/nested")).expect("actual tree");
        std::os::unix::fs::symlink(dir.path().join("actual/nested"), dir.path().join("link"))
            .expect("directory symlink");
        assert!(
            aliases(
                &dir.path().join("link/../new"),
                &dir.path().join("actual/new"),
            )
            .expect("same planned file")
        );
    }

    #[cfg(windows)]
    #[test]
    fn missing_output_case_aliases_are_refused_on_windows() {
        let dir = tempfile::tempdir().expect("temporary directory");
        assert!(
            aliases(&dir.path().join("NEW.JSON"), &dir.path().join("new.json"))
                .expect("Windows case aliases")
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_normalization_cannot_merge_a_fresh_artifact_and_its_sidecar() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let artifact = dir.path().join("artifact");
        for suffix in [".", " "] {
            let sidecar = dir.path().join(format!("artifact{suffix}"));
            assert!(
                validate_distinct(&[artifact.clone(), sidecar]).is_err(),
                "Win32 publication treats the missing artifact{suffix:?} as artifact"
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn ambiguous_windows_destinations_fail_before_any_temporary_or_device_is_opened() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let artifact = dir.path().join("artifact");
        std::fs::write(&artifact, b"previous artifact").expect("existing artifact");
        for name in [
            "artifact.",
            "artifact ",
            "parent./artifact",
            "parent /artifact",
            "CON",
            "nul.txt",
            "NUL .json",
            "AUX",
            "prn",
            "COM9.json",
            "LPT1",
            "COM¹",
            "LPT².log",
            "CONIN$",
            "CONOUT$",
            "artifact:metadata",
            "artifact::$DATA",
        ] {
            let destination = dir.path().join(name);
            let error = validate_distinct(std::slice::from_ref(&destination))
                .expect_err("ambiguous destination must fail before probing devices");
            assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{name}: {error}");
            let error = atomic_write(&destination, b"sidecar")
                .expect_err("standalone atomic publication has the same safety rule");
            assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{name}: {error}");
            assert_eq!(std::fs::read(&artifact).unwrap(), b"previous artifact");
            assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        }
        for destination in [
            PathBuf::from(r"\\.\PhysicalDrive0"),
            PathBuf::from(r"\\?\GLOBALROOT\Device\HarddiskVolume1\output"),
            PathBuf::from("C:drive-relative"),
            dir.path().canonicalize().unwrap().join("artifact."),
        ] {
            let error = validate_distinct(std::slice::from_ref(&destination))
                .expect_err("non-file namespaces and ambiguous verbatim paths are refused");
            assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        }
    }

    #[cfg(windows)]
    #[test]
    fn ordinary_windows_names_and_canonical_verbatim_paths_remain_writable() {
        let dir = tempfile::tempdir().expect("temporary directory");
        for name in [
            ".config",
            "auxiliary",
            "comic1",
            "COM10",
            "lpt10.txt",
            "日本語.json",
        ] {
            let destination = dir.path().canonicalize().unwrap().join(name);
            validate_distinct(std::slice::from_ref(&destination)).expect("ordinary output path");
            atomic_write(&destination, b"complete document").expect("ordinary publication");
            assert_eq!(std::fs::read(destination).unwrap(), b"complete document");
        }
    }
}
