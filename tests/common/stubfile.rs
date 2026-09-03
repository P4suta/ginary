// SPDX-License-Identifier: MIT OR Apache-2.0
//! Building the files a stub test needs: identity markers, bytes that hold
//! them, and copies of this test run's own `ginary` binary that carry one.
//!
//! Two shapes of fixture, because the module under test has two halves.
//! [`Marker::bytes`] and [`with_markers`] are plain byte vectors, which is all
//! `stubid::scan` reads: a scanner test needs no executable, and building one
//! would tie the assertion to whatever the linker did that day. [`stub_copy`]
//! is the other half — `stub::verify` reads a *file* and looks at its object
//! header, so its fixtures are real ELF binaries, made by copying the `ginary`
//! this test run built and rewriting the marker in it.
//!
//! [`stub_copy`] replaces the marker when the binary already carries one and
//! appends one when it does not, so the fixture is the same shape whether or
//! not the build embeds a marker yet. Appending to an ELF is safe: nothing in
//! the file's headers describes the bytes past the last section, which is the
//! same property `payload.rs` relies on.
//!
//! The needle is assembled here from its two halves for the same reason
//! `stubid::scan` assembles its own: a helper that held `GINARY-STUB-ID\0`
//! contiguously would put a second marker into every test binary that links
//! it, and `tests/stubid.rs` would then be scanning itself.

use std::path::{Path, PathBuf};

use ginary::target::Target;

/// The first half of the needle.
pub const HEAD: &[u8] = b"GINARY-STUB";

/// The second half, ending in the NUL that closes the name.
pub const TAIL: &[u8] = b"-ID\0";

/// The length of a whole marker, name and padding included.
pub const MARKER_LEN: usize = 128;

/// The version of the `ginary` this test run built.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The payload format version this ginary writes.
pub const FORMAT_VERSION: u32 = ginary::manifest::FORMAT_VERSION;

/// The needle, assembled rather than stored.
pub fn needle() -> Vec<u8> {
    let mut bytes = HEAD.to_vec();
    bytes.extend_from_slice(TAIL);
    bytes
}

/// The four fields of a marker, each as text so that a test can write a value
/// no parser would produce.
#[derive(Clone, Debug)]
pub struct Marker {
    /// The `v` field.
    pub version: String,
    /// The `t` field.
    pub target: String,
    /// The `f` field.
    pub format: String,
    /// The `k` field.
    pub flavor: String,
}

impl Marker {
    /// The marker a stub of this ginary, for the host, would carry.
    pub fn host() -> Self {
        Self::for_target(&Target::host())
    }

    /// The marker a stub of this ginary, for `target`, would carry.
    pub fn for_target(target: &Target) -> Self {
        Self {
            version: VERSION.to_owned(),
            target: target.name(),
            format: FORMAT_VERSION.to_string(),
            flavor: "full".to_owned(),
        }
    }

    /// The same marker with another `v`.
    pub fn version(mut self, value: &str) -> Self {
        self.version = value.to_owned();
        self
    }

    /// The same marker with another `t`.
    pub fn target(mut self, value: &str) -> Self {
        self.target = value.to_owned();
        self
    }

    /// The same marker with another `f`.
    pub fn format(mut self, value: &str) -> Self {
        self.format = value.to_owned();
        self
    }

    /// The same marker with another `k`.
    pub fn flavor(mut self, value: &str) -> Self {
        self.flavor = value.to_owned();
        self
    }

    /// The `v=...;t=...;f=...;k=...` body, without its terminating NUL.
    pub fn body(&self) -> String {
        format!(
            "v={};t={};f={};k={}",
            self.version, self.target, self.format, self.flavor
        )
    }

    /// The whole marker: name, body, NUL and zero padding.
    pub fn bytes(&self) -> [u8; MARKER_LEN] {
        marker_from_body(self.body().as_bytes())
    }
}

/// A marker whose body is `body`, terminated and zero-padded.
///
/// The body is bytes rather than text so that a test can write one that is not
/// UTF-8.
///
/// # Panics
///
/// If the body does not fit.
pub fn marker_from_body(body: &[u8]) -> [u8; MARKER_LEN] {
    let mut bytes = [0u8; MARKER_LEN];
    let name = needle();
    let end = name.len() + body.len() + 1;
    assert!(
        end <= MARKER_LEN,
        "a {} byte body does not fit in a {MARKER_LEN} byte marker",
        body.len()
    );
    bytes[..name.len()].copy_from_slice(&name);
    bytes[name.len()..name.len() + body.len()].copy_from_slice(body);
    // The NUL that terminates the body is written by the zero fill; the assert
    // above is what guarantees there is room for it.
    bytes
}

