// SPDX-License-Identifier: MIT OR Apache-2.0
//! Where the bundled runtime comes from, and what it turns out to be.
//!
//! The module is the build's single trust anchor: whatever a configuration
//! claimed, the emulator itself is read and the target, the linkage and the
//! minimum glibc come out of that file. Two halves are tested here.
//!
//! The **grammar** is text: the five spellings a `[tools.ginary.target.<name>]
//! erts` can take, and the four ways one can be wrong.
//!
//! The **resolution** needs a runtime tree, and a machine with no cross
//! toolchain has no aarch64 `beam.smp` to point it at. So the ELF inspection
//! is a parameter: `resolve_with` takes the function that reads the emulator,
//! and a `FakeOtp` root plus a hand-written [`ElfFacts`] is a whole musl
//! runtime as far as the plumbing above the reader is concerned. The two
//! gated tests at the end run the real reader over the host's own `beam.smp`,
//! which is the half a fake cannot cover.
// The command line half of the suite: every claim in this file is about a
// module the `cli` feature carries, so a `--no-default-features` build has
// nothing here to run. See `docs/dev/log/C2.md`.
#![cfg(feature = "cli")]

mod common;

use std::path::{Path, PathBuf};

use ginary::elf::ElfError;
use ginary::erts_source::{
    self, ElfFacts, ErtsError, ErtsSourceSpec, ResolvedErts, SpecError, emulator_path,
};
use ginary::manifest::{LibcRequirement, OtpProvenance};
use ginary::target::{Linkage, Target};

use crate::common::fake_otp::FakeOtp;
use crate::common::tools::require_tools;

/// A temporary directory the test owns.
fn tempdir() -> tempfile::TempDir {
    tempfile::tempdir().expect("a temporary directory")
}

/// The facts a glibc x86-64 emulator reads back as.
fn gnu_facts() -> ElfFacts {
    ElfFacts {
        machine: "x86_64".to_owned(),
        interp: Some("/lib64/ld-linux-x86-64.so.2".to_owned()),
        needed: vec!["libc.so.6".to_owned(), "libm.so.6".to_owned()],
        glibc_max: Some("2.34".to_owned()),
    }
}

/// The facts a dynamically linked musl aarch64 emulator reads back as.
fn musl_facts() -> ElfFacts {
    ElfFacts {
        machine: "aarch64".to_owned(),
        interp: Some("/lib/ld-musl-aarch64.so.1".to_owned()),
        needed: vec!["libc.musl-aarch64.so.1".to_owned()],
        glibc_max: None,
    }
}

/// The facts a fully static emulator reads back as: no interpreter, nothing
/// needed, and therefore nothing to `dlopen` a NIF with.
///
/// It carries a `glibc_max` all the same, because a statically linked glibc
/// build really does hold versioned symbols in its own symbol table. The
/// number is what a runtime was *built* against and not a floor anything
/// resolves at load time, so the fixture is what makes the suppression rule
/// below testable rather than assumed.
fn static_facts(machine: &str) -> ElfFacts {
    ElfFacts {
        machine: machine.to_owned(),
        interp: None,
        needed: Vec::new(),
        glibc_max: Some("2.34".to_owned()),
    }
}

/// Resolves `spec` for `requested`, with every emulator reading back as
/// `facts`.
fn resolve_facts(
    spec: &ErtsSourceSpec,
    requested: &Target,
    facts: ElfFacts,
) -> Result<ResolvedErts, ErtsError> {
    erts_source::resolve_with(spec, requested, |path| {
        assert!(
            path.ends_with("beam.smp"),
            "the emulator is the file that is read, not {}",
            path.display()
        );
        Ok(facts.clone())
    })
}

/// The named target.
fn target(name: &str) -> Target {
    name.parse()
        .unwrap_or_else(|error| panic!("`{name}` must be a target: {error}"))
}

// ------------------------------------------------------- the grammar --

#[test]
fn the_two_available_sources_are_spelled_as_bare_words() {
    assert_eq!("host".parse::<ErtsSourceSpec>(), Ok(ErtsSourceSpec::Host));
    assert_eq!(
        "catalog".parse::<ErtsSourceSpec>(),
        Ok(ErtsSourceSpec::Catalog)
    );
}

#[test]
fn a_dir_source_keeps_the_path_exactly_as_it_was_written() {
    assert_eq!(
        "dir:/opt/otp-29-musl".parse::<ErtsSourceSpec>(),
        Ok(ErtsSourceSpec::Dir(PathBuf::from("/opt/otp-29-musl")))
    );
    // A relative path is a path: what it resolves against is the caller's
    // question, and this parser does not answer it.
    assert_eq!(
        "dir:vendor/otp".parse::<ErtsSourceSpec>(),
        Ok(ErtsSourceSpec::Dir(PathBuf::from("vendor/otp")))
    );
}

