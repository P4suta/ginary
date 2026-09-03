// SPDX-License-Identifier: MIT OR Apache-2.0
//! Laying out a payload entry by entry, so the index can be made to lie.
//!
//! [`crate::common::artifact::SyntheticArtifact`] builds an artifact through
//! the real [`ginary::payload::pack`], which computes `ginary.index.json` from
//! the tree it is packing. That is the right thing for every test about a
//! *correct* artifact and it makes the four defects `ginary verify` exists to
//! find unreachable: an index that disagrees with the payload cannot be
//! produced by the code that writes both from one walk.
//!
//! So this builder writes the archive itself. It stages the same tree, builds
//! the same index, and then — after the digests are taken — rewrites a file's
//! bytes, drops a row, or adds a row for a file nobody packed. What comes out
//! is a whole artifact with a trailer whose digest matches the payload it
//! describes, exactly as a real one has: the payload hash is *not* the check
//! that catches these, which is the whole reason `ginary verify` is more than
//! `ginary inspect --verify`.
//!
//! `appended` is the same idea one step further out. `pack` refuses a staging
//! listing that names a reserved path and only ever writes regular entries, so
//! a second `ginary.json`, a directory entry and a symlink are three shapes the
//! format has rules about and which ginary's own packer can never produce. This
//! builder writes them, because `ginary verify` is aimed at artifacts other
//! people built.
//!
//! It also carries the one thing the synthetic tree deliberately has not got:
//! a real ELF. [`NATIVE_PATH`] is the committed `x86_64` Linux ELF fixture,
//! copied in under an application's `priv`, so the object table is built from a
//! file a linker wrote rather than from one a test made up — and from a genuine
//! ELF whatever host runs the test, which the test run's own binary is not
//! (a PE on Windows, a Mach-O on macOS). See `tests/fixtures/elf/README.md`.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use ginary::manifest::{Index, IndexFile, Manifest, NativeRef};
use ginary::target::{Arch, Libc, Os, Target};
use ginary::trailer::Trailer;

use super::artifact::{ArtifactOptions, LEVEL};

/// Where the real ELF is staged: under the packaged application's `priv`,
/// which is where a NIF or a helper program actually lives.
pub const NATIVE_PATH: &str = "lib/hello/priv/bin/tool";

/// `e_machine` for x86-64.
pub const EM_X86_64: u16 = 62;

/// `e_machine` for AArch64.
pub const EM_AARCH64: u16 = 183;

/// The offset of `e_machine` in an ELF header, in both classes.
pub const E_MACHINE_OFFSET: usize = 18;

/// How the payload is made to disagree with itself.
#[derive(Default)]
pub struct RepackOptions {
    /// How the staging tree is built; see [`ArtifactOptions`].
    pub artifact: ArtifactOptions,
    /// The target the manifest claims, when it is not the canonical one.
    pub target: Option<Target>,
    /// Files whose bytes are rewritten *after* the index has hashed them.
    pub corrupt: Vec<(String, Vec<u8>)>,
    /// Index rows to delete, leaving the file in the payload and unnamed.
    pub drop_from_index: Vec<String>,
    /// Index rows to invent, naming a file the payload does not carry.
    pub ghost_index_rows: Vec<String>,
    /// Index rows whose `mode` is rewritten after the tree has been read.
    ///
    /// The tar header keeps the staged file's own mode, so the row and the
    /// entry the launcher will extract disagree about a column
    /// `ginary verify` has to check: an index that promises an executable
    /// over a payload that carries a `0644` entry describes a file the
    /// artifact does not hold.
    pub index_mode_lies: Vec<(String, u32)>,
    /// Index rows whose `size` is rewritten after the tree has been read.
    ///
    /// The digest still describes the bytes, so the length is the only column
    /// that disagrees and nothing else can account for the finding.
    pub index_size_lies: Vec<(String, u64)>,
    /// The `native` list the manifest claims, when it is not the empty one.
    ///
    /// `pack` derives nothing about native code from the tree, so a manifest
    /// that names a file the payload does not carry — or that records the
    /// wrong machine for one it does — cannot be produced by the build. It is
    /// exactly the lie `ginary verify` cross-checks the manifest for, so the
    /// list is planted here.
    pub native: Vec<NativeRef>,
    /// Entries appended after every staged file, which no index row describes.
    ///
    /// This is how an archive that `payload::pack` would refuse to write is
    /// put in front of a reader: a second `ginary.json`, a directory entry, a
    /// symlink. Each one is a shape the format has a rule about and which the
    /// packer therefore never produces.
    pub appended: Vec<AppendedEntry>,
}

