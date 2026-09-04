// SPDX-License-Identifier: MIT OR Apache-2.0
//! How a host spells a path in the output the suite reads back.
//!
//! Six rules, and each of them is a rule the suite already applies — badly.
//! A test that asserts on a *rendered* path is asserting on two things at
//! once: what ginary decided, and how the machine underneath it writes a path
//! down. The second half is not the subject, and every place it leaked in is a
//! Windows failure on a green Linux tree:
//!
//! ```text
//! ---- doctor_text_names_the_otp_root_and_version stdout ----
//! no absolute `otp root:` line in: ... otp root: d:/a/_temp/.setup-beam/otp
//!
//! ---- the_failure_messages_read_as_sentences stdout ----
//! +D:\a\ginary\ginary\tests/fixtures/app\malformed.app: line 5, column 3: ...
//!
//! ---- the_shadowing_warning_names_both_directories stdout ----
//! +`crypto` is in both trees; using the shipment copy at `<tmp>\shipment\crypto\ebin`
//! ```
//!
//! (`Windows build and exit-code propagation`
//! <https://github.com/P4suta/ginary/actions/runs/33739517757/job/100597889388>.)
//!
//! Each rule is a pure function of text, so each is decided here rather than
//! by the machine the suite happens to be running on, and each is exercised on
//! Linux against the spellings only Windows produces. That is the same
//! technique `ginary::cache::xdg_base_is_absolute` uses for the XDG rule and
//! `ginary::winpath` uses for the `\\?\` prefix; see
//! `tests/regressions/e7_the_xdg_rule_used_the_hosts_idea_of_an_absolute_path.rs`.

use std::path::Path;

use ginary::target::Os;

/// Whether `text` is an absolute path as `os` spells one.
///
/// | os | absolute | not absolute |
/// |---|---|---|
/// | `Linux`, `Macos` | `/opt/otp` | `otp`, `C:\otp` |
/// | `Windows` | `C:\otp`, `d:/a/otp`, `\\srv\share`, `\\?\C:\otp` | `otp`, `/opt/otp` |
///
/// A Windows drive letter may be either case and either separator may follow
/// it: `erl -noshell` prints `d:/a/_temp/.setup-beam/otp` and `cmd` prints
/// `D:\a\_temp\.setup-beam\otp`, and both name the same absolute directory.
pub fn is_absolute_for(os: Os, text: &str) -> bool {
    match os {
        Os::Linux | Os::Macos => text.starts_with('/'),
        Os::Windows => {
            let mut characters = text.chars();
            match (characters.next(), characters.next(), characters.next()) {
                // A UNC or verbatim path: `\\server\share`, `\\?\C:\...`, and the
                // forward-slash spelling the runtime is equally happy to print.
                (Some('\\'), Some('\\'), _) | (Some('/'), Some('/'), _) => true,
                // Drive-absolute. Both halves matter: `d:` alone names a
                // drive's *current* directory, which is not an absolute path,
                // and a lone leading separator is relative to the current
                // drive, which is why `/opt/otp` is not absolute here.
                (Some(drive), Some(':'), Some('\\' | '/')) => drive.is_ascii_alphabetic(),
                _ => false,
            }
        }
    }
}

/// `text` with every `\` respelled as `/`.
///
/// For a snapshot whose subject is a path and whose sentences carry no
/// backslash of their own, which is what the closure and inspect snapshots
/// are: a path list, a hint, and no escape sequence anywhere.
pub fn slashed(text: &str) -> String {
    text.replace('\\', "/")
}

/// `text` with `dir`, and the separator that follows it, removed — whichever
/// separator the host joined the two with.
///
/// `Path::join` uses the host's separator, and a fixture directory built by
/// joining a relative path onto `CARGO_MANIFEST_DIR` carries whatever
/// separators each half was written with, so the rendered path is
/// `D:\a\ginary\ginary\tests/fixtures/app\malformed.app`: three components
/// joined with `\` and two with `/`. A message that names such a file is
/// about the file name; the directory in front of it is the machine.
pub fn strip_dir(text: &str, dir: &Path) -> String {
    // The comparison is made on the forward-slash spelling of both sides, and
    // the cut is made in the original: replacing `\` with `/` swaps one ASCII
    // byte for another, so a byte offset into the respelled text is the same
    // offset into the text itself and nothing outside the directory is
    // rewritten.
    let prefix = format!("{}/", slashed(&dir.display().to_string()));
    let haystack = slashed(text);
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    while let Some(found) = haystack[at..].find(&prefix) {
        let start = at + found;
        out.push_str(&text[at..start]);
        at = start + prefix.len();
    }
    out.push_str(&text[at..]);
    out
}

