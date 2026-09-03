// SPDX-License-Identifier: MIT OR Apache-2.0
//! How a host spells a path in the output the suite reads back.
//!
//! Three rules, and each of them is a rule the suite already applies — badly.
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
