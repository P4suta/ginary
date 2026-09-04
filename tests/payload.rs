// SPDX-License-Identifier: MIT OR Apache-2.0
//! The payload: deterministic packing, and an unpacker that is hostile to its
//! input.
//!
//! Every archive in the second half of this file is built by hand, block by
//! block, by `tests/common/payload.rs`. That is deliberate: the `tar` crate
//! refuses to *write* most of what `src/payload.rs` has to refuse to *read*,
//! so an archive holding `../x`, an absolute path, a symlink or a `ustar`
//! prefix cannot be produced by the same library that is being tested with it.
//!
//! Each of those tests asserts two things: the exact error, and that nothing
//! appeared outside the destination directory. A rejection that had already
//! written the file it rejected would satisfy the first assertion alone.

mod common;

use std::io::Read;

use common::payload::{
    CountingReader, RawEntry, RawTar, TYPE_CHAR_DEVICE, TYPE_DIRECTORY, TYPE_FIFO,
    TYPE_GNU_LONG_NAME, TYPE_HARDLINK, TYPE_SYMLINK, recorded_mode, sample_manifest,
    sample_manifest_json, sha256, staging_tree, tree_listing, zstd_bytes,
};
// A mode is a unix idea, and so is the one assertion that reads one.
#[cfg(unix)]
use common::payload::mode_of;
use ginary::manifest::{INDEX_NAME, Index, MANIFEST_NAME};
use ginary::payload::{
    MAX_FRONT_ENTRY_BYTES, PayloadError, pack, read_index, read_manifest, unpack,
};
use proptest::prelude::*;

/// The compression level the tests pack at. Low, because none of them is
/// measuring the compressor.
const LEVEL: i32 = 3;

/// The entry order the format fixes, followed by the five files
/// `staging_tree` writes.
const EXPECTED_ENTRIES: [&str; 7] = [
    "ginary.json",
    "ginary.index.json",
    "bin/no_dot_erlang.boot",
    "erts-17.0.5/bin/erlexec",
    "lib/hello/ebin/hello.app",
    "lib/hello/ebin/hello.beam",
    "lib/hello/priv/greeting.txt",
];

/// Packs the standard staging tree and returns the payload bytes.
fn pack_sample() -> (Vec<u8>, ginary::payload::Packed, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let tree = staging_tree(dir.path());
    let mut out = Vec::new();
    let packed = pack(&tree.root, &sample_manifest(), LEVEL, &mut out).expect("pack the tree");
    (out, packed, dir)
}

/// The tar archive inside a payload.
fn tar_of(payload: &[u8]) -> Vec<u8> {
    zstd::stream::decode_all(payload).expect("the payload is one zstd stream")
}

/// The entry names of an archive, in order.
fn entry_names(archive: &[u8]) -> Vec<String> {
    let mut reader = tar::Archive::new(archive);
    reader
        .entries()
        .expect("entries")
        .map(|entry| {
            entry
                .expect("an entry")
                .path()
                .expect("a path")
                .to_string_lossy()
                .into_owned()
        })
        .collect()
}

/// A valid front matter: entry 0 the manifest, entry 1 an empty index.
fn front() -> RawTar {
    let index = serde_json::to_vec(&Index { files: Vec::new() }).expect("serialise an index");
    RawTar::new()
        .push(RawEntry::file(MANIFEST_NAME, &sample_manifest_json()))
        .push(RawEntry::file(INDEX_NAME, &index))
}

/// A payload from a hand-built archive: the bytes, their length and digest.
fn payload_of(archive: &RawTar) -> (Vec<u8>, u64, [u8; 32]) {
    let bytes = archive.build_zstd(LEVEL);
    let len = bytes.len() as u64;
    let sha = sha256(&bytes);
    (bytes, len, sha)
}

/// A destination directory with a sibling the unpacker must not touch.
struct Destination {
    _dir: tempfile::TempDir,
    outside: std::path::PathBuf,
    dest: std::path::PathBuf,
}

