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
//! The thirteen facts, and where each came from:
//!
//! | rule | the runner that found it |
//! |---|---|
//! | [`has_unix_modes`], [`modeless_mode`] | `ginary verify` reported five mode mismatches against a healthy Windows-built artifact |
//! | [`flush_needs_write_access`] | every cold-cache extraction on Windows failed with `Access is denied` |
//! | [`rename_refuses_open_children`] | every prune and uninstall on Windows reported `unremovable` |
//! | [`erl_program`] | the beam strip step skipped every module, looking for `bin\erl` |
//! | [`probe_program`], [`probe_suffix`] | `ginary doctor` reported every healthy Windows cache as one no program can be run from |
//! | [`has_local_app_data`] | every Windows build failed with `CacheDir(Unresolved)` before it started |
//! | [`null_device`] | the beam step's argument vector named `/dev/null` on a host that has no such file |
//! | [`temp_dir_var`] | `cache dir --json` reported a `TEMP fallback` a test only knew as `TMPDIR` |
//! | [`object_format`] | thirteen tests took the running executable for an ELF |
//! | [`object_format_of`] | `ginary verify` read a Windows artifact and reported `objects: 0` |
//! | [`crypto_nif`] | `doctor` looked for `crypto.so` in an installation that spells it `crypto.dll` |
//! | [`is_legal_file_name`] | two fixtures built directories named `out*` and `x:` |
//!
//! `docs/dev/log/E8.md` records the excerpt behind each row, and
//! `docs/dev/log/E11.md` the six E11 added.

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

/// The container format an operating system's own executables and shared
/// libraries are written in.
///
/// Moved here from [`crate::native`], which re-exports it: the format is a
/// fact about a platform rather than about one shipment's `priv` directory,
/// and half the suite needs to ask [`object_format`] the question without
/// the `cli` feature's scanner in scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectFormat {
    /// An ELF object, the Linux shape.
    Elf,
    /// A PE object, the Windows shape.
    Pe,
    /// A Mach-O object, the macOS shape.
    MachO,
}

impl ObjectFormat {
    /// The word this format prints as in a table and in a manifest.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Elf => "elf",
            Self::Pe => "pe",
            Self::MachO => "macho",
        }
    }
}

/// The object format `os` writes its own executables in.
///
/// The fact thirteen Windows failures turned on. A test that copies
/// `std::env::current_exe()` somewhere and then reads it back with
/// [`crate::elf`] is asserting that this host links ELF, and five of them
/// said so out loud in their own comments — "the only real, unstripped,
/// dynamically linked ELF a test can count on". It is not one on Windows,
/// where the same line produces `NotElf`, and it is not one on macOS either.
///
/// Stated as a function of a named `os` so a Linux machine can assert all
/// three answers, exactly as [`erl_program`] and [`probe_program`] are.
pub const fn object_format(os: Os) -> ObjectFormat {
    match os {
        Os::Linux => ObjectFormat::Elf,
        Os::Macos => ObjectFormat::MachO,
        Os::Windows => ObjectFormat::Pe,
    }
}

/// The path of the file `os` throws writes away into.
///
/// `/dev/null` on unix and `nul` on Windows, where it is a reserved *device
/// name* rather than a file: it resolves in every directory and it has no
/// `\\?\` spelling at all, which is why the value is a name and not a path.
///
/// [`crate::strip`] passes it as `ERL_CRASH_DUMP` to the runtime it starts
/// for `beam_lib:strip_files/1`, so that a runtime which dies mid-strip does
/// not drop an `erl_crash.dump` into the directory the developer happened to
/// be standing in. [`crate::process`] holds the same constant for the same
/// reason and now derives it from here, so the two cannot drift.
pub const fn null_device(os: Os) -> &'static str {
    match os {
        Os::Windows => "nul",
        Os::Linux | Os::Macos => "/dev/null",
    }
}

/// The environment variable `os` names the per-user temporary directory with.
///
/// `TMPDIR` on unix and `TEMP` on Windows. It is the variable
/// [`crate::cache::fallback_root`] and [`crate::cache::windows_fallback_root`]
/// read, and it is the word `ginary cache dir --json` puts in its `origin`
/// field — `"TMPDIR fallback"` or `"TEMP fallback"` — so a test that wants to
/// name the origin composes it from here rather than pinning one host's.
///
/// Windows also honours `TMP` when `TEMP` is unset, which is why this is the
/// *first* variable rather than the only one; see
/// [`crate::cache::windows_fallback_root`] for the whole ladder.
pub const fn temp_dir_var(os: Os) -> &'static str {
    match os {
        Os::Windows => "TEMP",
        Os::Linux | Os::Macos => "TMPDIR",
    }
}

/// Where the `crypto` NIF sits inside `lib/crypto-<vsn>` on `os`.
///
/// A NIF is a shared library and every platform spells one differently:
/// `priv/lib/crypto.so` on Linux and on macOS — OTP builds NIFs with the
/// `.so` suffix there rather than `.dylib` — and `priv/lib/crypto.dll` on
/// Windows. [`crate::doctor::crypto_report`] answers "does this installation
/// carry crypto, and what does it need" and answered [`None`] for every
/// healthy Windows installation because it looked for the unix name.
///
/// The path is `/`-separated because it is joined onto a root with
/// [`std::path::Path::join`], which reads either separator on every platform.
pub const fn crypto_nif(os: Os) -> &'static str {
    match os {
        Os::Windows => "priv/lib/crypto.dll",
        Os::Linux | Os::Macos => "priv/lib/crypto.so",
    }
}

