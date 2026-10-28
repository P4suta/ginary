// SPDX-License-Identifier: MIT OR Apache-2.0
//! The clean-room smoke build host and runtime share an Ubuntu compatibility floor.

use std::collections::BTreeSet;

use saphyr::YamlOwned;

use crate::common::repo::{read, shell_code, yaml};

const CI: &str = ".github/workflows/ci.yml";

#[test]
fn smoke_keeps_the_host_and_container_runtime_floor_together() {
    let workflow = yaml(CI);
    let smoke = workflow
        .as_mapping_get("jobs")
        .and_then(|jobs| jobs.as_mapping_get("smoke"))
        .expect("the release assurance smoke job");
    assert_eq!(
        smoke.as_mapping_get("runs-on").and_then(YamlOwned::as_str),
        Some("ubuntu-24.04"),
        "the smoke artifact is built on its host, so keep the declared Ubuntu 24.04 pair"
    );
    assert!(
        smoke.as_mapping_get("container").is_none(),
        "a job container would replace the declared build host environment"
    );
    let steps = smoke
        .as_mapping_get("steps")
        .and_then(YamlOwned::as_vec)
        .expect("smoke steps");
    let build = steps
        .iter()
        .position(|step| {
            step.as_mapping_get("run").and_then(YamlOwned::as_str)
                == Some("cargo build --release --locked")
        })
        .expect("the host builds the release binary");
    let run = steps
        .iter()
        .position(|step| {
            step.as_mapping_get("run").and_then(YamlOwned::as_str) == Some("scripts/smoke.sh")
        })
        .expect("the actual clean-room script runs");
    assert!(
        build < run,
        "build the host binary before packaging the fixture"
    );
    assert_eq!(
        steps[run]
            .as_mapping_get("env")
            .and_then(|env| env.as_mapping_get("GINARY_BIN"))
            .and_then(YamlOwned::as_str),
        Some("${{ github.workspace }}/target/release/ginary")
    );
    for env in [
        workflow.as_mapping_get("env"),
        smoke.as_mapping_get("env"),
        steps[run].as_mapping_get("env"),
    ] {
        assert!(
            env.and_then(|value| value.as_mapping_get("GINARY_IMAGE"))
                .is_none(),
            "the workflow must keep the smoke script's Ubuntu 24.04 runtime default"
        );
    }
    let script = read("scripts/smoke.sh");
    assert!(
        script
            .lines()
            .map(shell_code)
            .any(|line| line.trim() == "image=${GINARY_IMAGE:-ubuntu:24.04}"),
        "the clean-room runtime floor must stay Ubuntu 24.04"
    );
}

#[test]
fn only_smoke_uses_the_runner_version_covered_by_the_floor_exception() {
    let workflow = yaml(CI);
    let jobs = workflow
        .as_mapping_get("jobs")
        .and_then(YamlOwned::as_mapping)
        .expect("CI jobs");
    let floor_jobs: BTreeSet<&str> = jobs
        .iter()
        .filter(|(_, job)| {
            job.as_mapping_get("runs-on").and_then(YamlOwned::as_str) == Some("ubuntu-24.04")
        })
        .map(|(name, _)| name.as_str().expect("a job identifier"))
        .collect();
    assert_eq!(
        floor_jobs,
        BTreeSet::from(["smoke"]),
        "the file-scoped Renovate floor exception must affect only smoke"
    );
}
