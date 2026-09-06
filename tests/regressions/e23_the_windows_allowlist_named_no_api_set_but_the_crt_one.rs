// SPDX-License-Identifier: MIT OR Apache-2.0
//! `ginary verify` reported two system DLLs of every Windows artifact as
//! libraries the artifact expects a stranger's machine to supply.
//!
//! **What went wrong.** [`ginary::verify::WINDOWS_NEEDED_ALLOWLIST`] is the
//! floor a Windows target guarantees, and it admits the Universal CRT's
//! forwarding libraries through the companion rule: an allowlist that names
//! `ucrtbase.dll` admits `api-ms-win-crt-*`. The Universal CRT is not the only
//! API set family. Windows forwards its *base* through `api-ms-win-core-*`,
//! and every program links `ntdll.dll` — the lowest library there is, below
//! `KERNEL32.dll` itself. Neither was on the list, so a PE this toolchain
//! links raised two findings against an artifact with nothing wrong with it:
//!
//! ```text
//! erts-17.0.5/bin/erl.exe: needs `api-ms-win-core-synch-l1-2-0.dll`, which the artifact does not carry
//! erts-17.0.5/bin/erl.exe: needs `ntdll.dll`, which the artifact does not carry
//! ```
//!
//! **The input.** Any real PE. This is E12's defect again — the allowlist
//! carried one Visual C++ runtime of three, and
//! `tests/regressions/e12_the_windows_allowlist_carried_one_vc_runtime_of_three.rs`
//! added the other two — and it stayed invisible for the same reason: no test
//! had ever put a genuine Windows binary inside a payload. E23 does, because
//! the launcher fixture's runtime stub has to be a program Windows will start,
//! and the first artifact built that way reported both names at once.
//!
//! **The correct behaviour.** An API set is not a file a machine may lack. The
//! loader resolves every `api-ms-win-*` name through the apiset schema onto a
//! system DLL, so an allowlist that admits the Win32 base admits the base's
//! API sets, exactly as one that admits `ucrtbase.dll` admits the CRT's. The
//! rule is written the way the CRT rule already is — a prefix and a companion
//! — so an allowlist a test narrows to `&[]` still admits nothing, and
//! `ntdll.dll` joins the list by name.
//!
//! `ext-ms-win-*` is deliberately not admitted. Extension API sets are the
//! half of the mechanism that may legitimately be absent on an edition of
//! Windows, which is the finding this check exists for.
//!
//! `verify` is a `cli` module, so a launcher-only build has nothing here to
//! run — the same gate every other claim about the verifier carries.
#![cfg(feature = "cli")]

use ginary::verify::{
    self, WINDOWS_CORE_COMPANION, WINDOWS_CORE_PREFIX, WINDOWS_CRT_COMPANION, WINDOWS_CRT_PREFIX,
    WINDOWS_NEEDED_ALLOWLIST,
};

/// The two names the first real PE in a payload reported.
const REPORTED: [&str; 2] = ["api-ms-win-core-synch-l1-2-0.dll", "ntdll.dll"];

#[test]
fn the_names_a_real_pe_reported_are_on_the_windows_floor() {
    for name in REPORTED {
        assert!(
            verify::needed_is_allowed(name, &WINDOWS_NEEDED_ALLOWLIST),
            "`{name}` is on every supported Windows and an artifact cannot carry it, so \
             reporting it is telling a user to fix something that is not broken"
        );
    }
}

#[test]
fn the_core_api_set_family_is_admitted_by_its_companion_and_not_alone() {
    let member = format!("{WINDOWS_CORE_PREFIX}synch-l1-2-0.dll");
    for spelling in [member.to_ascii_lowercase(), member.to_ascii_uppercase()] {
        assert!(
            verify::needed_is_allowed(&spelling, &WINDOWS_NEEDED_ALLOWLIST),
            "a PE import table spells one file both ways, so `{spelling}` is the same library"
        );
        assert!(
            !verify::needed_is_allowed(&spelling, &[]),
            "an allowlist a test narrowed to nothing admits nothing, family rules included: \
             `{spelling}`"
        );
        assert!(
            verify::needed_is_allowed(&spelling, &[WINDOWS_CORE_COMPANION]),
            "the companion is what admits the family, the way `ucrtbase.dll` admits the CRT's"
        );
    }
    assert!(
        !verify::needed_is_allowed(&member, &[WINDOWS_CRT_COMPANION]),
        "the two families have two companions: the C runtime does not vouch for the Win32 base"
    );
}

#[test]
fn an_extension_api_set_is_still_reported() {
    // `ext-ms-win-*` is the half of the mechanism an edition of Windows may
    // not have. A program that needs one needs a machine that supplies it,
    // which is exactly what the portability check is for.
    assert!(
        !verify::needed_is_allowed(
            "ext-ms-win-ntuser-window-l1-1-0.dll",
            &WINDOWS_NEEDED_ALLOWLIST
        ),
        "an extension API set may be absent, so an artifact that needs one is a finding"
    );
}

#[test]
fn a_name_that_only_begins_like_a_contract_is_still_reported() {
    // Reviewed on the pull request, and correct: a companion admitted the
    // whole of its prefix, and a prefix is the beginning of a contract name
    // and equally the beginning of any file somebody chooses to call that.
    // The loader's own grammar ends a contract in `l<n>-<n>-<n>`, so a name
    // that does not is a filename wearing the family's clothes — the same
    // "satisfied by something that is not the thing" this milestone is about,
    // in the rule this milestone added.
    for family in [WINDOWS_CRT_PREFIX, WINDOWS_CORE_PREFIX] {
        for tail in ["not-a-contract", "synch", "synch-l1-2", "synch-lx-1-0"] {
            let planted = format!("{family}{tail}.dll");
            assert!(
                !verify::needed_is_allowed(&planted, &WINDOWS_NEEDED_ALLOWLIST),
                "`{planted}` is not spelled the way the loader spells an API set, so the \
                 companion does not vouch for it"
            );
        }
        // And the grammar has to still admit the real thing, or the rule is
        // simply a refusal.
        let real = format!("{family}synch-l1-2-0.dll");
        assert!(
            verify::needed_is_allowed(&real, &WINDOWS_NEEDED_ALLOWLIST),
            "`{real}` is a contract name and its family's companion is on the allowlist"
        );
    }
}
