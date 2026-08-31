// SPDX-License-Identifier: MIT OR Apache-2.0
//! The identity marker every ginary binary carries, and the scanner that reads
//! it back out of a file.
//!
//! A stub is a copy of ginary with no payload: the half of the binary a build
//! appends a runtime to. Nothing about a file's *name* says which ginary built
//! it, which target it is for or whether it holds the command line half, so
//! every build embeds [`GINARY_STUB_ID`]: [`MARKER_LEN`] bytes of
//!
//! ```text
//! GINARY-STUB-ID\0v=<version>;t=<target>;f=<format>;k=<flavor>\0<zero padding>
//! ```
//!
//! The name and the body are one array so that a scan finds the whole record
//! at one offset, and the padding is zero so that two builds of the same
//! ginary produce the same bytes.
//!
//! [`scan`] is the reader. It assembles its needle at run time out of
//! [`NEEDLE_HEAD`] and [`NEEDLE_TAIL`], so the scanner's own `.rodata` never
//! holds the needle contiguously and a ginary scanning *itself* — or a
//! `ginary` binary embedded in a test fixture — is not a second hit. Exactly
//! one occurrence is a stub; none is [`StubIdError::NotAStub`] and more than
//! one is [`StubIdError::Ambiguous`], because a file with two identities has
//! none.

use std::fmt;

use crate::target::{ParseTargetError, Target};

/// The length of the embedded marker, in bytes.
pub const MARKER_LEN: usize = 128;

/// The first half of the needle a scan assembles at run time.
pub const NEEDLE_HEAD: &str = "GINARY-STUB";

/// The second half of the needle, ending in the NUL that closes the name.
pub const NEEDLE_TAIL: &str = "-ID\0";

/// The canonical name of the target this build runs on.
///
/// `build.rs` maps Cargo's `TARGET` onto it, because a cross-compiled stub is
/// exactly the case where `std::env::consts` would answer for the wrong
/// machine.
pub const TARGET_NAME: &str = env!("GINARY_TARGET");

/// Which half of ginary this build is, `full` or `stub`.
///
/// `build.rs` derives it from `CARGO_FEATURE_CLI`.
pub const FLAVOR_NAME: &str = env!("GINARY_FLAVOR");

/// The embedded identity marker of this build.
///
/// Rendered at compile time by the private `marker` const fn, so two builds
/// of one ginary for one target produce the same 128 bytes down to the
/// padding.
///
/// `#[used]` keeps the *compiler* from dropping a static nothing reads. It is
/// not enough on its own — a linker that garbage-collects unreferenced
/// sections would still drop it — so `src/main.rs` takes its address through
/// `std::hint::black_box` on the one path every run takes. That is a reference
/// the optimiser may not remove and it costs a single instruction, which is
/// cheaper than the alternative: a stub whose identity the linker deleted is
/// a file `stub::verify` cannot tell from any other executable.
#[used]
pub static GINARY_STUB_ID: [u8; MARKER_LEN] = marker();

/// Renders this build's marker.
///
/// A `const fn` rather than a `concat!`, because the format version is a
/// number and the name of the record is deliberately split: everything here is
/// evaluated while the crate is compiled, and the array it returns is the one
/// the linker writes into the binary.
///
/// # Panics
///
/// At compile time, and only if the four fields do not fit in [`MARKER_LEN`]
/// bytes. There is no run-time path into this function: its only caller is the
/// initialiser of a `static`.
const fn marker() -> [u8; MARKER_LEN] {
    let buf = [0u8; MARKER_LEN];
    let (buf, at) = put(buf, 0, NEEDLE_HEAD.as_bytes());
    let (buf, at) = put(buf, at, NEEDLE_TAIL.as_bytes());
    let (buf, at) = put(buf, at, b"v=");
    let (buf, at) = put(buf, at, env!("CARGO_PKG_VERSION").as_bytes());
    let (buf, at) = put(buf, at, b";t=");
    let (buf, at) = put(buf, at, TARGET_NAME.as_bytes());
    let (buf, at) = put(buf, at, b";f=");
    let (buf, at) = put_u32(buf, at, crate::manifest::FORMAT_VERSION);
    let (buf, at) = put(buf, at, b";k=");
    let (buf, at) = put(buf, at, FLAVOR_NAME.as_bytes());
    // The terminating NUL and the padding are the zero fill the buffer started
    // as; the assertion is what guarantees there is room for the first of them.
    assert!(
        at < MARKER_LEN,
        "the identity marker does not fit in its 128 bytes"
    );
    buf
}

