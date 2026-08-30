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
//! So the launcher carries named points a test can arm from the environment.
//! `GINARY_FAULT=<point>[:<action>]` is read once, and the points are:
//!
//! | point | action | effect |
//! |---|---|---|
//! | `after-extract` | `pause` | sleep [`PAUSE`] with the temporary tree on disk, so a test can `SIGKILL` the process |
//! | `rename` | `eexist` | the rename onto the cache entry reports `EEXIST`, as a lost race does |
//! | `unpack` | `corrupt` | a byte of the manifest is flipped in memory, so the digest cannot match |
//! | `launcher` | `panic` | the launcher panics, so the panic hook `main` installs is the thing under test |
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

/// The actions a point may be armed with.
///
/// A closed set, because [`point`] answers with a `&'static str` and because an
/// action this build does not implement must arm nothing rather than arm the
/// default: `GINARY_FAULT=rename:enospc` is a test asking for a fault that is
/// not here, and silently giving it `on` would make that test pass for the
/// wrong reason.
#[cfg_attr(
    not(feature = "fault-injection"),
    expect(dead_code, reason = "only the fault-injection build reads the table")
)]
const ACTIONS: [&str; 5] = ["on", "pause", "eexist", "corrupt", "panic"];

/// The action armed for `name`, reading [`VAR`] once per process.
#[cfg(feature = "fault-injection")]
fn armed(name: &str) -> Option<&'static str> {
    static SPEC: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    let spec = SPEC
        .get_or_init(|| std::env::var(VAR).ok())
        .as_deref()
        .unwrap_or_default();
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

    #[cfg(feature = "fault-injection")]
    #[test]
    fn an_action_this_build_does_not_implement_arms_nothing() {
        // `armed` reads the environment once per process, so what is asserted
        // is the closed set it filters through: a spec that names an action
        // this build has no branch for must arm nothing rather than fall back
        // to `on`.
        for spec in ["rename:enospc", "after-extract:kill", "unpack:truncate"] {
            let (_, action) = parse(spec).expect("a well-formed spec still parses");
            assert!(
                !ACTIONS.contains(&action),
                "`{spec}` names an action this build implements, so it would arm"
            );
        }
    }
}