#[test]
fn a_tarball_source_keeps_its_path_too() {
    assert_eq!(
        "tarball:/srv/otp-29.0.5-linux-aarch64-musl.tar.zst".parse::<ErtsSourceSpec>(),
        Ok(ErtsSourceSpec::Tarball(PathBuf::from(
            "/srv/otp-29.0.5-linux-aarch64-musl.tar.zst"
        )))
    );
}

#[test]
fn a_docker_source_keeps_the_whole_image_reference() {
    // The tag holds a colon of its own, so only the first one is the prefix.
    assert_eq!(
        "docker:erlang:29-alpine".parse::<ErtsSourceSpec>(),
        Ok(ErtsSourceSpec::Docker("erlang:29-alpine".to_owned()))
    );
}

#[test]
fn a_source_that_is_none_of_the_five_names_all_five() {
    let error = "system"
        .parse::<ErtsSourceSpec>()
        .expect_err("not a source");

    assert_eq!(
        error,
        SpecError::Unknown {
            value: "system".to_owned()
        }
    );
    assert_eq!(
        error.to_string(),
        "expected `host`, `catalog`, `dir:PATH`, `tarball:PATH` or `docker:IMAGE`, not `system`"
    );
}

#[test]
fn a_path_prefix_with_nothing_after_it_is_refused() {
    assert_eq!(
        "dir:".parse::<ErtsSourceSpec>(),
        Err(SpecError::EmptyPath { prefix: "dir" })
    );
    assert_eq!(
        "tarball:".parse::<ErtsSourceSpec>(),
        Err(SpecError::EmptyPath { prefix: "tarball" })
    );
}

#[test]
fn a_docker_source_with_no_image_is_refused() {
    assert_eq!(
        "docker:".parse::<ErtsSourceSpec>(),
        Err(SpecError::EmptyImage)
    );
}

#[test]
fn the_label_of_a_source_is_the_spelling_it_was_written_as() {
    // The label is what a manifest records and what `doctor` prints, so it is
    // the input string and not a debug rendering of the variant.
    assert_eq!(ErtsSourceSpec::Host.label(), "host");
    assert_eq!(ErtsSourceSpec::Catalog.label(), "catalog");
    assert_eq!(
        ErtsSourceSpec::Dir(PathBuf::from("/opt/otp")).label(),
        "dir:/opt/otp"
    );
    assert_eq!(
        ErtsSourceSpec::Docker("erlang:29-alpine".to_owned()).label(),
        "docker:erlang:29-alpine"
    );
}

// ------------------------- the two that need a context, and the one that --
// ---------------------------------------------------------- is not here --

#[test]
fn a_tarball_source_resolved_without_a_context_says_it_needs_one() {
    let dir = tempdir();
    let spec = ErtsSourceSpec::Tarball(dir.path().join("otp.tar.zst"));

    let error = resolve_facts(&spec, &Target::host(), gnu_facts())
        .expect_err("a tarball fills a cache, and this entry point has no cache root");

    assert!(
        matches!(&error, ErtsError::NeedsContext { spec: written } if written == &spec.label()),
        "{error:?}"
    );
    assert!(
        error.to_string().contains("needs a cache root"),
        "the sentence says what is missing rather than naming a milestone: {error}"
    );
    assert!(
        spec.milestone().is_none(),
        "and the source is not one that is still to come"
    );
}

#[test]
fn a_catalog_source_resolved_without_a_context_says_it_needs_one() {
    let error = resolve_facts(&ErtsSourceSpec::Catalog, &Target::host(), gnu_facts())
        .expect_err("the catalogue needs a catalogue, which this entry point has not got");

    assert!(
        matches!(&error, ErtsError::NeedsContext { spec } if spec == "catalog"),
        "{error:?}"
    );
    assert!(
        !error.to_string().contains("milestone"),
        "a source that shipped does not arrive with a milestone: {error}"
    );
}

