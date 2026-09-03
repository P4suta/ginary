// SPDX-License-Identifier: MIT OR Apache-2.0
//! What ginary has to know about the platform under it, stated as pure
//! functions of an [`Os`] rather than as `#[cfg]` arms.
//!
//! [`crate::winpath`] set the precedent: a rule that is *syntax* or *policy*
//! rather than a system call is written once, compiled everywhere, and unit
//! tested on the machine ginary is developed on. Everything here is a fact
//! about an operating system that a Linux developer cannot observe and can
//! still be held to — each one arrived as a live-runner failure and each one
//! is now a value a test can name.
//!
//! The seven facts, and where each came from:
//!
//! | rule | the runner that found it |
//! |---|---|
//! | [`has_unix_modes`], [`modeless_mode`] | `ginary verify` reported five mode mismatches against a healthy Windows-built artifact |
//! | [`flush_needs_write_access`] | every cold-cache extraction on Windows failed with `Access is denied` |
//! | [`rename_refuses_open_children`] | every prune and uninstall on Windows reported `unremovable` |
//! | [`erl_program`] | the beam strip step skipped every module, looking for `bin\erl` |
//! | [`probe_program`], [`probe_suffix`] | `ginary doctor` reported every healthy Windows cache as one no program can be run from |
//! | [`has_local_app_data`] | every Windows build failed with `CacheDir(Unresolved)` before it started |
//!
//! `docs/dev/log/E8.md` records the excerpt behind each row.

use crate::target::Os;

/// The operating system this build of ginary is running on.
///
/// The one impure value in the module, and it is a constant rather than a
/// function so that a caller cannot pass it where a rule wants a named
/// platform: every function below takes the [`Os`] it is asserting about.
pub const HOST: Os = crate::target::Target::host().os;

/// Whether a file on `os` carries POSIX permission bits.
///
/// `false` on Windows, where access is an ACL and the only bit a file has is
/// read-only. Nothing a POSIX mode word says survives a round trip through
/// such a filesystem, which is why [`modeless_mode`] exists and why the
/// `mode` column of `ginary.index.json` is informational on an artifact
/// staged there.
pub const fn has_unix_modes(os: Os) -> bool {
    match os {
        Os::Linux | Os::Macos => true,
        Os::Windows => false,
    }
}

/// The `mode` column recorded for a staged file on a platform that has none.
///
/// `0o755` for a directory and `0o644` for everything else: exactly what the
/// `tar` crate itself writes into a header on such a platform, so that the
/// index column and the payload header agree and `ginary verify` compares two
/// readings of one fact rather than two different facts.
///
/// One rule, one answer. [`crate::assemble`]'s staging listing,
/// [`crate::manifest::Index`] and the payload header all record the mode of
/// the same file, and a platform where one of them invents a value the other
/// two cannot see is a platform where verification reports a defect nobody
/// introduced.
pub const fn modeless_mode(is_dir: bool) -> u32 {
    if is_dir { 0o755 } else { 0o644 }
}

/// The `mode` column recorded for a file: `raw_mode` where the platform has a
/// mode word ([`has_unix_modes`] is `true`), and [`modeless_mode`] where it
/// does not.
///
/// The decision is split out from the `#[cfg(unix)]` read of `st_mode` so that
/// both arms compile and are asserted on one machine: the metadata read is
/// platform-specific, but *which* value the listing records is a pure function
/// of the platform and the file, and it is the one place a divergence between
/// the listing, [`crate::manifest::Index`] and the payload header could
/// reappear. `raw_mode` is ignored where the platform has no mode word, so a
/// caller may pass any value — `0` — for it there.
pub const fn recorded_mode(has_unix_modes: bool, raw_mode: u32, is_dir: bool) -> u32 {
    if has_unix_modes {
        raw_mode
    } else {
        modeless_mode(is_dir)
    }
}

/// Whether flushing a file to disk on `os` needs a handle opened for writing.
///
/// `true` on Windows: the durability barrier there is `FlushFileBuffers`,
/// which the kernel refuses on a handle that was not opened with write
/// access, with `ERROR_ACCESS_DENIED`. `fsync(2)` asks nothing of the
/// descriptor's access mode, so a unix flush may open read-only — and must,
/// because a staged tree holds files a build has already made read-only.
pub const fn flush_needs_write_access(os: Os) -> bool {
    match os {
        Os::Windows => true,
        Os::Linux | Os::Macos => false,
    }
}

