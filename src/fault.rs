// SPDX-License-Identifier: MIT OR Apache-2.0
//! Fault injection for the launcher tests.
//!
//! Three of the launcher's guarantees are about what happens when a run does
//! *not* finish: a process killed mid-extraction leaves a temporary tree that
//! the next run sweeps, a process that loses the rename race deletes its own
//! tree and uses the winner's, and a payload whose bytes changed under the
//! reader is refused rather than extracted. None of the three can be reached
//! by feeding the launcher a different artifact, because all three are about
//! *timing*.
//!
//! Two more are about the same thing on the launch side: an entry that is
//! removed while the launcher is on its way to the lock, and a build that
//! stops half-way. So the launcher carries named points a test can arm from
//! the environment.
//! `GINARY_FAULT=<point>[:<action>]` is read once, and the points are:
//!
//! | point | action | effect |
//! |---|---|---|
//! | `after-extract` | `pause` | sleep [`PAUSE`] with the temporary tree on disk, so a test can `SIGKILL` the process |
//! | `rename` | `eexist` | the rename onto the cache entry reports `EEXIST`, as a lost race does |
//! | `unpack` | `corrupt` | a byte of the manifest is flipped in memory, so the digest cannot match |
//! | `before-lock` | `on` | the cache entry is removed between the preflight and the shared lock, which is what a prune that won the race leaves behind |
//! | `launcher` | `panic` | the launcher panics, so the panic hook `main` installs is the thing under test |
//! | `pack` | `fail` | `bundle::build` stops between the stub and the payload, so a test can assert that a failed build leaves neither a work directory nor a half-written artifact |
//!
//! `pack` is the one point on the *build* side rather than the launcher's, and
//! it is here for the same reason as the others: "a build that fails
//! half-way cleans up after itself" cannot be reached by handing the builder a
//! different project, because every input that would fail a build fails it
//! before anything has been written.
//!
//! The whole module compiles to a no-op unless the `fault-injection` feature
//! is on, and that feature is off by default: a release artifact holds none of
//! the points, never reads the variable, and cannot be talked into any of this
//! by a hostile environment. `mise run test` and CI turn it on.

/// The variable that arms a point.
pub const VAR: &str = "GINARY_FAULT";

/// How long `pause` sleeps: long enough for a test to notice the process, kill
/// it and assert on what it left behind, and short enough that a test which
/// forgets to kill it still ends.
pub const PAUSE: std::time::Duration = std::time::Duration::from_secs(10);

/// Fires the fault point `name` and returns the action armed for it.
///
/// `pause` and `panic` are performed here, because the caller has nothing to
/// do with either: one is a sleep and the other never returns. Every other
/// action is returned for the caller to act on, so that the branch a fault
/// takes is visible at the point it is taken.
///
/// Returns [`None`] whenever the `fault-injection` feature is off, whatever
/// the environment says. That is also where the one `panic!` in this crate's
/// launcher path lives: a default build compiles none of this, so the artifact
/// a user runs cannot be talked into panicking by its environment.
///
/// # Panics
///
/// With `panic`, which is the point of that action: the launcher's panic hook
/// is a promise about what a user sees when ginary has a bug, and a promise
/// with nothing to trigger it is a promise no test can check.
pub fn point(name: &str) -> Option<&'static str> {
    #[cfg(not(feature = "fault-injection"))]
    {
        let _ = name;
        None
    }

    #[cfg(feature = "fault-injection")]
    {
        let action = armed(name)?;
        match action {
            "pause" => std::thread::sleep(PAUSE),
            "panic" => panic!("{PANIC_MESSAGE}"),
            _ => {}
        }
        Some(action)
    }
}

/// What the `panic` action panics with.
///
/// A constant so that the test which asserts the panic hook's one line can
/// assert the whole of it rather than a prefix.
pub const PANIC_MESSAGE: &str = "GINARY_FAULT=launcher:panic";

/// Every fault point this crate carries, in the order the module table lists
/// them.
///
/// The table above, `docs/dev/debugging.md` and `docs/dev/testing.md` all claim
/// to enumerate the points, and three prose lists drift the moment a point is
/// added. This constant is the one the tests hold the other lists against: a
/// new [`point`] call site whose name is not here fails
/// `every_call_site_is_a_listed_point`, and a point missing from either
/// document fails `both_documents_list_every_point`.
pub const FAULT_POINTS: [&str; 6] = [
    "after-extract",
    "rename",
    "unpack",
    "before-lock",
    "launcher",
    "pack",
];

