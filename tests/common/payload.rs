// SPDX-License-Identifier: MIT OR Apache-2.0
//! Building the inputs the payload-format tests read.
//!
//! Three of the four helpers here exist because the tests must not be written
//! in terms of the code they test.
//!
//! [`RawTar`] writes tar headers a byte at a time. The `tar` crate refuses to
//! build most of the archives `src/payload.rs` has to reject — that is the
//! point of using it in the product — so an archive holding `../x`, an
//! absolute path, a symlink or a `ustar` prefix is assembled here from the
//! 512-byte blocks up. Nothing but the checksum is computed for the caller.
//!
//! [`staging_tree`] writes the smallest staging root that still has one file
//! of every shape the format cares about: an executable, a plain 0644 file,
//! two `.beam`-shaped files in different categories, and the
//! `ginary.stage.json` that `assemble::stage` leaves behind and `payload::pack`
//! must not pack.
//!
//! [`CountingReader`] answers the only question `payload::read_manifest`
//! promises: how many bytes of the compressed stream did reading one entry
//! actually consume.
//!
//! [`tree_listing`] is what a test compares before and against after, to say
//! that an archive wrote nothing outside the directory it was unpacked into.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use ginary::assemble::{Category, StageListing, StagedApp, StagedFile, StagedSource};
use ginary::manifest::{AppRef, LaunchSpec, Manifest, NativeKind, NativeRef};
use ginary::target::Target;

use sha2::{Digest, Sha256};

/// The tar type flag for a regular file.
pub const TYPE_REGULAR: u8 = b'0';
/// The tar type flag for a hard link.
pub const TYPE_HARDLINK: u8 = b'1';
/// The tar type flag for a symbolic link.
pub const TYPE_SYMLINK: u8 = b'2';
/// The tar type flag for a character device.
pub const TYPE_CHAR_DEVICE: u8 = b'3';
/// The tar type flag for a FIFO.
pub const TYPE_FIFO: u8 = b'6';
/// The tar type flag for a directory.
pub const TYPE_DIRECTORY: u8 = b'5';
/// The tar type flag for a contiguous file, the one type that reads like a
/// regular file and is not one.
pub const TYPE_CONTIGUOUS: u8 = b'7';
/// The GNU type flag whose body is the next entry's long path name.
pub const TYPE_GNU_LONG_NAME: u8 = b'L';

/// One tar entry, described by its bytes rather than by a file on disk.
#[derive(Clone, Debug)]
pub struct RawEntry {
    /// The `name` field, written verbatim and truncated at 100 bytes.
    pub name: String,
    /// The `prefix` field; `ustar` joins it to the name with a `/`.
    pub prefix: String,
    /// The type flag.
    pub typeflag: u8,
    /// The permission bits written into the `mode` field.
    pub mode: u32,
    /// The `linkname` field, for a symlink or a hard link.
    pub linkname: String,
    /// The entry body.
    pub data: Vec<u8>,
}

impl RawEntry {
    /// A regular 0644 file.
    pub fn file(name: &str, data: &[u8]) -> Self {
        Self {
            name: name.to_owned(),
            prefix: String::new(),
            typeflag: TYPE_REGULAR,
            mode: 0o644,
            linkname: String::new(),
            data: data.to_vec(),
        }
    }

    /// A regular file with the mode the caller names.
    pub fn file_with_mode(name: &str, mode: u32, data: &[u8]) -> Self {
        Self {
            mode,
            ..Self::file(name, data)
        }
    }

    /// An entry of any type flag, with an empty body.
    pub fn special(name: &str, typeflag: u8, linkname: &str) -> Self {
        Self {
            name: name.to_owned(),
            prefix: String::new(),
            typeflag,
            mode: 0o644,
            linkname: linkname.to_owned(),
            data: Vec::new(),
        }
    }

