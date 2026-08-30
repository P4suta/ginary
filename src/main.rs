// SPDX-License-Identifier: MIT OR Apache-2.0
//! The `ginary` executable.
//!
//! This entry point stays deliberately thin. Once the launcher exists it will
//! branch here — on a trailer at the end of its own file — before any command
//! line parsing happens, so a packaged application never pays for clap and
//! never mistakes its own arguments for ginary's.

use std::process::ExitCode;

/// Runs the command line and turns an error chain into an exit status.
fn main() -> ExitCode {
    match ginary::cli::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            report(&error);
            ExitCode::FAILURE
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
