// SPDX-License-Identifier: MIT OR Apache-2.0
//! The fixture for "a process that is still running" was `/bin/sh -c
//! 'sleep 30'`, and there is no `/bin/sh` on every host.
//!
//! **What went wrong.** `cache::sweep` removes the leftover temporary trees of
//! processes that are gone and keeps the one belonging to a process that is
//! still alive. Proving the second half needs a live process, and the test
//! spawned a shell:
//!
//! ```text
//! ---- a_live_process_s_temporary_tree_is_kept ----
//! spawn a live process: Os { code: 3, kind: NotFound,
//!   message: "The system cannot find the path specified." }
//! ```
//!
//! (`Windows build and exit-code propagation`
//! <https://github.com/P4suta/ginary/actions/runs/33751715516/job/100636537290>,
//! `tests/cache.rs:402`.)
//!
//! Unlike the two tests in
//! `tests/regressions/e11_a_shell_script_test_ran_on_a_host_with_no_posix_shell.rs`,
//! nothing here is about a shell. The subject is a live process id, and a
//! shell was merely the shortest way to get one. So this is not a skip: it is
//! a fixture written in a form that exists on both platforms.
//!
//! **The input.** Any host with no `/bin/sh`.
//!
//! **The correct behaviour.** `common::script` already plants a program the
//! host can start — a shell script on unix, the compiled
//! `examples/ginary_test_shim.rs` on Windows — from a closed list of steps.
//! Staying alive becomes one more step, so the fixture is one description with
//! two renderings rather than a shell line only one platform can run.

use crate::common::script::{ShimStep, compiled_steps_text, shell_script_body};

#[test]
fn staying_alive_is_a_step_both_renderings_carry() {
    let steps = [ShimStep::Sleep(30_000)];

    let shell = shell_script_body(&steps);
    assert!(
        shell.contains("sleep 30\n"),
        "the shell rendering waits, in whole seconds, which is all `sleep` takes:\n{shell}"
    );

    let compiled = compiled_steps_text(&steps);
    assert_eq!(
        compiled, "sleep 30000\n",
        "and the sidecar carries the milliseconds the step was written with"
    );
}

#[test]
fn a_step_neither_rendering_drops_is_a_step_both_perform() {
    // The invariant `common::script` exists for: a step that renders in one
    // form and not the other is a fixture that behaves differently on two
    // platforms, which is the class of defect E10 removed and this step must
    // not reintroduce.
    let steps = [
        ShimStep::RecordArgv,
        ShimStep::Sleep(1_500),
        ShimStep::Exit(3),
    ];
    let shell = shell_script_body(&steps);
    let compiled = compiled_steps_text(&steps);

    assert_eq!(
        compiled.lines().count(),
        3,
        "one line per step in the sidecar: {compiled:?}"
    );
    assert!(
        shell.contains("sleep 2\n"),
        "a fraction of a second is rounded up rather than dropped, so the shell rendering \
         never waits less than the step asked for:\n{shell}"
    );
    assert!(
        compiled.contains("sleep 1500\n"),
        "and the compiled one waits exactly that long: {compiled:?}"
    );
}

#[test]
fn the_planted_program_really_stays_alive_and_can_be_reaped() {
    // The rendering is a rule; that a live process id comes out of it is the
    // thing `tests/cache.rs` actually needs.
    let dir = tempfile::tempdir().expect("a temporary directory");

    let mut child = crate::common::script::live_process(dir.path(), 30_000);
    // Long enough that a program which does not wait has certainly exited,
    // and far short of the thirty seconds one that does waits for: the
    // assertion has to fail on a fixture that ignores the step rather than
    // race it.
    std::thread::sleep(std::time::Duration::from_millis(750));
    let alive = child.try_wait().expect("the child is watchable");
    let _ = child.kill();
    let _ = child.wait();

    assert!(
        alive.is_none(),
        "a process the sweep must keep is one that has not exited, and this one is gone \
         with {alive:?}"
    );
}