/// The character `os` joins path components with.
///
/// `\` on Windows and `/` everywhere else. Asked of a named `os` rather than
/// of `std::path::MAIN_SEPARATOR`, for the reason every rule in this module
/// is: both answers are then asserted on one machine.
pub const fn separator_for(os: Os) -> char {
    match os {
        Os::Windows => '\\',
        Os::Linux | Os::Macos => '/',
    }
}

/// `root`, joined to the `/`-separated `relative`, as `os` would spell the
/// whole path.
///
/// The rule four expectations got wrong. A test that wants the path ginary
/// *walked to* writes the relative half the way a listing carries it — a
/// `/`-separated `lib/kernel-11.0.3/ebin/kernel.beam` — and then reaches for
/// `Path::join`, which appends the host separator between the two halves and
/// leaves every separator *inside* the relative half exactly as it was
/// written. The result on Windows is the mixed spelling nothing produces:
///
/// ```text
///   left: "C:\\Users\\RUNNER~1\\...\\out\\lib\\kernel-11.0.3\\ebin\\kernel.beam"
///  right: "C:\\Users\\RUNNER~1\\...\\out\\lib/kernel-11.0.3/ebin/kernel.beam"
/// ```
///
/// (`Windows build and exit-code propagation`
/// <https://github.com/P4suta/ginary/actions/runs/33751715516/job/100636537290>,
/// `no_directory_is_passed_to_the_runtime_where_a_module_belongs`.)
///
/// `root` is left as it is: it came from the host and is already spelled the
/// way the host spells it. Only the relative half is respelled, and only
/// where the platform puts a backslash there in the first place — a `\` is an
/// ordinary character in a unix file name, which is why this is a function of
/// `os` and not a blanket `replace`.
pub fn joined_for(os: Os, root: &str, relative: &str) -> String {
    let separator = separator_for(os);
    let relative = relative.replace('/', &separator.to_string());
    if root.is_empty() {
        return relative;
    }
    if relative.is_empty() {
        return root.to_owned();
    }
    format!("{root}{separator}{relative}")
}

/// [`joined_for`] asked about the host this suite is running on.
pub fn joined(root: &Path, relative: &str) -> String {
    joined_for(
        ginary::platform::HOST,
        &root.display().to_string(),
        relative,
    )
}

/// `text` as it appears *inside* a JSON string.
///
/// A trace record is a JSON document, and one of its values is itself a JSON
/// document, so a Windows path inside it is escaped twice: the four
/// characters `\\\\` stand for the one separator a person typed. A test that
/// looks for the raw path in the rendered line finds nothing, and says the
/// path is missing when it is there:
///
/// ```text
/// the record must name the entry that vanished, and it is:
/// {"phase":"prune","kv":{"removed_paths":"[\"C:\\\\Users\\\\RUNNER~1\\\\...\\\\0000000000000000\"]"}}
/// ```
///
/// (`b1_the_prune_trace_named_nothing_it_removed`, same job.) The escaping is
/// JSON's and not the platform's, so it is applied on every host; on unix
/// there is nothing in a path for it to act on and the answer is the argument.
pub fn json_escaped(text: &str) -> String {
    // `serde_json`'s own escaping and not a hand-rolled `replace`, so the
    // needle is produced by the same code that produced the haystack. The
    // rendered string carries the surrounding quotes; the needle is what is
    // between them.
    let rendered = serde_json::to_string(text).expect("a string always renders as JSON");
    rendered
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .unwrap_or(&rendered)
        .to_owned()
}

