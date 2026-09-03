// SPDX-License-Identifier: MIT OR Apache-2.0
//! A real program a test can plant where a shell script will not do.
//!
//! `tests/common/script.rs` plants throwaway programs — a stub `erl` under a
//! fake OTP root, an `erl` on `PATH` that reports a broken code root — and on
//! unix a `#!/bin/sh` file is one. On Windows nothing reads a shebang:
//! `CreateProcess` looks for `MZ` in the image header, finds `#!`, and refuses
//! the file with `%1 is not a valid Win32 application. (os error 193)`. Every
//! `cli`, `strip` and `stage_run` target that plants an `erl` failed with it on
//! the Windows runner of
//! <https://github.com/P4suta/ginary/actions/runs/33739517757>, inside the
//! fixture builder, before a line of ginary ran.
//!
//! So on Windows the fixture plants *this*, compiled for the host, and hands it
//! its behaviour in a sidecar file beside it. The behaviour is the same small
//! closed set the shell rendering performs, and
//! `tests/common/script.rs::ShimStep` is the one description both read.
//!
//! # The sidecar
//!
//! `<program>.steps`, one step per line, in the order they run:
//!
//! ```text
//! record-argv                 truncate <program>.argv and write one argument per line
//! print <text>                write <text> and a newline to standard output
//! replace-beam-arguments      copy <program>.module over every argument ending in `.beam`
//! print-stderr-file           write <program>.stderr, then a newline, to standard error
//! exit <status>               stop, with this status
//! ```
//!
//! A run with no `exit` step exits `0`. The first argument
//! `--ginary-exec-probe` exits `0` before any step, so that the planter can
//! prove the file starts without causing its side effects.
//!
//! This is an example rather than a `[[bin]]` so that it is built by
//! `cargo test` and `cargo build` without becoming a command ginary installs.

use std::io::Write as _;
use std::path::{Path, PathBuf};

/// The argument that makes the program exit before its steps run.
const EXEC_PROBE: &str = "--ginary-exec-probe";

/// Runs the steps in `<program>.steps` over this run's arguments.
fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments.first().is_some_and(|first| first == EXEC_PROBE) {
        return;
    }
    let program = match std::env::current_exe() {
        Ok(path) => path,
        Err(error) => fail(&format!("cannot find my own path: {error}")),
    };
    let steps = sidecar(&program, "steps");
    let text = match std::fs::read_to_string(&steps) {
        Ok(text) => text,
        Err(error) => fail(&format!("cannot read {}: {error}", steps.display())),
    };

    for line in text.lines() {
        let (verb, rest) = line.split_once(' ').unwrap_or((line, ""));
        match verb {
            "" => {}
            "record-argv" => record_argv(&program, &arguments),
            "print" => println!("{rest}"),
            "replace-beam-arguments" => replace_beam_arguments(&program, &arguments),
            "print-stderr-file" => print_stderr_file(&program),
            "exit" => std::process::exit(rest.parse::<i32>().unwrap_or(1)),
            other => fail(&format!("{}: unknown step `{other}`", steps.display())),
        }
    }
}

/// `program`'s own file name with `.<suffix>` appended, which is what `"$0.<suffix>"`
/// names in the shell rendering of the same steps.
fn sidecar(program: &Path, suffix: &str) -> PathBuf {
    let name = program.file_name().map_or_else(
        || String::from("program"),
        |name| name.to_string_lossy().into_owned(),
    );
    program.with_file_name(format!("{name}.{suffix}"))
}

/// Truncates `<program>.argv` and writes one argument per line.
fn record_argv(program: &Path, arguments: &[String]) {
    let mut text = String::new();
    for argument in arguments {
        text.push_str(argument);
        text.push('\n');
    }
    let log = sidecar(program, "argv");
    if let Err(error) = std::fs::write(&log, text) {
        fail(&format!("cannot write {}: {error}", log.display()));
    }
}

/// Copies `<program>.module` over every argument that names a `.beam` file.
fn replace_beam_arguments(program: &Path, arguments: &[String]) {
    let module = sidecar(program, "module");
    for argument in arguments {
        if !argument.ends_with(".beam") {
            continue;
        }
        if let Err(error) = std::fs::copy(&module, argument) {
            fail(&format!(
                "cannot copy {} onto {argument}: {error}",
                module.display()
            ));
        }
    }
}

/// Writes `<program>.stderr`, then a newline, to standard error.
fn print_stderr_file(program: &Path) {
    let path = sidecar(program, "stderr");
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) => fail(&format!("cannot read {}: {error}", path.display())),
    };
    let mut stderr = std::io::stderr();
    if stderr
        .write_all(&bytes)
        .and_then(|()| stderr.write_all(b"\n"))
        .is_err()
    {
        std::process::exit(70);
    }
}

/// Reports a fault in the fixture itself and stops with a status no step
/// chooses, so a test cannot mistake it for the behaviour it asked for.
fn fail(message: &str) -> ! {
    eprintln!("ginary_test_shim: {message}");
    std::process::exit(70);
}