    /// The same entry with a body, for a type flag whose body is not file
    /// contents — a GNU long name, for instance.
    pub fn with_data(mut self, data: &[u8]) -> Self {
        self.data = data.to_vec();
        self
    }

    /// The same entry with a `ustar` prefix field set.
    pub fn with_prefix(mut self, prefix: &str) -> Self {
        self.prefix = prefix.to_owned();
        self
    }

    /// The 512-byte header plus the padded body.
    pub fn to_blocks(&self) -> Vec<u8> {
        let mut header = [0u8; 512];
        write_field(&mut header[0..100], self.name.as_bytes());
        write_octal(&mut header[100..108], u64::from(self.mode), 7);
        write_octal(&mut header[108..116], 0, 7);
        write_octal(&mut header[116..124], 0, 7);
        write_octal(&mut header[124..136], self.data.len() as u64, 11);
        write_octal(&mut header[136..148], 0, 11);
        header[148..156].fill(b' ');
        header[156] = self.typeflag;
        write_field(&mut header[157..257], self.linkname.as_bytes());
        header[257..263].copy_from_slice(b"ustar\0");
        header[263..265].copy_from_slice(b"00");
        write_field(&mut header[345..500], self.prefix.as_bytes());

        let sum: u32 = header.iter().map(|byte| u32::from(*byte)).sum();
        write_octal(&mut header[148..154], u64::from(sum), 6);
        header[154] = 0;
        header[155] = b' ';

        let mut blocks = header.to_vec();
        blocks.extend_from_slice(&self.data);
        let padding = (512 - self.data.len() % 512) % 512;
        blocks.extend(std::iter::repeat_n(0u8, padding));
        blocks
    }
}

fn write_field(field: &mut [u8], value: &[u8]) {
    let take = value.len().min(field.len());
    field[..take].copy_from_slice(&value[..take]);
}

fn write_octal(field: &mut [u8], value: u64, digits: usize) {
    let text = format!("{value:0digits$o}");
    let bytes = text.as_bytes();
    let take = bytes.len().min(digits);
    field[..take].copy_from_slice(&bytes[..take]);
    if take < field.len() {
        field[take] = 0;
    }
}

/// A tar archive assembled from [`RawEntry`] values.
#[derive(Clone, Debug, Default)]
pub struct RawTar {
    entries: Vec<RawEntry>,
}

impl RawTar {
    /// An archive with no entries.
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends an entry.
    pub fn push(mut self, entry: RawEntry) -> Self {
        self.entries.push(entry);
        self
    }

    /// The uncompressed archive, terminated by the two zero blocks tar wants.
    pub fn build(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        for entry in &self.entries {
            bytes.extend(entry.to_blocks());
        }
        bytes.extend(std::iter::repeat_n(0u8, 1024));
        bytes
    }

    /// The archive as a zstd stream at `level`.
    pub fn build_zstd(&self, level: i32) -> Vec<u8> {
        zstd_bytes(&self.build(), level)
    }
}

/// Compresses `bytes` as a single zstd stream.
pub fn zstd_bytes(bytes: &[u8], level: i32) -> Vec<u8> {
    let mut encoder = zstd::stream::Encoder::new(Vec::new(), level).expect("zstd encoder");
    encoder.write_all(bytes).expect("compress");
    encoder.finish().expect("finish")
}

/// The SHA-256 of `bytes`.
pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().into()
}

/// The SHA-256 of `bytes`, in lower-case hexadecimal.
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(sha256(bytes))
}

/// A reader that counts the bytes taken out of it.
pub struct CountingReader<R> {
    inner: R,
    read: Arc<AtomicU64>,
}

impl<R: Read> CountingReader<R> {
    /// Wraps `inner`, returning the reader and the counter it updates.
    pub fn new(inner: R) -> (Self, Arc<AtomicU64>) {
        let read = Arc::new(AtomicU64::new(0));
        (
            Self {
                inner,
                read: Arc::clone(&read),
            },
            read,
        )
    }
}