impl Destination {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let outside = dir.path().to_path_buf();
        let dest = outside.join("dest");
        std::fs::create_dir(&dest).expect("create dest");
        std::fs::write(outside.join("sentinel.txt"), b"untouched").expect("sentinel");
        Self {
            _dir: dir,
            outside,
            dest,
        }
    }

    /// Everything under the parent that is not under `dest`.
    fn outside_listing(&self) -> Vec<String> {
        tree_listing(&self.outside)
            .into_iter()
            .filter(|path| !path.starts_with("dest"))
            .collect()
    }

    fn assert_nothing_escaped(&self) {
        assert_eq!(
            self.outside_listing(),
            ["sentinel.txt"],
            "the rejected archive wrote something outside the destination"
        );
    }
}

// ------------------------------------------------------- the fixtures --

/// The malicious archives are only evidence if the `tar` crate agrees they are
/// archives; a header this file wrote wrongly would fail the tests it feeds
/// for the wrong reason. This is the same rule `tests/appfile.rs` follows for
/// the `.app` files `FakeOtp` generates.
#[test]
fn the_hand_built_archives_read_back_as_the_entries_they_were_written_as() {
    assert_eq!(
        entry_names(&front().build()),
        [MANIFEST_NAME, INDEX_NAME],
        "the front matter"
    );

    let long = "../".repeat(40) + "escaped.txt";
    let archive = front()
        .push(RawEntry::special("././@LongLink", TYPE_GNU_LONG_NAME, "").with_data(long.as_bytes()))
        .push(RawEntry::file("placeholder", b"owned"))
        .build();
    assert_eq!(
        entry_names(&archive),
        [MANIFEST_NAME, INDEX_NAME, long.as_str()],
        "a GNU long name replaces the following entry\'s path"
    );

    let with_symlink = front()
        .push(RawEntry::special("link", TYPE_SYMLINK, "/etc/passwd"))
        .build();
    let mut reader = tar::Archive::new(with_symlink.as_slice());
    let kinds: Vec<tar::EntryType> = reader
        .entries()
        .expect("entries")
        .map(|entry| entry.expect("an entry").header().entry_type())
        .collect();
    assert_eq!(
        kinds,
        [
            tar::EntryType::Regular,
            tar::EntryType::Regular,
            tar::EntryType::Symlink,
        ]
    );

    let with_prefix = front()
        .push(RawEntry::file("escaped.txt", b"owned").with_prefix(".."))
        .build();
    assert_eq!(
        entry_names(&with_prefix)[2],
        "../escaped.txt",
        "the `ustar` prefix is joined to the name with a `/`"
    );

    let manifest_bytes = entry_bytes(&front().build(), MANIFEST_NAME).expect("entry 0");
    assert_eq!(manifest_bytes, sample_manifest_json(), "the body survives");
}

// ---------------------------------------------------------------- packing --

#[test]
fn pack_writes_the_manifest_first_the_index_second_and_the_tree_sorted() {
    let (payload, _, _dir) = pack_sample();

    assert_eq!(entry_names(&tar_of(&payload)), EXPECTED_ENTRIES);
}

#[test]
fn pack_leaves_the_staging_listing_out_of_the_payload() {
    let (payload, _, _dir) = pack_sample();

    let names = entry_names(&tar_of(&payload));

    assert!(
        !names.iter().any(|name| name == "ginary.stage.json"),
        "the index supersedes the staging listing, which is not packed: {names:?}"
    );
}

#[test]
fn packed_reports_the_length_and_digest_of_exactly_what_it_wrote() {
    let (payload, packed, _dir) = pack_sample();

    assert_eq!(packed.len, payload.len() as u64);
    assert_eq!(packed.sha256, sha256(&payload));
}

#[test]
fn packing_the_same_tree_and_manifest_twice_produces_the_same_bytes() {
    let first = tempfile::tempdir().expect("tempdir");
    let second = tempfile::tempdir().expect("tempdir");
    let one = staging_tree(first.path());
    let two = staging_tree(second.path());

    let mut left = Vec::new();
    let mut right = Vec::new();
    let packed_left = pack(&one.root, &sample_manifest(), LEVEL, &mut left).expect("pack");
    let packed_right = pack(&two.root, &sample_manifest(), LEVEL, &mut right).expect("pack");

    assert_eq!(left, right, "identical input produces identical bytes");
    assert_eq!(packed_left, packed_right);
}

