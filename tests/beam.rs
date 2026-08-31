// SPDX-License-Identifier: MIT OR Apache-2.0
//! The IFF chunk reader: `ginary::beam`.
//!
//! Two kinds of input, for two different reasons. The hand-built byte strings
//! pin the *grammar* — a zero-length chunk, the four-byte padding, a length
//! field of `u32::MAX`, a header cut in half — where a real file gives no
//! coverage at all because a real compiler never emits any of them. The three
//! modules under `tests/fixtures/beam/` pin the *shape a compiler emits*, which
//! is the half a reader written against its own test data always gets wrong.
//!
//! The third kind of assertion is the never-panic policy every binary parser in
//! this crate is held to. `chunks` is fed random bytes and every truncation of
//! a real module, and a panic anywhere in that space is a defect: the file it
//! reads comes out of a build tree ginary does not control, and a packaging
//! tool that panics on a damaged file has told its user nothing.
// The command line half of the suite: every claim in this file is about a
// module the `cli` feature carries, so a `--no-default-features` build has
// nothing here to run. See `docs/dev/log/C2.md`.
#![cfg(feature = "cli")]

mod common;

use std::path::{Path, PathBuf};

use flate2::Compression;
use flate2::write::GzEncoder;
use ginary::beam::{self, BeamError, CODE_CHUNK, Chunk, DEBUG_INFO_CHUNK, DOCS_CHUNK, LINE_CHUNK};
use proptest::prelude::*;

use crate::common::fake_otp::{DUMMY_BEAM, beam_bytes};

/// The three real modules under `tests/fixtures/beam/`.
const FIXTURES: [&str; 3] = ["gleam@bool.beam", "gleam@list.beam", "gleam@string.beam"];

/// The smallest of the three, small enough to name every chunk offset in.
const SMALL_FIXTURE: &str = "gleam@bool.beam";

/// The largest of the three, used for the truncation sweep.
const LARGE_FIXTURE: &str = "gleam@list.beam";

/// The path of a fixture module.
fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/beam")
        .join(name)
}

/// The bytes of a fixture module.
fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(fixture_path(name))
        .unwrap_or_else(|error| panic!("cannot read the {name} fixture: {error}"))
}

/// A `Chunk`, spelled the way a test writes one.
fn chunk(id: &[u8; 4], offset: usize, len: u32) -> Chunk {
    Chunk {
        id: *id,
        offset,
        len,
    }
}

/// The chunk identifiers of `bytes`, as text, or a panic naming the failure.
fn ids(bytes: &[u8]) -> Vec<String> {
    beam::chunks(bytes)
        .unwrap_or_else(|error| panic!("these bytes should read as a BEAM file: {error}"))
        .iter()
        .map(Chunk::id_str)
        .collect()
}

/// `bytes` wrapped in a gzip member, the way `beam_lib` writes a module it
/// rewrote.
///
/// `zlib:gzip/1` on the Erlang side and this are the same container, which is
/// the whole of what the reader has to unwrap: a module that has been stripped
/// is not an IFF form on disk, and a reader that did not know it would report
/// every module ginary ships as "not a BEAM file".
fn gzipped(bytes: &[u8]) -> Vec<u8> {
    use std::io::Write as _;

    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(bytes)
        .expect("writing into a vector cannot fail");
    encoder.finish().expect("the member closes")
}

#[test]
fn a_form_with_no_chunks_at_all_reads_as_an_empty_table() {
    // A container with nothing in it is a legal container. Calling it a failure
    // would make the reader disagree with the format for no gain.
    let bytes = beam_bytes(&[]);

    assert_eq!(bytes.len(), 12, "a bare form is the twelve-byte header");
    assert_eq!(
        beam::chunks(&bytes).expect("a bare form parses"),
        Vec::<Chunk>::new()
    );
}