/// Bytes that are not a marker, and hold no needle.
///
/// A fixed multiplier and a caller-chosen seed, so a failure is reproducible:
/// "random bytes" that differ between runs would make a scanner bug that
/// depends on one byte in ten thousand a flake rather than a failure.
///
/// # Panics
///
/// If the bytes happen to hold the needle, which is what makes the fixture a
/// negative case rather than an accident.
pub fn noise(len: usize, seed: u64) -> Vec<u8> {
    let mut state = seed | 1;
    let mut bytes = Vec::with_capacity(len);
    for _ in 0..len {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        bytes.push((state >> 24) as u8);
    }
    assert!(
        offsets(&bytes).is_empty(),
        "the noise for seed {seed} holds a needle; pick another seed"
    );
    bytes
}

/// Noise with each marker planted in it, one after another.
///
/// The markers are separated by noise so that a scanner which found the second
/// one by walking on from the first cannot be confused with one that searched
/// the whole buffer.
pub fn with_markers(markers: &[[u8; MARKER_LEN]]) -> Vec<u8> {
    let mut bytes = noise(2048, 0x5eed);
    for (index, marker) in markers.iter().enumerate() {
        bytes.extend_from_slice(marker);
        bytes.extend_from_slice(&noise(1024, 0xa11ce + index as u64));
    }
    bytes
}

/// Every offset the needle appears at.
pub fn offsets(bytes: &[u8]) -> Vec<usize> {
    let needle = needle();
    if needle.len() > bytes.len() {
        return Vec::new();
    }
    (0..=bytes.len() - needle.len())
        .filter(|start| bytes[*start..*start + needle.len()] == needle[..])
        .collect()
}

/// This test run's own `ginary` binary.
pub fn ginary_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_ginary"))
}

/// A copy of the `ginary` binary at `<dir>/<name>`, carrying exactly `marker`.
///
/// # Panics
///
/// If the binary cannot be read or the copy cannot be written, or if the
/// binary carries more than one marker already — a `ginary` with two
/// identities is a bug this helper must not paper over.
pub fn stub_copy(dir: &Path, name: &str, marker: &[u8; MARKER_LEN]) -> PathBuf {
    let mut bytes = std::fs::read(ginary_bin()).expect("the ginary binary is readable");
    let found = offsets(&bytes);
    match found.as_slice() {
        [] => bytes.extend_from_slice(marker),
        [offset] => bytes[*offset..*offset + MARKER_LEN].copy_from_slice(marker),
        many => panic!("the ginary binary carries {} markers", many.len()),
    }
    write_executable(dir, name, &bytes)
}

/// A copy of the `ginary` binary with no identity marker at all.
///
/// # Panics
///
/// As [`stub_copy`].
pub fn stub_copy_without_marker(dir: &Path, name: &str) -> PathBuf {
    let mut bytes = std::fs::read(ginary_bin()).expect("the ginary binary is readable");
    for offset in offsets(&bytes) {
        bytes[offset..offset + MARKER_LEN].fill(0);
    }
    write_executable(dir, name, &bytes)
}

/// A file that carries `marker` and is not an object file.
///
/// The marker gates and the object gate are separate claims, and this is what
/// tells them apart: everything the marker says is right, and the file is a
/// shell script.
///
/// # Panics
///
/// If the file cannot be written.
pub fn text_with_marker(dir: &Path, name: &str, marker: &[u8; MARKER_LEN]) -> PathBuf {
    let mut bytes = b"#!/bin/sh\necho not a stub\n".to_vec();
    bytes.extend_from_slice(marker);
    write_executable(dir, name, &bytes)
}

/// Writes `bytes` to `<dir>/<name>`, executable, and returns the path.
///
/// # Panics
///
/// If the directory or the file cannot be written.
pub fn write_executable(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    std::fs::create_dir_all(dir).expect("the fixture directory");
    let path = dir.join(name);
    std::fs::write(&path, bytes).expect("the fixture file");
    set_executable(&path);
    path
}

