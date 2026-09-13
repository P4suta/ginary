// SPDX-License-Identifier: MIT OR Apache-2.0
//! Reading the front of a Mach-O required the rest of it, so a large macOS
//! build of ginary could not be the command line tool.
//!
//! **What went wrong.** `main()` decides mode by reading the running
//! executable: no payload means this copy is the build tool. On macOS that
//! question is asked by [`ginary::payload::locate`], which reads at most
//! `MACHO_HEAD_CAP` — sixteen mebibytes — of the file's front, because "a
//! Mach-O header and every one of its load commands sit at the front of the
//! file" and the payload itself can be gigabytes.
//!
//! That premise is true, and the implementation did not keep to it.
//! `locate` handed the capped head to `ginary::macho::read`, which hands it to
//! `object::File::parse`, and `object` validates more than the load commands:
//! it resolves `LC_SYMTAB`, whose `symoff` points into `__LINKEDIT` at the
//! *end* of the file. For any Mach-O larger than the cap the head stops before
//! `__LINKEDIT`, the parse fails, and a file that simply has no payload was
//! reported as a broken artifact:
//!
//! ```console
//! $ ./target/debug/ginary version
//! ginary: the Mach-O `__GINARY,__payload` section could not be read: cannot
//! parse the Mach-O file: Invalid Mach-O symbol table offset or size
//! $ echo $?
//! 122
//! ```
//!
//! A debug build of ginary for `aarch64-apple-darwin` is 32 MB, so on macOS
//! *every* command of the debug binary exited 122 — `version`, `doctor`,
//! `build`, all of them. The release binary is under the cap, which is why the
//! `macos` CI job, which builds `--release`, never saw it. It is also why
//! `tests/smoke_cli.rs` had never run green on a macOS host.
//!
//! `ginary::macho::code_signature` was already written the right way, and says
//! so: "`bytes` may be only the head of a large artifact, which is enough:
//! this walks the command area by hand rather than through `object`, so it
//! never needs a byte past the last load command it reads."
//! `ginary::macho::section` claims the same rationale in its documentation —
//! "the same reason [`section`] is its own function" — and then delegated to
//! `read`. The documentation was right and the implementation was not.
//!
//! **The input.** Any thin Mach-O whose `__LINKEDIT` is not among the bytes
//! handed over. The committed `aarch64-apple-darwin` fixture truncated at its
//! own `__LINKEDIT` offset is that file in 64 KB instead of 32 MB, and
//! produces the identical `object` error for the identical reason.
//!
//! **The correct behaviour.** A question about the load commands is answered
//! from the load commands. `section` walks them by hand exactly as
//! `code_signature` does, `locate` reads the fat/thin distinction off the
//! magic it has already matched, and a Mach-O carrying no ginary payload is
//! `Ok(None)` — this copy is the command line tool — however much of the file
//! the caller chose to read.

use crate::common::macho;
use ginary::macho::{PAYLOAD_SECTION, PAYLOAD_SEGMENT, code_signature, section};
use ginary::payload::locate;
use std::fs::File;
use std::io::Write as _;

/// The file offset `__LINKEDIT` begins at, read out of the segment command.
///
/// `LC_SEGMENT_64` is `cmd`, `cmdsize`, a 16-byte `segname`, `vmaddr` and
/// `vmsize`, so `fileoff` starts 40 bytes into the command.
fn linkedit_offset(bytes: &[u8]) -> usize {
    let command = macho::segment_command_offset(bytes, "__LINKEDIT")
        .expect("the committed fixture carries a __LINKEDIT segment");
    let field = command + 40;
    let raw: [u8; 8] = bytes[field..field + 8]
        .try_into()
        .expect("eight bytes of fileoff");
    usize::try_from(u64::from_le_bytes(raw)).expect("an offset that fits this machine")
}

/// The fixture's bytes, and the same bytes stopping where `__LINKEDIT` does.
///
/// The truncated half is the head `locate` would read of a Mach-O larger than
/// its cap: every load command, and nothing `__LINKEDIT` holds.
fn whole_and_head() -> (Vec<u8>, Vec<u8>) {
    let whole = macho::real_fixture_bytes();
    let head = whole[..linkedit_offset(&whole)].to_vec();
    assert!(
        head.len() < whole.len(),
        "the fixture must have a __LINKEDIT to cut off, or this rule has no subject"
    );
    (whole, head)
}

#[test]
fn a_section_is_found_from_the_load_commands_without_linkedit() {
    let (whole, head) = whole_and_head();
    let expected = section(&whole, "__TEXT", "__text")
        .expect("the committed fixture carries a __TEXT,__text section");

    assert_eq!(
        section(&head, "__TEXT", "__text"),
        Some(expected),
        "a section header is a load command and sits at the front of the file. Answering `None` \
         for a section the load commands plainly describe means the lookup read past them — and \
         `None` is indistinguishable from `this Mach-O carries no such section`, which is how a \
         payload-less binary became a broken artifact"
    );
}

#[test]
fn a_payload_less_macho_without_linkedit_is_the_command_line_tool() {
    let (_, head) = whole_and_head();
    let work = tempfile::tempdir().expect("a temporary directory");
    let path = work.path().join("ginary");
    let mut file = File::create(&path).expect("the truncated fixture is written");
    file.write_all(&head).expect("the head is written");
    drop(file);

    let file = File::open(&path).expect("the truncated fixture is opened");
    let found = locate(&file).unwrap_or_else(|error| {
        panic!(
            "a Mach-O with no `{PAYLOAD_SEGMENT},{PAYLOAD_SECTION}` section carries no payload, \
             and `main()` reads that as `this copy is the build tool`. Refusing it is exit 122 \
             for every command the binary has: {error}"
        )
    });
    assert_eq!(
        found, None,
        "the fixture carries no ginary payload, so there is no location to report"
    );
}

#[test]
fn the_code_signature_lookup_already_answered_from_the_head_and_still_does() {
    let (whole, head) = whole_and_head();
    assert_eq!(
        code_signature(&head),
        code_signature(&whole),
        "`code_signature` walks the load commands by hand and never needed `__LINKEDIT`. It is \
         the control: the fix makes `section` answer the way this already did, and must not \
         change what this one says"
    );
}