#[test]
fn a_docker_source_is_refused_and_names_the_milestone_it_arrives_with() {
    let spec = ErtsSourceSpec::Docker("erlang:29-alpine".to_owned());

    let error =
        resolve_facts(&spec, &Target::host(), gnu_facts()).expect_err("docker is not here yet");

    assert!(
        matches!(
            &error,
            ErtsError::NotYetAvailable { spec: written, milestone }
                if written == "docker:erlang:29-alpine" && *milestone == "container image"
        ),
        "one milestone, and it is the one `milestone()` reports: {error:?}"
    );
    assert_eq!(
        spec.milestone(),
        Some("container image"),
        "which is what `ginary doctor` prints for the same value"
    );
    let sentence = error.to_string();
    assert!(
        sentence.contains("arrives with the container image milestone"),
        "the rendered sentence names it: {sentence}"
    );
    assert!(
        sentence.contains("`tarball:PATH` and `catalog` are available today"),
        "and does not say the two sources C3 shipped are still to come: {sentence}"
    );
}

// ----------------------------------------------- a directory, inspected --

#[test]
fn a_directory_source_reads_the_emulator_and_reports_what_it_found() {
    let dir = tempdir();
    let root = dir.path().join("otp");
    FakeOtp::new().build_in(&root);

    let resolved = resolve_facts(
        &ErtsSourceSpec::Dir(root.clone()),
        &target("linux-x86_64-gnu"),
        gnu_facts(),
    )
    .expect("a whole runtime root resolves");

    assert_eq!(resolved.target, target("linux-x86_64-gnu"));
    assert_eq!(resolved.linkage, Linkage::Dynamic);
    assert_eq!(resolved.libc_min.as_deref(), Some("2.34"));
    assert!(resolved.nif_loading, "a dynamic runtime loads NIFs");
    assert_eq!(resolved.otp.root, root);
    assert_eq!(
        resolved.provenance,
        format!("dir:{}", root.display()),
        "the provenance is the spelling, and the spelling names the directory"
    );
}

#[test]
fn the_provenance_block_is_the_one_the_manifest_records() {
    let dir = tempdir();
    let root = dir.path().join("otp");
    FakeOtp::new().build_in(&root);

    let resolved = resolve_facts(
        &ErtsSourceSpec::Dir(root.clone()),
        &target("linux-x86_64-gnu"),
        gnu_facts(),
    )
    .expect("a whole runtime root resolves");

    assert_eq!(
        resolved.provenance_block(),
        OtpProvenance {
            linkage: "dynamic".to_owned(),
            libc: Some(LibcRequirement {
                kind: "gnu".to_owned(),
                min: Some("2.34".to_owned()),
            }),
            nif_loading: true,
            source: format!("dir:{}", root.display()),
        }
    );
}

#[test]
fn a_musl_runtime_resolves_to_a_musl_target() {
    let dir = tempdir();
    let root = dir.path().join("otp");
    FakeOtp::new().build_in(&root);

    let resolved = resolve_facts(
        &ErtsSourceSpec::Dir(root),
        &target("linux-aarch64-musl"),
        musl_facts(),
    )
    .expect("a musl runtime resolves for a musl build");

    assert_eq!(resolved.target, target("linux-aarch64-musl"));
    assert_eq!(resolved.linkage, Linkage::Dynamic);
    assert_eq!(
        resolved.libc_min, None,
        "musl carries no symbol versions to derive a minimum from"
    );
    assert_eq!(resolved.libc_kind(), Some("musl"));
}

#[test]
fn a_static_runtime_is_reported_as_the_target_that_asked_for_it_and_loads_no_nifs() {
    let dir = tempdir();
    let root = dir.path().join("otp");
    FakeOtp::new().build_in(&root);

    let resolved = resolve_facts(
        &ErtsSourceSpec::Dir(root),
        &target("linux-aarch64-musl"),
        static_facts("aarch64"),
    )
    .expect("a static runtime resolves for the target that asked for it");

    assert_eq!(resolved.target, target("linux-aarch64-musl"));
    assert_eq!(resolved.linkage, Linkage::Static);
    assert!(
        !resolved.nif_loading,
        "a statically linked emulator has no dynamic loader to call"
    );
    assert_eq!(
        resolved.libc_min, None,
        "the emulator carries symbol versions and resolves none at load time, so there is no \
         minimum to promise"
    );
}

