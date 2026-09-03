// SPDX-License-Identifier: MIT OR Apache-2.0
//! Four snapshot tests pinned the separator the host writes paths with, so a
//! green tree went red on Windows without any behaviour changing.
//!
//! **What went wrong.** `appfile`, `closure` (twice) and `inspect` failed on
//! the Windows runner with diffs that are entirely about spelling:
//!
//! ```text
//! -  searched: <tmp>/shipment/crypto/ebin/crypto.app
//! +  searched: <tmp>\shipment\crypto\ebin\crypto.app
//!
//! -malformed.app: line 5, column 3: expected `,` or `}`, found `{`
//! +D:\a\ginary\ginary\tests/fixtures/app\malformed.app: line 5, column 3: ...
//! ```
//!
//! (`Windows build and exit-code propagation`
//! <https://github.com/P4suta/ginary/actions/runs/33739517757/job/100597889388>.)
//!
//! **The input.** Any snapshot of a message that names a path. The scrubber
//! replaces the temporary directory and leaves the separators inside the
//! remainder alone; and the appfile test strips its fixture directory by
//! gluing a `/` onto it, which matches nothing when `Path::join` used `\`.
//! Note the mixed spelling in the second diff: `D:\a\ginary\ginary` was joined
//! with `\` and `tests/fixtures/app` was written with `/`, so neither
//! separator alone describes the string.
//!
//! **The correct behaviour.** Both rules are decided by the suite rather than
//! by the machine under it, so the snapshot pins the sentence and the shape of
//! the path, which is what it is for.

use std::path::Path;

use crate::common::hostpath::{slashed, strip_dir};

#[test]
fn a_path_snapshot_is_respelled_with_forward_slashes_whoever_wrote_it() {
    assert_eq!(
        slashed(r"  searched: <tmp>\shipment\crypto\ebin\crypto.app"),
        "  searched: <tmp>/shipment/crypto/ebin/crypto.app"
    );
    assert_eq!(
        slashed(
            "`crypto` is in both trees; using the shipment copy at `<tmp>\\shipment\\crypto\\ebin` and ignoring the OTP copy at `<tmp>\\otp\\lib\\crypto-5.9.2\\ebin`"
        ),
        "`crypto` is in both trees; using the shipment copy at `<tmp>/shipment/crypto/ebin` and ignoring the OTP copy at `<tmp>/otp/lib/crypto-5.9.2/ebin`"
    );
    assert_eq!(
        slashed("program: <cache>/<app>/<key>\\erts-17.0.5/bin\\erlexec"),
        "program: <cache>/<app>/<key>/erts-17.0.5/bin/erlexec",
        "a path a Windows host joined onto a `/`-spelled one carries both separators"
    );
    assert_eq!(
        slashed("  searched: <tmp>/shipment/crypto/ebin/crypto.app"),
        "  searched: <tmp>/shipment/crypto/ebin/crypto.app",
        "and a path that is already spelled that way is untouched"
    );
}

#[test]
fn the_fixture_directory_is_stripped_whichever_separator_joined_it() {
    let windows = Path::new(r"D:\a\ginary\ginary\tests/fixtures/app");
    assert_eq!(
        strip_dir(
            r"D:\a\ginary\ginary\tests/fixtures/app\malformed.app: line 5, column 3: expected `,` or `}`, found `{`",
            windows,
        ),
        "malformed.app: line 5, column 3: expected `,` or `}`, found `{`"
    );

    let unix = Path::new("/opt/build/ginary/tests/fixtures/app");
    assert_eq!(
        strip_dir(
            "/opt/build/ginary/tests/fixtures/app/malformed.app: line 5, column 3: expected `,` or `}`, found `{`",
            unix,
        ),
        "malformed.app: line 5, column 3: expected `,` or `}`, found `{`",
        "the rule is the same one on the host that already passed"
    );
    assert_eq!(
        strip_dir("nothing here names the fixture directory", unix),
        "nothing here names the fixture directory"
    );
}