/// Whether `name` is a file name `os` will let a directory or a file be
/// created under.
///
/// Almost anything is a unix file name: the two characters that are not are
/// the separator and NUL. Windows reserves nine printable characters —
/// `< > : " / \ | ? *` — and a name may not end in a dot or a space, because
/// the normalisation step strips both and a file created as `a.` is a file
/// called `a`.
///
/// Two fixtures built directories that are not names there. `out*` is the
/// staged root
/// `tests/regressions/a2_the_staged_root_became_a_wildcard.rs` uses to prove
/// that a `filelib:wildcard` prefix does not reach a sibling, and `x:` is the
/// awkward path
/// `tests/regressions/c3_otp_update_truncated_the_catalog_it_replaced.rs`
/// uses to prove that `//` in a path is not a URL scheme. Both failed at
/// `create_dir_all` with `ERROR_INVALID_NAME` before the assertion they carry
/// was reached.
///
/// Reserved device names — `nul`, `con`, `aux`, `com1` and the rest — are
/// *not* rejected here. They are a separate rule about a name that is legal
/// and resolves somewhere surprising, and no fixture builds one; adding them
/// would be a rule with no failure behind it.
pub fn is_legal_file_name(os: Os, name: &str) -> bool {
    if name.is_empty() || name.contains(['/', '\0']) {
        return false;
    }
    match os {
        Os::Linux | Os::Macos => true,
        Os::Windows => {
            !name.contains(WINDOWS_RESERVED_CHARACTERS)
                && !name.ends_with('.')
                && !name.ends_with(' ')
        }
    }
}

/// The container format the first bytes of a file name, when they name one.
///
/// The magic and nothing else: `\x7fELF`, `MZ`, and the four Mach-O magics
/// (32- and 64-bit, either byte order). A file's *name* decides nothing — a
/// `priv/lib/x.so` that is really a shell wrapper is not an object, and a NIF
/// may be called anything at all.
///
/// The rule three call sites each spell for themselves, and each of them
/// spells only the ELF half:
///
/// - [`crate::verify`]'s entry reader treats a payload entry as an object
///   only when it begins `\x7fELF`, so a Windows artifact — every object of
///   which is a PE — is reported as having none;
/// - [`crate::strip`]'s ELF phase collects only such files, so a tree with
///   fifteen megabytes of PE emulator in it answers
///   [`crate::strip::ElfOutcome::NothingToStrip`];
/// - [`crate::report::measure`] reaches the same decision for the `needs:`
///   line, which then reads `needs: (none)`.
///
/// The Windows runner shows all three at once:
///
/// ```text
/// ---- a_real_artifact_verifies_clean ----
/// not one of the artifact's objects was found in the installation at
/// d:/a/_temp/.setup-beam/otp, so the expectation below is empty because
/// nothing was read rather than because nothing is wrong. The objects are []
///
/// ---- the_needs_line_lists_the_libraries_the_runtime_loads ----
/// `libc.so.6` is what beam.smp loads, and an artifact that does not say so
/// is a trap:
/// needs: (none)
/// ```
///
/// Answering `None` for a PE is what makes those two silent. A caller that
/// knows the file is an object it cannot read can say so; a caller that was
/// told the file is not an object has nothing to report.
pub fn object_format_of(head: &[u8]) -> Option<ObjectFormat> {
    if head.starts_with(ELF_MAGIC) {
        return Some(ObjectFormat::Elf);
    }
    if head.starts_with(PE_MAGIC) {
        return Some(ObjectFormat::Pe);
    }
    if MACHO_MAGICS.iter().any(|magic| head.starts_with(magic)) {
        return Some(ObjectFormat::MachO);
    }
    None
}

/// The four bytes every ELF object begins with.
const ELF_MAGIC: &[u8] = &[0x7f, b'E', b'L', b'F'];

/// The two bytes every PE object begins with, the DOS header's `e_magic`.
///
/// Two bytes is the whole of the magic: what follows is a DOS stub whose
/// length is not fixed, and the `PE\0\0` signature it points at is a *later*
/// reader's business. A file that begins `MZ` and carries no signature is a
/// broken PE, which a caller reports, and not a file of some other kind.
const PE_MAGIC: &[u8] = b"MZ";

/// The four magics a Mach-O object begins with: 32- and 64-bit, either byte
/// order.
///
/// A `fat`/universal archive begins `0xcafebabe` instead and is deliberately
/// not here: ginary neither writes nor reads one, and a caller handed one
/// would be told it holds an object it can read when it does not.
const MACHO_MAGICS: [&[u8]; 4] = [
    &[0xfe, 0xed, 0xfa, 0xce],
    &[0xce, 0xfa, 0xed, 0xfe],
    &[0xfe, 0xed, 0xfa, 0xcf],
    &[0xcf, 0xfa, 0xed, 0xfe],
];

/// The nine printable characters Windows reserves in a file name.
///
/// `/` is checked separately, because it is not a file-name character on any
/// platform: it is the separator, and a `name` holding one is two components
/// rather than an illegal one.
const WINDOWS_RESERVED_CHARACTERS: [char; 8] = ['<', '>', ':', '"', '\\', '|', '?', '*'];
