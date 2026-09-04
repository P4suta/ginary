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
//! [`scan`] is the reader, and it has to be able to read a file that holds a
//! copy of the scanner. Two rules keep it honest, and a Windows build found
//! that either one alone is not enough.
//!
//! **The needle is never stored.** What this module stores is one masked image:
//! the needle with every byte masked, which spells nothing. [`needle`] unmasks
//! it at run time, so the compiled scanner's own read-only data holds the
//! fifteen bytes it looks for exactly once — inside [`GINARY_STUB_ID`], where
//! they belong. The obvious weaker version of this, storing the name in two
//! halves and joining them, is what shipped until E10, and a linker laid the
//! two halves side by side in the Windows `ginary.exe`: two constants a linker
//! is free to place adjacently are, between them, one contiguous needle. See
//! [`needle_fragments`] and
//! `tests/regressions/e10_the_needle_halves_were_stored_side_by_side.rs`.
//!
//! **A scan counts records, not hits.** Fifteen bytes of data are not an
//! identity. [`candidates`] is every offset the needle appears at and
//! [`records`] is every offset a whole, terminated, zero-padded, four-field
//! record begins at; [`scan`] counts the second. Exactly one record is a stub;
//! none is [`StubIdError::NotAStub`] (or the typed reason the one candidate is
//! not a record) and more than one is [`StubIdError::Ambiguous`], because a
//! file with two identities has none.

use std::fmt;

use crate::target::{ParseTargetError, Target};

/// The length of the embedded marker, in bytes.
pub const MARKER_LEN: usize = 128;

/// The length of the needle: the record's name and the NUL that closes it.
pub const NEEDLE_LEN: usize = 15;

/// The byte [`NEEDLE_IMAGE`] is masked with.
///
/// Any non-zero value does the job; this one turns every character of the name
/// into a byte no ASCII text carries, so the stored image does not read as a
/// truncated word either.
const NEEDLE_MASK: u8 = 0x5a;

/// The one image of the needle this build stores: every byte of the name
/// exclusive-ored with [`NEEDLE_MASK`].
///
/// Masked rather than split, because splitting is not a defence. Two constants
/// are two objects and a linker may lay two objects out adjacently; a masked
/// one is fifteen bytes that spell the needle in no arrangement whatsoever, so
/// no linker has a choice to get wrong.
static NEEDLE_IMAGE: [u8; NEEDLE_LEN] = mask(needle_plain());

/// The needle itself, written a byte at a time.
///
/// Not a string literal: a literal is an image the compiler may place in the
/// binary, and the whole point is that this build carries exactly one image of
/// these fifteen bytes. Every caller is a constant evaluation — the
/// [`NEEDLE_IMAGE`] initialiser and [`marker`] — so the array exists while the
/// crate is compiled and never afterwards.
const fn needle_plain() -> [u8; NEEDLE_LEN] {
    [
        b'G', b'I', b'N', b'A', b'R', b'Y', b'-', b'S', b'T', b'U', b'B', b'-', b'I', b'D', 0,
    ]
}

/// Exclusive-ors every byte with [`NEEDLE_MASK`]. Its own inverse, which is
/// why one function both stores and reads the needle.
const fn mask(mut bytes: [u8; NEEDLE_LEN]) -> [u8; NEEDLE_LEN] {
    let mut index = 0;
    while index < NEEDLE_LEN {
        bytes[index] ^= NEEDLE_MASK;
        index += 1;
    }
    bytes
}

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
/// number and the record's name is [`needle_plain`] rather than a literal:
/// everything here is evaluated while the crate is compiled, and the array it
/// returns is the one the linker writes into the binary.
///
/// # Panics
///
/// At compile time, and only if the four fields do not fit in [`MARKER_LEN`]
/// bytes. There is no run-time path into this function: its only caller is the
/// initialiser of a `static`.
const fn marker() -> [u8; MARKER_LEN] {
    let buf = [0u8; MARKER_LEN];
    let name = needle_plain();
    let (buf, at) = put(buf, 0, &name);
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

/// The needle, unmasked rather than stored.
///
/// The stored image is masked, so the bytes a scan looks for do not appear
/// contiguously anywhere in the scanner's own read-only data: a ginary that
/// held them would find a second identity in every copy of itself, and every
/// artifact built from a stub embeds one ginary inside another.
///
/// The mask is read through [`std::hint::black_box`] so that the optimiser may
/// not fold the loop back into a plain fifteen-byte constant — which would put
/// the image back in the binary and undo the whole arrangement. It costs one
/// instruction on a path that is taken once per scan.
pub fn needle() -> Vec<u8> {
    let mask = std::hint::black_box(NEEDLE_MASK);
    NEEDLE_IMAGE.iter().map(|byte| byte ^ mask).collect()
}

/// The byte images this build stores the needle in.
///
/// One image, and a masked one. The accessor exists so that
/// the invariant can be *checked* rather than argued — no two of the images a
/// build stores may be concatenable into the needle, in any order — because
/// the arrangement it replaced could not pass that check.
///
/// Until E10 this module stored the name in two halves and joined them at run
/// time. That keeps the needle out of any single constant and does nothing
/// about the linker: two constants are two objects, a linker may lay two
/// objects out adjacently, and the Windows `ginary.exe` of
/// <https://github.com/P4suta/ginary/actions/runs/33739517757> held
/// `GINARY-STUB` immediately followed by `-ID\0` at an address no code in this
/// crate chose. `stubid::scan` reported two identity markers in a file that
/// carries one. See
/// `tests/regressions/e10_the_needle_halves_were_stored_side_by_side.rs`.
pub fn needle_fragments() -> Vec<&'static [u8]> {
    vec![&NEEDLE_IMAGE]
}