/// Gives a file mode 0o755, where the platform has modes.
///
/// Portable on purpose: [`write_executable`] calls it for every fixture it
/// writes and those fixtures are read on all three operating systems, so only
/// the chmod is gated. On Windows an executable is decided by its extension
/// and there is nothing here to do.
///
/// # Panics
///
/// If the mode cannot be set.
pub fn set_executable(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
            .expect("the fixture is executable");
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// The name `mise run stubs:build` writes for `target`.
pub fn stub_file_name(version: &str, target: &Target) -> String {
    format!(
        "ginary-stub-{version}-{}{}",
        target.name(),
        target.exe_suffix()
    )
}

/// The second name a stub directory is searched for.
pub fn plain_file_name(version: &str, target: &Target) -> String {
    format!("ginary-{version}-{}{}", target.name(), target.exe_suffix())
}

/// `<cache>/stubs/<version>/<target>`, where a fetched stub is kept.
pub fn cache_stub_path(cache_dir: &Path, version: &str, target: &Target) -> PathBuf {
    cache_dir
        .join("stubs")
        .join(version)
        .join(format!("{}{}", target.name(), target.exe_suffix()))
}

/// The variable that says the cross-built stubs are supposed to be on this
/// machine, so a missing one is a failure rather than a skip.
///
/// It is deliberately *not* [`crate::common::tools::REQUIRE_VAR`].
/// `GINARY_REQUIRE_TOOLCHAIN` is a claim about programs the machine installs —
/// `erl`, `gleam`, `strip`, `docker` — and a cross-built stub is none of
/// those: it is a file `mise run stubs:build` produces after minutes of
/// `cross` in a docker container, and a job that never ran that command has
/// not got one however complete its toolchain is. Conflating the two is what
/// failed the `test` and `coverage` jobs of the first pull-request run; see
/// `tests/regressions/e6_the_toolchain_flag_required_a_cross_stub_nobody_built.rs`.
#[cfg(feature = "cli")]
pub const REQUIRE_STUBS_VAR: &str = "GINARY_REQUIRE_STUBS";

/// What a stub-gated test should do, given what the environment says.
#[cfg(feature = "cli")]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StubChoice {
    /// Run against this file.
    Run(PathBuf),
    /// Do not run, and print this reason on standard error.
    Skip(String),
    /// Fail, with this message: the caller promised the stubs were here.
    Fail(String),
}

/// Decides between running, skipping and failing, without touching `PATH` or
/// the environment.
///
/// `dirs` are the directories searched, in order; `is_file` answers whether a
/// candidate exists, passed in so the rule can be asserted without a
/// filesystem. `required_toolchain` is whether `GINARY_REQUIRE_TOOLCHAIN` is
/// `1` and `required_stubs` is whether [`REQUIRE_STUBS_VAR`] is: the first
/// changes nothing here, and that is the whole rule.
///
/// Three answers, in the order the questions are asked:
///
/// 1. the first directory that holds the file wins, whatever either flag
///    says — a stub that is present is never skipped and never a failure;
/// 2. otherwise, `required_stubs` makes it a failure, because a job that
///    obtained the stubs — `smoke-matrix` cross-builds them, `coverage`
///    downloads them — and then cannot find one has a broken step, not a
///    machine without a toolchain;
/// 3. otherwise it is a skip that names the command which would fix it.
///
/// # Panics
///
/// Never.
#[cfg(feature = "cli")]
pub fn choose_cross_stub(
    name: &str,
    dirs: &[PathBuf],
    required_toolchain: bool,
    required_stubs: bool,
    is_file: &dyn Fn(&Path) -> bool,
) -> StubChoice {
    // Taken and deliberately not read. Dropping the parameter would leave the
    // rule invisible at the call site, and the whole defect this function
    // exists for was `GINARY_REQUIRE_TOOLCHAIN` being consulted here.
    let _ = required_toolchain;

    for dir in dirs {
        let candidate = dir.join(name);
        if is_file(&candidate) {
            return StubChoice::Run(candidate);
        }
    }
    if required_stubs {
        return StubChoice::Fail(format!(
            "no {name} in any of {dirs:?}: {REQUIRE_STUBS_VAR}=1 says this job obtained the \
             stubs, so the step that built or downloaded them produced nothing for this target \
             — check the step that fills target/stubs in this job"
        ));
    }
    StubChoice::Skip(format!(
        "no {name} in any of {dirs:?}: run `mise run stubs:build` or set {}",
        ginary::stub::STUB_DIR_VAR
    ))
}