/// Whether `os` refuses to rename a directory that still holds an open handle
/// somewhere inside it.
///
/// `true` on Windows, and `FILE_SHARE_DELETE` on the inner handle does not
/// buy it back: sharing deletion permits *that file* to be deleted or
/// renamed, not an ancestor directory of it. So a removal that takes
/// [`crate::cache_lock::try_exclusive`] on `<entry>/.lock` and then renames
/// `<entry>` aside while still holding the lock cannot succeed there. The
/// lock proves nobody is using the entry; the rename is the claim; the two
/// have to happen in that order rather than at once.
pub const fn rename_refuses_open_children(os: Os) -> bool {
    match os {
        Os::Windows => true,
        Os::Linux | Os::Macos => false,
    }
}

/// Whether `os` spells the path separator with a backslash.
///
/// `true` on Windows. It is the question `assemble::listed_relative`
/// asks before respelling a walked path into the `/`-separated spelling
/// `ginary.stage.json` and `ginary.index.json` carry: a `\` is an ordinary
/// character in a unix file name, so the respelling is applied only where the
/// platform put the backslash there as a separator in the first place. Asked
/// of a named `os` rather than of `std::path::MAIN_SEPARATOR` so the call site
/// is a pure function a Linux machine can assert both answers of.
pub const fn separates_paths_with_backslash(os: Os) -> bool {
    match os {
        Os::Windows => true,
        Os::Linux | Os::Macos => false,
    }
}

/// The name of the runtime's own launcher under `<otp root>/bin` on `os`.
///
/// `erl.exe` on Windows and `erl` everywhere else. It is the program
/// [`crate::strip`] runs to reach `beam_lib:strip_files/1`, and it is looked
/// for on the *host*'s installation rather than in the artifact, so it is a
/// question about this machine and not about [`crate::target::Target`] —
/// which is why it is here and not beside
/// [`crate::target::Target::launch_program`].
pub const fn erl_program(os: Os) -> &'static str {
    match os {
        Os::Windows => "erl.exe",
        Os::Linux | Os::Macos => "erl",
    }
}

/// The bytes of the smallest program `os` will start.
///
/// [`crate::doctor::probe_cache_dir`] answers "can a program be run out of
/// this cache directory?" the only honest way there is: it writes a program
/// there and starts it. Which bytes are a program is a property of the
/// platform, not of the directory. A `#!` line is one on unix, where the
/// kernel reads it; on Windows nothing reads it, `CreateProcessW` refuses the
/// file with `ERROR_BAD_EXE_FORMAT`, and a healthy cache is reported as one a
/// program cannot be run from.
///
/// `@exit /b 0` in a batch file is Windows' counterpart: the smallest thing
/// there that a `std::process::Command` over the written path starts and that
/// exits zero. It has to be paired with [`probe_suffix`], because on Windows
/// it is the suffix that makes the bytes a program at all.
///
/// The line ending is the platform's own, so the file is what an editor on
/// that platform would have written.
pub const fn probe_program(os: Os) -> &'static [u8] {
    match os {
        Os::Windows => b"@exit /b 0\r\n",
        Os::Linux | Os::Macos => b"#!/bin/sh\nexit 0\n",
    }
}

/// The file name suffix that makes [`probe_program`]'s bytes a program on
/// `os`.
///
/// Empty on unix, where the execute bit decides and the name is free; `.cmd`
/// on Windows, where the extension is the whole of the decision — an
/// extensionless file there is data whatever its contents and whatever its
/// ACL says.
pub const fn probe_suffix(os: Os) -> &'static str {
    match os {
        Os::Windows => ".cmd",
        Os::Linux | Os::Macos => "",
    }
}

/// Whether `os` places a user's per-user cache under `%LOCALAPPDATA%`.
///
/// `true` on Windows, where `HOME` and `XDG_CACHE_HOME` are unix conventions
/// no shell exports and the per-user application data directory is the base
/// every tool uses. It is the question [`crate::cache_dir::resolve`] asks
/// before choosing between [`crate::cache::resolve`] and
/// [`crate::cache::resolve_windows`] — the launcher already dispatched on it,
/// and the build side answered `Unresolved` for every ordinary Windows
/// environment until it did too.
pub const fn has_local_app_data(os: Os) -> bool {
    match os {
        Os::Windows => true,
        Os::Linux | Os::Macos => false,
    }
}
