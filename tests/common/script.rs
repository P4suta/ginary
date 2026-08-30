// SPDX-License-Identifier: MIT OR Apache-2.0
//! Writing throwaway executables that an integration test puts on `PATH`.
//!
//! A test that drives the real binary against a *stub* toolchain needs a
//! program on disk: `ginary doctor` looks `erl` up on `PATH` and runs it, so a
//! test about what `doctor` reports when discovery fails has to supply an `erl`
//! that fails in the chosen way. `src/process.rs` has the same helper for the
//! unit tests; this is its counterpart on the integration side, and it carries
//! the same `ETXTBSY` retry loop for the same reason — see
//! [`wait_until_executable`].

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

/// The argument that makes a script written by [`script`] exit before its body
/// runs, so exec-ability can be probed without any side effect.
const EXEC_PROBE: &str = "--ginary-exec-probe";

/// Creates an executable `/bin/sh` script and returns its path.
///
/// The script is not returned until it has actually been exec'd once.
///
/// # Panics
///
/// If the script cannot be written, marked executable, or exec'd.
pub fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(
        &path,
        format!("#!/bin/sh\ncase \"$1\" in {EXEC_PROBE}) exit 0;; esac\n{body}\n"),
    )
    .unwrap_or_else(|error| panic!("cannot write {}: {error}", path.display()));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .unwrap_or_else(|error| panic!("cannot chmod {}: {error}", path.display()));
    }
    wait_until_executable(&path);
    path
}

/// Blocks until the freshly written script can be exec'd.
///
/// Cargo runs the tests of one binary as threads of a single process. While one
/// thread holds a write descriptor on a new file, a sibling thread's
/// `Command::spawn` forks and inherits a duplicate of it, and any exec of the
/// inode inside that window fails with `ETXTBSY`. The window is microseconds
/// long and cannot reopen once no descriptor is left, so one bounded retry loop
/// closes it for good.
///
/// # Panics
///
/// If the script is still not executable after the retry budget.
fn wait_until_executable(path: &Path) {
    for _ in 0..500 {
        match Command::new(path)
            .arg(EXEC_PROBE)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(mut child) => {
                let _ = child.wait();
                return;
            }
            Err(error) if error.kind() == std::io::ErrorKind::ExecutableFileBusy => {
                std::thread::sleep(Duration::from_millis(2));
            }
            Err(error) => panic!("cannot exec {}: {error}", path.display()),
        }
    }
    panic!("{} is still not executable", path.display());
}
