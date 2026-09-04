// SPDX-License-Identifier: MIT OR Apache-2.0
//! The `Coverage` job measured the 90% floor against a tree with no cross
//! stubs, where nine end-to-end tests skip and the number is 89.92%.
//!
//! **What went wrong.** E6's first fix stopped `GINARY_REQUIRE_TOOLCHAIN=1`
//! from turning a missing cross stub into a panic, which is right: a runner
//! with a complete Erlang toolchain has not thereby run `cross`. But the
//! `coverage` job then went on measuring line coverage over a tree where every
//! stub-gated test *skips*, and the floor it enforces was measured on a
//! machine where they do not:
//!
//! ```text
//! with target/stubs present:   90.26% (14140/15665) lines -> gate rc=0
//! with target/stubs parked:    89.92% (14086/15665) lines -> rc=1
//! coverage-gate: line coverage 89.92% is below the 90% floor
//! ```
//!
//! Fifty-four lines, which is exactly the code those tests reach. So the fix
//! moved the failure from `cargo llvm-cov` to `scripts/ci/coverage-gate.sh`
//! rather than removing it, and nobody saw that because the gate had never
//! executed on a runner at all — both prior runs died before it, in the
//! llvm-cov step. (`Coverage`
//! <https://github.com/P4suta/ginary/actions/runs/33681144884/job/100417746014>.)
//!
//! **The input.** Any job that enforces a coverage floor without obtaining
//! the artifacts the gated tests need. The floor is not the thing that is
//! wrong — lowering it would be weakening a gate to go green — so the job has
//! to acquire what the measurement assumes.
//!
//! **The correct behaviour.** `coverage` waits on `cross-build`, downloads the
//! stubs that job already uploads, points `GINARY_STUB_DIR` at them and sets
//! `GINARY_REQUIRE_STUBS=1`, so a download that produced nothing fails the job
//! instead of quietly removing the coverage it was measuring. And the rule
//! generalises: a job may declare `GINARY_REQUIRE_STUBS` exactly when it
//! obtains the stubs, which is what makes the variable a promise rather than
//! a wish.
//!
//! **The second artifact.** The stubs alone carry those tests exactly as far
//! as the runtime resolver. Seven of the nine — `tests/e2e_cross.rs` times
//! three, `tests/e2e_native.rs` times four — write `erts = "catalog"` into the
//! fixture and build against `dist/otp/catalog.json`, and only the catalog is
//! committed: `.gitignore` keeps every tarball it names out of the tree. So on
//! a fresh checkout `catalog()` finds the file, returns `Some` and does not
//! skip, and the build it guards dies one layer further in:
//!
//! ```text
//! error: cannot resolve the runtime to bundle
//!   caused by: cannot use the catalog
//!   caused by: cannot use dist/otp/otp-29.0.5-linux-x86_64-musl-static.tar.zst:
//!              No such file or directory (os error 2)
//! ```
//!
//! Downloading the stubs therefore *moved* the coverage failure from
//! `stubfile::cross_stub` to `ginary build` rather than removing it. The two
//! artifacts travel together: a job that obtains the cross stubs has to obtain
//! the runtimes those same tests build from, which `smoke-matrix` already does
//! with an `otp repack` step and `coverage` did not.

use std::collections::BTreeSet;

use crate::common::repo::{WorkflowJob, workflow_jobs, workflow_steps};
use crate::common::stubfile::REQUIRE_STUBS_VAR;

/// The workflow every job in this file belongs to.
const CI: &str = ".github/workflows/ci.yml";

/// The directory variable a job points at the stubs it obtained.
const STUB_DIR_VAR: &str = "GINARY_STUB_DIR";

/// Where the repack pipeline writes the runtimes the catalog names.
const CATALOG_DIR: &str = "dist/otp";

/// Whether a job obtains the cross-built stubs, by either of the two means
/// the workflow has: building them with `cross`, or downloading what the
/// `cross-build` job uploaded.
fn obtains_stubs(job: &WorkflowJob) -> bool {
    let builds = job.runs("cross build") && job.runs("target/stubs");
    let downloads =
        job.uses_action("download-artifact") && job.needs.iter().any(|n| n == "cross-build");
    builds || downloads
}

