// SPDX-License-Identifier: MIT OR Apache-2.0
//! A build hook's `{out_dir}` stopped being one argument at the first space.
//!
//! `native::run_hook` substituted the two tokens into the command line with a
//! plain `str::replace` and handed the result to `sh -c`, so the README's own
//! worked example
//!
//! ```text
//! build = "sh scripts/build_nif.sh {target} {out_dir}"
//! ```
//!
//! became, for a project under `~/My Documents/app`,
//!
//! ```text
//! sh scripts/build_nif.sh linux-aarch64-musl /home/u/My Documents/app/build/…
//! ```
//!
//! — four arguments where the script expects two, and a `$(...)` or a `;` in
//! the path would have been worse than a broken command. A space is the
//! ordinary case: `My Documents`, `Application Support`. The same defect in
//! `catalog::fetch_command` is the first entry in this milestone's log, and the
//! helper it added — `process::shell_quote` — sits one module away.
//!
//! The right behaviour: both tokens arrive as exactly one shell word, whatever
//! the path holds. This test proves it the way the catalogue one does, by
//! running the command: the hook records `$#` and `$1`, and the directory it
//! was told about has a space in its name.
#![cfg(feature = "cli")]

use std::path::Path;

use ginary::native::{self, HookCtx};
use ginary::target::{Arch, Libc, Os, Target};

use crate::common::fake_otp::{DEFAULT_ERTS_VSN, DEFAULT_OTP_VERSION, FakeOtp};

/// The project directory name that broke the hook.
const AWKWARD: &str = "my project";

#[test]
fn a_hook_is_handed_its_output_directory_as_one_word() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let project = dir.path().join(AWKWARD);
    std::fs::create_dir_all(&project).expect("the project directory");
    let otp = FakeOtp::new().build_in(dir.path().join("otp"));
    // Every argument the shell split the command into, and nothing else: a
    // hook that received two words where it expected one is the whole bug.
    std::fs::write(
        project.join("build_nif.sh"),
        "#!/bin/sh\n{ echo \"argc=$#\"; echo \"argv1=$1\"; echo \"argv2=$2\"; } > \
         \"$OUT_DIR/argv.txt\"\n",
    )
    .expect("the hook script");
    let out_dir = project.join("out dir");
    let target = Target::new(Os::Linux, Arch::Aarch64, Libc::Musl);

    native::run_hook(
        "esqlite",
        "sh build_nif.sh {out_dir} {target}",
        &HookCtx {
            target: &target,
            out_dir: &out_dir,
            project_root: &project,
            erts_root: &otp.root,
            erts_version: DEFAULT_ERTS_VSN,
            otp_version: DEFAULT_OTP_VERSION,
        },
    )
    .expect("the hook runs");

    let recorded = argv(&out_dir);
    assert_eq!(
        recorded.first().map(String::as_str),
        Some("argc=2"),
        "two tokens are two arguments, however many spaces the path holds: {recorded:?}"
    );
    assert_eq!(
        recorded.get(1).map(String::as_str),
        Some(format!("argv1={}", out_dir.display()).as_str()),
        "and the first of them is the whole directory: {recorded:?}"
    );
    assert_eq!(
        recorded.get(2).map(String::as_str),
        Some("argv2=linux-aarch64-musl"),
        "{recorded:?}"
    );
}

/// The lines the hook script recorded.
fn argv(out_dir: &Path) -> Vec<String> {
    let path = out_dir.join("argv.txt");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("the hook wrote no {}: {error}", path.display()))
        .lines()
        .map(str::to_owned)
        .collect()
}