/// One entry written after the staged files, described by hand.
#[derive(Clone, Debug)]
pub struct AppendedEntry {
    /// The path the entry lands on.
    pub name: String,
    /// What kind of entry it is.
    pub kind: tar::EntryType,
    /// The bytes, for a regular entry.
    pub bytes: Vec<u8>,
    /// The link target, for a symlink entry.
    pub link: Option<String>,
    /// Whether the header is written with `set_path_absolute`.
    ///
    /// `tar::Header::set_path` refuses a name that leaves the extracted root,
    /// which is the whole point of it. `set_path_absolute` is the one door the
    /// crate leaves open, and it is the only way a test can lay down the entry
    /// `payload::unpack` calls `UnsafePath` — an archive somebody else's
    /// packer wrote, which is exactly the kind `ginary verify` is aimed at.
    pub absolute: bool,
    /// Whether an index row describing these bytes is added for this name.
    ///
    /// Off by default, so an appended entry is an orphan the index does not
    /// name. On, the row carries the real digest of `bytes`, which is how a
    /// hostile artifact is expressed: the index and the payload agree, and
    /// every check that compares the two therefore passes.
    pub indexed: bool,
}

impl AppendedEntry {
    /// A regular file entry holding `bytes`.
    pub fn file(name: &str, bytes: &[u8]) -> Self {
        Self {
            name: name.to_owned(),
            kind: tar::EntryType::Regular,
            bytes: bytes.to_vec(),
            link: None,
            absolute: false,
            indexed: false,
        }
    }

    /// A directory entry, which carries no data.
    pub fn directory(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            kind: tar::EntryType::Directory,
            bytes: Vec::new(),
            link: None,
            absolute: false,
            indexed: false,
        }
    }

    /// A symlink entry pointing at `target`.
    pub fn symlink(name: &str, target: &str) -> Self {
        Self {
            name: name.to_owned(),
            kind: tar::EntryType::Symlink,
            bytes: Vec::new(),
            link: Some(target.to_owned()),
            absolute: false,
            indexed: false,
        }
    }

    /// The same entry, with its name written as an absolute path.
    #[must_use]
    pub fn absolute(mut self) -> Self {
        self.absolute = true;
        self
    }

    /// The same entry, with an index row that describes its bytes truthfully.
    #[must_use]
    pub fn indexed(mut self) -> Self {
        self.indexed = true;
        self
    }
}

/// An artifact whose payload was laid out by hand.
#[derive(Debug)]
pub struct Repacked {
    dir: PathBuf,
    path: PathBuf,
    manifest: Manifest,
    index: Index,
    trailer: Trailer,
}

impl Repacked {
    /// The artifact executable.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The temporary directory it lives in.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The manifest that was packed.
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// The index that was packed, after any edits.
    pub fn index(&self) -> &Index {
        &self.index
    }

    /// The trailer that was appended.
    pub fn trailer(&self) -> &Trailer {
        &self.trailer
    }
}

