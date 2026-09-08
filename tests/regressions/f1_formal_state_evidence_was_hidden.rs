// SPDX-License-Identifier: MIT OR Apache-2.0
//! TLC states lived below `.cache`, but upload-artifact excludes hidden directories
//! unless explicitly enabled. An upload containing only the visible log was incomplete.

#[test]
fn formal_upload_retains_the_scoped_hidden_model_states() {
    let workflow = crate::common::repo::yaml(".github/workflows/nightly.yml");
    let steps = workflow["jobs"]["formal"]["steps"]
        .as_sequence()
        .expect("formal job steps");
    let upload = steps
        .iter()
        .find(|step| {
            step.as_mapping_get("uses")
                .and_then(saphyr::YamlOwned::as_str)
                .is_some_and(|action| action.starts_with("actions/upload-artifact@"))
        })
        .expect("formal evidence upload");
    assert_eq!(
        upload["with"]["path"]
            .as_str()
            .expect("explicit evidence paths")
            .lines()
            .collect::<Vec<_>>(),
        ["target/assurance/formal", ".cache/tla/states"],
        "including hidden files must remain scoped to the model's own evidence"
    );
    assert_eq!(
        upload["with"]
            .as_mapping_get("include-hidden-files")
            .and_then(saphyr::YamlOwned::as_bool),
        Some(true),
        "TLC states below .cache must survive upload-artifact's default hidden filtering"
    );
}
