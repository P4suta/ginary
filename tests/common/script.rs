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
//!
//! [`program`] is the one entry point, and it plants what the host can start.
//! The behaviour is a list of [`ShimStep`] rather than a line of shell, because
//! the same behaviour has to exist in two forms: a `/bin/sh` script on unix and
//! the compiled `examples/ginary_test_shim.rs` on Windows, where nothing reads
//! a shebang and `CreateProcess` refuses the file outright. [`shim_form`] and
//! [`shim_file_name`] are those two rules, stated as functions of the platform
//! and checked on Linux by
//! `tests/regressions/e10_a_fake_otp_wrote_an_erl_windows_cannot_start.rs`.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use ginary::target::Os;

/// How a throwaway program a test puts on disk is realised on `os`.
///
/// A `#!/bin/sh` file is a program on unix and a data file everywhere else.
/// Windows starts a program by its image: `CreateProcess` reads the PE header,
/// finds `#!` where `MZ` belongs, and refuses the file — which is what every
/// `stage`, `strip` and `stage_run` target on the Windows runner failed with:
///
/// ```text
/// cannot exec C:\Users\RUNNER~1\AppData\Local\Temp\.tmpnCZUIT\otp\bin\erl:
/// %1 is not a valid Win32 application. (os error 193)
/// ```
///
/// (`Windows build and exit-code propagation`
/// <https://github.com/P4suta/ginary/actions/runs/33739517757/job/100597889388>.)
///
/// Renaming the file does not help and neither does a `.cmd`: `CreateProcess`
/// starts neither a batch file nor a shebang. The form has to change with the
/// platform, and `src/strip.rs` is right to look for the name
/// [`ginary::platform::erl_program`] gives — so it is the fixture that has to
/// be a program.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShimForm {
    /// A `/bin/sh` script with a shebang, marked executable.
    ShellScript,
    /// A real executable image, compiled for the host. Nothing else is a
    /// program on Windows.
    CompiledProgram,
}

/// Which of the two forms a throwaway program takes on `os`.
pub const fn shim_form(os: Os) -> ShimForm {
    match os {
        Os::Linux | Os::Macos => ShimForm::ShellScript,
        Os::Windows => ShimForm::CompiledProgram,
    }
}

/// The file name a throwaway program called `name` is written under on `os`.
///
/// The same rule [`ginary::platform::erl_program`] states for `erl`, for any
/// program a test plants: on Windows a program is named `<name>.exe`, and a
/// file without that suffix is not one a `Command` will start.
pub fn shim_file_name(name: &str, os: Os) -> String {
    match os {
        Os::Windows => format!("{name}.exe"),
        Os::Linux | Os::Macos => name.to_owned(),
    }
}

/// One step of a planted program's behaviour.
///
/// The set is closed and small on purpose: it is the whole of what the three
/// fixtures that plant a program actually do, and it has to be expressible
/// twice — as `/bin/sh` on unix and as the compiled `examples/ginary_test_shim.rs`
/// on Windows. A fixture that needed a fourth thing would add a step here
/// rather than a line of shell only one platform can run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShimStep {
    /// Truncate `<program>.argv` and write one argument per line.
    RecordArgv,
    /// Write these lines, each followed by a newline, to standard output.
    Print(Vec<String>),
    /// Copy `<program>.module` over every argument naming a `.beam` file.
    ReplaceBeamArguments,
    /// Write `<program>.stderr`, then a newline, to standard error.
    PrintStderrFile,
    /// Stop, with this status. A program with no such step exits `0`.
    Exit(i32),
}

/// The argument that makes a planted program exit before its steps run, so
/// exec-ability can be probed without any side effect.
const EXEC_PROBE: &str = "--ginary-exec-probe";

/// The compiled program Windows is handed, from this test run's own target
/// directory.
///
/// `cargo test` builds every example alongside the test binaries, so it is
/// there whenever the tests are. Located from the `ginary` binary's own path
/// rather than from `CARGO_TARGET_DIR`, which a caller may have moved.
fn compiled_shim() -> PathBuf {
    let target = Path::new(env!("CARGO_BIN_EXE_ginary"))
        .parent()
        .expect("the ginary binary is in a directory")
        .to_path_buf();
    let path = target
        .join("examples")
        .join(format!("ginary_test_shim{}", std::env::consts::EXE_SUFFIX));
    assert!(
        path.is_file(),
        "the compiled shim {} is not built; `cargo test` builds every example, so run the suite \
         through cargo or `cargo build --example ginary_test_shim` first",
        path.display()
    );
    path
}