/// Every offset in `bytes` at which the needle appears, whether or not a whole
/// identity record follows it.
///
/// A hit is a *candidate*: the fifteen bytes that open a marker can also be
/// unrelated data a linker happened to place there, or a fragment of a payload
/// that carries a ginary of its own. [`records`] is the stricter question.
pub fn candidates(bytes: &[u8]) -> Vec<usize> {
    hits(bytes, &needle())
}

/// Every offset in `bytes` at which a whole, well-formed identity record
/// begins: [`MARKER_LEN`] bytes, opened by the needle, carrying a terminated
/// UTF-8 body of the four known fields and zero padding after it.
///
/// This is what "the file carries one identity" means. A stray needle is not
/// an identity, so it is not counted as one, and a file that holds one record
/// and one stray hit is a stub rather than an ambiguity.
pub fn records(bytes: &[u8]) -> Vec<usize> {
    let needle = needle();
    hits(bytes, &needle)
        .into_iter()
        .filter(|offset| parse_at(bytes, *offset, needle.len()).is_ok())
        .collect()
}

/// Every offset in `bytes` at which `needle` occurs.
///
/// `windows` yields nothing for a buffer shorter than the needle, which is the
/// "not a stub" answer a six-byte file deserves.
fn hits(bytes: &[u8], needle: &[u8]) -> Vec<usize> {
    bytes
        .windows(needle.len())
        .enumerate()
        .filter(|(_, window)| *window == needle)
        .map(|(offset, _)| offset)
        .collect()
}

/// Reads the identity marker out of a whole file's bytes.
///
/// The needle is unmasked here rather than stored, so that a binary holding
/// this function does not itself match, and what is counted is whole
/// [`records`] rather than needle [`candidates`]: exactly one record is
/// required.
///
/// # Errors
///
/// [`StubIdError::NotAStub`] when the needle is nowhere in `bytes`,
/// [`StubIdError::Ambiguous`] when more than one whole record is there, and
/// one of the typed field errors when the needle is there and no candidate is
/// a record — the reason the *first* candidate is not one, which for a file
/// carrying a single malformed marker is the reason that marker is malformed.
pub fn scan(bytes: &[u8]) -> Result<StubId, StubIdError> {
    let needle = needle();
    let candidates = hits(bytes, &needle);
    let Some(&first) = candidates.first() else {
        return Err(StubIdError::NotAStub);
    };
    let records: Vec<usize> = candidates
        .iter()
        .copied()
        .filter(|offset| parse_at(bytes, *offset, needle.len()).is_ok())
        .collect();
    match records.as_slice() {
        // No candidate is a whole record. The file is not a stub, and the
        // useful answer is why the first hit is not one rather than a bare
        // "no marker": a truncated, mistyped or badly padded marker is a
        // ginary build gone wrong, not a file that was never ginary.
        [] => parse_at(bytes, first, needle.len()),
        [only] => parse_at(bytes, *only, needle.len()),
        many => Err(StubIdError::Ambiguous { count: many.len() }),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The marker this build renders at compile time is the same array the
    /// run-time `const fn` produces, and it reads back through [`scan`] as this
    /// build's own identity.
    ///
    /// Calling `marker` at run time is what exercises the digit-writing of
    /// `put_u32` and the byte copy of `put` under coverage; the round trip is
    /// the assertion that the number field in particular is rendered correctly,
    /// since a wrong `f` value would not parse back to
    /// [`crate::manifest::FORMAT_VERSION`].
    #[test]
    fn the_rendered_marker_round_trips_to_this_builds_identity() {
        let rendered = marker();
        assert_eq!(
            rendered, GINARY_STUB_ID,
            "the run-time marker must equal the linked static"
        );

        let Ok(id) = scan(&rendered) else {
            panic!("this build's own marker must scan as exactly one stub");
        };
        assert_eq!(
            id.version,
            env!("CARGO_PKG_VERSION"),
            "the v field is this ginary's version"
        );
        assert_eq!(
            id.format_version,
            crate::manifest::FORMAT_VERSION,
            "the f field is the number put_u32 wrote, read back"
        );
        assert_eq!(
            id.target.name(),
            TARGET_NAME,
            "the t field is this build's target"
        );
        assert_eq!(
            id.flavor.as_str(),
            FLAVOR_NAME,
            "the k field is this build's flavor"
        );
    }

    /// A format version with more than one decimal digit is written whole, so
    /// the number-writing path is exercised past its single-digit case.
    #[test]
    fn a_multi_digit_format_version_is_rendered_and_parses_back() {
        // Drive `put_u32` through its multi-digit loop directly, then read the
        // field back the way `parse_body` does, so a mutation in the digit
        // arithmetic is caught here rather than only at the current one-digit
        // format version.
        let mut buf = [0u8; MARKER_LEN];
        let (written, at) = put_u32(buf, 0, 12345);
        buf = written;
        let text = std::str::from_utf8(&buf[..at]).expect("decimal digits are UTF-8");
        assert_eq!(
            text, "12345",
            "put_u32 writes the whole number, low digit last"
        );
    }
}
