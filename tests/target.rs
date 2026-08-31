// SPDX-License-Identifier: MIT OR Apache-2.0
//! The target model beyond its names: the container platform, what an ELF
//! header says a binary is for, and the list of targets one build produces.
//!
//! The names, the round trips and the seven supported combinations are unit
//! tests in `src/target.rs`, because they are the module talking to itself.
//! What is here is the part other modules ask for. `erts_source` asks
//! [`Target::from_elf`] what a runtime really is, `bundle` asks
//! [`resolve_targets`] what to build, and both answers end up in a manifest,
//! so both are pinned here in full rather than in passing.
//!
//! Nothing in this file touches the filesystem, spawns a process or needs a
//! toolchain: every input is a string a test wrote.

use ginary::target::{
    ALL, Arch, ElfTarget, HOST_SELECTION, Libc, Linkage, Os, PSEUDO_TARGETS, Target, TargetError,
    names_a_target, resolve_targets,
};

/// The canonical names of `flags` and `config`, resolved, or the error.
fn resolve(flags: &[&str], config: &[&str]) -> Result<Vec<String>, TargetError> {
    let owned =
        |names: &[&str]| -> Vec<String> { names.iter().map(|name| (*name).to_owned()).collect() };
    resolve_targets(&owned(flags), &owned(config))
        .map(|targets| targets.iter().map(|target| target.name()).collect())
}

/// The named target, which every test in this file spells canonically.
fn target(name: &str) -> Target {
    name.parse()
        .unwrap_or_else(|error| panic!("`{name}` must be a target: {error}"))
}

// --------------------------------------------------- docker platforms --

#[test]
fn every_linux_target_carries_the_docker_platform_that_runs_it() {
    let mapping: Vec<String> = ALL
        .iter()
        .filter(|target| target.os == Os::Linux)
        .map(|target| format!("{} => {:?}", target.name(), target.docker_platform()))
        .collect();

    assert_eq!(
        mapping,
        [
            "linux-x86_64-gnu => Some(\"linux/amd64\")",
            "linux-x86_64-musl => Some(\"linux/amd64\")",
            "linux-aarch64-gnu => Some(\"linux/arm64\")",
            "linux-aarch64-musl => Some(\"linux/arm64\")",
        ]
    );
}

#[test]
fn a_target_no_container_runs_has_no_docker_platform() {
    // macOS and Windows are not Linux container platforms, and a value that
    // looked like one would fail at `docker create` rather than here.
    for name in ["macos-x86_64", "macos-aarch64", "windows-x86_64"] {
        assert_eq!(target(name).docker_platform(), None, "{name}");
    }
}

// ---------------------------------------------- what an ELF header says --

#[test]
fn the_glibc_loader_names_a_gnu_target() {
    assert_eq!(
        Target::from_elf("x86_64", Some("/lib64/ld-linux-x86-64.so.2")),
        Some(ElfTarget::Dynamic(target("linux-x86_64-gnu")))
    );
    assert_eq!(
        Target::from_elf("aarch64", Some("/lib/ld-linux-aarch64.so.1")),
        Some(ElfTarget::Dynamic(target("linux-aarch64-gnu")))
    );
}

#[test]
fn the_musl_loader_names_a_musl_target() {
    assert_eq!(
        Target::from_elf("x86_64", Some("/lib/ld-musl-x86_64.so.1")),
        Some(ElfTarget::Dynamic(target("linux-x86_64-musl")))
    );
    assert_eq!(
        Target::from_elf("aarch64", Some("/lib/ld-musl-aarch64.so.1")),
        Some(ElfTarget::Dynamic(target("linux-aarch64-musl")))
    );
}

#[test]
fn an_emulator_with_no_interpreter_is_static_and_names_no_libc() {
    // The two static runtimes are the same bytes as far as the header goes:
    // a musl-static build carries no `PT_INTERP` and neither does a
    // glibc-static one, so the answer says the architecture and stops.
    let answer = Target::from_elf("aarch64", None).expect("a static aarch64 binary is a target");

    assert_eq!(answer, ElfTarget::StaticLinux(Arch::Aarch64));
    assert_eq!(answer.linkage(), Linkage::Static);
    assert_eq!(answer.target(), None, "nothing read the libc");
}

