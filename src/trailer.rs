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

        let payload_offset = read_u64::<8>(raw);
        let payload_len = read_u64::<16>(raw);
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

/// What a read that ended before the buffer was full has to say.
const SHORT_FILE: &str = "the file ended before its last 64 bytes had been read";

/// One positional read: the single operation [`read_exact_at`] is built on.
///
/// The trait is the seam. A real `pread(2)` under load is allowed to answer
/// with fewer bytes than were asked for and to fail with `EINTR`, and no file
/// on a test machine does either on demand, so the loop that copes with them
/// was never once executed by the suite. `cargo mutants` found it: seven of
/// the trailer shard's eight survivors in run 33969332537 were inside it. A
/// scripted reader is how the two answers get made; see `docs/dev/log/E21.md`.
trait ReadAt {
    /// Reads into `buffer` from `offset` without moving the reader's own
    /// cursor, and answers how many bytes it managed.
    fn read_at(&self, buffer: &mut [u8], offset: u64) -> std::io::Result<usize>;
}

/// `pread(2)`: a positional read that leaves the file's cursor alone.
#[cfg(unix)]
impl ReadAt for File {
    fn read_at(&self, buffer: &mut [u8], offset: u64) -> std::io::Result<usize> {
        std::os::unix::fs::FileExt::read_at(self, buffer, offset)
    }
}

/// `seek_read`, an overlapped `ReadFile`, which gives the same guarantee about
/// the cursor.
#[cfg(windows)]
impl ReadAt for File {
    fn read_at(&self, buffer: &mut [u8], offset: u64) -> std::io::Result<usize> {
        std::os::windows::fs::FileExt::seek_read(self, buffer, offset)
    }
}

/// Fills the whole of `buffer` from `offset`, one positional read at a time.
///
/// Generic over [`ReadAt`] rather than written twice behind a `#[cfg]`, so
/// that one loop serves both platforms and the suite that exercises it runs
/// wherever the tests run.
fn fill_exact_at<R: ReadAt + ?Sized>(
    reader: &R,
    buffer: &mut [u8],
    offset: u64,
) -> std::io::Result<()> {
    // How much of `buffer` has been filled. It only ever grows, and it grows
    // by what a read actually answered, so `filled` and `offset + filled` are
    // the two things the next read is asked for.
    let mut filled = 0usize;
    while filled < buffer.len() {
        // `offset` is a byte position in a file and `filled` is at most 64, so
        // the sum cannot exceed a `u64` in practice; `saturating_add` says so
        // without arithmetic that could overflow on the launcher path.
        let at = offset.saturating_add(filled as u64);
        match reader.read_at(&mut buffer[filled..], at) {
            // A read that answers nothing when there is still buffer to fill
            // has reached the end of the file. A read that answers *less* than
            // it was asked for has not: `pread(2)` is allowed to stop early,
            // and a loop that took that for the end would report a whole
            // artifact as a truncated one.
            Ok(0) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    SHORT_FILE,
                ));
            }
            Ok(read) => filled = filled.saturating_add(read),
            // `EINTR` is the kernel saying a signal arrived while the read was
            // in flight, not that the read failed. Nothing has been consumed,
            // so the same request is made again.
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

/// Fills `buffer` from `offset`, without moving the file's own cursor.
///
/// `pread(2)`, which is what the launcher needs here: `main` reads the trailer
/// out of the running executable and then hands the same open file to the
/// payload reader, so a read that moved the cursor would be a read the next
/// stage has to undo. `pub(crate)` rather than private: [`crate::payload::locate`]
/// reuses it for the same reason, to read a Mach-O section's inner trailer
/// without disturbing the file's own cursor.
///
/// A read that answers zero bytes before the buffer is full has hit the end of
/// the file, and 64 bytes that are not there are not a trailer.
pub(crate) fn read_exact_at(file: &File, buffer: &mut [u8], offset: u64) -> std::io::Result<()> {
    fill_exact_at(file, buffer, offset)
}

