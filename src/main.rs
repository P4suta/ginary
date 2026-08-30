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
    match ginary::launcher::mode() {
        Ok(Some((exe, exe_path, trailer))) => {
            // Launcher path only: the command line half is a developer tool
            // and its panics are worth seeing in full.
            ginary::error::install_panic_hook();
            ginary::launcher::run(exe, exe_path, trailer)
        }
        Ok(None) => match ginary::cli::run() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                report(&error);
                ExitCode::FAILURE
            }
        },
        Err(error) => {
            eprintln!("{}", error.report());
            ExitCode::from(error.exit_code())
        }
    }
}

/// Prints an error and its causes, one cause per line.
fn report(error: &anyhow::Error) {
    eprintln!("error: {error}");
    for cause in error.chain().skip(1) {
        eprintln!("  caused by: {cause}");
    }
}