/// Copies `text` into `buf` at `at`, and returns where the next field starts.
const fn put(mut buf: [u8; MARKER_LEN], mut at: usize, text: &[u8]) -> ([u8; MARKER_LEN], usize) {
    let mut index = 0;
    while index < text.len() {
        assert!(
            at < MARKER_LEN,
            "the identity marker does not fit in its 128 bytes"
        );
        buf[at] = text[index];
        at += 1;
        index += 1;
    }
    (buf, at)
}

/// Writes `value` in decimal, the one field of the marker that is a number.
const fn put_u32(
    mut buf: [u8; MARKER_LEN],
    at: usize,
    mut value: u32,
) -> ([u8; MARKER_LEN], usize) {
    let mut digits = 1;
    let mut probe = value;
    while probe >= 10 {
        probe /= 10;
        digits += 1;
    }
    let end = at + digits;
    assert!(
        end <= MARKER_LEN,
        "the identity marker does not fit in its 128 bytes"
    );
    // Written from the back, because the low digit is the one division yields.
    let mut index = end;
    while index > at {
        index -= 1;
        buf[index] = b'0' + (value % 10) as u8;
        value /= 10;
    }
    (buf, end)
}

/// Which half of ginary a binary holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Flavor {
    /// The command line tool: the `cli` feature is on and clap is linked in.
    Full,
    /// The launcher only: built with `--no-default-features`.
    Stub,
}

impl Flavor {
    /// The spelling used in the marker's `k` field.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Stub => "stub",
        }
    }
}

impl fmt::Display for Flavor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The identity a marker records.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StubId {
    /// The `ginary` version that built the file, from `CARGO_PKG_VERSION`.
    pub version: String,
    /// The target the file runs on.
    pub target: Target,
    /// The payload format version the file can read.
    pub format_version: u32,
    /// Whether the file holds the command line half.
    pub flavor: Flavor,
}

/// Why a file's identity marker could not be read.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum StubIdError {
    /// The needle is not in the bytes at all.
    #[error("no ginary identity marker: this file was not built by ginary")]
    NotAStub,
    /// The needle is there more than once.
    ///
    /// A ginary binary carries one marker. Two means the file holds a copy of
    /// another ginary — an artifact whose payload was not compressed, a
    /// concatenation, a test fixture — and a file with two identities has
    /// none.
    #[error("{count} ginary identity markers: exactly one is a stub, so this file is not one")]
    Ambiguous {
        /// How many occurrences were found.
        count: usize,
    },
    /// The marker starts fewer than [`MARKER_LEN`] bytes before the end, or
    /// its text has no terminating NUL.
    #[error("the marker at offset {offset} is not terminated within its 128 bytes")]
    Truncated {
        /// Where the needle was found.
        offset: usize,
    },
    /// The marker's text is not UTF-8.
    #[error("the marker at offset {offset} is not UTF-8")]
    NotUtf8 {
        /// Where the needle was found.
        offset: usize,
    },
    /// One of the four fields is absent.
    #[error("the marker has no `{field}` field; a ginary marker names v, t, f and k")]
    MissingField {
        /// The field that is not there: `v`, `t`, `f` or `k`.
        field: &'static str,
    },
    /// The marker names a field this ginary does not know.
    #[error("the marker names the unknown field `{key}`; a ginary marker names v, t, f and k")]
    UnknownKey {
        /// The key, as it was written.
        key: String,
    },
    /// The `t` field is not a target name.
    #[error("the marker names the target `{name}`, which this ginary has no name for")]
    UnknownTarget {
        /// The target name, as it was written.
        name: String,
        /// What the target parser said.
        #[source]
        source: ParseTargetError,
    },
    /// The `f` field is not a number.
    #[error("the marker's format version `{value}` is not a number")]
    BadFormatVersion {
        /// The value, as it was written.
        value: String,
    },
    /// The `k` field is neither flavor.
    #[error("the marker's flavor `{value}` is neither `full` nor `stub`")]
    UnknownFlavor {
        /// The value, as it was written.
        value: String,
    },
    /// A byte after the terminating NUL is not zero.
    ///
    /// The padding is part of the record: a marker whose tail holds anything
    /// else was written by something other than a ginary build, and two builds
    /// of one ginary would no longer produce the same bytes.
    #[error("the marker at offset {offset} has a non-zero byte at {byte} of its padding")]
    NotPadded {
        /// Where the needle was found.
        offset: usize,
        /// The index within the marker of the first non-zero padding byte.
        byte: usize,
    },
}

/// The four fields of the record, in the order a marker writes them.
const FIELDS: [&str; 4] = ["v", "t", "f", "k"];

