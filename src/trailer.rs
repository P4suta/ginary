// SPDX-License-Identifier: MIT OR Apache-2.0
//! The 64-byte trailer at the end of a packaged application.
//!
//! The trailer is what `main()` reads before anything else: a file that ends
//! with it is a packaged application and runs the launcher, a file that does
//! not is the ginary command line tool. `docs/format.md` is the normative
//! description of the 64 bytes; this module is its only reader and writer.
//!
//! ```text
//! offset  length  field
//! 0       8       magic, `GINARY\0\x01`, byte 7 is the trailer version
//! 8       8       payload_offset, u64 little-endian
//! 16      8       payload_len, u64 little-endian
//! 24      32      sha256 of exactly payload_len payload bytes
//! 56      8       reserved, must be zero
//! ```
//!
//! The distinction the parser draws is the whole point of the module. Bytes
//! that are not the magic mean *this copy is the tool*, and the answer is
//! [`None`]. Bytes that are the magic and then do not add up mean *this is a
//! broken application*, and the answer is an error, because a damaged artifact
//! must never present ginary's help text instead of saying what is wrong.

use std::fs::File;

/// The eight bytes a trailer starts with.
///
/// Byte 7 is the trailer format version and moves independently of the
/// manifest's `format_version`; it changes only when these 64 bytes are
/// re-laid-out.
pub const MAGIC: [u8; 8] = *b"GINARY\0\x01";

/// The length of the trailer in bytes.
pub const TRAILER_LEN: u64 = 64;

/// Where the payload is, how long it is and what it hashes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Trailer {
    /// Absolute offset of the first payload byte.
    pub payload_offset: u64,
    /// The payload length in bytes.
    pub payload_len: u64,
    /// SHA-256 of exactly [`Trailer::payload_len`] payload bytes.
    pub payload_sha256: [u8; 32],
}

/// Why a file that starts a trailer does not hold one.
///
/// The variants are deliberately not [`Clone`] or [`PartialEq`]: an I/O
/// failure carries its cause, and a test asserts on the variant and its fields
/// rather than on the whole value.
#[derive(Debug, thiserror::Error)]
pub enum TrailerError {
    /// The magic matched but byte 7 is a trailer version this build cannot
    /// read.
    #[error(
        "this artifact carries trailer format version {found}, and this ginary reads version \
         {supported}"
    )]
    UnsupportedVersion {
        /// The version byte the file carries.
        found: u8,
        /// The version byte this build understands.
        supported: u8,
    },
    /// The trailer describes a payload of no bytes.
    ///
    /// Separate from [`TrailerError::Geometry`] because the fault is a
    /// different one: the file is not short, it carries a trailer that says
    /// there is no application inside it, and a message that named a length
    /// would send a reader looking for a truncation that did not happen.
    #[error(
        "the trailer says the payload is zero bytes long, so this artifact carries no \
         application"
    )]
    EmptyPayload,
    /// The reserved bytes at offset 56 are not zero.
    #[error("the trailer's reserved bytes are not zero, so this artifact was not built by ginary")]
    Reserved,
    /// The offset and length do not describe this file.
    #[error(
        "the trailer says the file is {expected} bytes long and it is {actual}, so it was \
         truncated or something was appended to it"
    )]
    Geometry {
        /// `payload_offset + payload_len + TRAILER_LEN`, what the file should
        /// be.
        expected: u64,
        /// The length the file actually has.
        actual: u64,
    },
    /// The last 64 bytes could not be read.
    #[error("reading the last {TRAILER_LEN} bytes of the artifact failed")]
    Io(#[from] std::io::Error),
    /// The file begins with a Mach-O magic and is a fat (universal) binary.
    ///
    /// A fat binary carries more than one architecture, so there is no
    /// single `__GINARY,__payload` section to find without first choosing
    /// which slice is meant; see [`crate::payload::locate`].
    #[error(
        "this is a fat Mach-O carrying more than one architecture, so its payload cannot be \
         located without choosing an architecture"
    )]
    Fat,
    /// The file begins with a Mach-O magic, but its `__GINARY,__payload`
    /// section — when it has one — could not be read.
    ///
    /// The reason travels as text rather than as [`crate::macho::MachoError`]
    /// itself, so that this module, which every target's artifact reads,
    /// stays free of a dependency only macOS needs.
    #[error("the Mach-O `__GINARY,__payload` section could not be read: {message}")]
    Section {
        /// What [`crate::macho`] said.
        message: String,
    },
}

impl Trailer {
    /// Encodes the trailer as the 64 bytes that go at the end of a file.
    pub fn to_bytes(&self) -> [u8; 64] {
        let mut raw = [0u8; 64];
        raw[0..8].copy_from_slice(&MAGIC);
        raw[8..16].copy_from_slice(&self.payload_offset.to_le_bytes());
        raw[16..24].copy_from_slice(&self.payload_len.to_le_bytes());
        raw[24..56].copy_from_slice(&self.payload_sha256);
        raw
    }

