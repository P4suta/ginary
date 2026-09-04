// SPDX-License-Identifier: MIT OR Apache-2.0
//! Finding an OTP installation and judging whether it can be packaged.
//!
//! Almost everything here runs against a [`FakeOtp`] tree in a temporary
//! directory, because the interesting cases are the broken ones and no machine
//! has a broken Erlang installed on purpose. The handful of tests that do reach
//! the host toolchain are gated and say so.
// The command line half of the suite: every claim in this file is about a
// module the `cli` feature carries, so a `--no-default-features` build has
// nothing here to run. See `docs/dev/log/C2.md`.
#![cfg(feature = "cli")]

mod common;

use std::path::Path;

use ginary::otp::{
    self, MIN_RELEASE, OtpError, REQUIRED_ERTS_BINARIES, boot_lib_dirs, discover, inspect_root,
};

use crate::common::fake_otp::{FakeOtp, FakeOtpRoot};
use crate::common::tools::require_tools;

/// Inspects a root or fails the test with the error.
fn inspect(root: &Path) -> otp::OtpInfo {
    match inspect_root(root) {
        Ok(info) => info,
        Err(error) => panic!("{} should be usable, but: {error}", root.display()),
    }
}

/// The error from inspecting a root, which must fail.
fn inspect_failure(root: &Path) -> OtpError {
    match inspect_root(root) {
        Ok(info) => panic!(
            "{} should be rejected, but produced {info:?}",
            root.display()
        ),
        Err(error) => error,
    }
}

/// A default fake root in a temporary directory that the caller keeps alive.
fn fake(dir: &tempfile::TempDir) -> FakeOtpRoot {
    FakeOtp::new().build_in(dir.path())
}

// ------------------------------------------------------------- inspect_root

#[test]
fn inspect_root_reads_every_field_from_the_tree() {
    let dir = tempfile::tempdir().expect("tempdir");
    let otp = FakeOtp::new()
        .erts_vsn("17.0.5")
        .release(29)
        .otp_version("29.0.5")
        .build_in(dir.path());

    let info = inspect(&otp.root);

    assert_eq!(info.root, otp.root);
    assert_eq!(info.release, 29);
    assert_eq!(info.erts_vsn, "17.0.5");
    assert_eq!(info.otp_version, "29.0.5");
    assert_eq!(info.erts_bin, otp.erts_bin());
    assert_eq!(info.lib, otp.lib());
}

#[test]
fn inspect_root_takes_the_release_from_the_second_field_of_start_erl_data() {
    let dir = tempfile::tempdir().expect("tempdir");
    let otp = FakeOtp::new()
        .erts_vsn("16.1.2")
        .release(27)
        .build_in(dir.path());

    // The file is `<erts vsn> <release>`; reading the first field would give 16.
    assert_eq!(
        std::fs::read_to_string(otp.releases().join("start_erl.data")).expect("readable"),
        "16.1.2 27\n"
    );
    assert_eq!(inspect(&otp.root).release, 27);
}

#[test]
fn inspect_root_falls_back_to_the_single_numeric_release_directory() {
    let dir = tempfile::tempdir().expect("tempdir");
    let otp = FakeOtp::new()
        .release(28)
        .without_start_erl_data()
        .build_in(dir.path());
    // A real installation has these next to the numeric directory; neither is
    // all digits, so neither may be mistaken for the release.
    std::fs::write(otp.releases().join("RELEASES"), "[].").expect("writable");
    std::fs::create_dir(otp.releases().join("backup")).expect("writable");

    assert_eq!(inspect(&otp.root).release, 28);
}

#[test]
fn inspect_root_cannot_guess_the_release_from_two_numeric_directories() {
    let dir = tempfile::tempdir().expect("tempdir");
    let otp = FakeOtp::new()
        .release(28)
        .without_start_erl_data()
        .build_in(dir.path());
    std::fs::create_dir(otp.releases().join("27")).expect("writable");

    let error = inspect_failure(&otp.root);
    assert!(
        matches!(error, OtpError::UnknownRelease { .. }),
        "{error:?}"
    );
}

#[test]
fn inspect_root_falls_back_to_the_release_string_without_an_otp_version_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let otp = FakeOtp::new()
        .release(29)
        .without_otp_version()
        .build_in(dir.path());

    assert_eq!(inspect(&otp.root).otp_version, "29");
}

#[test]
fn inspect_root_trims_the_otp_version_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let otp = fake(&dir);
    std::fs::write(otp.release_dir().join("OTP_VERSION"), "  29.0.5 \n\n").expect("writable");

    assert_eq!(inspect(&otp.root).otp_version, "29.0.5");
}