/// Which of the two requirement variables the environment sets, as
/// `(required_toolchain, required_stubs)`.
///
/// The seam between [`cross_stub`] and [`choose_cross_stub`], and the half of
/// this module that the first pull-request run actually got wrong: the rule
/// was never in doubt, the *wiring* read `GINARY_REQUIRE_TOOLCHAIN` where it
/// meant [`REQUIRE_STUBS_VAR`]. `lookup` is passed in rather than read from
/// the process so that swapping the two names back is a test failure and not
/// a green suite; a test that mutated the real environment would race every
/// other test in the binary.
///
/// Only the exact value `1` counts, for either: an empty variable is how a
/// shell spells "unset" and `GINARY_REQUIRE_STUBS=0` is a contributor saying
/// no.
#[cfg(feature = "cli")]
pub fn stub_requirement(lookup: &dyn Fn(&str) -> Option<std::ffi::OsString>) -> (bool, bool) {
    let set = |name: &str| lookup(name).is_some_and(|value| value == "1");
    (
        set(crate::common::tools::REQUIRE_VAR),
        set(REQUIRE_STUBS_VAR),
    )
}

/// A prebuilt cross stub for `target`, or a printed skip.
///
/// `GINARY_STUB_DIR` first, then `target/stubs` in the repository, which is
/// where `mise run stubs:build` puts them. A test that needs a real
/// cross-built ELF cannot build one itself — `cross` needs a docker daemon and
/// minutes — so a machine without one skips, loudly, naming the command that
/// would produce it.
///
/// The switch that forbids the skip is [`REQUIRE_STUBS_VAR`] and not
/// `GINARY_REQUIRE_TOOLCHAIN`: see [`choose_cross_stub`] for why the two are
/// different questions.
///
/// # Panics
///
/// If the stub is missing and `GINARY_REQUIRE_STUBS=1`.
#[cfg(feature = "cli")]
pub fn cross_stub(target: &Target) -> Option<PathBuf> {
    let name = stub_file_name(VERSION, target);
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(dir) = std::env::var_os(ginary::stub::STUB_DIR_VAR) {
        dirs.push(PathBuf::from(dir));
    }
    dirs.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("target/stubs"));

    let (required_toolchain, required_stubs) =
        stub_requirement(&|name: &str| std::env::var_os(name));

    match choose_cross_stub(
        &name,
        &dirs,
        required_toolchain,
        required_stubs,
        &|path: &Path| path.is_file(),
    ) {
        StubChoice::Run(path) => Some(path),
        StubChoice::Skip(reason) => {
            eprintln!("skipping: {reason}");
            None
        }
        StubChoice::Fail(reason) => panic!("{reason}"),
    }
}

/// The `IMAGE_FILE_MACHINE_AMD64` a 64-bit x86 PE names in its COFF header.
pub const PE_MACHINE_AMD64: u16 = 0x8664;

/// The `IMAGE_FILE_MACHINE_ARM64` an aarch64 PE names.
pub const PE_MACHINE_ARM64: u16 = 0xAA64;

/// The size of a PE32+ optional header with sixteen data directories.
const PE_OPTIONAL_HEADER_LEN: u16 = 240;