#[test]
fn the_packed_headers_carry_no_time_and_no_owner() {
    let (payload, _, _dir) = pack_sample();
    let archive = tar_of(&payload);
    let mut reader = tar::Archive::new(archive.as_slice());

    for entry in reader.entries().expect("entries") {
        let entry = entry.expect("an entry");
        let header = entry.header();
        let path = entry.path().expect("a path").to_string_lossy().into_owned();
        assert_eq!(header.mtime().expect("mtime"), 0, "{path} carries an mtime");
        assert_eq!(header.uid().expect("uid"), 0, "{path} carries a uid");
        assert_eq!(header.gid().expect("gid"), 0, "{path} carries a gid");
    }
}

#[test]
fn the_packed_headers_keep_the_mode_the_file_has_on_disk() {
    let (payload, _, _dir) = pack_sample();
    let archive = tar_of(&payload);
    let mut reader = tar::Archive::new(archive.as_slice());
    let modes: Vec<(String, u32)> = reader
        .entries()
        .expect("entries")
        .map(|entry| {
            let entry = entry.expect("an entry");
            let path = entry.path().expect("a path").to_string_lossy().into_owned();
            (path, entry.header().mode().expect("mode") & 0o7777)
        })
        .collect();

    assert_eq!(
        modes
            .iter()
            .find(|(path, _)| path == "erts-17.0.5/bin/erlexec")
            .map(|(_, mode)| *mode),
        Some(recorded_mode(0o755, false)),
        "the execute bit survives packing where the filesystem has one: {modes:?}"
    );
    assert_eq!(
        modes
            .iter()
            .find(|(path, _)| path == "lib/hello/priv/greeting.txt")
            .map(|(_, mode)| *mode),
        Some(recorded_mode(0o644, false)),
        "and a plain file does not gain one: {modes:?}"
    );
}

#[test]
fn pack_refuses_a_staging_root_that_holds_no_listing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let tree = staging_tree(dir.path());
    std::fs::remove_file(tree.root.join("ginary.stage.json")).expect("remove the listing");
    let mut out = Vec::new();

    let error = pack(&tree.root, &sample_manifest(), LEVEL, &mut out)
        .expect_err("the index has nowhere to get its categories from");

    match error {
        PayloadError::Listing { path, .. } => {
            assert_eq!(path, tree.root.join("ginary.stage.json"));
        }
        other => panic!("expected PayloadError::Listing, got {other:?}"),
    }
}

#[test]
fn pack_refuses_a_staging_root_that_holds_a_file_the_listing_does_not_name() {
    let dir = tempfile::tempdir().expect("tempdir");
    let tree = staging_tree(dir.path());
    std::fs::write(tree.root.join("lib/hello/ebin/stray.beam"), b"FOR1").expect("write a stray");
    let mut out = Vec::new();

    let error = pack(&tree.root, &sample_manifest(), LEVEL, &mut out)
        .expect_err("a file the index does not describe is neither packed nor dropped");

    match error {
        PayloadError::Unlisted { path, listing } => {
            assert_eq!(path, "lib/hello/ebin/stray.beam");
            assert_eq!(listing, "ginary.stage.json");
        }
        other => panic!("expected Unlisted, got {other:?}"),
    }
}

#[test]
fn the_staging_listing_is_the_one_file_pack_does_not_need_named() {
    let dir = tempfile::tempdir().expect("tempdir");
    let tree = staging_tree(dir.path());
    assert!(
        !tree.paths().iter().any(|path| path == "ginary.stage.json"),
        "the listing does not name itself, which is what makes this the exemption"
    );

    let mut out = Vec::new();
    pack(&tree.root, &sample_manifest(), LEVEL, &mut out)
        .expect("the listing itself is the one unlisted file the tree may hold");
}