#[test]
fn inspect_root_rejects_a_root_without_an_erts_directory() {
    let dir = tempfile::tempdir().expect("tempdir");
    let otp = fake(&dir);
    std::fs::remove_dir_all(otp.erts_bin().parent().expect("erts dir")).expect("removable");

    let error = inspect_failure(&otp.root);
    assert!(matches!(error, OtpError::NoErts { .. }), "{error:?}");
    assert!(
        error.to_string().contains("erts-*"),
        "the message must name what is missing: {error}"
    );
}

/// A mistyped override path is the likeliest way to reach `inspect_root` with
/// something that is not a root, and "has no `erts-*` directory" would be the
/// wrong thing to say about a directory that is not there at all.
#[test]
fn inspect_root_says_when_the_root_does_not_exist() {
    let dir = tempfile::tempdir().expect("tempdir");
    let missing = dir.path().join("no-such-otp");

    let error = inspect_failure(&missing);

    assert!(matches!(error, OtpError::NoSuchRoot { .. }), "{error:?}");
    assert_eq!(
        error.to_string(),
        format!(
            "`{}` is not a directory, so it cannot be an OTP installation",
            missing.display()
        )
    );
}

#[test]
fn inspect_root_says_when_the_root_is_a_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("erlang.tar.gz");
    std::fs::write(&file, b"not a directory").expect("writes the file");

    let error = inspect_failure(&file);

    assert!(matches!(error, OtpError::NoSuchRoot { .. }), "{error:?}");
}

#[test]
fn inspect_root_rejects_two_erts_directories() {
    let dir = tempfile::tempdir().expect("tempdir");
    let otp = fake(&dir);
    std::fs::create_dir(otp.root.join("erts-16.0.1")).expect("writable");

    let error = inspect_failure(&otp.root);
    let OtpError::AmbiguousErts { found, .. } = &error else {
        panic!("expected AmbiguousErts, got {error:?}");
    };
    assert_eq!(found, &["erts-16.0.1".to_owned(), "erts-17.0.5".to_owned()]);
}

#[test]
fn inspect_root_names_whichever_erts_binary_is_missing() {
    for missing in REQUIRED_ERTS_BINARIES {
        let dir = tempfile::tempdir().expect("tempdir");
        let otp = fake(&dir);
        let path = otp.erts_bin().join(missing);
        std::fs::remove_file(&path).expect("removable");

        let error = inspect_failure(&otp.root);
        let OtpError::MissingErtsBinary { path: reported } = &error else {
            panic!("expected MissingErtsBinary for {missing}, got {error:?}");
        };
        assert_eq!(reported, &path);
    }
}

#[cfg(unix)]
#[test]
fn inspect_root_rejects_an_erts_binary_that_is_not_executable() {
    let dir = tempfile::tempdir().expect("tempdir");
    let otp = fake(&dir);
    let path = otp.erts_bin().join("beam.smp");
    crate::common::fake_otp::make_non_executable(&path);

    let error = inspect_failure(&otp.root);
    let OtpError::ErtsBinaryNotExecutable { path: reported } = &error else {
        panic!("expected ErtsBinaryNotExecutable, got {error:?}");
    };
    assert_eq!(reported, &path);
    assert!(
        error.to_string().contains("chmod"),
        "the message must say what to do: {error}"
    );
}

#[test]
fn inspect_root_rejects_a_missing_boot_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let otp = fake(&dir);
    std::fs::remove_file(otp.boot_file()).expect("removable");

    let error = inspect_failure(&otp.root);
    let OtpError::MissingBootFile { path } = &error else {
        panic!("expected MissingBootFile, got {error:?}");
    };
    assert_eq!(path, &otp.boot_file());
}

#[test]
fn inspect_root_rejects_a_root_without_kernel() {
    let dir = tempfile::tempdir().expect("tempdir");
    let otp = fake(&dir);
    std::fs::remove_dir_all(otp.app_dir("kernel")).expect("removable");

    let error = inspect_failure(&otp.root);
    assert!(
        matches!(&error, OtpError::MissingLibApp { name, .. } if *name == "kernel"),
        "{error:?}"
    );
}

#[test]
fn inspect_root_rejects_a_root_without_stdlib() {
    let dir = tempfile::tempdir().expect("tempdir");
    let otp = fake(&dir);
    std::fs::remove_dir_all(otp.app_dir("stdlib")).expect("removable");

    let error = inspect_failure(&otp.root);
    assert!(
        matches!(&error, OtpError::MissingLibApp { name, .. } if *name == "stdlib"),
        "{error:?}"
    );
}