#[test]
fn a_hand_built_module_reports_each_chunks_id_data_offset_and_length() {
    // `Code` is five bytes, so the reader has to skip three bytes of padding
    // before `Line`. A reader that added the declared length alone would put
    // `Line` at 25 and read four bytes of rubbish as its identifier.
    let bytes = beam_bytes(&[
        (CODE_CHUNK, b"12345".as_slice()),
        (LINE_CHUNK, b"ab".as_slice()),
    ]);

    assert_eq!(
        beam::chunks(&bytes).expect("a hand-built module parses"),
        vec![chunk(&CODE_CHUNK, 20, 5), chunk(&LINE_CHUNK, 36, 2),]
    );
}

#[test]
fn a_chunks_offset_names_its_data_and_not_its_header() {
    let bytes = beam_bytes(&[(CODE_CHUNK, b"payload!".as_slice())]);
    let chunks = beam::chunks(&bytes).expect("a hand-built module parses");
    let only = chunks.first().expect("one chunk");

    assert_eq!(
        &bytes[only.offset..only.offset + only.len as usize],
        b"payload!",
        "offset + len has to slice the chunk out of the file"
    );
}

#[test]
fn a_zero_length_chunk_is_a_chunk() {
    // Every module a real compiler emits holds a `StrT` of length zero.
    let bytes = beam_bytes(&[(*b"StrT", b"".as_slice()), (CODE_CHUNK, b"x".as_slice())]);

    assert_eq!(
        beam::chunks(&bytes).expect("a module with an empty chunk parses"),
        vec![chunk(b"StrT", 20, 0), chunk(&CODE_CHUNK, 28, 1)]
    );
}

#[test]
fn the_chunks_come_back_in_the_order_the_file_holds_them() {
    let bytes = beam_bytes(&[
        (*b"AtU8", b"a".as_slice()),
        (CODE_CHUNK, b"c".as_slice()),
        (DEBUG_INFO_CHUNK, b"d".as_slice()),
        (LINE_CHUNK, b"l".as_slice()),
    ]);

    assert_eq!(ids(&bytes), ["AtU8", "Code", "Dbgi", "Line"]);
}

#[test]
fn bytes_after_the_end_of_the_form_are_ignored() {
    // IFF says the form's size field bounds the chunks. Trailing bytes are
    // somebody else's business, not a parse error and not a fifteenth chunk.
    let mut bytes = beam_bytes(&[(CODE_CHUNK, b"x".as_slice())]);
    bytes.extend_from_slice(b"trailing rubbish");

    assert_eq!(ids(&bytes), ["Code"]);
}

#[test]
fn a_file_shorter_than_the_iff_header_is_too_short() {
    let bytes = b"FOR1\x00\x00\x00\x04BEA";

    match beam::chunks(bytes) {
        Err(BeamError::TooShort { len }) => assert_eq!(len, 11),
        other => panic!("expected TooShort, got {other:?}"),
    }
}

#[test]
fn an_empty_file_is_too_short_rather_than_an_empty_table() {
    match beam::chunks(b"") {
        Err(BeamError::TooShort { len }) => assert_eq!(len, 0),
        other => panic!("expected TooShort, got {other:?}"),
    }
}

#[test]
fn a_file_that_does_not_start_with_for1_is_not_an_iff_file() {
    let bytes = b"\x7fELF\x02\x01\x01\x00\x00\x00\x00\x00";

    match beam::chunks(bytes) {
        Err(BeamError::NotIff { magic }) => assert_eq!(&magic, b"\x7fELF"),
        other => panic!("expected NotIff, got {other:?}"),
    }
}

#[test]
fn an_iff_file_of_another_form_is_not_a_beam_file() {
    let bytes = b"FOR1\x00\x00\x00\x04AIFF";

    match beam::chunks(bytes) {
        Err(BeamError::NotBeam { form }) => assert_eq!(&form, b"AIFF"),
        other => panic!("expected NotBeam, got {other:?}"),
    }
}