#[test]
fn pack_refuses_a_staging_listing_that_is_not_the_json_it_wrote() {
    let dir = tempfile::tempdir().expect("tempdir");
    let tree = staging_tree(dir.path());
    let listing = tree.root.join("ginary.stage.json");
    std::fs::write(&listing, b"{not json").expect("truncate the listing to rubbish");
    let mut out = Vec::new();

    let error = pack(&tree.root, &sample_manifest(), LEVEL, &mut out)
        .expect_err("a listing left behind by an interrupted staging run is not a listing");

    match error {
        PayloadError::ListingFormat { path, .. } => assert_eq!(path, listing),
        other => panic!("expected ListingFormat, got {other:?}"),
    }
}

// -------------------------------------------------------------- unpacking --

#[test]
fn a_packed_tree_unpacks_to_the_same_bytes_and_the_same_modes() {
    let source = tempfile::tempdir().expect("tempdir");
    let tree = staging_tree(source.path());
    let mut payload = Vec::new();
    let packed = pack(&tree.root, &sample_manifest(), LEVEL, &mut payload).expect("pack");
    let destination = Destination::new();

    unpack(
        payload.as_slice(),
        packed.len,
        &packed.sha256,
        &destination.dest,
    )
    .expect("the payload this ginary wrote unpacks");

    for file in tree.files() {
        let extracted = destination.dest.join(&file.path);
        assert_eq!(
            std::fs::read(&extracted).expect("read the extracted file"),
            std::fs::read(tree.root.join(&file.path)).expect("read the staged file"),
            "{} does not hold the bytes it was packed from",
            file.path
        );
        #[cfg(unix)]
        assert_eq!(
            mode_of(&extracted),
            file.mode,
            "{} lost its mode",
            file.path
        );
    }
}

#[test]
fn unpack_returns_the_manifest_of_the_first_entry() {
    let source = tempfile::tempdir().expect("tempdir");
    let tree = staging_tree(source.path());
    let mut payload = Vec::new();
    let packed = pack(&tree.root, &sample_manifest(), LEVEL, &mut payload).expect("pack");
    let destination = Destination::new();

    let manifest = unpack(
        payload.as_slice(),
        packed.len,
        &packed.sha256,
        &destination.dest,
    )
    .expect("unpack");

    assert_eq!(manifest, sample_manifest());
}

#[test]
fn a_payload_that_does_not_hash_to_the_trailer_is_refused_with_both_digests() {
    let (payload, len, sha) = payload_of(&front());
    let mut wrong = sha;
    wrong[0] ^= 0xff;
    let destination = Destination::new();

    let error = unpack(payload.as_slice(), len, &wrong, &destination.dest)
        .expect_err("a digest that does not match is refused");

    match error {
        PayloadError::ChecksumMismatch { expected, actual } => {
            assert_eq!(expected, hex::encode(wrong));
            assert_eq!(actual, hex::encode(sha));
        }
        other => panic!("expected ChecksumMismatch, got {other:?}"),
    }
}

#[test]
fn a_payload_cut_in_half_is_an_error_rather_than_a_panic() {
    let (payload, len, sha) = payload_of(&front());
    let truncated = &payload[..payload.len() / 2];
    let destination = Destination::new();

    let error = unpack(truncated, len, &sha, &destination.dest)
        .expect_err("half a zstd stream is not a payload");

    assert!(
        matches!(error, PayloadError::Io(_)),
        "half a zstd stream ends as a read failure, not a panic: {error:?}"
    );
    destination.assert_nothing_escaped();
}

#[test]
fn an_empty_payload_is_refused_for_having_no_manifest() {
    let (payload, len, sha) = payload_of(&RawTar::new());
    let destination = Destination::new();

    let error = unpack(payload.as_slice(), len, &sha, &destination.dest)
        .expect_err("an archive with no entries has no manifest");

    match error {
        PayloadError::MissingEntry { position, expected } => {
            assert_eq!(position, 0);
            assert_eq!(expected, MANIFEST_NAME);
        }
        other => panic!("expected MissingEntry, got {other:?}"),
    }
    destination.assert_nothing_escaped();
}

