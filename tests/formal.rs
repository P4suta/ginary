// SPDX-License-Identifier: MIT OR Apache-2.0
//! The TLA+ model of the cache protocol, held against the repository.
//!
//! `formal/Cache.tla` is not code the test suite can run — checking it needs a
//! JVM and `tla2tools.jar`, which is what `mise run formal` is for. What this
//! file pins is everything about the model that can rot silently: that it is
//! committed at all, that its configuration names the four invariants the
//! protocol is supposed to have, that the task which checks it exists and
//! pins the tool it downloads by digest, and that the document mapping the
//! model onto `src/cache.rs` is there.
//!
//! A model nobody runs is worse than no model, because it reads as evidence.
//! These are the assertions that make "the protocol is model-checked" a claim
//! the repository can be held to.

use std::path::{Path, PathBuf};

/// The repository root.
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Reads a repository file as text.
///
/// # Panics
///
/// If the file is not there, which is what these tests are about.
fn read(relative: &str) -> String {
    let path = root().join(relative);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()))
}

#[test]
fn the_model_and_its_configuration_are_committed() {
    for relative in ["formal/Cache.tla", "formal/Cache.cfg"] {
        assert!(
            root().join(relative).is_file(),
            "{relative} is what `mise run formal` checks"
        );
    }
}

#[test]
fn the_model_names_every_action_the_protocol_has() {
    let model = read("formal/Cache.tla");

    for action in [
        "BeginExtract",
        "CrashMidExtract",
        "FinishExtract",
        "Sweep",
        "Hit",
        "TakeSharedLock",
        "ReleaseOnExit",
        "PruneCheck",
        "PruneRemove",
    ] {
        assert!(
            model.contains(action),
            "the model does not describe `{action}`, which src/cache.rs does"
        );
    }
}

#[test]
fn the_model_names_every_state_an_entry_can_be_in() {
    let model = read("formal/Cache.tla");

    for state in ["Absent", "TmpPartial", "Complete", "Trashed"] {
        assert!(model.contains(state), "the model has no `{state}` state");
    }
}

#[test]
fn the_configuration_names_the_constants_and_the_four_invariants() {
    let config = read("formal/Cache.cfg");

    assert!(config.contains("CONSTANTS"), "{config}");
    for constant in ["Procs", "Keys"] {
        assert!(
            config.contains(constant),
            "the model has to be finite, and `{constant}` is what bounds it:\n{config}"
        );
    }
    for invariant in ["I1", "I2", "I3", "I4"] {
        assert!(
            config.contains(invariant),
            "an invariant the configuration does not name is an invariant TLC does not \
             check: `{invariant}` is missing from\n{config}"
        );
    }
}

#[test]
fn the_task_that_checks_the_model_pins_the_tool_it_downloads() {
    let mise = read("mise.toml");

    assert!(
        mise.contains("[tasks.formal]"),
        "there is no `mise run formal`"
    );
    let task = mise
        .split("[tasks.formal]")
        .nth(1)
        .expect("the task body follows its heading");
    assert!(
        task.contains("tla2tools.jar"),
        "the task does not name the checker:\n{task}"
    );
    assert!(
        task.contains("sha256"),
        "a jar fetched over the network without a digest is a jar nobody checked:\n{task}"
    );
    // Behaviour rather than the string: `-deadlock` *disables* TLC's deadlock
    // check, this model has no terminal state, and passing the flag would give
    // up an invariant rather than add one. An assertion that only asked for
    // the text would be satisfied by the comment that explains the decision,
    // and would then pass just as well if somebody put the flag back.
    let (comments, commands): (Vec<&str>, Vec<&str>) = task
        .lines()
        .partition(|line| line.trim_start().starts_with('#'));
    assert!(
        commands.iter().all(|line| !line.contains("-deadlock")),
        "`-deadlock` turns TLC's deadlock check *off*, and this model has no terminal state:\n{}",
        commands.join("\n")
    );
    assert!(
        comments.iter().any(|line| line.contains("-deadlock")),
        "the task has to say where it stands on deadlock checking:\n{task}"
    );
}

#[test]
fn the_model_is_mapped_onto_the_code_it_is_about() {
    let document = read("docs/dev/formal.md");

    for subject in [
        "src/cache.rs",
        "ensure_extracted",
        "prune",
        "cache_lock",
        "mtime",
        "fsync",
    ] {
        assert!(
            document.contains(subject),
            "docs/dev/formal.md does not say what `{subject}` maps to, or what the model \
             leaves out"
        );
    }
}

/// `formal/` holds one model and one configuration and no second of either.
///
/// Only the *model* files are counted. `mise run formal` passes `-metadir` so
/// that TLC's state directory lands under `.cache/`, but a run from the
/// Toolbox or from the command line in the plan writes `formal/states/` beside
/// the spec, and a developer who checks the model by hand must not have to
/// debug a red suite afterwards.
#[test]
fn the_formal_directory_holds_one_model_and_one_configuration() {
    let dir: &Path = &root().join("formal");
    let Ok(entries) = std::fs::read_dir(dir) else {
        panic!("formal/ is not there");
    };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".tla") || name.ends_with(".cfg"))
        .collect();
    names.sort();
    assert_eq!(
        names,
        ["Cache.cfg", "Cache.tla"],
        "a second model is a model somebody has to be told about"
    );
}