/// `text` as it appears inside `depth` nested JSON strings.
///
/// [`json_escaped`] is one level, and one level is the right answer for a
/// value that is written into a JSON document once. A `prune` record is not
/// that: `cache::record_prune` renders the path list with
/// `ginary::launch::json_array` and hands the rendered *string* to
/// `Diag::kv`, which renders the record, so a separator a person typed once
/// is four characters by the time it is on the line:
///
/// ```text
/// {"phase":"prune","kv":{"removed_paths":"[\"C:\\\\Users\\\\RUNNER~1\\\\hello\"]"}}
/// ```
///
/// (`b1_the_prune_trace_named_nothing_it_removed`,
/// <https://github.com/P4suta/ginary/actions/runs/33823103540/job/100869848230>.)
/// The depth is the caller's, because it is a property of the record being
/// read and not of the path: a value written straight into one document is
/// `depth` 1, and one that travels through a rendered document on the way is
/// 2.
pub fn nested_json_escaped(text: &str, depth: usize) -> String {
    // One application of the rule per document, and the rule is
    // `json_escaped` rather than a second copy of it: the needle for a value
    // written into a document that is itself written into a document is the
    // needle for one document, escaped again.
    (0..depth).fold(text.to_owned(), |escaped, _| json_escaped(&escaped))
}

/// Whether `left` and `right` name the same file, whichever spelling each
/// carries.
///
/// `ginary::cache::ensure_extracted` deliberately answers with the verbatim
/// `\\?\` spelling — `ginary::winpath` says why — and a test that built the
/// same directory by hand holds the ordinary one. Both name one directory:
///
/// ```text
///   left: "\\\\?\\C:\\Users\\RUNNER~1\\...\\cache\\hello\\1179d51043100e24"
///  right: "C:\\Users\\RUNNER~1\\...\\cache\\hello\\1179d51043100e24"
/// ```
///
/// (`a_cold_cache_extracts_into_the_key_directory`, same job.) The comparison
/// is made on the plain spelling of both sides, which is
/// `ginary::winpath::plain_path`'s whole purpose and is the identity on unix.
pub fn same_path(left: &Path, right: &Path) -> bool {
    ginary::winpath::plain_path(left) == ginary::winpath::plain_path(right)
}

/// Whether `left` and `right` are two spellings of one path, as `os` spells
/// paths.
///
/// The rule two "a real thing really works" tests were missing. A packaged
/// application prints the directory it started in with `file:get_cwd/0`, and
/// the runtime writes that back the way the C library gives it: a lower-case
/// drive letter and forward separators. The test held the spelling
/// `std::fs::canonicalize` produced, which on Windows is the verbatim
/// `\\?\` form with an upper-case drive and backslashes. One directory, two
/// spellings, and a `String::contains` that says they are different things:
///
/// ```text
/// ---- the_built_artifact_runs_the_application_with_no_erlang_on_the_machine ----
/// the application must start where the user is, not where the runtime unpacked:
/// ...cwd=c:/Users/RUNNER~1/AppData/Local/Temp/.tmpGAJaYB/args-cwd
///
/// ---- a_staged_hello_ffi_prints_its_arguments_and_its_priv_file ----
/// the application did not start in the directory it was given:
/// ...cwd=c:/Users/RUNNER~1/AppData/Local/Temp/.tmpyueybk/run/cwd
/// ```
///
/// (`Windows build and exit-code propagation`
/// <https://github.com/P4suta/ginary/actions/runs/33823103540/job/100869848230>,
/// `tests/e2e_hello.rs:117` and `tests/stage_run.rs:104`.)
///
/// Three differences and each is Windows path *syntax*, so each is decided
/// here rather than by the machine the suite runs on: the verbatim prefix,
/// which [`ginary::winpath::plain_path_str`] already removes; the separator,
/// which is `\` and `/` for one directory on that platform and one character
/// of a file's name on unix; and the drive letter, whose case a Windows
/// filesystem does not distinguish. Nothing else is folded — the rest of the
/// path is compared byte for byte, because a case-insensitive comparison of a
/// whole path would call two different unix files one file.
pub fn same_directory_text(os: Os, left: &str, right: &str) -> bool {
    one_spelling(os, left) == one_spelling(os, right)
}

