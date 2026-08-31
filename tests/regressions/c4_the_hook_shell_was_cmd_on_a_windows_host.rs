// SPDX-License-Identifier: MIT OR Apache-2.0
//! A build hook's tokens were quoted for `/bin/sh` and, on a Windows host,
//! handed to `cmd`.
//!
//! `run_hook` substitutes `{target}` and `{out_dir}` through
//! `process::shell_quote`, whose own documentation says what it is for: "This
//! renders for `/bin/sh`. It is not an escaper for `cmd.exe`, and nothing here
//! builds a command line for one." That sentence had stopped being true.
//! `HOOK_SHELL` carried a `cfg(windows)` alternative — `cmd` with `/C` — and
//! `cmd` does not strip single quotes, so on that host an ordinary
//! `C:\Users\me\build\native\…` reached the hook wrapped in quote characters
//! that were part of the path as far as the compiler it ran was concerned.
//! Quoting had made the Windows case worse than leaving it alone.
//!
//! The right behaviour is the one the substitution was written for: a build
//! hook is run through a POSIX shell on every host. A hook compiles native
//! code for a Linux target and needs a POSIX toolchain anyway, and a machine
//! with no `/bin/sh` gets `NativeError::HookProcess` naming it rather than a
//! command line a different shell would read differently.
//!
//! The failure this pins is a Windows one, so it cannot be produced here. The
//! claim is the one that matters on every host: the shell that reads the line
//! is the shell the line was quoted for.
#![cfg(feature = "cli")]

use ginary::native;

#[test]
fn a_hook_command_line_is_read_by_the_shell_its_tokens_were_quoted_for() {
    assert_eq!(
        native::HOOK_SHELL,
        "/bin/sh",
        "`process::shell_quote` renders for a POSIX shell, and this is the one \
         that reads what it rendered"
    );
    assert_eq!(native::HOOK_SHELL_FLAG, "-c", "and its command-line flag");
}