/// The actions a point may be armed with.
///
/// A closed set, because [`point`] answers with a `&'static str` and because an
/// action this build does not implement must arm nothing rather than arm the
/// default: `GINARY_FAULT=rename:enospc` is a test asking for a fault that is
/// not here, and silently giving it `on` would make that test pass for the
/// wrong reason.
#[cfg_attr(
    not(any(test, feature = "fault-injection")),
    expect(dead_code, reason = "only the fault-injection build reads the table")
)]
const ACTIONS: [&str; 6] = ["on", "pause", "eexist", "corrupt", "panic", "fail"];

/// The action armed for `name`, reading [`VAR`] once per process.
#[cfg(feature = "fault-injection")]
fn armed(name: &str) -> Option<&'static str> {
    static SPEC: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    let spec = SPEC
        .get_or_init(|| std::env::var(VAR).ok())
        .as_deref()
        .unwrap_or_default();
    armed_by(spec, name)
}

/// The action `spec` arms for `name`, or [`None`] when it arms none for it.
///
/// The whole of [`armed`] except the one read of [`VAR`], which a process may
/// do only once and a test may therefore not vary. Splitting it here is what
/// lets the closed-set filter be asserted over many specs rather than over the
/// single spec the test process happens to have been started with.
#[cfg_attr(
    not(any(test, feature = "fault-injection")),
    expect(dead_code, reason = "only the fault-injection build resolves a spec")
)]
fn armed_by(spec: &str, name: &str) -> Option<&'static str> {
    let (point, action) = parse(spec)?;
    if point != name {
        return None;
    }
    ACTIONS.into_iter().find(|known| *known == action)
}

