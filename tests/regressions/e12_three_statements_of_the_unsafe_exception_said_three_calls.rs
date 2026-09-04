// SPDX-License-Identifier: MIT OR Apache-2.0
//! The crate's unsafe-code exception is stated in four places and E12 taught
//! one of them about the fourth call.
//!
//! **What went wrong.** `#![deny(unsafe_code)]` is lifted for exactly one
//! module, and a reader is sent to three documents to find out why: the crate
//! comment in `src/lib.rs`, the Prohibitions section of `CLAUDE.md`, and
//! `docs/adr/0015-windows-launcher-stays-resident.md`. All three described the
//! exception as the resident launcher's own — "the three `kernel32` calls the
//! Windows launcher cannot be written without", the console control handler
//! and the job object. E12 added a fourth, `win32::process_is_alive`, which is
//! not the launcher's at all: it is how `cache::sweep` asks whether the
//! process that owns a temporary tree still exists. Only the module's own
//! comment was updated.
//!
//! The ADR's block count was wrong before that and got worse: it says "one
//! module, four `unsafe` blocks" and the module held five at E11 and holds
//! seven now. The module also became `pub(crate) mod win32` from a private
//! one, so the crate-visible unsafe surface widened, and no document outside
//! the module said so.
//!
//! **The input.** Reading any of the three statements. Nothing fails and
//! nothing is unsound; what is wrong is that the reviewable surface
//! `CLAUDE.md` sends a reviewer to is described smaller than it is, which is
//! the one thing a bounded exception cannot afford.
//!
//! **The correct behaviour.** A count that is stated is a count that is
//! checked, and a statement of the exception that stops at the launcher's own
//! calls is a statement of a different exception. No new
//! `#[allow(unsafe_code)]` was added by E12 and none is owed an ADR of its
//! own; only the counts were wrong.
//!
//! **Round 2 widened the scan twice, for two things it would not have seen.**
//! The prose statements were fixed and the file that *declares* the call
//! surface was not: `Cargo.toml`'s two dependency comments still said three
//! rustix calls and "the two Win32 facilities … and nothing else", so it joins
//! [`STATEMENTS`] and gets a check of its own in
//! [`the_dependency_comments_name_every_call_the_launcher_path_makes`]. And
//! the premise test read the top level of `src/` only, so it stated a rule
//! about the whole crate — "a second `#[allow(unsafe_code)]` anywhere" — while
//! a module added as `src/foo/bar.rs`, or `build.rs`, could have carried one
//! unseen; it walks the tree now.

use crate::common::repo::{read, root};

/// The one file allowed to lift `#![deny(unsafe_code)]`.
const EXCEPTED: &str = "src/launch_windows.rs";

/// The places a reader is sent to for the exception.
///
/// `Cargo.toml` is one of them and was the statement round 1 missed: the two
/// dependency comments are what declare the call surface the crate may reach
/// on the launcher path, and the windows one said "the two Win32 facilities …
/// and nothing else" after E12 had added the third.
const STATEMENTS: [&str; 4] = [
    "src/lib.rs",
    "CLAUDE.md",
    "docs/adr/0015-windows-launcher-stays-resident.md",
    "Cargo.toml",
];

/// The English numbers this project's prose writes a count with.
const NUMBER_WORDS: [(&str, usize); 12] = [
    ("one", 1),
    ("two", 2),
    ("three", 3),
    ("four", 4),
    ("five", 5),
    ("six", 6),
    ("seven", 7),
    ("eight", 8),
    ("nine", 9),
    ("ten", 10),
    ("eleven", 11),
    ("twelve", 12),
];

/// The attribute, spelled once.
const ALLOW: &str = "#[allow(unsafe_code)]";

/// How many `unsafe` blocks the excepted module really holds.
fn unsafe_blocks() -> usize {
    read(EXCEPTED).matches("unsafe {").count()
}