/// The little-endian `u64` at `AT`.
///
/// `AT` is a *const* parameter rather than an argument, so that an offset from
/// which eight bytes cannot be read is a compile error rather than an answer.
/// The `zip` below stops at whichever side runs out, and with a runtime offset
/// past 56 that is the slice: the high bytes would read as zero and the
/// function would hand back a quietly wrong number. Skipping is a reported
/// decision or an error, never a default, and here it is neither — so the
/// offset is checked once, at compile time, for every call there will ever be.
///
/// Copied into an array and handed to [`u64::from_le_bytes`] rather than
/// assembled by hand: a `try_into().expect(..)` may not appear on the launcher
/// path, and the fold that replaced it carried its own arithmetic — a `<<` and
/// a `|` — which is one more thing to get right and, as run 33969332537
/// pointed out, one more thing no test can pin. Each `value << 8` leaves the
/// low eight bits zero, so `|` and `^` were interchangeable there and a
/// mutation of one into the other could never be caught. This spelling has no
/// operator to mutate.
///
/// The copy is written as a `zip` rather than as `copy_from_slice`, which
/// panics on a length mismatch, or as an index, which panics out of range.
/// Neither can happen at the two offsets the layout fixes; neither is spelled
/// in a way that could.
fn read_u64<const AT: usize>(raw: &[u8; 64]) -> u64 {
    const {
        assert!(
            AT + 8 <= 64,
            "a trailer field starts within the 64 bytes and has eight of them"
        );
    }
    let mut bytes = [0u8; 8];
    for (byte, source) in bytes.iter_mut().zip(raw.iter().skip(AT)) {
        *byte = *source;
    }
    u64::from_le_bytes(bytes)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::io::ErrorKind;

    use super::{ReadAt, fill_exact_at, read_u64};

    /// What a scripted read answers when the loop reaches it.
    #[derive(Clone, Copy, Debug)]
    enum Answer {
        /// At most this many bytes, taken from the reader's own content.
        Bytes(usize),
        /// This failure, with no byte moved.
        Fails(ErrorKind),
    }

    /// A positional reader that answers a script rather than a disk.
    ///
    /// The two answers a real `pread(2)` gives under load and a file on a test
    /// machine does not: fewer bytes than were asked for, and `EINTR`. It also
    /// records what it was asked, because *where* the second read starts is
    /// the whole of what a short-read loop has to get right — a loop that
    /// re-read from the original offset would fill the buffer with the same
    /// bytes twice and still answer `Ok`.
    struct Scripted {
        content: Vec<u8>,
        answers: RefCell<VecDeque<Answer>>,
        asked: RefCell<Vec<(u64, usize)>>,
    }

    impl Scripted {
        /// A reader over `length` distinct bytes that answers `answers` in
        /// order, and then answers with as much as it can.
        fn new(length: usize, answers: &[Answer]) -> Self {
            Self {
                content: (0..length).map(|index| (index % 251) as u8).collect(),
                answers: RefCell::new(answers.iter().copied().collect()),
                asked: RefCell::new(Vec::new()),
            }
        }

        /// The `(offset, length)` of every read the loop asked for, in order.
        fn asked(&self) -> Vec<(u64, usize)> {
            self.asked.borrow().clone()
        }
    }

    impl ReadAt for Scripted {
        fn read_at(&self, buffer: &mut [u8], offset: u64) -> std::io::Result<usize> {
            self.asked.borrow_mut().push((offset, buffer.len()));
            let answer = self
                .answers
                .borrow_mut()
                .pop_front()
                .unwrap_or(Answer::Bytes(usize::MAX));
            let want = match answer {
                Answer::Fails(kind) => {
                    return Err(std::io::Error::new(kind, "a scripted positional read"));
                }
                Answer::Bytes(want) => want,
            };
            let start = usize::try_from(offset).expect("a fixture offset fits a usize");
            let available = self.content.len().saturating_sub(start);
            let count = want.min(buffer.len()).min(available);
            buffer[..count].copy_from_slice(&self.content[start..start + count]);
            Ok(count)
        }
    }

    #[test]
    fn a_read_shorter_than_the_buffer_is_continued_rather_than_reported() {
        let reader = Scripted::new(64, &[Answer::Bytes(16); 4]);
        let mut buffer = [0u8; 64];

        let result = fill_exact_at(&reader, &mut buffer, 0);

        assert!(
            result.is_ok(),
            "a positional read is allowed to answer with fewer bytes than it was asked for, and \
             an artifact whose last 64 bytes arrive in four answers is not a truncated one: {:?}",
            result.err()
        );
        assert_eq!(
            buffer.to_vec(),
            reader.content,
            "and the buffer holds the file's bytes in the file's order"
        );
    }

    #[test]
    fn each_read_starts_where_the_last_one_stopped() {
        let reader = Scripted::new(100, &[Answer::Bytes(10); 4]);
        let mut buffer = [0u8; 40];

        let result = fill_exact_at(&reader, &mut buffer, 30);

        assert!(result.is_ok(), "four ten-byte answers fill forty bytes");
        assert_eq!(
            reader.asked(),
            vec![(30, 40), (40, 30), (50, 20), (60, 10)],
            "every read starts at the offset it was given plus what has been filled, and asks \
             only for what is left. A loop that re-read from the base offset would copy the same \
             bytes twice and still answer `Ok`"
        );
        assert_eq!(
            buffer.to_vec(),
            reader.content[30..70].to_vec(),
            "so the buffer is the forty bytes at offset 30 and not four copies of the first ten"
        );
    }

    #[test]
    fn a_read_that_answers_nothing_before_the_buffer_is_full_is_the_end_of_the_file() {
        let reader = Scripted::new(64, &[Answer::Bytes(32), Answer::Bytes(0)]);
        let mut buffer = [0u8; 64];

        let error = fill_exact_at(&reader, &mut buffer, 0)
            .expect_err("a reader that stops answering has hit the end of the file");

        assert_eq!(
            error.kind(),
            ErrorKind::UnexpectedEof,
            "64 bytes that are not there are not a trailer"
        );
        assert_eq!(
            reader.asked().len(),
            2,
            "and it took a second read to learn that. A short answer is not the end of a file — \
             only a zero-byte one is — so the loop asks again before it gives up: {:?}",
            reader.asked()
        );
    }

    #[test]
    fn an_interrupted_read_is_tried_again() {
        let reader = Scripted::new(64, &[Answer::Fails(ErrorKind::Interrupted)]);
        let mut buffer = [0u8; 64];

        let result = fill_exact_at(&reader, &mut buffer, 0);

        assert!(
            result.is_ok(),
            "`EINTR` is the kernel saying a signal arrived, not that the read failed. The \
             launcher reads its own trailer, and a signal delivered at that moment must not turn \
             a packaged application into a numbered exit code: {:?}",
            result.err()
        );
        assert_eq!(
            buffer.to_vec(),
            reader.content,
            "the retry fills the buffer from the same offset"
        );
        assert_eq!(reader.asked().len(), 2, "one interruption, then one read");
    }

    #[test]
    fn an_error_that_is_not_an_interruption_ends_the_read() {
        let reader = Scripted::new(
            64,
            &[
                Answer::Bytes(16),
                Answer::Fails(ErrorKind::PermissionDenied),
            ],
        );
        let mut buffer = [0u8; 64];

        let error = fill_exact_at(&reader, &mut buffer, 0)
            .expect_err("a read that fails for any other reason fails the trailer");

        assert_eq!(
            error.kind(),
            ErrorKind::PermissionDenied,
            "the cause travels as it arrived. Reporting it as the end of the file would tell a \
             reader their artifact is truncated when it is not"
        );
    }

    /// Not a mutation-testing failure: the survivor `cargo mutants` reported
    /// here — `|` replaced by `^` in the fold — is an *equivalent* mutant. Each
    /// `value << 8` leaves the low eight bits zero, so an `|` and a `^` with a
    /// byte can never differ, and no input distinguishes them. What can be
    /// held is the behaviour, so that the rewrite which removes the operator
    /// is a rewrite and not a change. See `docs/dev/log/E21.md`.
    #[test]
    fn a_u64_is_read_from_eight_little_endian_bytes_and_every_byte_counts() {
        let mut raw = [0u8; 64];
        raw[8..16].copy_from_slice(&0x0123_4567_89ab_cdefu64.to_le_bytes());
        raw[16..24].copy_from_slice(&u64::MAX.to_le_bytes());

        assert_eq!(read_u64::<8>(&raw), 0x0123_4567_89ab_cdef);
        assert_eq!(read_u64::<16>(&raw), u64::MAX);
        assert_eq!(read_u64::<24>(&raw), 0, "and the reserved bytes are zero");
        assert_eq!(
            read_u64::<56>(&raw),
            0,
            "and the last field the layout has room for reads its own eight bytes rather than \
             running off the end: `read_u64::<57>` does not compile"
        );
    }
}