#[test]
fn a_first_entry_that_is_not_the_manifest_is_refused() {
    let archive = RawTar::new()
        .push(RawEntry::file("lib/hello/ebin/hello.beam", b"FOR1"))
        .push(RawEntry::file(MANIFEST_NAME, &sample_manifest_json()));
    let (payload, len, sha) = payload_of(&archive);
    let destination = Destination::new();

    let error = unpack(payload.as_slice(), len, &sha, &destination.dest)
        .expect_err("the manifest is not looked for, it is entry 0");

    match error {
        PayloadError::UnexpectedEntry {
            position,
            expected,
            found,
        } => {
            assert_eq!(position, 0);
            assert_eq!(expected, MANIFEST_NAME);
            assert_eq!(found, "lib/hello/ebin/hello.beam");
        }
        other => panic!("expected UnexpectedEntry, got {other:?}"),
    }
    destination.assert_nothing_escaped();
}

#[test]
fn an_entry_that_climbs_out_of_the_destination_is_refused() {
    let archive = front().push(RawEntry::file("../escaped.txt", b"owned"));
    let (payload, len, sha) = payload_of(&archive);
    let destination = Destination::new();

    let error = unpack(payload.as_slice(), len, &sha, &destination.dest)
        .expect_err("a `..` component is refused");

    match error {
        PayloadError::UnsafePath { path } => assert_eq!(path, "../escaped.txt"),
        other => panic!("expected UnsafePath, got {other:?}"),
    }
    destination.assert_nothing_escaped();
}

#[test]
fn an_absolute_entry_path_is_refused() {
    let archive = front().push(RawEntry::file("/escaped.txt", b"owned"));
    let (payload, len, sha) = payload_of(&archive);
    let destination = Destination::new();

    let error = unpack(payload.as_slice(), len, &sha, &destination.dest)
        .expect_err("an absolute path is refused");

    match error {
        PayloadError::UnsafePath { path } => assert_eq!(path, "/escaped.txt"),
        other => panic!("expected UnsafePath, got {other:?}"),
    }
    destination.assert_nothing_escaped();
}

#[test]
fn a_ustar_prefix_that_climbs_out_of_the_destination_is_refused() {
    let archive = front().push(RawEntry::file("escaped.txt", b"owned").with_prefix(".."));
    let (payload, len, sha) = payload_of(&archive);
    let destination = Destination::new();

    let error = unpack(payload.as_slice(), len, &sha, &destination.dest)
        .expect_err("the prefix field is part of the path");

    match error {
        PayloadError::UnsafePath { path } => assert_eq!(path, "../escaped.txt"),
        other => panic!("expected UnsafePath, got {other:?}"),
    }
    destination.assert_nothing_escaped();
}

#[test]
fn a_long_name_entry_that_climbs_out_of_the_destination_is_refused() {
    let long = "../".repeat(40) + "escaped.txt";
    let archive = front()
        .push(RawEntry::special("././@LongLink", TYPE_GNU_LONG_NAME, "").with_data(long.as_bytes()))
        .push(RawEntry::file("placeholder", b"owned"));
    let (payload, len, sha) = payload_of(&archive);
    let destination = Destination::new();

    let error = unpack(payload.as_slice(), len, &sha, &destination.dest)
        .expect_err("a long name is still a path");

    match error {
        PayloadError::UnsafePath { path } => assert_eq!(path, long),
        other => panic!("expected UnsafePath, got {other:?}"),
    }
    destination.assert_nothing_escaped();
}

#[test]
fn a_symlink_entry_is_refused() {
    let archive = front().push(RawEntry::special(
        "lib/hello/priv/link",
        TYPE_SYMLINK,
        "/etc/passwd",
    ));
    let (payload, len, sha) = payload_of(&archive);
    let destination = Destination::new();

    let error = unpack(payload.as_slice(), len, &sha, &destination.dest)
        .expect_err("a symlink is not a file or a directory");

    match error {
        PayloadError::UnsupportedEntry { path, kind } => {
            assert_eq!(path, "lib/hello/priv/link");
            assert_eq!(kind, "symlink");
        }
        other => panic!("expected UnsupportedEntry, got {other:?}"),
    }
    destination.assert_nothing_escaped();
}