/// `text` with every run of whitespace collapsed to one space.
///
/// Prose here hard-wraps at about a hundred columns, so a phrase this scan
/// looks for is as likely to be split across two lines as not, and a scan that
/// only found the unwrapped spelling would report a document clean because its
/// author had reached the margin.
fn unwrapped(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Every Rust source the crate compiles, as a repository-relative path.
///
/// The whole tree rather than the top level of `src/`: the rule this file
/// checks is stated as "anywhere", `src/` is flat only by habit, and `build.rs`
/// is compiled too. A scan that stops at the first directory reports a rule it
/// did not check.
fn rust_sources() -> Vec<String> {
    let root = root();
    let mut found: Vec<String> = Vec::new();
    let mut pending = vec![root.join("src")];
    while let Some(dir) = pending.pop() {
        let entries = std::fs::read_dir(&dir)
            .unwrap_or_else(|error| panic!("cannot list {}: {error}", dir.display()));
        for entry in entries {
            let path = entry.expect("a directory entry").path();
            if path.is_dir() {
                pending.push(path);
                continue;
            }
            if path.extension().is_none_or(|suffix| suffix != "rs") {
                continue;
            }
            let relative = path
                .strip_prefix(&root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            found.push(relative);
        }
    }
    if root.join("build.rs").is_file() {
        found.push("build.rs".to_owned());
    }
    found.sort();
    found
}

#[test]
fn the_exception_is_still_one_module() {
    // The premise every statement below rests on, and the one `CLAUDE.md`
    // makes a rule: a second `#[allow(unsafe_code)]` anywhere needs an ADR of
    // its own, so a second one appearing without one is the finding rather
    // than a count to update.
    let sources = rust_sources();
    assert!(
        sources.contains(&EXCEPTED.to_owned()),
        "the scan reaches the one module that takes the exception: {sources:?}"
    );

    let mut carriers: Vec<String> = Vec::new();
    for relative in sources {
        // The attribute, and not a mention of it: two of the statements
        // discuss the exception in prose, and a scan that counted the
        // discussion would report the module that argues for the exception as
        // a second one taking it.
        let allows = read(&relative)
            .lines()
            .filter(|line| line.trim() == ALLOW)
            .count();
        for _ in 0..allows {
            carriers.push(relative.clone());
        }
    }
    carriers.sort();

    assert_eq!(
        carriers,
        vec![EXCEPTED.to_owned()],
        "the crate carries exactly one `#[allow(unsafe_code)]`, in the module ADR 0015 argues \
         for; a second one is a decision, not a detail"
    );
}

#[test]
fn every_stated_block_count_is_the_count_the_module_really_has() {
    let blocks = unsafe_blocks();
    assert!(blocks > 0, "the excepted module holds `unsafe` blocks");

    let mut stated = 0usize;
    for path in STATEMENTS {
        let text = unwrapped(&read(path));
        for (word, value) in NUMBER_WORDS {
            let phrase = format!("{word} `unsafe` blocks");
            if text.contains(&phrase) {
                stated += 1;
                assert_eq!(
                    value, blocks,
                    "{path} says `{phrase}` and {EXCEPTED} holds {blocks}: a count nobody \
                     recomputes is a count that drifts, and this one drifted twice"
                );
            }
        }
    }
    assert_eq!(
        stated, 1,
        "exactly one of the statements carries the block count, so there is one number to \
         keep true rather than four"
    );
}

#[test]
fn every_statement_of_the_exception_covers_the_call_that_is_not_the_launchers() {
    // The drift itself. `process_is_alive` is `cache::sweep`'s, not the
    // resident launcher's, so a statement that describes the exception as the
    // console handler and the job object describes a smaller surface than the
    // crate has.
    for path in STATEMENTS {
        let text = read(path);
        assert!(
            text.contains("process_is_alive") || text.contains("OpenProcess"),
            "{path} states the crate's unsafe-code exception and does not mention the call \
             `cache::sweep` makes through it: a reviewer sent here is told about a smaller \
             module than the one that carries the `#[allow(unsafe_code)]`"
        );
    }
}

/// The rustix calls the unix launcher path makes, and the word the dependency
/// comment has to name each of them by.
///
/// The comment is the fourth statement of the same fact the three prose ones
/// carry, and the one that decides what the crate may reach: a call that is
/// made and not named here is a call nobody agreed to.
const RUSTIX_CALLS: [(&str, &str); 5] = [
    ("fcntl_setfd", "`fcntl`"),
    ("flock", "`flock`"),
    ("getuid", "`getuid`"),
    ("syncfs", "`syncfs`"),
    ("test_kill_process", "`kill(pid, 0)`"),
];

/// The one windows-sys feature that is declared for a signature rather than
/// for a call of the crate's own.
///
/// The job-object entry points take a `SECURITY_ATTRIBUTES` pointer, so the
/// module that declares them does not compile without it, and no `use` line
/// names it.
const WINDOWS_FEATURE_FOR_A_SIGNATURE: &str = "Win32_Security";

/// The text of one `[target.'cfg(<platform>)'.dependencies]` section.
fn dependency_block(platform: &str) -> String {
    let manifest = read("Cargo.toml");
    let header = format!("[target.'cfg({platform})'.dependencies]");
    let start = manifest
        .find(&header)
        .unwrap_or_else(|| panic!("Cargo.toml carries {header}"));
    let rest = &manifest[start + header.len()..];
    let end = rest.find("\n[").unwrap_or(rest.len());
    rest[..end].to_owned()
}

/// The comment lines of a dependency section, as one string.
fn comment_of(block: &str) -> String {
    block
        .lines()
        .filter(|line| line.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The quoted names of a dependency section's `features = [ .. ]` array.
///
/// The array rather than every quoted string in the section: the version is
/// quoted too, and a check that swept it up would be comparing a feature list
/// against a number.
fn declared_features(block: &str) -> Vec<String> {
    let start = block
        .find("features = [")
        .expect("the section declares a feature array");
    let rest = &block[start..];
    let end = rest.find(']').expect("the feature array is closed");
    let mut found: Vec<String> = rest[..end]
        .split('"')
        .skip(1)
        .step_by(2)
        .map(str::to_owned)
        .collect();
    found.sort();
    found
}

/// The identifier at the front of `text`, and what follows it.
fn identifier(text: &str) -> (&str, &str) {
    let end = text
        .find(|character: char| !character.is_ascii_alphanumeric() && character != '_')
        .unwrap_or(text.len());
    text.split_at(end)
}

/// The `::`-separated segments following each occurrence of `prefix` in
/// `text`, with whether the path ended at a `{` group.
///
/// The flag is what tells a module path from an item: `Win32::System::
/// JobObjects::{ .. }` ends at a brace and every segment of it is a module,
/// where `Win32::System::Console::SetConsoleCtrlHandler` ends at the item it
/// imports.
fn segments_after<'a>(text: &'a str, prefix: &str) -> Vec<(Vec<&'a str>, bool)> {
    let mut found = Vec::new();
    for (index, _) in text.match_indices(prefix) {
        let mut rest = &text[index + prefix.len()..];
        let mut path = Vec::new();
        let mut group = false;
        loop {
            let (segment, tail) = identifier(rest);
            if segment.is_empty() {
                break;
            }
            path.push(segment);
            let Some(tail) = tail.strip_prefix("::") else {
                break;
            };
            if tail.trim_start().starts_with('{') {
                group = true;
                break;
            }
            rest = tail;
        }
        found.push((path, group));
    }
    found
}

/// Every rustix function the crate calls, by name.
///
/// A path of exactly two segments whose second begins in lower case: a longer
/// one is an associated function on a type — `rustix::io::FdFlags::empty` —
/// and an upper-case second segment is the type itself.
fn rustix_calls() -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for relative in rust_sources() {
        for (path, _) in segments_after(&read(&relative), "rustix::") {
            let [_, call] = path[..] else { continue };
            if call.starts_with(|character: char| character.is_ascii_lowercase()) {
                found.push(call.to_owned());
            }
        }
    }
    found.sort();
    found.dedup();
    found
}

/// Every windows-sys feature the excepted module's `use` lines need.
///
/// The module path is every segment after `Win32::` but the item being
/// imported; a `use` that imports a group names no item of its own, so every
/// segment of it is a module.
fn windows_features_used() -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for (path, group) in segments_after(&read(EXCEPTED), "windows_sys::Win32::") {
        let modules: &[&str] = if group {
            &path
        } else {
            match path.split_last() {
                Some((_, modules)) => modules,
                None => continue,
            }
        };
        if modules.is_empty() {
            continue;
        }
        found.push(format!("Win32_{}", modules.join("_")));
    }
    found.sort();
    found.dedup();
    found
}

#[test]
fn the_dependency_comments_name_every_call_the_launcher_path_makes() {
    let calls = rustix_calls();
    let named: Vec<String> = RUSTIX_CALLS
        .iter()
        .map(|(call, _)| (*call).to_owned())
        .collect();
    assert_eq!(
        calls, named,
        "this table is the list `Cargo.toml`'s unix comment is checked against, so a rustix \
         call it does not know about is a call the comment was never asked to name"
    );

    let comment = comment_of(&dependency_block("unix"));
    for (call, word) in RUSTIX_CALLS {
        assert!(
            comment.contains(word),
            "`{call}` is reached from the unix launcher path and the dependency comment does \
             not name it as {word}: the comment is what says which system calls this crate \
             may reach, and one it does not list is one nobody agreed to"
        );
    }
}

#[test]
fn the_windows_dependency_declares_what_the_module_uses_and_says_why() {
    let block = dependency_block("windows");
    let declared = declared_features(&block);
    let used = windows_features_used();
    assert!(
        !used.is_empty(),
        "the excepted module imports from windows-sys: {EXCEPTED}"
    );

    for feature in &used {
        assert!(
            declared.contains(feature),
            "{EXCEPTED} imports from `{feature}` and Cargo.toml does not declare it: {declared:?}"
        );
    }

    let comment = comment_of(&block);
    for feature in &declared {
        if used.contains(feature) {
            continue;
        }
        assert_eq!(
            feature, WINDOWS_FEATURE_FOR_A_SIGNATURE,
            "`{feature}` is declared and no `use` line in {EXCEPTED} needs it: a feature that \
             is on for nobody is a wider surface than the comment describes"
        );
        assert!(
            comment.contains(feature),
            "`{feature}` is declared for a signature rather than for a call, and the comment \
             does not say so: {comment}"
        );
    }
}