/// Splits `<point>[:<action>]`, defaulting the action to `on`.
///
/// An empty point name is no point at all rather than a point called `""`,
/// because `GINARY_FAULT=` is what a shell leaves behind and must not arm
/// anything.
#[cfg_attr(
    not(any(test, feature = "fault-injection")),
    expect(
        dead_code,
        reason = "only the fault-injection build and the unit tests read a spec"
    )
)]
fn parse(spec: &str) -> Option<(&str, &str)> {
    let (point, action) = match spec.split_once(':') {
        Some((point, action)) => (point, action),
        None => (spec, ""),
    };
    if point.is_empty() {
        return None;
    }
    Some((point, if action.is_empty() { "on" } else { action }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_point_defaults_to_the_on_action() {
        assert_eq!(parse("after-extract"), Some(("after-extract", "on")));
    }

    #[test]
    fn a_point_and_an_action_split_on_the_first_colon() {
        assert_eq!(
            parse("after-extract:pause"),
            Some(("after-extract", "pause"))
        );
        assert_eq!(parse("rename:eexist"), Some(("rename", "eexist")));
        assert_eq!(parse("unpack:corrupt"), Some(("unpack", "corrupt")));
    }

    #[test]
    fn a_second_colon_belongs_to_the_action() {
        assert_eq!(parse("write:enospc:2"), Some(("write", "enospc:2")));
    }

    #[test]
    fn an_empty_spec_arms_nothing() {
        assert_eq!(parse(""), None);
        assert_eq!(parse(":pause"), None);
    }

    #[test]
    fn an_empty_action_is_the_default_rather_than_a_nameless_one() {
        assert_eq!(parse("rename:"), Some(("rename", "on")));
    }

    // Absent rather than vacuous under the feature: `mise run test` and CI run
    // with `--features fault-injection`, and a test that reports `ok` there
    // while asserting nothing would read as a guarantee the project's primary
    // test command never checks.
    #[cfg(not(feature = "fault-injection"))]
    #[test]
    fn nothing_is_armed_without_the_feature() {
        // The default build must not have a fault point at all, whatever the
        // environment of the process says.
        assert_eq!(point("after-extract"), None);
    }

    #[test]
    fn an_action_this_build_implements_arms_that_action() {
        assert_eq!(
            armed_by("after-extract:pause", "after-extract"),
            Some("pause")
        );
        assert_eq!(armed_by("rename:eexist", "rename"), Some("eexist"));
        assert_eq!(armed_by("unpack:corrupt", "unpack"), Some("corrupt"));
        assert_eq!(armed_by("launcher:panic", "launcher"), Some("panic"));
        assert_eq!(armed_by("rename", "rename"), Some("on"));
    }

    #[test]
    fn an_action_this_build_does_not_implement_arms_nothing() {
        // The closed set asserted through the resolver rather than against the
        // table: a spec naming an action this build has no branch for must arm
        // nothing rather than fall back to `on`, and reading that off `ACTIONS`
        // would only restate the table to itself.
        for spec in ["rename:enospc", "after-extract:kill", "unpack:truncate"] {
            let (point, _) = parse(spec).expect("a well-formed spec still parses");
            assert_eq!(armed_by(spec, point), None, "`{spec}` armed something");
        }
    }

    #[test]
    fn a_spec_arms_only_the_point_it_names() {
        assert_eq!(armed_by("rename:eexist", "after-extract"), None);
        assert_eq!(armed_by("", "rename"), None);
    }

    #[test]
    fn a_point_no_spec_can_name_is_never_armed() {
        // Through the public entry point, so that the environment-reading path
        // is exercised too. The name is one no build has a point for, so the
        // assertion holds whatever `GINARY_FAULT` this process was started
        // with, feature on or off.
        assert_eq!(point("a-point-this-build-does-not-have"), None);
    }

    /// The crate's own sources, minus this module: the call sites the table
    /// claims to enumerate.
    fn call_site_sources() -> Vec<(String, String)> {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut out = Vec::new();
        for entry in std::fs::read_dir(&dir).expect("`src/` is readable") {
            let path = entry.expect("a readable directory entry").path();
            if path.extension().is_some_and(|ext| ext == "rs")
                && path.file_name().is_some_and(|name| name != "fault.rs")
            {
                let text = std::fs::read_to_string(&path).expect("a readable source file");
                out.push((path.display().to_string(), text));
            }
        }
        assert!(!out.is_empty(), "no sources found next to `fault.rs`");
        out
    }

    #[test]
    fn every_call_site_is_a_listed_point() {
        // The direction that catches a point added to the code and to neither
        // the module table nor the documents.
        for (path, text) in call_site_sources() {
            for tail in text.split("fault::point(\"").skip(1) {
                let name = tail.split('"').next().expect("a terminated string literal");
                assert!(
                    FAULT_POINTS.contains(&name),
                    "`{path}` arms `{name}`, which `FAULT_POINTS` does not list"
                );
            }
        }
    }

    #[test]
    fn every_listed_point_has_a_call_site() {
        // And the direction that catches a point removed from the code but left
        // in the lists, which would document a fault a test can never trigger.
        let sources = call_site_sources();
        for name in FAULT_POINTS {
            let needle = format!("fault::point(\"{name}\")");
            assert!(
                sources.iter().any(|(_, text)| text.contains(&needle)),
                "`FAULT_POINTS` lists `{name}`, which nothing arms"
            );
        }
    }

    #[test]
    fn both_documents_list_every_point() {
        // `docs/dev/debugging.md` and `docs/dev/testing.md` both say "the
        // points are", so both are wrong the moment one is missing. Matching on
        // the backticked name covers `after-extract:pause` as well as a bare
        // `before-lock`, which is how a point whose action is `on` reads.
        for doc in ["docs/dev/debugging.md", "docs/dev/testing.md"] {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(doc);
            let text = std::fs::read_to_string(&path).expect("a readable document");
            for name in FAULT_POINTS {
                assert!(
                    text.contains(&format!("`{name}`")) || text.contains(&format!("`{name}:")),
                    "`{doc}` does not list the `{name}` fault point"
                );
            }
        }
    }

    #[test]
    fn no_document_lists_a_point_this_build_does_not_have() {
        // A backticked `<point>:<action>` whose action this build implements is
        // an enumeration entry, so its point must be one of ours: this is what
        // catches a renamed point left behind in the prose.
        for doc in ["docs/dev/debugging.md", "docs/dev/testing.md"] {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(doc);
            let text = std::fs::read_to_string(&path).expect("a readable document");
            for tail in text.split('`').skip(1).step_by(2) {
                let Some((name, action)) = tail.split_once(':') else {
                    continue;
                };
                if !ACTIONS.contains(&action) {
                    continue;
                }
                assert!(
                    FAULT_POINTS.contains(&name),
                    "`{doc}` names the `{name}` fault point, which this build does not have"
                );
            }
        }
    }
}