/// `program`'s own file name with `.<suffix>` appended.
///
/// The one naming rule for a planted program's sidecar files, so the shell
/// rendering's `"$0.argv"` and the compiled shim's `current_exe()` derivation
/// name the same file — including on Windows, where the program is `erl.exe`
/// and its log is therefore `erl.exe.argv`.
pub fn shim_sidecar(program: &Path, suffix: &str) -> PathBuf {
    let name = program.file_name().map_or_else(
        || String::from("program"),
        |name| name.to_string_lossy().into_owned(),
    );
    program.with_file_name(format!("{name}.{suffix}"))
}

/// Plants a program called `name` in `dir` that performs `steps`, and returns
/// its path.
///
/// The path is [`shim_file_name`]'s — `erl` on unix, `erl.exe` on Windows —
/// and the program is not returned until it has actually been started once.
///
/// # Panics
///
/// If the program cannot be written, marked executable, or started.
pub fn program(dir: &Path, name: &str, steps: &[ShimStep]) -> PathBuf {
    let path = dir.join(shim_file_name(name, ginary::platform::HOST));
    match shim_form(ginary::platform::HOST) {
        ShimForm::ShellScript => write_shell_script(&path, steps),
        ShimForm::CompiledProgram => write_compiled_program(&path, steps),
    }
    wait_until_executable(&path);
    path
}

/// Writes `steps` as a `/bin/sh` script and marks it executable.
fn write_shell_script(path: &Path, steps: &[ShimStep]) {
    let mut body = format!("#!/bin/sh\ncase \"$1\" in {EXEC_PROBE}) exit 0;; esac\n");
    for step in steps {
        match step {
            ShimStep::RecordArgv => body.push_str(
                ": > \"$0.argv\"\n\
                 for arg in \"$@\"; do printf '%s\\n' \"$arg\" >> \"$0.argv\"; done\n",
            ),
            ShimStep::Print(lines) => {
                for line in lines {
                    body.push_str(&format!("printf '%s\\n' {}\n", single_quoted(line)));
                }
            }
            ShimStep::ReplaceBeamArguments => body.push_str(
                "for arg in \"$@\"; do\n\
                 \x20 case \"$arg\" in *.beam) cp \"$0.module\" \"$arg\" ;; esac\n\
                 done\n",
            ),
            ShimStep::PrintStderrFile => {
                body.push_str("cat \"$0.stderr\" >&2\nprintf '\\n' >&2\n");
            }
            ShimStep::Exit(status) => body.push_str(&format!("exit {status}\n")),
        }
    }
    body.push_str("exit 0\n");
    std::fs::write(path, body)
        .unwrap_or_else(|error| panic!("cannot write {}: {error}", path.display()));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
            .unwrap_or_else(|error| panic!("cannot chmod {}: {error}", path.display()));
    }
}

/// Copies the compiled shim onto `path` and writes `steps` beside it.
fn write_compiled_program(path: &Path, steps: &[ShimStep]) {
    let mut text = String::new();
    for step in steps {
        match step {
            ShimStep::RecordArgv => text.push_str("record-argv\n"),
            ShimStep::Print(lines) => {
                for line in lines {
                    text.push_str(&format!("print {line}\n"));
                }
            }
            ShimStep::ReplaceBeamArguments => text.push_str("replace-beam-arguments\n"),
            ShimStep::PrintStderrFile => text.push_str("print-stderr-file\n"),
            ShimStep::Exit(status) => text.push_str(&format!("exit {status}\n")),
        }
    }
    let sidecar = shim_sidecar(path, "steps");
    std::fs::write(&sidecar, text)
        .unwrap_or_else(|error| panic!("cannot write {}: {error}", sidecar.display()));
    let shim = compiled_shim();
    std::fs::copy(&shim, path).unwrap_or_else(|error| {
        panic!(
            "cannot copy {} onto {}: {error}",
            shim.display(),
            path.display()
        )
    });
}

/// `text` as one `/bin/sh` word, safe to interpolate whatever it holds.
///
/// A single-quoted string ends at the first quote, so an embedded one is
/// written as the four characters that close, escape and reopen it.
fn single_quoted(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
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