/// A minimal PE32+ executable for `machine`, with `marker` appended.
///
/// Written by hand, for the reason `tests/common/payload.rs` writes tar
/// headers by hand: there is no Windows toolchain here, and the claim under
/// test is what `stub::verify` reads out of a COFF header — the format and the
/// machine — rather than anything a linker would have to produce. Everything
/// past those two fields is the smallest well-formed shape `object` will
/// parse: a DOS stub whose `e_lfanew` points at `PE\0\0`, one section, and the
/// sixteen empty data directories the optional header's size promises.
///
/// The marker goes after the section data, where nothing in the headers
/// describes it, which is the same property [`stub_copy`] relies on for ELF.
pub fn pe_bytes(machine: u16, marker: &[u8; MARKER_LEN]) -> Vec<u8> {
    let mut bytes: Vec<u8> = Vec::new();
    // The DOS header: the signature, then zeros up to `e_lfanew` at 0x3c.
    bytes.extend_from_slice(b"MZ");
    bytes.resize(0x3c, 0);
    bytes.extend_from_slice(&0x40u32.to_le_bytes());
    bytes.resize(0x40, 0);

    // The COFF header.
    bytes.extend_from_slice(b"PE\0\0");
    bytes.extend_from_slice(&machine.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes()); // one section
    bytes.extend_from_slice(&0u32.to_le_bytes()); // time stamp
    bytes.extend_from_slice(&0u32.to_le_bytes()); // no symbol table
    bytes.extend_from_slice(&0u32.to_le_bytes()); // no symbols
    bytes.extend_from_slice(&PE_OPTIONAL_HEADER_LEN.to_le_bytes());
    bytes.extend_from_slice(&0x0022u16.to_le_bytes()); // executable, large address aware

    // The PE32+ optional header, laid out field by field so that the 240 the
    // COFF header promises is a length this code actually writes.
    let optional_at = bytes.len();
    bytes.extend_from_slice(&0x020bu16.to_le_bytes()); // PE32+
    bytes.extend_from_slice(&[14, 0]); // linker version
    for _ in 0..5 {
        bytes.extend_from_slice(&0u32.to_le_bytes()); // the five size and address fields
    }
    bytes.extend_from_slice(&0x0000_0001_4000_0000u64.to_le_bytes()); // image base
    bytes.extend_from_slice(&0x1000u32.to_le_bytes()); // section alignment
    bytes.extend_from_slice(&0x200u32.to_le_bytes()); // file alignment
    for value in [0u16, 0, 0, 0, 6, 0] {
        bytes.extend_from_slice(&value.to_le_bytes()); // the six version fields
    }
    bytes.extend_from_slice(&0u32.to_le_bytes()); // Win32VersionValue
    bytes.extend_from_slice(&0x2000u32.to_le_bytes()); // size of image
    bytes.extend_from_slice(&0x200u32.to_le_bytes()); // size of headers
    bytes.extend_from_slice(&0u32.to_le_bytes()); // checksum
    bytes.extend_from_slice(&3u16.to_le_bytes()); // console subsystem
    bytes.extend_from_slice(&0u16.to_le_bytes()); // DLL characteristics
    for _ in 0..4 {
        bytes.extend_from_slice(&0x10_0000u64.to_le_bytes()); // stack and heap
    }
    bytes.extend_from_slice(&0u32.to_le_bytes()); // loader flags
    bytes.extend_from_slice(&16u32.to_le_bytes()); // sixteen data directories
    for _ in 0..16 {
        bytes.extend_from_slice(&0u64.to_le_bytes());
    }
    assert_eq!(
        bytes.len() - optional_at,
        usize::from(PE_OPTIONAL_HEADER_LEN),
        "the optional header has to be the length the COFF header promised"
    );

    // One section header, and the section's own bytes at the file alignment.
    bytes.extend_from_slice(b".text\0\0\0");
    bytes.extend_from_slice(&0x10u32.to_le_bytes()); // virtual size
    bytes.extend_from_slice(&0x1000u32.to_le_bytes()); // virtual address
    bytes.extend_from_slice(&0x200u32.to_le_bytes()); // size of raw data
    bytes.extend_from_slice(&0x200u32.to_le_bytes()); // pointer to raw data
    bytes.extend_from_slice(&0u32.to_le_bytes()); // no relocations
    bytes.extend_from_slice(&0u32.to_le_bytes()); // no line numbers
    bytes.extend_from_slice(&0u16.to_le_bytes());
    bytes.extend_from_slice(&0u16.to_le_bytes());
    bytes.extend_from_slice(&0x6000_0020u32.to_le_bytes()); // code, read, execute
    bytes.resize(0x400, 0);

    bytes.extend_from_slice(marker);
    bytes
}

/// A minimal PE32+ for `machine` written to `<dir>/<name>`.
///
/// # Panics
///
/// If the file cannot be written.
pub fn pe_with_marker(dir: &Path, name: &str, machine: u16, marker: &[u8; MARKER_LEN]) -> PathBuf {
    write_executable(dir, name, &pe_bytes(machine, marker))
}