#[test]
fn a_hard_link_entry_is_refused() {
    let archive = front().push(RawEntry::special(
        "lib/hello/priv/link",
        TYPE_HARDLINK,
        "ginary.json",
    ));
    let (payload, len, sha) = payload_of(&archive);
    let destination = Destination::new();

    let error = unpack(payload.as_slice(), len, &sha, &destination.dest)
        .expect_err("a hard link is not a file or a directory");

    match error {
        PayloadError::UnsupportedEntry { path, kind } => {
            assert_eq!(path, "lib/hello/priv/link");
            assert_eq!(kind, "hardlink");
        }
        other => panic!("expected UnsupportedEntry, got {other:?}"),
    }
    destination.assert_nothing_escaped();
}

#[test]
fn a_device_entry_is_refused() {
    let archive = front().push(RawEntry::special("dev/null", TYPE_CHAR_DEVICE, ""));
    let (payload, len, sha) = payload_of(&archive);
    let destination = Destination::new();

    let error = unpack(payload.as_slice(), len, &sha, &destination.dest)
        .expect_err("a device node is not a file or a directory");

    match error {
        PayloadError::UnsupportedEntry { path, kind } => {
            assert_eq!(path, "dev/null");
            assert_eq!(kind, "character device");
        }
        other => panic!("expected UnsupportedEntry, got {other:?}"),
    }
    destination.assert_nothing_escaped();
}

#[test]
fn a_fifo_entry_is_refused() {
    let archive = front().push(RawEntry::special("pipe", TYPE_FIFO, ""));
    let (payload, len, sha) = payload_of(&archive);
    let destination = Destination::new();

    let error = unpack(payload.as_slice(), len, &sha, &destination.dest)
        .expect_err("a FIFO is not a file or a directory");

    match error {
        PayloadError::UnsupportedEntry { path, kind } => {
            assert_eq!(path, "pipe");
            assert_eq!(kind, "fifo");
        }
        other => panic!("expected UnsupportedEntry, got {other:?}"),
    }
    destination.assert_nothing_escaped();
}

#[test]
fn a_directory_entry_is_unpacked_rather_than_refused() {
    let archive = front().push(RawEntry::special(
        "lib/hello/priv/empty/",
        TYPE_DIRECTORY,
        "",
    ));
    let (payload, len, sha) = payload_of(&archive);
    let destination = Destination::new();

    unpack(payload.as_slice(), len, &sha, &destination.dest)
        .expect("a directory is one of the two legal entry types");

    assert!(destination.dest.join("lib/hello/priv/empty").is_dir());
}

// ------------------------------------------------------- streaming reads --

#[test]
fn read_manifest_returns_the_first_entry() {
    let (payload, _, _) = payload_of(&front());

    let manifest = read_manifest(payload.as_slice()).expect("the front matter is a manifest");

    assert_eq!(manifest, sample_manifest());
}

#[test]
fn read_manifest_refuses_a_first_entry_that_is_not_the_manifest() {
    let archive = RawTar::new().push(RawEntry::file("notes.txt", b"hello"));
    let (payload, _, _) = payload_of(&archive);

    let error = read_manifest(payload.as_slice()).expect_err("entry 0 is fixed by the format");

    match error {
        PayloadError::UnexpectedEntry {
            position, found, ..
        } => {
            assert_eq!(position, 0);
            assert_eq!(found, "notes.txt");
        }
        other => panic!("expected UnexpectedEntry, got {other:?}"),
    }
}