    /// Reads the 64 bytes of a file whose total length is `file_len`.
    ///
    /// Returns [`None`] when the bytes do not begin with [`MAGIC`]: that file
    /// is the ginary command line tool rather than a packaged application.
    ///
    /// A payload of no bytes is [`TrailerError::EmptyPayload`] rather than a
    /// geometry failure: such a file is not truncated and its lengths do add
    /// up, it simply carries no application, and that is what the message has
    /// to say.
    ///
    /// # Errors
    ///
    /// [`TrailerError`] when the magic matches and the rest does not agree
    /// with itself or with `file_len`.
    pub fn parse(raw: &[u8; 64], file_len: u64) -> Result<Option<Self>, TrailerError> {
        if raw[0..7] != MAGIC[0..7] {
            return Ok(None);
        }
        if raw[7] != MAGIC[7] {
            return Err(TrailerError::UnsupportedVersion {
                found: raw[7],
                supported: MAGIC[7],
            });
        }
        if raw[56..64] != [0u8; 8] {
            return Err(TrailerError::Reserved);
        }

        let payload_offset = read_u64(raw, 8);
        let payload_len = read_u64(raw, 16);
        let mut payload_sha256 = [0u8; 32];
        payload_sha256.copy_from_slice(&raw[24..56]);

        if payload_len == 0 {
            return Err(TrailerError::EmptyPayload);
        }

        // Saturating rather than wrapping: an offset near `u64::MAX` must not
        // add up to a small number that happens to equal `file_len`.
        let expected = payload_offset
            .saturating_add(payload_len)
            .saturating_add(TRAILER_LEN);
        if expected != file_len {
            return Err(TrailerError::Geometry {
                expected,
                actual: file_len,
            });
        }

        Ok(Some(Self {
            payload_offset,
            payload_len,
            payload_sha256,
        }))
    }

    /// Reads the last 64 bytes of `file`.
    ///
    /// A file shorter than [`TRAILER_LEN`] cannot hold one and is [`None`].
    ///
    /// # Errors
    ///
    /// [`TrailerError`] for the reasons [`Trailer::parse`] gives.
    pub fn read_from(file: &File) -> Result<Option<Self>, TrailerError> {
        let file_len = file.metadata()?.len();
        let Some(offset) = file_len.checked_sub(TRAILER_LEN) else {
            return Ok(None);
        };

        let mut raw = [0u8; 64];
        read_exact_at(file, &mut raw, offset)?;
        Self::parse(&raw, file_len)
    }

    /// The cache directory name for this payload: the first eight bytes of the
    /// digest, in lower-case hexadecimal, so sixteen characters.
    pub fn cache_key(&self) -> String {
        hex::encode(&self.payload_sha256[..8])
    }
}

/// Fills `buffer` from `offset`, without moving the file's own cursor.
///
/// `pread(2)`, which is what the launcher needs here: `main` reads the trailer
/// out of the running executable and then hands the same open file to the
/// payload reader, so a read that moved the cursor would be a read the next
/// stage has to undo. `pub(crate)` rather than private: [`crate::payload::locate`]
/// reuses it for the same reason, to read a Mach-O section's inner trailer
/// without disturbing the file's own cursor.
#[cfg(unix)]
pub(crate) fn read_exact_at(file: &File, buffer: &mut [u8], offset: u64) -> std::io::Result<()> {
    use std::os::unix::fs::FileExt as _;

    file.read_exact_at(buffer, offset)
}

/// Fills `buffer` from `offset`, without moving the file's own cursor.
///
/// The Windows counterpart, `seek_read`, is an overlapped `ReadFile` and gives
/// the same guarantee about the cursor — but it is allowed to answer with fewer
/// bytes than were asked for, so the loop is this function's own rather than
/// the standard library's. A read that answers zero bytes before the buffer is
/// full has hit the end of the file, and 64 bytes that are not there are not a
/// trailer.
#[cfg(windows)]
pub(crate) fn read_exact_at(file: &File, buffer: &mut [u8], offset: u64) -> std::io::Result<()> {
    use std::os::windows::fs::FileExt as _;

    let mut filled = 0usize;
    while filled < buffer.len() {
        let at = offset.saturating_add(filled as u64);
        match file.seek_read(&mut buffer[filled..], at) {
            Ok(0) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "the file ended before its last 64 bytes had been read",
                ));
            }
            Ok(read) => filled = filled.saturating_add(read),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

/// The little-endian `u64` at `offset`.
///
/// `offset` is always one of the two the layout fixes, so the slice is always
/// eight bytes and the conversion cannot fail; it is written as a fold rather
/// than as a `try_into().expect(..)` because nothing on the launcher path may
/// panic.
fn read_u64(raw: &[u8; 64], offset: usize) -> u64 {
    raw[offset..offset + 8]
        .iter()
        .rev()
        .fold(0u64, |value, byte| (value << 8) | u64::from(*byte))
}