/// Whether a job obtains the OTP runtimes `dist/otp/catalog.json` names.
///
/// One means, because the workflow has one: repacking them from upstream into
/// `dist/otp`, which is what `smoke-matrix` and `nightly.yml` do. A job that
/// grew a second means — downloading a `dist/otp` artifact, say — widens this
/// helper rather than deleting the rule.
fn obtains_runtimes(job: &WorkflowJob) -> bool {
    job.runs("otp repack") && job.runs(CATALOG_DIR)
}

/// Every variable name any step of `job` sets, job-level and step-level.
fn variables_set_by(job: &str) -> BTreeSet<String> {
    workflow_steps(CI)
        .into_iter()
        .filter(|step| step.job == job)
        .flat_map(|step| step.env.into_keys())
        .collect()
}

#[test]
fn the_job_that_enforces_the_coverage_floor_obtains_the_stubs_it_measures_with() {
    let jobs = workflow_jobs(CI);
    let coverage = jobs
        .iter()
        .find(|job| job.id == "coverage")
        .expect("ci.yml declares a coverage job");
    assert!(
        coverage.runs("coverage-gate.sh"),
        "this test is about the job that enforces the floor; `coverage` no longer runs the gate \
         script, so the subject has moved"
    );
    assert!(
        obtains_stubs(coverage),
        "the `coverage` job enforces a 90% line floor that was measured on a machine holding \
         target/stubs, and obtains no stubs itself: nine stub-gated tests skip there and the \
         same measurement is 89.92%. It waits on {:?} and uses {:?}",
        coverage.needs,
        coverage.uses
    );
    assert!(
        variables_set_by("coverage").contains(STUB_DIR_VAR),
        "the `coverage` job downloads the stubs and never tells the suite where they are; \
         {STUB_DIR_VAR} is how a test finds a stub outside the repository's own target/stubs"
    );
    assert!(
        obtains_runtimes(coverage),
        "the stubs alone carry the nine gated tests as far as the runtime resolver: seven of \
         them build with `erts = \"catalog\"` and {CATALOG_DIR} holds only the committed \
         catalog, every tarball it names being gitignored. Without an `otp repack` step the \
         floor is measured over seven builds that die with `cannot use the catalog`. \
         `coverage` runs {:?}",
        coverage.commands
    );
}

#[test]
fn a_job_that_obtains_the_stubs_also_obtains_the_runtimes_those_tests_build_from() {
    let mut offenders = Vec::new();
    for job in workflow_jobs(CI) {
        if obtains_stubs(&job) && !obtains_runtimes(&job) {
            offenders.push(format!(
                "`{}` obtains the cross stubs and never fills {CATALOG_DIR}: the seven \
                 stub-gated tests that build with `erts = \"catalog\"` reach the runtime \
                 resolver and fail there, because the catalog is committed and the tarballs it \
                 names are not",
                job.id
            ));
        }
    }
    assert!(
        offenders.is_empty(),
        "the stubs and the runtimes are one artifact set, not two:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn a_job_promises_the_stubs_exactly_when_it_obtains_them() {
    let mut offenders = Vec::new();
    for job in workflow_jobs(CI) {
        let promises = job.env.get(REQUIRE_STUBS_VAR).is_some_and(|v| v == "1");
        let obtains = obtains_stubs(&job);
        if promises && !obtains {
            offenders.push(format!(
                "`{}` sets {REQUIRE_STUBS_VAR}=1 and builds or downloads no stub: every \
                 stub-gated test in it fails rather than skips",
                job.id
            ));
        }
        if obtains && !promises {
            offenders.push(format!(
                "`{}` obtains the cross stubs and does not set {REQUIRE_STUBS_VAR}=1: a cross \
                 build or a download that produced nothing would silently remove the tests it \
                 was run for",
                job.id
            ));
        }
    }
    assert!(
        offenders.is_empty(),
        "{REQUIRE_STUBS_VAR} is a promise a job keeps, not a wish:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn the_jobs_that_build_no_stub_never_require_one() {
    for id in ["test", "smoke", "msrv", "lint"] {
        let jobs = workflow_jobs(CI);
        let Some(job) = jobs.iter().find(|job| job.id == id) else {
            continue;
        };
        assert!(
            !job.env.contains_key(REQUIRE_STUBS_VAR),
            "`{id}` runs `cross` nowhere and downloads nothing, so {REQUIRE_STUBS_VAR} there \
             would be the first pull-request run's failure again under a new name"
        );
    }
}