/// Builds an artifact in `dir` from the options.
///
/// # Panics
///
/// If any part of the assembly fails. Each one is a bug in the test tree
/// rather than a property of the machine.
pub fn build(dir: &Path, options: &RepackOptions) -> Repacked {
    let staging = dir.join("staging");
    let listing = super::artifact::stage(&staging, &options.artifact);

    let mut manifest = super::artifact::canonical_manifest();
    if let Some(target) = options.target {
        manifest.target = target;
    }
    if let Some(app) = &options.artifact.app {
        manifest.app.clone_from(app);
    }
    if !options.native.is_empty() {
        manifest.native.clone_from(&options.native);
    }

    let mut index = Index::from_staged(&staging, &listing.files)
        .unwrap_or_else(|error| panic!("cannot index the staging tree: {error}"));

    // After the index: a file whose bytes changed once the artifact was
    // described is exactly the defect `IndexMismatch` names.
    for (path, bytes) in &options.corrupt {
        let full = staging.join(path);
        assert!(full.exists(), "{path} is not in the staging tree");
        std::fs::write(&full, bytes)
            .unwrap_or_else(|error| panic!("cannot rewrite {}: {error}", full.display()));
    }
    index
        .files
        .retain(|file| !options.drop_from_index.contains(&file.path));
    for (path, mode) in &options.index_mode_lies {
        let row = index
            .files
            .iter_mut()
            .find(|file| &file.path == path)
            .unwrap_or_else(|| panic!("{path} has no index row to rewrite"));
        row.mode = *mode;
    }
    for (path, size) in &options.index_size_lies {
        let row = index
            .files
            .iter_mut()
            .find(|file| &file.path == path)
            .unwrap_or_else(|| panic!("{path} has no index row to rewrite"));
        row.size = *size;
    }
    for path in &options.ghost_index_rows {
        index.files.push(IndexFile {
            path: path.clone(),
            size: 0,
            mode: 0o644,
            sha256: "0".repeat(64),
            category: ginary::assemble::Category::Other,
        });
    }
    // An appended entry the test asked to be accounted for: the row carries
    // the digest of the bytes that are really in the archive, so the payload
    // and the index agree about a file neither of them should hold.
    for entry in &options.appended {
        if entry.indexed {
            index.files.push(IndexFile {
                path: entry.name.clone(),
                size: entry.bytes.len() as u64,
                mode: 0o644,
                sha256: hex::encode(sha256(&entry.bytes)),
                category: ginary::assemble::Category::Other,
            });
        }
    }
    index
        .files
        .sort_by(|left, right| left.path.cmp(&right.path));

    let payload = compress(&tar_bytes(
        &staging,
        &manifest,
        &index,
        &listing.files,
        &options.appended,
    ));
    let stub = std::fs::read(env!("CARGO_BIN_EXE_ginary"))
        .unwrap_or_else(|error| panic!("cannot read the ginary binary: {error}"));
    let trailer = Trailer {
        payload_offset: stub.len() as u64,
        payload_len: payload.len() as u64,
        payload_sha256: sha256(&payload),
    };

    let path = dir.join(&manifest.app);
    let mut bytes = stub;
    bytes.extend_from_slice(&payload);
    bytes.extend_from_slice(&trailer.to_bytes());
    write_executable(&path, &bytes);

    for name in ["home", "xdg", "emptybin"] {
        std::fs::create_dir_all(dir.join(name))
            .unwrap_or_else(|error| panic!("cannot create {name}: {error}"));
    }

    Repacked {
        dir: dir.to_path_buf(),
        path,
        manifest,
        index,
        trailer,
    }
}

/// The tar archive: the manifest, the index, then every staged file.
///
/// The files come from the *listing* rather than from the index, so an index
/// row this builder invented does not become a tar entry and a row it deleted
/// does not take the file with it.
fn tar_bytes(
    staging: &Path,
    manifest: &Manifest,
    index: &Index,
    files: &[ginary::assemble::StagedFile],
    appended: &[AppendedEntry],
) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    append(
        &mut builder,
        ginary::manifest::MANIFEST_NAME,
        0o644,
        &json(manifest),
    );
    append(
        &mut builder,
        ginary::manifest::INDEX_NAME,
        0o644,
        &json(index),
    );
    for file in files {
        let full = staging.join(&file.path);
        let bytes = std::fs::read(&full)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", full.display()));
        append(&mut builder, &file.path, file.mode, &bytes);
    }
    for entry in appended {
        append_raw(&mut builder, entry);
    }
    builder
        .into_inner()
        .unwrap_or_else(|error| panic!("cannot finish the archive: {error}"))
}

