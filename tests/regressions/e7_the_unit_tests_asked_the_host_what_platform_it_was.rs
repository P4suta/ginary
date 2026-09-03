// SPDX-License-Identifier: MIT OR Apache-2.0
//! Five unit tests compared a value against the host they happened to run on,
//! so on the first Windows runner they asserted things nobody had written.
//!
//! **What went wrong.** The Windows job compiled the suite for the first time
//! — E6 gated the 43 ungated `std::os::unix` reaches that had stopped it — and
//! ran it. Seven library tests failed; five of them are this defect, in two
//! shapes.
//!
//! `runtime_bins` is given a [`ginary::target::Target`] and appends that
//! target's executable suffix, which is correct. Three of its tests passed
//! `Target::host()` and compared the answer against the unix spelling:
//!
//! ```text
//! ---- bundle::tests::distribution_adds_epmd_and_heart_adds_heart stdout ----
//! thread '...' panicked at src\bundle.rs:1847:9:
//! assertion `left == right` failed: a distributed artifact has to carry the
//! daemon it is allowed to start
//!   left: ["epmd.exe"]
//!  right: ["epmd"]
//! ```
//!
//! And three `error` tests built an `io::Error` from a raw OS number and
//! compared the whole rendered message against glibc's `strerror` text:
//!
//! ```text
//! ---- error::tests::an_exec_failure_is_125_and_names_the_program stdout ----
//! thread '...' panicked at src\error.rs:314:9:
//!   left: "ginary: cannot start /c/hello/k/erts-17.0.5/bin/erlexec: The system
//!          cannot find the file specified. (os error 2)"
//!  right: "ginary: cannot start /c/hello/k/erts-17.0.5/bin/erlexec: No such
//!          file or directory (os error 2)"
//! ```
//!
//! (`Windows build and exit-code propagation`
//! <https://github.com/P4suta/ginary/actions/runs/33702776627/job/100485421577>.)
//!
//! **The input.** Any host that is not Linux. There is no other way to see
//! either shape, which is why both are scanned rather than compiled: on Linux
//! the host *is* the platform the expectation was written for, so the
//! assertion is true and says nothing.
//!
//! **The correct behaviour.** A unit test names the target it is asserting
//! about. `runtime_bins(&opts, Target::new(Os::Linux, ..))` says which
//! spelling is expected and why; `runtime_bins(&opts, Target::host())` says
//! "whatever this machine is", and a test that cannot be wrong on one machine
//! cannot be right on another. The suite already holds
//! `a_windows_build_asks_for_the_programs_by_the_names_the_tree_spells`, which
//! pins the `.exe` half explicitly, so nothing is lost by naming the target in
//! the other three.
//!
//! And the operating system's own words are not ginary's to spell. What
//! [`ginary::error::LauncherError`] owns is the prefix, the path and the
//! numbered code; the text after the colon comes from `io::Error`'s own
//! `Display`, which every C library renders in its own words and some render
//! in the user's language. An expectation built from that same `Display` is
//! exact on every host; one that quotes glibc is exact on one.
//!
//! Both scanners are [`crate::common::srcscan`], calibrated below on source
//! they are handed before they are turned loose on the tree.

use crate::common::repo::read;
use crate::common::srcscan::{calls_with, literal_sites};

/// The tail every operating system's `io::Error` message carries and no
/// message ginary writes does.
const OS_ERROR_TAIL: &str = "(os error ";

#[test]
fn no_program_list_test_asks_the_host_which_names_the_runtime_spells() {
    // The instrument: a call whose target argument is the host, a call that
    // names a target, a multi-line call, and a name that merely ends in the
    // callee's.
    let planted = r#"
        assert_eq!(runtime_bins(&opts, Target::host()), Vec::<String>::new());
        assert_eq!(runtime_bins(&opts, Target::new(Os::Linux, Arch::X86_64, Libc::Gnu)), want);
        assert_eq!(
            runtime_bins(
                &options(root, "distribution = true\n"),
                Target::host()
            ),
            want,
        );
        assert!(check_runtime_bins(&opts, Target::host()).is_ok());
"#;
    assert_eq!(
        calls_with(planted, "runtime_bins", "Target::host()"),
        vec![2, 5],
        "the scanner balances the argument list across lines, and `check_runtime_bins` is not \
         `runtime_bins`"
    );

    assert_eq!(
        calls_with(&read("src/bundle.rs"), "runtime_bins", "Target::host()"),
        Vec::<usize>::new(),
        "a test that asks the host which target it is asserts a different thing on every \
         machine, and the machine where it is wrong is the one nobody runs it on. Name the \
         target"
    );
}

#[test]
fn no_launcher_message_test_spells_the_operating_systems_own_words() {
    // The instrument: the tail is found in code and ignored in the comment
    // that has to be able to describe the rule.
    let planted = "\
// the message ends `(os error 2)` on glibc and says something else elsewhere\n\
    assert_eq!(error.to_string(), \"ginary: cannot start x: No such file (os error 2)\");\n\
    assert_eq!(error.to_string(), format!(\"ginary: cannot start x: {source}\"));\n";
    assert_eq!(
        literal_sites(planted, OS_ERROR_TAIL),
        vec![2],
        "a comment may describe the rule; a line of code may not quote one C library's words"
    );

    assert_eq!(
        literal_sites(&read("src/error.rs"), OS_ERROR_TAIL),
        Vec::<usize>::new(),
        "the text after ginary's own colon belongs to `io::Error`, and an expectation built \
         from that same value is exact on every host. Quoting glibc is exact on one"
    );
}