/// `kernel-doc` sits next to `kernel-11.0.3` in a documentation install and is
/// not a second copy of `kernel`. A glob of `kernel-*` would see two.
#[test]
fn inspect_root_ignores_a_lib_directory_without_a_numeric_version() {
    let dir = tempfile::tempdir().expect("tempdir");
    let otp = fake(&dir);
    for decoy in ["kernel-doc", "stdlib-doc", "kernel-", "kernel-1a.2"] {
        std::fs::create_dir(otp.lib().join(decoy)).expect("writable");
    }

    let info = inspect(&otp.root);
    assert_eq!(info.release, 29);
}

#[test]
fn inspect_root_rejects_two_versions_of_the_same_library() {
    let dir = tempfile::tempdir().expect("tempdir");
    let otp = fake(&dir);
    std::fs::create_dir(otp.lib().join("stdlib-8.0.4")).expect("writable");

    let error = inspect_failure(&otp.root);
    let OtpError::AmbiguousLibApp { name, found, .. } = &error else {
        panic!("expected AmbiguousLibApp, got {error:?}");
    };
    assert_eq!(*name, "stdlib");
    assert_eq!(
        found,
        &["stdlib-8.0.3".to_owned(), "stdlib-8.0.4".to_owned()]
    );
}

#[test]
fn inspect_root_rejects_a_release_older_than_the_minimum() {
    let dir = tempfile::tempdir().expect("tempdir");
    let otp = FakeOtp::new().release(MIN_RELEASE - 1).build_in(dir.path());

    let error = inspect_failure(&otp.root);
    assert!(
        matches!(
            error,
            OtpError::ReleaseTooOld { release, minimum } if release == MIN_RELEASE - 1 && minimum == MIN_RELEASE
        ),
        "{error:?}"
    );
}

#[test]
fn inspect_root_accepts_the_oldest_supported_release() {
    let dir = tempfile::tempdir().expect("tempdir");
    let otp = FakeOtp::new().release(MIN_RELEASE).build_in(dir.path());

    assert_eq!(inspect(&otp.root).release, MIN_RELEASE);
    assert_eq!(MIN_RELEASE, 26, "the documented floor is OTP 26");
}

// -------------------------------------------------------------- boot_lib_dirs

#[test]
fn boot_lib_dirs_lists_each_library_once_in_order_of_appearance() {
    let boot = b"\x83\x68\x03noise$ROOT/lib/kernel-11.0.3/ebin\x00\
        more$ROOT/lib/stdlib-8.0.3/ebin\x00again$ROOT/lib/kernel-11.0.3/ebin";

    assert_eq!(
        boot_lib_dirs(boot),
        vec!["kernel-11.0.3".to_owned(), "stdlib-8.0.3".to_owned()]
    );
}

#[test]
fn boot_lib_dirs_ignores_anything_that_is_not_a_versioned_ebin_path() {
    let boot = b"$ROOT/lib/kernel-11.0.3/priv \
        $ROOT/lib/noversion/ebin \
        $ROOT/bin/erlexec \
        /usr/lib/erlang/lib/kernel-11.0.3/ebin \
        $ROOT/lib/ssl-11.7.4/ebin";

    assert_eq!(boot_lib_dirs(boot), vec!["ssl-11.7.4".to_owned()]);
}

#[test]
fn boot_lib_dirs_finds_nothing_in_bytes_that_hold_no_paths() {
    assert!(boot_lib_dirs(b"").is_empty());
    assert!(boot_lib_dirs(&[0_u8; 512]).is_empty());
}

#[test]
fn boot_lib_dirs_reads_the_boot_file_a_fake_root_writes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let otp = fake(&dir);

    assert_eq!(
        boot_lib_dirs(&otp.boot_bytes()),
        vec!["kernel-11.0.3".to_owned(), "stdlib-8.0.3".to_owned()]
    );
}

/// The real file is `term_to_binary` output, not the fake's hand-laid bytes.
#[test]
fn boot_lib_dirs_reads_the_real_no_dot_erlang_boot() {
    let Some(_tools) = require_tools(&["erl"]) else {
        return;
    };
    let info = match discover(None) {
        Ok(info) => info,
        Err(error) => panic!("`erl` is on PATH but discovery failed: {error}"),
    };
    let boot = info.root.join("bin").join("no_dot_erlang.boot");
    let bytes = std::fs::read(&boot).expect("the boot file is readable");

    let dirs = boot_lib_dirs(&bytes);

    assert!(
        dirs.iter().any(|dir| dir.starts_with("kernel-")),
        "no kernel in {dirs:?}"
    );
    assert!(
        dirs.iter().any(|dir| dir.starts_with("stdlib-")),
        "no stdlib in {dirs:?}"
    );
    for dir in &dirs {
        assert!(
            info.lib.join(dir).is_dir(),
            "{dir} is named by {} but is not under {}",
            boot.display(),
            info.lib.display()
        );
    }
}