#[test]
fn read_manifest_stops_after_the_first_entry_of_a_large_payload() {
    // Fifty megabytes of data that does not compress, so the payload really is
    // large: a stream of zeroes would make this test pass without the reader
    // stopping at all.
    let mut noise = vec![0u8; 50 * 1024 * 1024];
    let mut state: u64 = 0x2545_f491_4f6c_dd1d;
    for byte in noise.iter_mut() {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        *byte = (state >> 33) as u8;
    }
    let archive = front().push(RawEntry::file("lib/hello/priv/big.bin", &noise));
    let payload = archive.build_zstd(1);
    assert!(
        payload.len() > 8 * 1024 * 1024,
        "the payload has to be large for this to mean anything: {} bytes",
        payload.len()
    );

    let (reader, counted) = CountingReader::new(payload.as_slice());
    let manifest = read_manifest(reader).expect("the front matter is a manifest");

    assert_eq!(manifest, sample_manifest());
    let consumed = counted.load(std::sync::atomic::Ordering::SeqCst);
    assert!(
        consumed < 1024 * 1024,
        "reading entry 0 consumed {consumed} bytes of a {} byte payload",
        payload.len()
    );
}

#[test]
fn read_index_returns_both_front_entries() {
    let source = tempfile::tempdir().expect("tempdir");
    let tree = staging_tree(source.path());
    let mut payload = Vec::new();
    pack(&tree.root, &sample_manifest(), LEVEL, &mut payload).expect("pack");

    let (manifest, index) = read_index(payload.as_slice()).expect("the front matter");

    assert_eq!(manifest, sample_manifest());
    assert_eq!(
        index
            .files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>(),
        &EXPECTED_ENTRIES[2..],
        "the index names every entry but the two at the front"
    );
}

#[test]
fn read_index_refuses_a_second_entry_that_is_not_the_index() {
    let archive = RawTar::new()
        .push(RawEntry::file(MANIFEST_NAME, &sample_manifest_json()))
        .push(RawEntry::file("lib/hello/ebin/hello.beam", b"FOR1"));
    let (payload, _, _) = payload_of(&archive);

    let error = read_index(payload.as_slice()).expect_err("entry 1 is fixed by the format");

    match error {
        PayloadError::UnexpectedEntry {
            position,
            expected,
            found,
        } => {
            assert_eq!(position, 1);
            assert_eq!(expected, INDEX_NAME);
            assert_eq!(found, "lib/hello/ebin/hello.beam");
        }
        other => panic!("expected UnexpectedEntry, got {other:?}"),
    }
}

#[test]
fn a_front_entry_larger_than_the_limit_is_refused_rather_than_allocated() {
    let oversized = manifest_json_of_exactly(MAX_FRONT_ENTRY_BYTES as usize + 1);
    let archive = RawTar::new().push(RawEntry::file(MANIFEST_NAME, &oversized));
    let (payload, _, _) = payload_of(&archive);
    assert!(
        payload.len() < 64 * 1024,
        "a few kilobytes of zstd claiming eight megabytes of tar entry is the whole point: {} bytes",
        payload.len()
    );

    let error =
        read_manifest(payload.as_slice()).expect_err("entry 0 is read whole, so it is capped");

    match error {
        PayloadError::FrontEntryTooLarge { name, limit } => {
            assert_eq!(name, MANIFEST_NAME);
            assert_eq!(limit, MAX_FRONT_ENTRY_BYTES);
        }
        other => panic!("expected FrontEntryTooLarge, got {other:?}"),
    }
}

#[test]
fn a_front_entry_of_exactly_the_limit_is_read() {
    let exact = manifest_json_of_exactly(MAX_FRONT_ENTRY_BYTES as usize);
    let archive = RawTar::new().push(RawEntry::file(MANIFEST_NAME, &exact));
    let (payload, _, _) = payload_of(&archive);

    let manifest = read_manifest(payload.as_slice()).expect("the limit itself is not over it");

    assert_eq!(
        serde_json::to_vec(&manifest).expect("re-serialise").len(),
        MAX_FRONT_ENTRY_BYTES as usize,
        "the whole entry was read, padding and all, rather than truncated at the limit"
    );
}