#[test]
fn a_form_that_declares_more_bytes_than_the_file_holds_is_an_overrun() {
    // What every truncated module looks like: the header still says how long
    // the file was supposed to be.
    let mut bytes = beam_bytes(&[(CODE_CHUNK, b"12345678".as_slice())]);
    bytes.truncate(bytes.len() - 4);

    match beam::chunks(&bytes) {
        Err(BeamError::FormOverrun {
            declared,
            available,
        }) => {
            assert_eq!(declared, 20);
            assert_eq!(available, 16);
        }
        other => panic!("expected FormOverrun, got {other:?}"),
    }
}

#[test]
fn a_chunk_header_cut_in_half_is_reported_rather_than_indexed() {
    // The form is honest about its own length and still ends four bytes into a
    // chunk header. Slicing eight bytes here is the classic panic.
    let bytes = b"FOR1\x00\x00\x00\x08BEAMCode";

    match beam::chunks(bytes) {
        Err(BeamError::TruncatedChunkHeader { offset, available }) => {
            assert_eq!(offset, 12);
            assert_eq!(available, 4);
        }
        other => panic!("expected TruncatedChunkHeader, got {other:?}"),
    }
}

#[test]
fn a_chunk_length_of_u32_max_is_reported_rather_than_overflowing() {
    let bytes = b"FOR1\x00\x00\x00\x0cBEAMCode\xff\xff\xff\xff";

    match beam::chunks(bytes) {
        Err(BeamError::ChunkOverrun {
            id,
            offset,
            len,
            available,
        }) => {
            assert_eq!(id, "Code");
            assert_eq!(offset, 20);
            assert_eq!(len, u32::MAX);
            assert_eq!(available, 0);
        }
        other => panic!("expected ChunkOverrun, got {other:?}"),
    }
}

#[test]
fn the_dummy_beam_the_fake_trees_write_is_a_module_with_code_and_line() {
    // The fake OTP and shipment builders write this into every `ebin`, and
    // stripping verifies exactly these two facts about every staged module. A
    // dummy that did not satisfy them would make the verification untestable.
    assert_eq!(ids(DUMMY_BEAM), ["AtU8", "Code", "Line"]);
    assert!(beam::has_chunk(DUMMY_BEAM, &CODE_CHUNK));
    assert!(!beam::has_chunk(DUMMY_BEAM, &DEBUG_INFO_CHUNK));
    assert!(!beam::has_chunk(DUMMY_BEAM, &DOCS_CHUNK));
}

#[test]
fn every_fixture_module_holds_the_chunks_a_real_compiler_emits() {
    for name in FIXTURES {
        let bytes = fixture(name);
        let ids = ids(&bytes);
        for wanted in ["AtU8", "Code", "ExpT", "Line", "Dbgi", "Docs"] {
            assert!(
                ids.contains(&wanted.to_owned()),
                "{name} holds no `{wanted}` chunk: {ids:?}"
            );
        }
    }
}

#[test]
fn the_small_fixtures_chunk_table_is_exactly_this() {
    // Fourteen chunks, in file order, with the data offset and length of each.
    // Written out in full rather than spot-checked: a reader whose padding is
    // wrong gets the first chunk right and every later offset wrong, and only
    // the whole table shows that.
    let bytes = fixture(SMALL_FIXTURE);

    assert_eq!(
        beam::chunks(&bytes).expect("a real module parses"),
        vec![
            chunk(b"AtU8", 20, 162),
            chunk(b"Code", 192, 489),
            chunk(b"StrT", 692, 0),
            chunk(b"ImpT", 700, 76),
            chunk(b"ExpT", 784, 148),
            chunk(b"LitT", 940, 37),
            chunk(b"Meta", 988, 45),
            chunk(b"LocT", 1044, 4),
            chunk(b"Attr", 1056, 39),
            chunk(b"CInf", 1104, 168),
            chunk(b"Dbgi", 1280, 1457),
            chunk(b"Docs", 2748, 1798),
            chunk(b"Line", 4556, 85),
            chunk(b"Type", 4652, 10),
        ]
    );
}

