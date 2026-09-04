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
//!
//! **What E12 corrected.** The second half of that answer only works where
//! the host's operating system has two published architectures, and
//! `windows-aarch64` is not a row of `ginary::target::ALL`, so on a Windows
//! host the marker parser refused the name before the gate was reached. The
//! header gate is no longer asked for a second *target*: it is asked for a
//! second *machine*, built by `stubfile::for_other_machine`, which rewrites
//! the machine field of the host's own object and leaves the container format
//! alone. The two tests below that stated the superseded rule state the
//! replacement instead, and the gate itself is run in
//! `tests/regressions/e12_the_cross_target_a_stub_test_used_had_no_name.rs`.
#![cfg(feature = "cli")]

use crate::common::stubfile::{
    Marker, PE_MACHINE_AMD64, for_other_machine, foreign_target_for, ginary_bin, other_arch,
    other_supported_target, pe_bytes,
};
use ginary::native::inspect_object_bytes;
use ginary::platform::{ObjectFormat, object_format, object_format_of};
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
fn the_file_the_header_gate_is_tested_with_shares_the_hosts_container() {
    // The rule E12 replaced this one's subject with: the fixture is the
    // host's own object with its machine field rewritten, so the container
    // format is the host's by construction and the gate reaches the machine
    // comparison on every platform rather than only where the host's
    // operating system has two published architectures.
    let host = Target::host();
    let image = std::fs::read(ginary_bin()).expect("the ginary binary is readable");
    let other = for_other_machine(&image);

    assert_eq!(
        object_format_of(&other),
        Some(object_format(host.os)),
        "the fixture stays an object of the host's own container format"
    );
    assert_eq!(
        inspect_object_bytes(&other)
            .expect("the rewritten copy is still an object")
            .machine,
        other_arch(host.arch).as_str(),
        "and its header names the machine the marker beside it does not"
    );
}

#[test]
fn the_pe_branch_of_the_header_gate_is_reachable_from_a_windows_host() {
    // The concrete case C2's `the_pe_gate_was_never_exercised` could not
    // reach, and it is now reachable from *this* machine as well: a PE whose
    // COFF machine field has been rewritten is still a PE, so on a Windows
    // host `check_object` takes its PE arm and compares machines.
    let windows: Target = "windows-x86_64".parse().expect("a target name");
    let pe = pe_bytes(PE_MACHINE_AMD64, &Marker::for_target(&windows).bytes());

    let other = for_other_machine(&pe);

    assert_eq!(object_format_of(&other), Some(ObjectFormat::Pe));
    assert_eq!(
        inspect_object_bytes(&other)
            .expect("the rewritten PE is still a PE")
            .machine,
        other_arch(windows.arch).as_str(),
        "the machine field is what changed, so the gate has a disagreement to refuse"
    );
    assert_eq!(
        other_supported_target(windows).name().parse::<Target>(),
        Ok(other_supported_target(windows)),
        "and the target gate beside it is asked about a name this ginary can read back"
    );
}