#[test]
fn an_emulator_that_names_libraries_and_no_loader_is_dynamic_after_all() {
    // The corroboration arm: a file with no `PT_INTERP` whose `DT_NEEDED` set
    // names shared libraries is not a static runtime, whatever its program
    // headers left out, and reporting it as one would write
    // `nif_loading: false` into the manifest of a runtime that loads them.
    let dir = tempdir();
    let root = dir.path().join("otp");
    FakeOtp::new().build_in(&root);

    let resolved = resolve_facts(
        &ErtsSourceSpec::Dir(root),
        &target("linux-x86_64-gnu"),
        ElfFacts {
            machine: "x86_64".to_owned(),
            interp: None,
            needed: vec!["libc.so.6".to_owned()],
            glibc_max: Some("2.34".to_owned()),
        },
    )
    .expect("an emulator for this machine resolves");

    assert_eq!(
        resolved.linkage,
        Linkage::Dynamic,
        "a file that names shared libraries resolves them at load time"
    );
    assert!(
        resolved.nif_loading,
        "and a runtime that has a loader can be given a NIF"
    );
    assert_eq!(
        resolved.libc_min, None,
        "no interpreter named the C library, so the target is the one that asked and its \
         version floor is not a fact anything read"
    );
}

#[test]
fn a_glibc_minimum_is_recorded_only_for_a_dynamic_gnu_runtime() {
    // Every case reads back a `glibc_max`, so the answer below is the rule's
    // and not the fixture's: only a runtime whose own interpreter named glibc,
    // and which resolves it at load time, has a floor to promise.
    let dir = tempdir();
    let root = dir.path().join("otp");
    FakeOtp::new().build_in(&root);
    let spec = ErtsSourceSpec::Dir(root);

    let gnu = resolve_facts(&spec, &target("linux-x86_64-gnu"), gnu_facts())
        .expect("a gnu runtime resolves for a gnu build");
    let musl = resolve_facts(
        &spec,
        &target("linux-aarch64-musl"),
        ElfFacts {
            glibc_max: Some("2.34".to_owned()),
            ..musl_facts()
        },
    )
    .expect("a musl runtime resolves for a musl build");
    let stat = resolve_facts(&spec, &target("linux-x86_64-gnu"), static_facts("x86_64"))
        .expect("a static runtime resolves for the target that asked for it");

    assert_eq!(gnu.libc_min.as_deref(), Some("2.34"));
    assert_eq!(
        musl.libc_min, None,
        "a musl runtime resolves no glibc, so a glibc version is not its minimum"
    );
    assert_eq!(
        stat.libc_min, None,
        "a static runtime resolves nothing at load time, so it promises no floor"
    );
}

#[test]
fn a_runtime_for_another_target_names_both_targets_and_the_flag_that_fixes_it() {
    let dir = tempdir();
    let root = dir.path().join("otp");
    FakeOtp::new().build_in(&root);

    let error = resolve_facts(
        &ErtsSourceSpec::Dir(root.clone()),
        &target("linux-aarch64-musl"),
        gnu_facts(),
    )
    .expect_err("an x86-64 glibc runtime is not an aarch64 musl one");

    assert!(
        matches!(
            &error,
            ErtsError::TargetMismatch { requested, actual, .. }
                if *requested == target("linux-aarch64-musl")
                    && *actual == target("linux-x86_64-gnu")
        ),
        "{error:?}"
    );
    let sentence = error.to_string();
    assert!(
        sentence.contains("linux-aarch64-musl") && sentence.contains("linux-x86_64-gnu"),
        "both targets have to be in the sentence: {sentence}"
    );
    assert!(
        sentence.contains("--target linux-x86_64-gnu"),
        "the message says which flag would make it right: {sentence}"
    );
}

#[test]
fn an_emulator_of_a_machine_ginary_has_no_target_for_is_refused() {
    let dir = tempdir();
    let root = dir.path().join("otp");
    FakeOtp::new().build_in(&root);

    let error = resolve_facts(
        &ErtsSourceSpec::Dir(root),
        &Target::host(),
        ElfFacts {
            machine: "riscv64".to_owned(),
            interp: Some("/lib/ld-linux-riscv64-lp64d.so.1".to_owned()),
            needed: vec!["libc.so.6".to_owned()],
            glibc_max: Some("2.35".to_owned()),
        },
    )
    .expect_err("riscv64 is not a target ginary packages for");

    assert!(
        matches!(
            &error,
            ErtsError::UnknownRuntimeTarget { machine, .. } if machine == "riscv64"
        ),
        "{error:?}"
    );
}

#[test]
fn an_emulator_that_is_not_an_elf_says_a_real_cross_tree_is_required() {
    // A `FakeOtp` writes shell scripts where the ERTS binaries belong, which
    // is exactly the shape of the mistake this error exists for: a runtime
    // tree assembled by hand, or a placeholder committed to a repository.
    let dir = tempdir();
    let root = dir.path().join("otp");
    FakeOtp::new().build_in(&root);

    let error = erts_source::resolve(&ErtsSourceSpec::Dir(root.clone()), &Target::host())
        .expect_err("a shell script is not an emulator");

    assert!(
        matches!(&error, ErtsError::NotAnElfRuntime { path, .. } if path.ends_with("beam.smp")),
        "{error:?}"
    );
    let sentence = error.to_string();
    assert!(
        sentence.contains("not an ELF binary"),
        "the sentence says what the file is not: {sentence}"
    );
    assert!(
        sentence.contains("cross-built"),
        "and that a real cross-built tree is what it takes: {sentence}"
    );
}