#[test]
fn every_fixture_module_still_carries_the_debug_information_stripping_removes() {
    // The fixtures are unstripped on purpose. One that had been stripped could
    // not show what stripping is for.
    for name in FIXTURES {
        let bytes = fixture(name);
        assert!(
            beam::has_chunk(&bytes, &DEBUG_INFO_CHUNK),
            "{name} has no Dbgi; the fixture is not the unstripped file it must be"
        );
        assert!(beam::has_chunk(&bytes, &DOCS_CHUNK), "{name} has no Docs");
    }
}

#[test]
fn has_chunk_answers_false_for_bytes_that_are_not_a_module() {
    // The question "does this still hold debug information" has an answer for
    // every file, including the ones that are not modules at all.
    assert!(!beam::has_chunk(b"not a beam file at all", &CODE_CHUNK));
    assert!(!beam::has_chunk(&[], &DEBUG_INFO_CHUNK));
}

#[test]
fn chunk_ids_reads_a_module_off_the_disk() {
    let found = beam::chunk_ids(&fixture_path(SMALL_FIXTURE)).expect("a real module parses");

    assert_eq!(
        found,
        [
            "AtU8", "Code", "StrT", "ImpT", "ExpT", "LitT", "Meta", "LocT", "Attr", "CInf", "Dbgi",
            "Docs", "Line", "Type",
        ]
    );
}

#[test]
fn chunk_ids_reports_a_missing_file_as_an_io_error_naming_it() {
    let missing = fixture_path("no_such_module.beam");

    match beam::chunk_ids(&missing) {
        Err(BeamError::Io { path, source }) => {
            assert_eq!(path, missing);
            assert_eq!(source.kind(), std::io::ErrorKind::NotFound);
        }
        other => panic!("expected Io, got {other:?}"),
    }
}

#[test]
fn a_gzipped_module_reads_exactly_like_the_form_inside_it() {
    // What a *stripped* module is on disk. The offsets are offsets into the
    // form the member holds, not into the file, which is the one thing a
    // caller slicing a chunk out has to be able to rely on.
    let form = beam_bytes(&[
        (CODE_CHUNK, b"code".as_slice()),
        (LINE_CHUNK, b"line".as_slice()),
    ]);
    let member = gzipped(&form);

    assert!(beam::is_gzipped(&member), "the fixture is a gzip member");
    assert_ne!(member, form, "the fixture is not the form itself");
    assert_eq!(
        beam::form(&member).expect("the member unwraps").as_ref(),
        form.as_slice()
    );
    assert_eq!(
        beam::chunks(&member).expect("a gzipped module parses"),
        beam::chunks(&form).expect("the form parses")
    );
    assert!(beam::has_chunk(&member, &CODE_CHUNK));
    assert!(!beam::has_chunk(&member, &DEBUG_INFO_CHUNK));
}

#[test]
fn chunk_ids_reads_a_gzipped_module_off_the_disk() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("notify.beam");
    std::fs::write(
        &path,
        gzipped(&beam_bytes(&[
            (CODE_CHUNK, b"code".as_slice()),
            (DOCS_CHUNK, b"docs".as_slice()),
        ])),
    )
    .expect("the module writes");

    assert_eq!(
        beam::chunk_ids(&path).expect("a stripped module lists its chunks"),
        vec!["Code".to_owned(), "Docs".to_owned()]
    );
}

#[test]
fn an_uncompressed_module_is_borrowed_rather_than_copied() {
    // `form` is the hot path of every verification stripping does, and the
    // common case must not cost a copy of every module in the tree.
    let bytes = beam_bytes(&[(CODE_CHUNK, b"code".as_slice())]);

    let form = beam::form(&bytes).expect("a bare form needs no unwrapping");

    assert!(matches!(form, std::borrow::Cow::Borrowed(_)));
}

#[test]
fn a_gzip_member_cut_short_is_reported_rather_than_read_as_garbage() {
    let member = gzipped(&beam_bytes(&[(CODE_CHUNK, b"code".as_slice())]));

    match beam::chunks(&member[..member.len() - 4]) {
        Err(BeamError::NotGzip { message }) => assert!(
            !message.is_empty(),
            "the decompressor's own words are what say which way it broke"
        ),
        other => panic!("expected NotGzip, got {other:?}"),
    }
}