/// `text` in the one spelling [`same_directory_text`] compares.
///
/// Three transformations on Windows and none anywhere else, and each is a
/// difference between two spellings of one directory rather than a
/// difference between two directories:
///
/// | in | out |
/// |---|---|
/// | `\\?\C:\a\b` | `c:/a/b` |
/// | `C:\a\b` | `c:/a/b` |
/// | `c:/a/b` | `c:/a/b` |
/// | `C:\a\B` | `c:/a/B` |
///
/// The drive letter is the only character whose case is folded. A Windows
/// filesystem does not distinguish case anywhere in a path, but a rule that
/// folded the whole of one would call two files one file on every host that
/// does — and the two spellings this exists to reconcile differ in the drive
/// letter alone, because that is the character `file:get_cwd/0` and
/// `std::fs::canonicalize` disagree about.
fn one_spelling(os: Os, text: &str) -> String {
    match os {
        Os::Linux | Os::Macos => text.to_owned(),
        Os::Windows => {
            let plain = slashed(&ginary::winpath::plain_path_str(text));
            let mut characters = plain.chars();
            match (characters.next(), characters.next()) {
                (Some(drive), Some(':')) if drive.is_ascii_alphabetic() => {
                    format!("{}:{}", drive.to_ascii_lowercase(), characters.as_str())
                }
                _ => plain,
            }
        }
    }
}

/// Whether the path a packaged application printed names `expected`.
///
/// [`same_directory_text`]'s two callers do not compare two strings out of a
/// log, they compare a string a running program printed against a directory
/// the test built, and on Windows those differ by one more thing text cannot
/// settle: `%TEMP%` on a GitHub runner is the 8.3 spelling
/// `C:\Users\RUNNER~1\...` while the long name is `C:\Users\runneradmin\...`,
/// and only the filesystem knows they are one directory.
///
/// So both sides are canonicalised where the filesystem can answer — which
/// resolves the short name, the verbatim prefix, the letter case and any
/// symbolic link in one step — and the pure rule decides it where it cannot,
/// so that a recorded spelling from another platform is still testable here.
pub fn names_the_same_directory(printed: &str, expected: &Path) -> bool {
    if let (Ok(left), Ok(right)) = (
        std::fs::canonicalize(Path::new(printed)),
        std::fs::canonicalize(expected),
    ) {
        return left == right;
    }
    // One side the filesystem could not resolve: a directory that has been
    // removed, a relative name, or a spelling recorded from another platform.
    // The pure rule is what is left, and it is still a comparison — two names
    // nothing can look up are not equal because neither could be looked up.
    same_directory_text(
        ginary::platform::HOST,
        printed,
        &expected.display().to_string(),
    )
}

/// The directory a `hello_ffi` run printed, out of its standard output.
///
/// The fixture ends its run with `cwd=` and whatever `file:get_cwd/0` gave
/// it, and the two tests that read that line built the needle with
/// `format!("cwd={}", expected.display())` and asked `String::contains`.
/// That is a comparison of two spellings as text, which is the defect
/// [`names_the_same_directory`] exists for; this is the other half of the
/// fix, which is having the printed spelling to hand at all.
///
/// [`None`] when no line begins `cwd=`, which is a failed run rather than a
/// mismatched directory and is worth a different message.
pub fn printed_cwd(stdout: &str) -> Option<&str> {
    stdout
        .lines()
        .find_map(|line| line.trim_end().strip_prefix("cwd="))
}

/// The emulator the runtime `os` ships carries, as a path suffix a report's
/// object list can be searched for.
///
/// `/beam.smp` on unix and `/beam.smp.dll` on Windows, where the emulator is
/// a library `erl.exe` loads rather than a program `erlexec` execs.
/// `ginary::target::Target::emulator_program` is the rule; this is that rule
/// with the separator a report path carries in front of it, so that a search
/// cannot match an application called `beam.smp`.
pub fn emulator_suffix(os: Os) -> String {
    format!(
        "/{}",
        ginary::target::Target::new(os, ginary::target::Arch::X86_64, ginary::target::Libc::None)
            .emulator_program()
    )
}