/// Appends one regular file with a deterministic header.
fn append(builder: &mut tar::Builder<Vec<u8>>, name: &str, mode: u32, data: &[u8]) {
    let mut header = tar::Header::new_ustar();
    header
        .set_path(name)
        .unwrap_or_else(|error| panic!("cannot set the tar path {name}: {error}"));
    header.set_size(data.len() as u64);
    header.set_mode(mode);
    header.set_mtime(0);
    header.set_uid(0);
    header.set_gid(0);
    header.set_entry_type(tar::EntryType::Regular);
    header.set_cksum();
    builder
        .append(&header, data)
        .unwrap_or_else(|error| panic!("cannot append {name}: {error}"));
}

/// Appends one entry exactly as it was described, whatever the format says.
fn append_raw(builder: &mut tar::Builder<Vec<u8>>, entry: &AppendedEntry) {
    let mut header = tar::Header::new_ustar();
    let laid = if entry.absolute {
        header.set_path_absolute(&entry.name)
    } else {
        header.set_path(&entry.name)
    };
    laid.unwrap_or_else(|error| panic!("cannot set the tar path {}: {error}", entry.name));
    header.set_size(entry.bytes.len() as u64);
    header.set_mode(if entry.kind == tar::EntryType::Directory {
        0o755
    } else {
        0o644
    });
    header.set_mtime(0);
    header.set_uid(0);
    header.set_gid(0);
    header.set_entry_type(entry.kind);
    if let Some(target) = &entry.link {
        header
            .set_link_name(target)
            .unwrap_or_else(|error| panic!("cannot set the link target {target}: {error}"));
    }
    header.set_cksum();
    builder
        .append(&header, entry.bytes.as_slice())
        .unwrap_or_else(|error| panic!("cannot append {}: {error}", entry.name));
}

/// Serialises a value the way `payload::pack` serialises the front matter.
fn json(value: &impl serde::Serialize) -> Vec<u8> {
    let mut bytes = serde_json::to_vec_pretty(value)
        .unwrap_or_else(|error| panic!("cannot serialise the front matter: {error}"));
    bytes.push(b'\n');
    bytes
}

/// Compresses the archive as one zstd stream.
fn compress(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = zstd::stream::Encoder::new(Vec::new(), LEVEL)
        .unwrap_or_else(|error| panic!("cannot start the encoder: {error}"));
    encoder
        .write_all(bytes)
        .unwrap_or_else(|error| panic!("cannot compress the payload: {error}"));
    encoder
        .finish()
        .unwrap_or_else(|error| panic!("cannot finish the encoder: {error}"))
}

/// The SHA-256 of the payload bytes.
fn sha256(bytes: &[u8]) -> [u8; 32] {
    use sha2::{Digest as _, Sha256};
    Sha256::digest(bytes).into()
}

fn write_executable(path: &Path, bytes: &[u8]) {
    let temporary = path.with_extension("writing");
    std::fs::write(&temporary, bytes)
        .unwrap_or_else(|error| panic!("cannot write {}: {error}", temporary.display()));
    // Only the chmod is unix; the write and the rename are the same everywhere.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o755))
            .unwrap_or_else(|error| panic!("cannot chmod {}: {error}", temporary.display()));
    }
    std::fs::rename(&temporary, path)
        .unwrap_or_else(|error| panic!("cannot rename onto {}: {error}", path.display()));
}

// ------------------------------------------------------- the real ELF --

