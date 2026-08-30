// SPDX-License-Identifier: MIT OR Apache-2.0
//! Opening the running executable.
//!
//! The launcher reads its own payload out of its own file, so it needs a
//! descriptor on the exact bytes it was started from. `/proc/self/exe` is that
//! descriptor: the kernel resolves it to the inode the process is running,
//! which is still readable after the file has been renamed, moved between
//! directories or unlinked. An artifact a user drags across their disk while
//! it is starting therefore still starts.
//!
//! [`std::env::current_exe`] is the fallback, for a machine with no `/proc`
//! mounted. It resolves a *path*, so it is a strictly weaker answer — a
//! renamed artifact is already gone by the time it is used — which is why it
//! is second rather than first.
//!
//! The [`PathBuf`] the pair carries is for diagnostics only. Nothing reopens
//! it, and nothing derives the cache key from it.

use std::fs::File;
use std::path::PathBuf;

use crate::error::LauncherError;

/// The link the kernel resolves to the running executable's inode.
pub const PROC_SELF_EXE: &str = "/proc/self/exe";

/// Opens the running executable for reading.
///
/// Returns the open file and the path it was resolved from — the target of
/// `/proc/self/exe` where that worked, and whatever
/// [`std::env::current_exe`] answered otherwise.
///
/// # Errors
///
/// [`LauncherError::SelfExe`] carrying the *fallback's* failure when neither
/// route works. A machine with no `/proc` is the ordinary case for the first
/// failure, so reporting it would send every reader down the wrong path.
pub fn open_self() -> Result<(File, PathBuf), LauncherError> {
    if let Ok(file) = File::open(PROC_SELF_EXE) {
        // The link's target is a diagnostic, not a handle: the descriptor above
        // is already on the right inode, so a target that cannot be read — or
        // that the kernel has annotated ` (deleted)` — costs nothing.
        let path =
            std::fs::read_link(PROC_SELF_EXE).unwrap_or_else(|_| PathBuf::from(PROC_SELF_EXE));
        return Ok((file, path));
    }

    let path = std::env::current_exe().map_err(LauncherError::SelfExe)?;
    let file = File::open(&path).map_err(LauncherError::SelfExe)?;
    Ok((file, path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn open_self_opens_the_test_binary() {
        let Ok((mut file, path)) = open_self() else {
            panic!("open_self must open the running test binary");
        };
        let mut magic = [0u8; 4];
        assert!(
            file.read_exact(&mut magic).is_ok(),
            "the running executable must be readable"
        );
        assert_eq!(
            magic,
            [0x7f, b'E', b'L', b'F'],
            "the running executable must begin with the ELF magic"
        );
        assert!(
            path.is_absolute(),
            "the resolved path {} must be absolute",
            path.display()
        );
    }

    #[test]
    fn the_resolved_path_is_the_running_binary() {
        let Ok((_, path)) = open_self() else {
            panic!("open_self must open the running test binary");
        };
        let Ok(current) = std::env::current_exe() else {
            panic!("current_exe must answer in a test process");
        };
        assert_eq!(
            path.file_name(),
            current.file_name(),
            "{} and {} name different files",
            path.display(),
            current.display()
        );
    }

    #[test]
    fn the_descriptor_is_the_whole_file() {
        // The launcher seeks to the payload offset and reads to the end, so a
        // descriptor that is not positioned at zero, or that is opened on a
        // truncated view, would read the wrong bytes.
        let Ok((file, _)) = open_self() else {
            panic!("open_self must open the running test binary");
        };
        let Ok(from_open) = file.metadata().map(|meta| meta.len()) else {
            panic!("the opened file must have metadata");
        };
        let Ok(current) = std::env::current_exe() else {
            panic!("current_exe must answer in a test process");
        };
        let Ok(from_path) = std::fs::metadata(&current).map(|meta| meta.len()) else {
            panic!("the running binary must have metadata");
        };
        assert_eq!(from_open, from_path);
    }
}
