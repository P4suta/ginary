// SPDX-License-Identifier: MIT OR Apache-2.0
//! `HEART_COMMAND` was built by joining on spaces, so `heart` restarted the
//! wrong program.
//!
//! **What went wrong.** `launch::plan` set `HEART_COMMAND` to the artifact's
//! own path followed by the user's arguments, separated by single spaces and
//! quoted nowhere. `heart` does not `execve` that value: it hands it to
//! `/bin/sh -c`, which splits it on whitespace again and expands whatever it
//! finds. An artifact at `/opt/my app/worker` therefore produced a restart
//! command that ran `/opt/my`, and an argument such as `--msg 'hello world'`
//! came back to the application as two.
//!
//! **The input.** A plan whose `self_exe` holds a space and whose user
//! arguments hold a space, an empty string and a shell metacharacter.
//!
//! **The correct behaviour.** The value is a shell *word list*: every element
//! that is not made entirely of safe characters is single-quoted, so that
//! `sh -c "$HEART_COMMAND"` re-runs the artifact with exactly the argument
//! vector it was given the first time. The proof is executable: the value is
//! given to a real `sh -c`, and what the program sees is compared with what
//! the plan was built from.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Command;

use ginary::cache::Env;
use ginary::launch::{self, HEART_COMMAND_VAR, LaunchPlan};

use crate::common::artifact::canonical_manifest;

/// A directory name with a space in it, which is where the artifact lives.
const AWKWARD_DIR: &str = "my artifacts";

/// The artifact's own file name.
const PROGRAM: &str = "worker";

/// The arguments the application was started with, and must be restarted with.
const USER_ARGS: [&str; 4] = ["--msg", "hello world", "", "a;b"];

/// The plan a manifest under `heart` produces for `self_exe` and `args`.
fn plan_for(self_exe: &Path, args: &[&str]) -> LaunchPlan {
    let mut manifest = canonical_manifest();
    manifest.launch.heart = true;
    let user: Vec<OsString> = args.iter().map(OsString::from).collect();
    launch::plan(
        Path::new("/cache/hello/0123456789abcdef"),
        &manifest,
        &user,
        &Env::from_pairs(std::iter::empty()),
        Path::new("/cache/hello"),
        self_exe,
    )
    .expect("the canonical manifest plans")
}

/// The value the plan sets for `HEART_COMMAND`.
fn heart_command(plan: &LaunchPlan) -> String {
    plan.set
        .iter()
        .find(|(name, _)| name == OsStr::new(HEART_COMMAND_VAR))
        .map(|(_, value)| value.to_string_lossy().into_owned())
        .expect("a manifest under heart sets HEART_COMMAND")
}

/// A script that prints one line per argument, under a directory with a space.
///
/// The trailing marker line is what tells an empty argument at the end from an
/// argument that was dropped.
fn awkward_artifact(dir: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt as _;

    let home = dir.join(AWKWARD_DIR);
    std::fs::create_dir_all(&home).expect("the directory with a space in it");
    let program = home.join(PROGRAM);
    std::fs::write(
        &program,
        "#!/bin/sh\nfor argument in \"$@\"; do printf '%s\\n' \"$argument\"; done\nprintf 'end\\n'\n",
    )
    .expect("write the program");
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755))
        .expect("make it executable");
    program
}

#[test]
fn the_heart_command_re_runs_the_artifact_with_the_argument_vector_it_was_given() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let program = awkward_artifact(dir.path());

    let plan = plan_for(&program, &USER_ARGS);
    let command = heart_command(&plan);

    // What `heart` does with the value, done here: `/bin/sh -c <value>`.
    let output = Command::new("/bin/sh")
        .arg("-c")
        .arg(&command)
        .output()
        .expect("run the heart command through a shell");
    assert!(
        output.status.success(),
        "`sh -c` on HEART_COMMAND={command:?} failed with {}\n--- stderr ---\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );

    let mut expected: Vec<String> = USER_ARGS.iter().map(|s| (*s).to_owned()).collect();
    expected.push("end".to_owned());
    let seen: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_owned)
        .collect();
    assert_eq!(
        seen, expected,
        "heart must restart the application with the arguments it had, and HEART_COMMAND={command:?} \
         gave it {seen:?}"
    );
}

#[test]
fn an_ordinary_path_and_ordinary_arguments_are_left_unquoted() {
    // The quoting is for the values that need it. A command a user can read
    // and paste is worth keeping, and the common case has nothing in it a
    // shell would touch.
    let plan = plan_for(Path::new("/opt/bin/hello"), &["--name", "world"]);

    assert_eq!(heart_command(&plan), "/opt/bin/hello --name world");
}