/// The needle, assembled rather than stored.
///
/// Two halves joined here, so that the bytes a scan looks for never appear
/// contiguously in the scanner's own `.rodata`: a ginary that held them would
/// find a second identity in every copy of itself, and every artifact built
/// from a stub embeds one ginary inside another.
fn needle() -> Vec<u8> {
    let mut bytes = Vec::with_capacity(NEEDLE_HEAD.len() + NEEDLE_TAIL.len());
    bytes.extend_from_slice(NEEDLE_HEAD.as_bytes());
    bytes.extend_from_slice(NEEDLE_TAIL.as_bytes());
    bytes
}

/// Reads the identity marker out of a whole file's bytes.
///
/// The needle is assembled here rather than stored, so that a binary holding
/// this function does not itself match. Exactly one occurrence is required.
///
/// # Errors
///
/// [`StubIdError::NotAStub`] when there is no marker,
/// [`StubIdError::Ambiguous`] when there is more than one, and one of the
/// typed field errors when the single marker does not parse.
pub fn scan(bytes: &[u8]) -> Result<StubId, StubIdError> {
    let needle = needle();
    // `windows` yields nothing for a buffer shorter than the needle, which is
    // the "not a stub" answer a six-byte file deserves.
    let mut offsets = bytes
        .windows(needle.len())
        .enumerate()
        .filter(|(_, window)| *window == needle.as_slice())
        .map(|(offset, _)| offset);
    let Some(offset) = offsets.next() else {
        return Err(StubIdError::NotAStub);
    };
    let count = 1 + offsets.count();
    if count > 1 {
        return Err(StubIdError::Ambiguous { count });
    }
    parse_at(bytes, offset, needle.len())
}

/// Reads the record whose needle starts at `offset`.
///
/// The gates are the record's own, in the order the bytes are laid out: the
/// whole 128 bytes are there, the body is terminated, it is text, the padding
/// after it is zero, and only then are the four fields read.
fn parse_at(bytes: &[u8], offset: usize, needle_len: usize) -> Result<StubId, StubIdError> {
    let end = offset
        .checked_add(MARKER_LEN)
        .ok_or(StubIdError::Truncated { offset })?;
    let marker = bytes
        .get(offset..end)
        .ok_or(StubIdError::Truncated { offset })?;

    let body = &marker[needle_len..];
    let nul = body
        .iter()
        .position(|byte| *byte == 0)
        .ok_or(StubIdError::Truncated { offset })?;
    let text = std::str::from_utf8(&body[..nul]).map_err(|_| StubIdError::NotUtf8 { offset })?;

    // The padding is part of the record: two builds of one ginary are the same
    // bytes only because everything after the terminating NUL is zero.
    if let Some(index) = body[nul + 1..].iter().position(|byte| *byte != 0) {
        return Err(StubIdError::NotPadded {
            offset,
            byte: needle_len + nul + 1 + index,
        });
    }

    parse_body(text)
}

/// Reads the four fields out of the marker's text.
///
/// Positional, because the marker is written by ginary in one order and
/// nothing else writes one: the field at each place must carry the key that
/// belongs there, so a record that names three fields says which one is
/// missing rather than which one is unexpected, and a fifth field is named as
/// the surplus it is.
fn parse_body(text: &str) -> Result<StubId, StubIdError> {
    let parts: Vec<&str> = text.split(';').collect();
    let mut values: [&str; FIELDS.len()] = [""; FIELDS.len()];
    for (index, field) in FIELDS.iter().enumerate() {
        let part = parts
            .get(index)
            .copied()
            .ok_or(StubIdError::MissingField { field })?;
        let (key, value) = part
            .split_once('=')
            .ok_or(StubIdError::MissingField { field })?;
        if key != *field {
            return Err(StubIdError::MissingField { field });
        }
        values[index] = value;
    }
    if let Some(extra) = parts.get(FIELDS.len()) {
        return Err(StubIdError::UnknownKey {
            key: extra
                .split_once('=')
                .map_or(*extra, |(key, _)| key)
                .to_owned(),
        });
    }

    let target = values[1]
        .parse::<Target>()
        .map_err(|source| StubIdError::UnknownTarget {
            name: values[1].to_owned(),
            source,
        })?;
    let format_version = values[2]
        .parse::<u32>()
        .map_err(|_| StubIdError::BadFormatVersion {
            value: values[2].to_owned(),
        })?;
    let flavor = match values[3] {
        "full" => Flavor::Full,
        "stub" => Flavor::Stub,
        other => {
            return Err(StubIdError::UnknownFlavor {
                value: other.to_owned(),
            });
        }
    };

    Ok(StubId {
        version: values[0].to_owned(),
        target,
        format_version,
        flavor,
    })
}