// ------------------------------------------------------------------ discover

#[test]
fn discover_with_an_override_inspects_that_root() {
    let dir = tempfile::tempdir().expect("tempdir");
    let otp = FakeOtp::new()
        .erts_vsn("15.2.1")
        .release(27)
        .otp_version("27.3.4")
        .build_in(dir.path());

    let info = match discover(Some(&otp.root)) {
        Ok(info) => info,
        Err(error) => panic!("the override root should be accepted: {error}"),
    };

    // Nothing here could have come from the machine's own Erlang.
    assert_eq!(info.root, otp.root);
    assert_eq!(info.erts_vsn, "15.2.1");
    assert_eq!(info.otp_version, "27.3.4");
    assert_eq!(info.release, 27);
}

#[test]
fn discover_reports_an_override_that_is_not_an_otp_installation() {
    let dir = tempfile::tempdir().expect("tempdir");

    let error = match discover(Some(dir.path())) {
        Ok(info) => panic!("an empty directory is not OTP, got {info:?}"),
        Err(error) => error,
    };

    assert!(matches!(error, OtpError::NoErts { .. }), "{error:?}");
}

#[test]
fn discover_reports_an_override_that_is_not_there() {
    let dir = tempfile::tempdir().expect("tempdir");
    let missing = dir.path().join("typo");

    let error = match discover(Some(&missing)) {
        Ok(info) => panic!("a path that does not exist is not OTP, got {info:?}"),
        Err(error) => error,
    };

    assert!(matches!(error, OtpError::NoSuchRoot { .. }), "{error:?}");
}

#[test]
fn discover_finds_the_erl_on_the_path() {
    let Some(_tools) = require_tools(&["erl"]) else {
        return;
    };

    let info = match discover(None) {
        Ok(info) => info,
        Err(error) => panic!("`erl` is on PATH but discovery failed: {error}"),
    };

    assert!(
        info.root.is_dir(),
        "{} is not a directory",
        info.root.display()
    );
    assert!(info.release >= MIN_RELEASE, "release {}", info.release);
    assert!(
        info.otp_version.starts_with(&info.release.to_string()),
        "otp_version {} does not start with release {}",
        info.otp_version,
        info.release
    );
    assert_eq!(info.lib, info.root.join("lib"));
    assert_eq!(
        info.erts_bin,
        info.root
            .join(format!("erts-{}", info.erts_vsn))
            .join("bin")
    );
    // Which programs a runtime tree must hold is a property of the *tree*, not
    // of the machine asking: `otp::check_erts_binaries` reads the flavour off
    // the directory with `assemble::is_windows_erts_bin` and measures a
    // Windows tree against `assemble::WINDOWS_REQUIRED_BINS` instead. This
    // assertion asked the unix question of every host and so failed on a
    // Windows runner against a healthy installation — `beam.smp is missing
    // from d:/a/_temp/.setup-beam/otp\erts-17.0.5\bin` — so it now asks the
    // same question the product does. On Linux the list is unchanged.
    let required: Vec<&str> = if ginary::assemble::is_windows_erts_bin(&info.erts_bin) {
        ginary::assemble::WINDOWS_REQUIRED_BINS.to_vec()
    } else {
        REQUIRED_ERTS_BINARIES.to_vec()
    };
    for name in required {
        assert!(
            info.erts_bin.join(name).is_file(),
            "{name} is missing from {}",
            info.erts_bin.display()
        );
    }
}

/// The `erl` program `discover` runs must actually print the three fields.
///
/// Running the constant rather than a copy of it is the point: a typo in
/// `DISCOVER_EVAL` would otherwise only show up as a discovery failure with no
/// explanation.
#[test]
fn the_discover_program_prints_root_release_and_erts_version() {
    let Some(tools) = require_tools(&["erl"]) else {
        return;
    };

    let output = std::process::Command::new(tools.path("erl"))
        .args(["-noshell", "-eval", otp::DISCOVER_EVAL])
        .output()
        .expect("erl runs");

    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).expect("utf-8");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 3, "{stdout:?}");
    assert!(Path::new(lines[0]).is_dir(), "code root {:?}", lines[0]);
    assert!(
        lines[1]
            .parse::<u32>()
            .is_ok_and(|release| release >= MIN_RELEASE),
        "release {:?}",
        lines[1]
    );
    assert!(
        lines[2].split('.').count() >= 2,
        "erts version {:?}",
        lines[2]
    );
}