/// The committed real ELF fixture a linker wrote: a genuine `x86_64` Linux
/// object whatever host reads it.
///
/// This used to be *this test run's own binary* (`current_exe()`), which is a
/// real ELF only when the host is Linux — on Windows it is a PE and on macOS a
/// Mach-O, so `elf::inspect_bytes` refused it and every "plant a real ELF"
/// test saw an empty object table. The fixture's machine comes from the file,
/// not from the host, so these tests read the same object on every runner. See
/// `tests/fixtures/elf/README.md` and `docs/dev/log/E9.md`.
pub fn test_binary() -> Vec<u8> {
    // Read straight from disk rather than through `common::native`, which is
    // gated behind the `cli` feature (it leans on `ginary::elf`): `repack` is
    // also compiled into the `--no-default-features` launcher-side test
    // binaries, and this helper has to build there too.
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/elf/inet_gethost-x86_64-linux-gnu");
    std::fs::read(&path)
        .unwrap_or_else(|error| panic!("cannot read the ELF fixture {}: {error}", path.display()))
}

/// The architecture [`test_binary`] is built for, read from the file's own
/// `e_machine` rather than assumed from the host.
///
/// Reads the two `e_machine` bytes directly rather than through
/// `ginary::elf`, for the same `--no-default-features` reason [`test_binary`]
/// reads the file directly.
///
/// # Panics
///
/// If the fixture is too short to be an ELF header.
pub fn native_arch() -> Arch {
    let bytes = test_binary();
    assert!(
        bytes.len() > E_MACHINE_OFFSET + 1,
        "the ELF fixture is longer than its header"
    );
    let machine = u16::from_le_bytes([bytes[E_MACHINE_OFFSET], bytes[E_MACHINE_OFFSET + 1]]);
    match machine {
        EM_AARCH64 => Arch::Aarch64,
        _ => Arch::X86_64,
    }
}

/// The machine string [`test_binary`] carries, e.g. `x86_64`.
pub fn native_machine() -> String {
    native_arch().as_str().to_owned()
}

/// A Linux target for [`test_binary`]'s own architecture.
pub fn native_target() -> Target {
    Target::new(Os::Linux, native_arch(), Libc::Gnu)
}

/// Staging options that put a real ELF at [`NATIVE_PATH`].
pub fn with_native_object() -> ArtifactOptions {
    ArtifactOptions {
        extra_files: vec![(
            NATIVE_PATH.to_owned(),
            0o755,
            test_binary(),
            ginary::assemble::Category::Priv,
        )],
        ..ArtifactOptions::default()
    }
}

/// The same ELF with its `e_machine` rewritten.
///
/// Two bytes of the header decide which machine a loader will refuse the file
/// on, and rewriting them is how a test on one architecture produces a binary
/// for another without a cross toolchain. Nothing else in the file is touched,
/// so it parses exactly as it did.
///
/// # Panics
///
/// If `bytes` is too short to be an ELF header.
pub fn patch_elf_machine(bytes: &[u8], machine: u16) -> Vec<u8> {
    let mut patched = bytes.to_vec();
    assert!(
        patched.len() > E_MACHINE_OFFSET + 1,
        "an ELF header is longer than this"
    );
    patched[E_MACHINE_OFFSET..E_MACHINE_OFFSET + 2].copy_from_slice(&machine.to_le_bytes());
    patched
}

/// The `e_machine` [`test_binary`] is *not* — the machine to rewrite its
/// header to so it becomes a foreign object, read from the fixture rather than
/// assumed from the host.
pub fn foreign_machine() -> u16 {
    match native_arch() {
        Arch::X86_64 => EM_AARCH64,
        Arch::Aarch64 => EM_X86_64,
    }
}

/// A Linux target for the architecture [`test_binary`] is not.
pub fn foreign_target() -> Target {
    let arch = match native_arch() {
        Arch::X86_64 => Arch::Aarch64,
        Arch::Aarch64 => Arch::X86_64,
    };
    Target::new(Os::Linux, arch, Libc::Gnu)
}