#[test]
fn the_gzip_magic_followed_by_rubbish_is_not_a_module() {
    let mut bytes = beam::GZIP_MAGIC.to_vec();
    bytes.extend_from_slice(&[0u8; 32]);

    assert!(matches!(
        beam::chunks(&bytes),
        Err(BeamError::NotGzip { .. })
    ));
    assert!(!beam::has_chunk(&bytes, &CODE_CHUNK));
}

#[test]
fn a_member_that_expands_past_the_limit_is_refused_rather_than_allocated() {
    // The bytes come from a file ginary did not write. A small member that
    // claims to expand without end must be an error and not the allocation
    // that ends the process.
    let bomb = gzipped(&vec![0u8; beam::MAX_FORM_BYTES + 1]);

    assert!(
        bomb.len() < 128 * 1024,
        "the bomb is small on disk, which is the point: {} bytes",
        bomb.len()
    );
    assert!(matches!(beam::chunks(&bomb), Err(BeamError::FormTooLarge)));
}

#[test]
fn a_member_that_expands_to_exactly_the_limit_is_read() {
    // The boundary the bomb test sits one byte above. A limit that refused the
    // largest legal input would be a defect nobody would find until a module
    // grew.
    let form = beam_bytes(&[(CODE_CHUNK, b"code".as_slice())]);
    let mut padded = form.clone();
    padded.resize(beam::MAX_FORM_BYTES, 0);

    let member = gzipped(&padded);

    assert_eq!(
        beam::chunks(&member).expect("the largest legal form parses"),
        beam::chunks(&form).expect("the form parses")
    );
}

#[test]
fn truncating_a_real_module_at_every_byte_offset_never_panics() {
    // Every prefix of a real file, one per byte. This is the sweep that finds
    // the slice a hand-picked truncation misses.
    let bytes = fixture(LARGE_FIXTURE);

    for end in 0..bytes.len() {
        let _ = beam::chunks(&bytes[..end]);
        let _ = beam::has_chunk(&bytes[..end], &DEBUG_INFO_CHUNK);
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// Arbitrary bytes are a typed error or a chunk table, never a panic.
    #[test]
    fn chunks_never_panics_on_arbitrary_bytes(bytes in prop::collection::vec(any::<u8>(), 0..512)) {
        let _ = beam::chunks(&bytes);
    }

    /// The same, over bytes that start like a BEAM file and then do not.
    #[test]
    fn chunks_never_panics_on_almost_a_beam_file(
        tail in prop::collection::vec(any::<u8>(), 0..256),
    ) {
        let mut bytes = b"FOR1".to_vec();
        bytes.extend_from_slice(&tail);
        let _ = beam::chunks(&bytes);

        let mut beamish = b"FOR1\x00\x00\x00\x04BEAM".to_vec();
        beamish.extend_from_slice(&tail);
        let _ = beam::chunks(&beamish);
    }

    /// The same, over bytes that start like a gzip member and then do not.
    ///
    /// The branch a random vector cannot reach: two bytes of magic is one in
    /// 65536 of the space, and a decodable deflate stream after it is rarer
    /// still, so the prefix is fixed and the tail is arbitrary.
    #[test]
    fn chunks_never_panics_on_almost_a_gzipped_beam_file(
        tail in prop::collection::vec(any::<u8>(), 0..256),
    ) {
        let mut bytes = beam::GZIP_MAGIC.to_vec();
        bytes.extend_from_slice(&tail);
        let _ = beam::chunks(&bytes);
        let _ = beam::form(&bytes);
        let _ = beam::has_chunk(&bytes, &DEBUG_INFO_CHUNK);
    }

    /// `has_chunk` answers rather than raising, whatever it is given.
    #[test]
    fn has_chunk_never_panics_on_arbitrary_bytes(
        bytes in prop::collection::vec(any::<u8>(), 0..512),
    ) {
        let _ = beam::has_chunk(&bytes, &DEBUG_INFO_CHUNK);
    }
}