#[test]
fn a_static_binary_is_reported_as_the_libc_the_build_asked_for() {
    let answer = Target::from_elf("x86_64", None).expect("a static x86_64 binary is a target");

    assert_eq!(answer.resolve(Libc::Musl), target("linux-x86_64-musl"));
    assert_eq!(answer.resolve(Libc::Gnu), target("linux-x86_64-gnu"));
}

#[test]
fn a_dynamic_answer_ignores_the_libc_it_is_offered() {
    // `resolve` fills in what a static binary does not say; it may not
    // overwrite what a dynamic one does.
    let answer = Target::from_elf("x86_64", Some("/lib/ld-musl-x86_64.so.1"))
        .expect("a musl binary is a target");

    assert_eq!(answer.resolve(Libc::Gnu), target("linux-x86_64-musl"));
    assert_eq!(answer.linkage(), Linkage::Dynamic);
}

#[test]
fn a_machine_ginary_has_no_target_for_is_not_a_target() {
    for machine in ["riscv64", "ppc64", "i386", ""] {
        assert_eq!(
            Target::from_elf(machine, Some("/lib/ld-linux-x86-64.so.2")),
            None,
            "{machine} is not a machine ginary packages for"
        );
    }
}

#[test]
fn an_interpreter_that_names_neither_loader_is_not_a_target() {
    // A guess here would be written into a manifest as though it had been
    // read off the file, so there is no guess.
    for interp in ["/lib/ld-uClibc.so.0", "/usr/lib/ld.so.1", ""] {
        assert_eq!(
            Target::from_elf("x86_64", Some(interp)),
            None,
            "`{interp}` names no C library ginary knows"
        );
    }
}

// ------------------------------------------------------ resolve_targets --

#[test]
fn a_build_that_names_nothing_builds_for_the_host() {
    assert_eq!(resolve(&[], &[]), Ok(vec![Target::host().name()]));
}

#[test]
fn the_configured_targets_are_used_when_no_flag_names_one() {
    assert_eq!(
        resolve(&[], &["linux-aarch64-musl", "macos-aarch64"]),
        Ok(vec![
            "linux-aarch64-musl".to_owned(),
            "macos-aarch64".to_owned()
        ])
    );
}

#[test]
fn the_flags_replace_the_configured_targets_rather_than_adding_to_them() {
    // A `--target` is what the user typed just now: it decides the list, and
    // a project that names four targets does not build five because one flag
    // was passed.
    assert_eq!(
        resolve(&["macos-x86_64"], &["linux-aarch64-musl", "macos-aarch64"]),
        Ok(vec!["macos-x86_64".to_owned()])
    );
}

#[test]
fn host_expands_to_the_one_target_this_machine_is() {
    assert_eq!(resolve(&["host"], &[]), Ok(vec![Target::host().name()]));
}

#[test]
fn all_expands_to_every_supported_target_in_the_order_all_declares() {
    let expected: Vec<String> = ALL.iter().map(|target| target.name()).collect();

    assert_eq!(resolve(&["all"], &[]), Ok(expected));
}

#[test]
fn a_target_named_twice_is_built_once_where_it_was_first_named() {
    assert_eq!(
        resolve(&["macos-aarch64", "linux-x86_64-gnu", "macos-aarch64"], &[]),
        Ok(vec![
            "macos-aarch64".to_owned(),
            "linux-x86_64-gnu".to_owned()
        ])
    );
}

#[test]
fn host_and_the_hosts_own_name_are_one_target() {
    // The suffix rule turns on whether a target was named, never on how it
    // was spelled, so the two spellings have to collapse here.
    let host = Target::host().name();

    assert_eq!(resolve(&["host", &host], &[]), Ok(vec![host]));
}