#[test]
fn read_index_refuses_a_second_entry_that_is_not_json() {
    let archive = RawTar::new()
        .push(RawEntry::file(MANIFEST_NAME, &sample_manifest_json()))
        .push(RawEntry::file(INDEX_NAME, b"{not json"));
    let (payload, _, _) = payload_of(&archive);

    let error = read_index(payload.as_slice()).expect_err("entry 1 has to parse");

    assert!(
        matches!(error, PayloadError::IndexFormat { .. }),
        "expected IndexFormat, got {error:?}"
    );
}

#[test]
fn a_payload_whose_manifest_is_not_json_is_refused() {
    let archive = RawTar::new().push(RawEntry::file(MANIFEST_NAME, b"{not json"));
    let (payload, _, _) = payload_of(&archive);

    let error = read_manifest(payload.as_slice()).expect_err("entry 0 has to parse");

    assert!(
        matches!(error, PayloadError::ManifestFormat { .. }),
        "expected ManifestFormat, got {error:?}"
    );
}

#[test]
fn a_payload_whose_manifest_is_from_a_newer_format_is_refused() {
    let mut object = serde_json::to_value(sample_manifest()).expect("to value");
    object["format_version"] = serde_json::json!(2);
    let bytes = serde_json::to_vec(&object).expect("serialise");
    let archive = RawTar::new().push(RawEntry::file(MANIFEST_NAME, &bytes));
    let (payload, _, _) = payload_of(&archive);

    let error = read_manifest(payload.as_slice()).expect_err("check_version runs on entry 0");

    assert!(
        matches!(
            error,
            PayloadError::Manifest(ginary::manifest::ManifestError::UnsupportedVersion {
                found: 2,
                ..
            })
        ),
        "expected an unsupported manifest version, got {error:?}"
    );
}

// ------------------------------------------------------------ properties --

proptest! {
    /// The payload is read out of a file somebody else may have edited. Every
    /// public entry point of a binary parser in this crate has this test.
    #[test]
    fn unpack_never_panics_on_arbitrary_bytes(
        bytes in proptest::collection::vec(any::<u8>(), 0..512),
    ) {
        let dir = tempfile::tempdir().expect("tempdir");
        let len = bytes.len() as u64;
        let _ = unpack(bytes.as_slice(), len, &[0u8; 32], dir.path());
    }

    /// The same for the streaming reader, which the launcher reaches first.
    #[test]
    fn read_manifest_never_panics_on_arbitrary_bytes(
        bytes in proptest::collection::vec(any::<u8>(), 0..512),
    ) {
        let _ = read_manifest(bytes.as_slice());
    }

    /// The branch a random vector cannot reach: a well-formed zstd stream
    /// whose contents are not a tar archive.
    #[test]
    fn read_manifest_never_panics_on_a_zstd_stream_of_rubbish(
        bytes in proptest::collection::vec(any::<u8>(), 0..2048),
    ) {
        let payload = zstd_bytes(&bytes, 1);
        let _ = read_manifest(payload.as_slice());
    }
}

/// A manifest whose JSON is exactly `bytes` long, padded through a key this
/// build does not know.
///
/// The padding is a run of one character, so the JSON string's length is its
/// byte length and the total is exact rather than approximately right.
fn manifest_json_of_exactly(bytes: usize) -> Vec<u8> {
    let mut manifest = sample_manifest();
    manifest
        .extra
        .insert("pad".to_owned(), serde_json::json!(""));
    let empty = serde_json::to_vec(&manifest).expect("serialise").len();
    let padding = bytes
        .checked_sub(empty)
        .expect("the target length is longer than the manifest itself");
    manifest
        .extra
        .insert("pad".to_owned(), serde_json::json!("a".repeat(padding)));
    let json = serde_json::to_vec(&manifest).expect("serialise");
    assert_eq!(json.len(), bytes, "the padding is exact");
    json
}

/// The body of one entry, by name.
fn entry_bytes(archive: &[u8], name: &str) -> Option<Vec<u8>> {
    let mut reader = tar::Archive::new(archive);
    for entry in reader.entries().ok()? {
        let mut entry = entry.ok()?;
        if entry.path().ok()?.to_string_lossy() == name {
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).ok()?;
            return Some(bytes);
        }
    }
    None
}