impl<R: Read> Read for CountingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let taken = self.inner.read(buf)?;
        self.read.fetch_add(taken as u64, Ordering::SeqCst);
        Ok(taken)
    }
}

/// A writer that appends to a buffer the test can read afterwards.
///
/// `Diag` takes `Box<dyn Write + Send>` sinks precisely so that a test can pass
/// one of these instead of standard error.
#[derive(Clone, Debug, Default)]
pub struct SharedSink {
    buffer: Arc<std::sync::Mutex<Vec<u8>>>,
}

impl SharedSink {
    /// An empty sink.
    pub fn new() -> Self {
        Self::default()
    }

    /// Everything written to it so far, as text.
    pub fn text(&self) -> String {
        let buffer = self.buffer.lock().expect("sink not poisoned");
        String::from_utf8_lossy(&buffer).into_owned()
    }

    /// The non-empty lines written to it so far.
    pub fn lines(&self) -> Vec<String> {
        self.text()
            .lines()
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect()
    }
}

impl Write for SharedSink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let mut buffer = self.buffer.lock().expect("sink not poisoned");
        buffer.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// What [`staging_tree`] wrote.
#[derive(Clone, Debug)]
pub struct StagingTree {
    /// The staging root itself.
    pub root: PathBuf,
    /// The listing written to `ginary.stage.json`.
    pub listing: StageListing,
}

impl StagingTree {
    /// The files of the listing, sorted by path, as the index will hold them.
    pub fn files(&self) -> &[StagedFile] {
        &self.listing.files
    }

    /// The paths of the listing, sorted, as the payload will hold them.
    pub fn paths(&self) -> Vec<String> {
        self.listing
            .files
            .iter()
            .map(|file| file.path.clone())
            .collect()
    }
}

/// Writes the smallest staging root the payload tests need.
///
/// The tree, in the order a sorted pack must produce:
///
/// ```text
/// bin/no_dot_erlang.boot        0644  boot
/// erts-17.0.5/bin/erlexec       0755  erts_binary
/// lib/hello/ebin/hello.app      0644  app_resource
/// lib/hello/ebin/hello.beam     0644  gleam_beam
/// lib/hello/priv/greeting.txt   0644  priv
/// ginary.stage.json             0644  not listed, and never packed
/// ```
pub fn staging_tree(root: &Path) -> StagingTree {
    let contents: [(&str, u32, &[u8], Category); 5] = [
        (
            "bin/no_dot_erlang.boot",
            0o644,
            b"boot script bytes".as_slice(),
            Category::Boot,
        ),
        (
            "erts-17.0.5/bin/erlexec",
            0o755,
            b"#!/bin/sh\nexit 0\n".as_slice(),
            Category::ErtsBinary,
        ),
        (
            "lib/hello/ebin/hello.app",
            0o644,
            b"{application, hello, [{vsn, \"1.2.3\"}]}.\n".as_slice(),
            Category::AppResource,
        ),
        (
            "lib/hello/ebin/hello.beam",
            0o644,
            b"FOR1\0\0\0\x04BEAM".as_slice(),
            Category::GleamBeam,
        ),
        (
            "lib/hello/priv/greeting.txt",
            0o644,
            b"hello, world\n".as_slice(),
            Category::Priv,
        ),
    ];

    let mut files = Vec::new();
    for (path, mode, data, category) in contents {
        let full = root.join(path);
        std::fs::create_dir_all(full.parent().expect("a parent")).expect("create staging dirs");
        std::fs::write(&full, data).expect("write staged file");
        set_mode(&full, mode);
        files.push(StagedFile {
            path: path.to_owned(),
            size: data.len() as u64,
            mode,
            category,
        });
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));

    let listing = StageListing {
        erts_vsn: "17.0.5".to_owned(),
        otp_release: 29,
        otp_version: "29.0.5".to_owned(),
        apps: vec![StagedApp {
            name: "hello".to_owned(),
            vsn: "1.2.3".to_owned(),
            source: StagedSource::Shipment,
            dir: "lib/hello".to_owned(),
            files: 3,
            bytes: files
                .iter()
                .filter(|file| file.path.starts_with("lib/hello/"))
                .map(|file| file.size)
                .sum(),
        }],
        files,
    };

    let json = serde_json::to_string_pretty(&listing).expect("serialise the listing");
    std::fs::write(root.join("ginary.stage.json"), format!("{json}\n")).expect("write the listing");

    StagingTree {
        root: root.to_path_buf(),
        listing,
    }
}

