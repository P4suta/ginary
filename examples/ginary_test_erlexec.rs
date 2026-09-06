// SPDX-License-Identifier: MIT OR Apache-2.0
//! The launcher fixture's runtime, as a program Windows will start.
//!
//! `tests/common/artifact.rs` builds a packaged application by hand and puts
//! a stub where the runtime belongs: it prints the environment variables the
//! launch contract names and its own arguments, and exits with whatever it
//! was told. On unix that stub is a `#!/bin/sh` script. On Windows nothing
//! reads a shebang — `CreateProcess` looks for `MZ` in the image header,
//! finds `#!`, and refuses the file — so every launcher test that started a
//! synthetic artifact there failed before the launcher had done anything
//! wrong:
//!
//! ```text
//! ginary: cannot start \\?\C:\Windows\Temp\ginary-unknown\hello\<key>\erts-17.0.5\bin\erlexec:
//! %1 は有効な Win32 アプリケーションではありません。 (os error 193)
//! ```
//!
//! `examples/ginary_test_shim.rs` solved the same problem for the build-side
//! fixtures in E10, and this is its counterpart for the launcher: the same
//! reasoning, a different contract. That one performs a closed list of steps
//! fixed before it starts; this one answers the *arguments it is given*, which
//! is the whole point of a stub standing in for `erlexec`.
//!
//! An example rather than a `[[bin]]`, for the reason
//! `examples/ginary_test_shim.rs` gives: `cargo test` and `cargo build` build
//! it, and it never becomes a command ginary installs.
//!
//! # The contract file
//!
//! Every literal this program compares against comes out of
//! `<program>.contract`, which `tests/common/artifact.rs` writes from the same
//! constants it renders the shell script from. Nothing here is spelled twice,
//! so the two renderings cannot drift:
//!
//! ```text
//! var <NAME>            report this variable, in this order (repeatable)
//! exit-arg <text>       the argument whose value is the exit code
//! signal-arg <text>     the argument whose value is a signal to raise
//! sleep-arg <text>      the argument whose value is a delay in seconds
//! dump-arg <text>       the argument that asks for a crash dump
//! halt-eval <text>      the `-eval` expression that means "exit zero"
//! default-exit <n>      the code when nothing said otherwise
//! dump-line <text>      one line of the crash dump (repeatable)
//! ```
//!
//! # What it prints
//!
//! `env:<NAME>=<value>` for each declared variable, `<unset>` when it has
//! none; `cwd:<directory>`; and `argv:<argument>` for each argument it was
//! given. On unix an argument and a value are written as bytes, so one that
//! is not valid UTF-8 arrives on standard output as itself.

use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::path::{Path, PathBuf};

/// What the fixture's runtime was told to do.
#[derive(Debug, Default)]
struct Contract {
    /// The variables to report, in the order they are reported.
    vars: Vec<String>,
    /// The argument whose value is the exit code.
    exit_arg: String,
    /// The argument whose value is a signal to raise.
    signal_arg: String,
    /// The argument whose value is a delay in seconds.
    sleep_arg: String,
    /// The argument that asks for a crash dump.
    dump_arg: String,
    /// The `-eval` expression that means "exit zero".
    halt_eval: String,
    /// The code to exit with when nothing said otherwise.
    default_exit: i32,
    /// The crash dump's lines, in order.
    dump_lines: Vec<String>,
}

fn main() {
    let program = match std::env::current_exe() {
        Ok(path) => path,
        Err(error) => fail(&format!("cannot find my own path: {error}")),
    };
    let contract = read_contract(&program);
    let arguments: Vec<OsString> = std::env::args_os().skip(1).collect();

    let mut out = std::io::stdout();
    for name in &contract.vars {
        match std::env::var_os(name) {
            Some(value) => write_prefixed(&mut out, &format!("env:{name}="), &value),
            None => {
                let _ = writeln!(out, "env:{name}=<unset>");
            }
        }
    }
    match std::env::current_dir() {
        Ok(cwd) => write_prefixed(&mut out, "cwd:", cwd.as_os_str()),
        Err(error) => fail(&format!("cannot read the working directory: {error}")),
    }

    let asked = read_arguments(&contract, &arguments);
    for argument in &arguments {
        write_prefixed(&mut out, "argv:", argument);
    }
    let _ = out.flush();

    if asked.dump {
        write_dump(&contract);
    }
    if let Some(seconds) = asked.sleep {
        std::thread::sleep(std::time::Duration::from_secs_f64(seconds));
    }
    if let Some(signal) = asked.signal {
        raise(signal);
    }
    std::process::exit(asked.code);
}

/// What the arguments asked for.
#[derive(Debug)]
struct Asked {
    /// The code to exit with.
    code: i32,
    /// The signal to raise instead of exiting, where a platform has them.
    signal: Option<i32>,
    /// How long to stay alive first.
    sleep: Option<f64>,
    /// Whether to write the crash dump.
    dump: bool,
}

