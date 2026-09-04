// SPDX-License-Identifier: MIT OR Apache-2.0
//! `tests/stub.rs` chose two fixed targets to stand for "not the host" and for
//! "the same file, another machine", and on a Windows host each stood for
//! something else.
//!
//! **What went wrong.** Two of the file's helpers encode assumptions about
//! which machine the suite is running on.
//!
//! `windows()` is used wherever a search must find nothing, and its comment
//! says why it was chosen: "it is not the host on any machine this suite runs
//! on, so the self-executable source never appears". Windows is now such a
//! machine, the running test binary answered the search, and the test failed
//! at the `expect_err`:
//!
//! ```text
//! ---- nothing_found_names_every_path_that_was_searched ----
//! the directories are empty:
//!   ("D:\\a\\ginary\\ginary\\target\\debug\\deps\\stub-7dd5c00ea0f19c37.exe", SelfExe)
//! ```
//!
//! `other_arch_target()` answers a *Linux* target with the other architecture,
//! and it is used to build the file whose object header must disagree with its
//! marker. On a Windows host the file planted is a PE and the `want` is Linux,
//! so the gate answers the question it was asked — a PE is not an ELF — and
//! never reaches the machine comparison the test is about:
//!
//! ```text
//! ---- a_marker_that_disagrees_with_the_file_is_refused_by_the_header ----
//! expected StubError::ObjectMismatch, got NotAnObject {
//!   path: "...\\liar", reason: "the file is 14432256 bytes and begins `MZ`" }
//! ```
//!
//! (`Windows build and exit-code propagation`
//! <https://github.com/P4suta/ginary/actions/runs/33751715516/job/100636537290>,
//! `tests/stub.rs:325` and `:440`.)
//!
//! **The input.** Any host the fixed target happens to name, and any host
//! whose object format is not the one the fixed target implies. C2's
//! `the_pe_gate_was_never_exercised` records that the PE branch of the header
//! gate had no test; this is why it still had none on the machine that has PE
//! files.
//!
//! **The correct behaviour.** Both are derived from the host rather than
//! written down. "Not the host" flips the architecture, which is the one field
//! the running binary can never disagree with the host on. "The same file,
//! another machine" keeps the host's operating system, so the header gate
//! compares the two machine fields, which is the gate the test is for.
#![cfg(feature = "cli")]

use crate::common::stubfile::{foreign_target_for, same_format_other_arch};
use ginary::platform::object_format;
use ginary::target::{ALL, Target};

#[test]
fn the_target_a_search_must_not_find_is_never_the_host_it_runs_on() {
    for host in ALL {
        assert_ne!(
            foreign_target_for(host),
            host,
            "a search asked about {host} on a {host} machine finds the running executable, \
             so the test that wants an empty search has nothing to assert"
        );
    }
}

#[test]
fn it_differs_by_architecture_so_the_running_binary_can_never_answer() {
    for host in ALL {
        let foreign = foreign_target_for(host);
        assert_ne!(
            foreign.arch, host.arch,
            "{host}: a target that differs only by libc or by operating system could still be \
             one some binary on this machine is for"
        );
    }
}

#[test]
fn the_target_the_header_gate_is_tested_with_shares_the_hosts_container() {
    for host in ALL {
        let other = same_format_other_arch(host);
        assert_eq!(
            other.os, host.os,
            "{host}: the file planted is this machine's own binary, so `want` has to name the \
             same operating system or the gate reads it as another platform's object"
        );
        assert_eq!(
            object_format(other.os),
            object_format(host.os),
            "{host}: the file planted is this machine's own binary, so `want` has to be a \
             target of the same container format or the gate refuses the file before it \
             compares machines"
        );
        assert_ne!(
            other.arch, host.arch,
            "{host}: and another machine, or there is no disagreement to refuse"
        );
    }
}

#[test]
fn the_pe_branch_of_the_header_gate_is_reachable_from_a_windows_host() {
    // The concrete case C2's `the_pe_gate_was_never_exercised` could not
    // reach: on a Windows host, the target the gate is tested with is a
    // Windows one, so `check_object` takes its PE arm.
    let windows: Target = "windows-x86_64".parse().expect("a target name");
    assert_eq!(same_format_other_arch(windows).os, windows.os);
}