#[test]
fn the_error_of_the_injected_reader_travels_rather_than_being_swallowed() {
    let dir = tempdir();
    let root = dir.path().join("otp");
    FakeOtp::new().build_in(&root);

    let error = erts_source::resolve_with(
        &ErtsSourceSpec::Dir(root),
        &Target::host(),
        |_| -> Result<ElfFacts, ElfError> { Err(ElfError::NotElf) },
    )
    .expect_err("a reader that fails fails the resolution");

    assert!(
        matches!(&error, ErtsError::NotAnElfRuntime { reason, .. } if reason.contains("not an ELF")),
        "the reader's own words are what the reason carries: {error:?}"
    );
}

#[test]
fn a_directory_that_is_not_a_runtime_root_is_the_otp_modules_error() {
    // Not this module's error: `otp::inspect_root` already says what a root is
    // missing, and a layer that reworded it would say less.
    let dir = tempdir();

    let error = erts_source::resolve(
        &ErtsSourceSpec::Dir(dir.path().to_path_buf()),
        &Target::host(),
    )
    .expect_err("an empty directory is not a runtime");

    assert!(matches!(&error, ErtsError::Otp(_)), "{error:?}");
}

#[test]
fn the_emulator_of_a_root_is_the_beam_smp_under_its_erts_bin() {
    let dir = tempdir();
    let root = dir.path().join("otp");
    let built = FakeOtp::new().erts_vsn("17.0.5").build_in(&root);
    let otp = ginary::otp::inspect_root(&built.root).expect("a whole fake root is a runtime");

    assert_eq!(emulator_path(&otp), root.join("erts-17.0.5/bin/beam.smp"));
}

// ------------------------------------------------------ the real thing --

#[test]
fn the_host_runtime_resolves_to_the_host_target() {
    let Some(_tools) = require_tools(&["erl"]) else {
        return;
    };

    let resolved = erts_source::resolve(&ErtsSourceSpec::Host, &Target::host())
        .expect("the host runtime is the host's");

    assert_eq!(resolved.target, Target::host());
    assert_eq!(
        resolved.linkage,
        Linkage::Dynamic,
        "a distribution's own emulator is dynamically linked"
    );
    assert!(resolved.nif_loading, "and therefore loads NIFs");
    let min = resolved
        .libc_min
        .as_deref()
        .expect("a gnu host reports a minimum glibc");
    assert!(
        min.split('.').all(|part| part.parse::<u32>().is_ok()),
        "the minimum is a version and not a sentence: {min}"
    );
    assert_eq!(
        resolved.provenance,
        format!("host:{}", resolved.otp.root.display()),
        "the provenance names the root the spelling resolved to"
    );
}

#[test]
fn the_host_root_resolves_through_dir_as_well_as_through_host() {
    let Some(_tools) = require_tools(&["erl"]) else {
        return;
    };
    let host = ginary::otp::discover(None).expect("the host runtime is discoverable");

    let resolved = erts_source::resolve(&ErtsSourceSpec::Dir(host.root.clone()), &Target::host())
        .expect("the host root is a runtime root like any other");

    assert_eq!(resolved.target, Target::host());
    assert_eq!(resolved.otp.erts_vsn, host.erts_vsn);
    assert_eq!(
        resolved.provenance,
        format!("dir:{}", host.root.display()),
        "the same tree through two spellings records two provenances"
    );
}

#[test]
fn the_host_emulator_is_a_file_the_elf_reader_can_read() {
    let Some(_tools) = require_tools(&["erl"]) else {
        return;
    };
    let host = ginary::otp::discover(None).expect("the host runtime is discoverable");
    let emulator: &Path = &emulator_path(&host);

    let info = ginary::elf::inspect(emulator).expect("the host emulator is an ELF");
    let facts = ElfFacts::of(&info);

    assert_eq!(facts.machine, info.machine);
    assert_eq!(facts.interp, info.interp);
    assert_eq!(
        Target::from_elf(&facts.machine, facts.interp.as_deref())
            .map(|elf| elf.resolve(Target::host().libc)),
        Some(Target::host()),
        "the emulator this machine runs says it is for this machine"
    );
}