/// Reads the arguments the way the shell rendering's `for` loop does: each
/// value is the argument *after* the one that names it.
fn read_arguments(contract: &Contract, arguments: &[OsString]) -> Asked {
    let mut asked = Asked {
        code: contract.default_exit,
        signal: None,
        sleep: None,
        dump: false,
    };
    let mut previous = OsString::new();
    for argument in arguments {
        let value = argument.to_string_lossy().into_owned();
        match previous.to_string_lossy().as_ref() {
            name if name == contract.exit_arg => asked.code = number(&contract.exit_arg, &value),
            name if name == contract.signal_arg => {
                asked.signal = Some(number(&contract.signal_arg, &value));
            }
            name if name == contract.sleep_arg => {
                asked.sleep = Some(match value.parse::<f64>() {
                    Ok(seconds) => seconds,
                    Err(error) => fail(&format!(
                        "`{} {value}` is not a number of seconds: {error}",
                        contract.sleep_arg
                    )),
                });
            }
            "-eval" if value == contract.halt_eval => asked.code = 0,
            _ => {}
        }
        if value == contract.dump_arg {
            asked.dump = true;
        }
        previous.clone_from(argument);
    }
    asked
}

/// `value` as a whole number, or a fault in the fixture that says which
/// argument was misspelled.
fn number(argument: &str, value: &str) -> i32 {
    match value.parse() {
        Ok(number) => number,
        Err(error) => fail(&format!("`{argument} {value}` is not a number: {error}")),
    }
}

/// Writes the crash dump into `$ERL_CRASH_DUMP`, when the launcher named one.
fn write_dump(contract: &Contract) {
    let Some(path) = std::env::var_os("ERL_CRASH_DUMP") else {
        return;
    };
    let mut text = String::new();
    for line in &contract.dump_lines {
        text.push_str(line);
        text.push('\n');
    }
    if let Err(error) = std::fs::write(&path, text) {
        fail(&format!(
            "cannot write {}: {error}",
            Path::new(&path).display()
        ));
    }
}

/// Refuses to raise a signal, loudly.
///
/// The one term of the contract this rendering does not perform. Raising a
/// numbered signal at oneself needs `libc` or `kill(1)`; ginary depends on
/// neither, and the runs that scrub the environment leave no `PATH` for the
/// second. Every test that asks for a signal is `#[cfg(unix)]` and unix takes
/// the shell rendering, where `kill` is a builtin — so the term is reachable
/// only if that ever changes, and a fixture that quietly exited zero instead
/// would make such a test pass for the wrong reason. Exit 70 is a fault in the
/// fixture, which is what this would be.
fn raise(signal: i32) -> ! {
    fail(&format!(
        "this rendering raises no signals, so signal {signal} was not sent"
    ))
}

/// Reads and parses `<program>.contract`.
fn read_contract(program: &Path) -> Contract {
    let path = sidecar(program, "contract");
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) => fail(&format!("cannot read {}: {error}", path.display())),
    };
    let mut contract = Contract::default();
    for line in text.lines() {
        let (verb, rest) = line.split_once(' ').unwrap_or((line, ""));
        match verb {
            "" => {}
            "var" => contract.vars.push(rest.to_owned()),
            "exit-arg" => contract.exit_arg = rest.to_owned(),
            "signal-arg" => contract.signal_arg = rest.to_owned(),
            "sleep-arg" => contract.sleep_arg = rest.to_owned(),
            "dump-arg" => contract.dump_arg = rest.to_owned(),
            "halt-eval" => contract.halt_eval = rest.to_owned(),
            "default-exit" => {
                contract.default_exit = match rest.parse() {
                    Ok(code) => code,
                    Err(error) => fail(&format!("`default-exit {rest}` is not a code: {error}")),
                };
            }
            "dump-line" => contract.dump_lines.push(rest.to_owned()),
            other => fail(&format!("{}: unknown term `{other}`", path.display())),
        }
    }
    contract
}

/// `program`'s own file name with `.<suffix>` appended, which is what
/// `"$0.<suffix>"` names in the shell rendering of the same contract.
fn sidecar(program: &Path, suffix: &str) -> PathBuf {
    let name = program.file_name().map_or_else(
        || String::from("program"),
        |name| name.to_string_lossy().into_owned(),
    );
    program.with_file_name(format!("{name}.{suffix}"))
}

/// Writes `prefix`, then `value`, then a newline.
///
/// On unix the value goes out as its own bytes, because an argument that is
/// not valid UTF-8 is one the launcher must have passed through unchanged and
/// a lossy rendering would hide the very byte under test.
fn write_prefixed(out: &mut impl Write, prefix: &str, value: &OsStr) {
    let _ = out.write_all(prefix.as_bytes());
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt as _;
        let _ = out.write_all(value.as_bytes());
    }
    #[cfg(not(unix))]
    {
        let _ = out.write_all(value.to_string_lossy().as_bytes());
    }
    let _ = out.write_all(b"\n");
}

/// Reports a fault in the fixture itself and stops with a status no contract
/// chooses, so a test cannot mistake it for the behaviour it asked for.
fn fail(message: &str) -> ! {
    eprintln!("ginary_test_erlexec: {message}");
    std::process::exit(70);
}