/// Sets a file's permission bits.
pub fn set_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).expect("set mode");
}

/// A file's permission bits, `st_mode & 0o7777`.
pub fn mode_of(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::symlink_metadata(path)
        .expect("stat")
        .permissions()
        .mode()
        & 0o7777
}

/// Every path under `root`, relative and `/`-separated, sorted.
///
/// A directory is listed with a trailing `/`, so a test can tell an empty
/// directory that was created from one that was not.
pub fn tree_listing(root: &Path) -> Vec<String> {
    let mut found = Vec::new();
    collect(root, root, &mut found);
    found.sort();
    found
}

fn collect(root: &Path, dir: &Path, found: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .expect("under the root")
            .to_string_lossy()
            .replace('\\', "/");
        let meta = std::fs::symlink_metadata(&path).expect("stat");
        if meta.is_dir() {
            found.push(format!("{relative}/"));
            collect(root, &path, found);
        } else {
            found.push(relative);
        }
    }
}

/// The manifest the format tests and the snapshot use.
///
/// Every declared field is filled with something recognisable, so a round trip
/// that drops one is visible rather than merely smaller. `extra` is empty on
/// purpose: this manifest is the one `docs/format.md` prints and the one the
/// snapshot pins, and an unknown key surviving a round trip is asserted by
/// `a_key_this_build_does_not_know_survives_a_round_trip`, which adds one.
pub fn sample_manifest() -> Manifest {
    Manifest {
        format_version: ginary::manifest::FORMAT_VERSION,
        app: "hello".to_owned(),
        app_version: "1.2.3".to_owned(),
        gleam_version: Some("1.18.1".to_owned()),
        otp_release: 29,
        otp_version: "29.0.5".to_owned(),
        erts_version: "17.0.5".to_owned(),
        target: "linux-x86_64-gnu".parse::<Target>().expect("a target"),
        otp_applications: vec![
            AppRef {
                name: "kernel".to_owned(),
                vsn: "11.0.3".to_owned(),
            },
            AppRef {
                name: "stdlib".to_owned(),
                vsn: "7.0.3".to_owned(),
            },
        ],
        gleam_applications: vec!["hello".to_owned()],
        launch: sample_launch(),
        native: vec![NativeRef {
            path: "lib/crypto-5.9.2/priv/lib/crypto.so".to_owned(),
            kind: NativeKind::Elf,
        }],
        created_at: "2026-08-31T00:00:00Z".to_owned(),
        ginary_version: "0.1.0".to_owned(),
        extra: BTreeMap::new(),
    }
}

/// The launch spec of [`sample_manifest`].
pub fn sample_launch() -> LaunchSpec {
    LaunchSpec {
        program: "erlexec".to_owned(),
        bindir: "erts-17.0.5/bin".to_owned(),
        boot: "bin/no_dot_erlang".to_owned(),
        pa: vec!["lib/hello/ebin".to_owned()],
        eval: "'hello@@main':run('hello')".to_owned(),
        erl_flags: vec!["+B".to_owned()],
    }
}

/// [`sample_manifest`] as the JSON bytes entry 0 of a payload holds.
pub fn sample_manifest_json() -> Vec<u8> {
    serde_json::to_vec(&sample_manifest()).expect("serialise the manifest")
}