#[test]
fn all_beside_a_name_it_already_holds_stays_seven_targets() {
    let expected: Vec<String> = ALL.iter().map(|target| target.name()).collect();

    assert_eq!(resolve(&["all", "macos-aarch64"], &[]), Ok(expected));
}

#[test]
fn an_unknown_selection_is_refused_and_lists_every_spelling_that_is_not() {
    let error = resolve(&["linux-riscv64-gnu"], &[]).expect_err("riscv64 is not a target");

    assert_eq!(
        error,
        TargetError::Unknown {
            name: "linux-riscv64-gnu".to_owned()
        }
    );
    assert_eq!(
        error.to_string(),
        "`linux-riscv64-gnu` is not a target; expected one of `host`, `all`, `linux-x86_64-gnu`, \
         `linux-x86_64-musl`, `linux-aarch64-gnu`, `linux-aarch64-musl`, `macos-x86_64`, \
         `macos-aarch64`, `windows-x86_64`"
    );
}

#[test]
fn an_unknown_selection_in_the_configuration_is_refused_the_same_way() {
    assert_eq!(
        resolve(&[], &["linux-x86_64"]),
        Err(TargetError::Unknown {
            name: "linux-x86_64".to_owned()
        }),
        "an ambiguous Linux name is not a target either"
    );
}

#[test]
fn the_first_unknown_selection_is_the_one_reported() {
    assert_eq!(
        resolve(&["macos-aarch64", "nonsense", "rubbish"], &[]),
        Err(TargetError::Unknown {
            name: "nonsense".to_owned()
        })
    );
}

// ------------------------------------------------------ names_a_target --

/// Whether `flags` over `config` name a target, spelled as the tests spell
/// selections everywhere else in this file.
fn names(flags: &[&str], config: &[&str]) -> bool {
    let owned =
        |names: &[&str]| -> Vec<String> { names.iter().map(|name| (*name).to_owned()).collect() };
    names_a_target(&owned(flags), &owned(config))
}

#[test]
fn selecting_the_host_names_no_target_however_it_is_spelled() {
    // `host` asks for the build a bare `ginary build` already performs, so
    // the three spellings of that one request answer alike and the artifact
    // keeps the plain name a script quotes.
    assert!(!names(&[], &[]), "nothing named");
    assert!(
        !names(&[], &[HOST_SELECTION]),
        "the table selected the host"
    );
    assert!(
        !names(&[HOST_SELECTION], &[]),
        "the command line selected the host"
    );
    assert!(
        !names(&[HOST_SELECTION], &[HOST_SELECTION]),
        "both said the same thing"
    );
}

#[test]
fn a_canonical_name_is_a_target_named_wherever_it_was_spelled() {
    // The finding this pins: the host's own canonical name is a target named,
    // and a project that writes it into `targets` has asked for the same file
    // name that `--target` does.
    let host = Target::host().name();

    assert!(names(&[&host], &[]), "on the command line");
    assert!(names(&[], &[&host]), "in the table");
    assert!(names(&[], &["all"]), "`all` names every target there is");
    assert!(
        names(&[], &[HOST_SELECTION, "linux-aarch64-musl"]),
        "one named target in a list is a list that names one"
    );
}

#[test]
fn the_flags_decide_whether_a_target_was_named_as_they_decide_the_list() {
    // The precedence is `resolve_targets`': a `--target host` build produces
    // the host and nothing else, so the table's names are not in the artifact
    // it writes either.
    assert!(
        !names(&[HOST_SELECTION], &["linux-aarch64-musl"]),
        "the flag replaced the list, so nothing the table named is built"
    );
    assert!(
        names(&["macos-x86_64"], &[HOST_SELECTION]),
        "and the flag's own name is the one that counts"
    );
}

#[test]
fn the_pseudo_names_are_the_two_that_expand() {
    // Pinned because the error message above lists them first and because
    // `[tools.ginary.target.<name>]` refuses exactly these two.
    assert_eq!(PSEUDO_TARGETS, ["host", "all"]);
}
