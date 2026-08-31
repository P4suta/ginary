// SPDX-License-Identifier: MIT OR Apache-2.0
//! The `ginary` executable.
//!
//! This entry point stays deliberately thin, and it branches before anything
//! else happens. A copy of this binary with a payload and a trailer appended
//! to it is a packaged application and goes straight to
//! [`ginary::launcher::run`]; a copy without one is the command line tool.
//!
//! The order matters twice. clap is never constructed on the launcher path, so
//! a packaged application does not pay for it and cannot mistake its own
//! arguments for ginary's. And a *damaged* artifact — one whose last 64 bytes
//! begin the magic and then do not describe the file — reports the damage and
//! exits 122 rather than falling through to a help text that would tell its
//! user nothing.

use std::process::ExitCode;

/// Chooses the mode and runs it.
fn main() -> ExitCode {
    // The identity marker is data nothing on either path reads, so a linker
    // that garbage-collects unreferenced sections would drop it and every stub
    // this ginary builds would be anonymous. Taking its address through
    // `black_box` is a reference the optimiser may not remove; it is one
    // instruction on a path that then does file I/O, and it is the cheapest
    // place to put it that runs in both modes. See `ginary::stubid`.
    std::hint::black_box(&ginary::stubid::GINARY_STUB_ID);

    match ginary::launcher::mode() {
        Ok(Some((exe, exe_path, trailer))) => {
            // Launcher path only: the command line half is a developer tool
            // and its panics are worth seeing in full.
            ginary::error::install_panic_hook();
            ginary::launcher::run(exe, exe_path, trailer)
        }
        Ok(None) => no_trailer(),
        Err(error) => {
            eprintln!("{}", error.report());
            ExitCode::from(error.exit_code())
        }
    }
}

/// What a copy of this binary with no payload appended to it does.
///
/// The two flavors answer differently, and the difference is the whole point
/// of the `cli` feature. A full build is the command line tool and runs it. A
/// stub build is the launcher and nothing else: it has no commands to offer,
/// so it says what it is and which target it is for, and leaves the same exit
/// code a usage error leaves.
#[cfg(feature = "cli")]
fn no_trailer() -> ExitCode {
    match ginary::cli::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            report(&error);
            ExitCode::FAILURE
        }
    }
}

/// As above, for a build with no command line half.
#[cfg(not(feature = "cli"))]
fn no_trailer() -> ExitCode {
    eprintln!(
        "{}",
        ginary::launcher::no_payload_line(ginary::target::Target::host())
    );
    ExitCode::from(ginary::launcher::CMD_USAGE_EXIT)
}

/// Prints an error and its causes, one cause per line.
#[cfg(feature = "cli")]
fn report(error: &anyhow::Error) {
    eprintln!("error: {error}");
    for cause in error.chain().skip(1) {
        eprintln!("  caused by: {cause}");
    }
}
