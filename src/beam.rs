// SPDX-License-Identifier: MIT OR Apache-2.0
//! Reading the chunk table of a compiled BEAM module.
//!
//! A `.beam` file is an IFF container: twelve bytes of `FOR1 <u32 be size>
//! BEAM`, then a sequence of chunks, each `<4-byte id><u32 be length>` followed
//! by that many bytes of data padded up to a four-byte boundary. Nothing else
//! about the format matters here. ginary does not load modules and does not
//! decode a single chunk's contents; it needs to know which chunks a file
//! holds, which is the whole of what [`chunks`] answers.
//!
//! Two callers want that answer. `src/strip.rs` verifies, after
//! `beam_lib:strip_files/1` has run, that no staged module still carries
//! [`DEBUG_INFO_CHUNK`] or [`DOCS_CHUNK`] and that every one still carries
//! [`CODE_CHUNK`]; a strip that silently did nothing, or that damaged a file,
//! is otherwise invisible until the packaged application fails to boot.
//! `ginary beam chunks` is the developer window onto the same data.
//!
//! The reader takes bytes ginary did not write, so it is written to the rule
//! every binary parser in this crate follows: **it never panics.** A truncated
//! file, a length field of `u32::MAX`, a zero-length chunk, a file that is not
//! a BEAM at all and a file of two bytes are all typed errors or empty results,
//! never an index out of bounds and never an arithmetic overflow. `tests/beam.rs`
//! holds the property tests that hold it to that.
//!
//! One wrinkle of the format is not IFF at all. `beam_lib:strip_files/1`
//! writes every module it rewrites through `zlib:gzip/1`, so a *stripped*
//! `.beam` on disk is a gzip member wrapping the form rather than the form
//! itself; the code server and `beam_lib` both unwrap it on the way in, and a
//! reader that did not would report every module ginary ships as "not a BEAM
//! file". [`form`] is that unwrapping, [`chunks`] does it for its caller, and
//! [`MAX_FORM_BYTES`] bounds it, because the bytes come from a file and an
//! unbounded decompressor is an unbounded allocation.
//!
//! [`Chunk::offset`] is the offset of a chunk's *data*, not of its eight-byte
//! header, so `&form[chunk.offset..][..chunk.len as usize]` is the chunk —
//! where `form` is [`form`]'s answer, which for an uncompressed module is the
//! file itself.

use std::borrow::Cow;
use std::io::Read as _;
use std::path::{Path, PathBuf};

/// The four bytes every IFF file starts with.
pub const IFF_MAGIC: [u8; 4] = *b"FOR1";

/// The form type a BEAM file declares at offset 8.
pub const BEAM_FORM: [u8; 4] = *b"BEAM";

/// The twelve bytes of `FOR1 <u32 be size> BEAM`.
pub const HEADER_LEN: usize = 12;

/// The eight bytes of a chunk header, `<4-byte id><u32 be length>`.
pub const CHUNK_HEADER_LEN: usize = 8;

/// The two bytes a gzip member starts with.
pub const GZIP_MAGIC: [u8; 2] = [0x1f, 0x8b];

/// The largest form [`form`] will decompress a module into.
///
/// The biggest module in an OTP release is a few megabytes, so this is two
/// orders of magnitude of headroom and still a bound: a `.beam` is a file
/// ginary did not write, and a gzip member that claims to expand without end
/// must be a reported error rather than an allocation that ends the process.
pub const MAX_FORM_BYTES: usize = 64 * 1024 * 1024;

/// The chunk holding the compiler's debug information.
///
/// This is the larger half of what `beam_lib:strip_files/1` removes, and on
/// a Gleam standard-library module it is over half the file.
pub const DEBUG_INFO_CHUNK: [u8; 4] = *b"Dbgi";

/// The chunk holding the module's documentation.
pub const DOCS_CHUNK: [u8; 4] = *b"Docs";

/// The chunk holding the byte code.
///
/// A module without it is not loadable, which is why stripping checks for it
/// rather than assuming an external tool did no harm.
pub const CODE_CHUNK: [u8; 4] = *b"Code";

/// The chunk mapping instructions back to source lines.
///
/// Deliberately kept: it costs little and it is what turns a crash report into
/// a stack trace with file names and line numbers. See ADR 0007.
pub const LINE_CHUNK: [u8; 4] = *b"Line";

/// One chunk of a BEAM file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chunk {
    /// The four-byte chunk identifier, for example `Code` or `Dbgi`.
    pub id: [u8; 4],
    /// The offset of the chunk's *data* within the file.
    ///
    /// The eight-byte header sits at `offset - CHUNK_HEADER_LEN`.
    pub offset: usize,
    /// The length of the chunk's data, before the padding to four bytes.
    pub len: u32,
}

impl Chunk {
    /// The identifier as text, with invalid UTF-8 replaced.
    ///
    /// Every identifier a compiler emits is four printable ASCII bytes, but
    /// the bytes come from a file ginary did not write, so decoding is lossy
    /// rather than fallible: a corrupt identifier should be readable in a
    /// report, not turn listing a file's chunks into an error.
    pub fn id_str(&self) -> String {
        String::from_utf8_lossy(&self.id).into_owned()
    }
}

/// Why a file could not be read as a BEAM module.
#[derive(Debug, thiserror::Error)]
pub enum BeamError {
    /// Fewer than the [`HEADER_LEN`] bytes an IFF header needs.
    #[error("not a BEAM file: {len} bytes is shorter than the {HEADER_LEN}-byte IFF header")]
    TooShort {
        /// How many bytes there were.
        len: usize,
    },
    /// The first four bytes are not [`IFF_MAGIC`].
    #[error("not a BEAM file: it starts with {magic:?}, not `FOR1`")]
    NotIff {
        /// The four bytes that were there.
        magic: [u8; 4],
    },
    /// The form type at offset 8 is not [`BEAM_FORM`].
    #[error("not a BEAM file: the IFF form is {form:?}, not `BEAM`")]
    NotBeam {
        /// The four bytes that were there.
        form: [u8; 4],
    },
    /// The `FOR1` size field runs past the end of the file.
    #[error(
        "truncated BEAM file: the form declares {declared} bytes after the size field, \
         and only {available} follow it"
    )]
    FormOverrun {
        /// The size the form declared.
        declared: u32,
        /// How many bytes actually followed the size field.
        available: usize,
    },
    /// A chunk header needs eight bytes and the form has fewer left.
    #[error(
        "truncated BEAM file: a chunk header at offset {offset} needs \
         {CHUNK_HEADER_LEN} bytes, and {available} remain"
    )]
    TruncatedChunkHeader {
        /// Where the header started.
        offset: usize,
        /// How many bytes remained in the form.
        available: usize,
    },
    /// A chunk's declared length runs past the end of the form.
    #[error(
        "truncated BEAM file: the `{id}` chunk at offset {offset} declares {len} bytes, \
         and {available} remain"
    )]
    ChunkOverrun {
        /// The chunk identifier, decoded lossily.
        id: String,
        /// The offset of the chunk's data.
        offset: usize,
        /// The length it declared.
        len: u32,
        /// How many bytes remained in the form.
        available: usize,
    },
    /// The file is a gzip member and the wrapper could not be unpacked.
    #[error("cannot decompress the gzip-wrapped BEAM file: {message}")]
    NotGzip {
        /// What the decompressor said.
        message: String,
    },
    /// The gzip member expands past [`MAX_FORM_BYTES`].
    #[error(
        "the gzip-wrapped BEAM file expands past the {MAX_FORM_BYTES}-byte limit; \
         it is not a module any compiler wrote"
    )]
    FormTooLarge,
    /// The file could not be read at all.
    #[error("cannot read `{path}`: {source}")]
    Io {
        /// The file that could not be read.
        path: PathBuf,
        /// The underlying operating system error.
        #[source]
        source: std::io::Error,
    },
}

/// Reads the chunk table of `bytes`.
///
/// The chunks come back in the order the file holds them, which is the order
/// the compiler wrote them and the order `beam_lib` reports. A well-formed file
/// with no chunks at all — a bare `FOR1 \0\0\0\4 BEAM` — is an empty vector and
/// not an error: an empty container is a legal container, and calling it a
/// failure would make the reader disagree with the format.
///
/// Bytes after the end of the form are ignored, as IFF says they are.
///
/// A module `beam_lib:strip_files/1` rewrote is a gzip member rather than a
/// bare form, and is unwrapped with [`form`] first; the offsets are then
/// offsets into the uncompressed form.
///
/// # Errors
///
/// [`BeamError::TooShort`], [`BeamError::NotIff`] and [`BeamError::NotBeam`]
/// when the file is not a BEAM module; [`BeamError::FormOverrun`],
/// [`BeamError::TruncatedChunkHeader`] and [`BeamError::ChunkOverrun`] when it
/// is one that has been cut short or whose length fields disagree with its
/// size; and [`BeamError::NotGzip`] or [`BeamError::FormTooLarge`] when it is a
/// gzip member that cannot be unwrapped.
pub fn chunks(bytes: &[u8]) -> Result<Vec<Chunk>, BeamError> {
    if bytes.len() < HEADER_LEN {
        return Err(BeamError::TooShort { len: bytes.len() });
    }
    chunks_of_form(&form(bytes)?)
}

/// Whether `bytes` is a gzip member rather than a bare IFF form.
///
/// The one shape a `.beam` takes that is not IFF: `beam_lib:strip_files/1`
/// writes what it rewrote through `zlib:gzip/1`, so every module in a stripped
/// artifact is one of these.
pub fn is_gzipped(bytes: &[u8]) -> bool {
    bytes.starts_with(&GZIP_MAGIC)
}

/// The IFF form of a module, unwrapped from its gzip member if it has one.
///
/// Borrowed for an uncompressed module, so the common case copies nothing. The
/// offsets in [`chunks`]'s answer are offsets into what this returns.
///
/// # Errors
///
/// [`BeamError::NotGzip`] when the bytes start like a gzip member and are not a
/// whole one, and [`BeamError::FormTooLarge`] when one expands past
/// [`MAX_FORM_BYTES`].
pub fn form(bytes: &[u8]) -> Result<Cow<'_, [u8]>, BeamError> {
    if !is_gzipped(bytes) {
        return Ok(Cow::Borrowed(bytes));
    }

    let mut reader = flate2::read::GzDecoder::new(bytes).take(MAX_FORM_BYTES as u64 + 1);
    let mut form = Vec::new();
    reader
        .read_to_end(&mut form)
        .map_err(|error| BeamError::NotGzip {
            message: error.to_string(),
        })?;
    if form.len() > MAX_FORM_BYTES {
        return Err(BeamError::FormTooLarge);
    }
    Ok(Cow::Owned(form))
}

/// Reads the chunk table of an IFF form that is already uncompressed.
fn chunks_of_form(bytes: &[u8]) -> Result<Vec<Chunk>, BeamError> {
    if bytes.len() < HEADER_LEN {
        return Err(BeamError::TooShort { len: bytes.len() });
    }

    let magic = four(bytes, 0);
    if magic != IFF_MAGIC {
        return Err(BeamError::NotIff { magic });
    }
    let form = four(bytes, 8);
    if form != BEAM_FORM {
        return Err(BeamError::NotBeam { form });
    }

    // The size field counts everything after itself, form type included, so
    // the form ends `declared` bytes past offset 8 and every chunk lives
    // inside that bound. Trailing bytes are somebody else's business.
    let declared = u32::from_be_bytes(four(bytes, 4));
    let available = bytes.len().saturating_sub(SIZE_FIELD_END);
    let Ok(declared_len) = usize::try_from(declared) else {
        return Err(BeamError::FormOverrun {
            declared,
            available,
        });
    };
    if declared_len > available {
        return Err(BeamError::FormOverrun {
            declared,
            available,
        });
    }
    let form_end = SIZE_FIELD_END.saturating_add(declared_len);

    let mut chunks = Vec::new();
    let mut offset = HEADER_LEN;
    while offset < form_end {
        let left = form_end - offset;
        if left < CHUNK_HEADER_LEN {
            return Err(BeamError::TruncatedChunkHeader {
                offset,
                available: left,
            });
        }
        let id = four(bytes, offset);
        let len = u32::from_be_bytes(four(bytes, offset + 4));
        let data = offset + CHUNK_HEADER_LEN;
        let room = form_end - data;
        // `len` is compared as a `u64` because a 32-bit host cannot hold every
        // `u32` in a `usize`, and a length field of `u32::MAX` is exactly the
        // input this reader has to answer rather than wrap on.
        if u64::from(len) > room as u64 {
            return Err(BeamError::ChunkOverrun {
                id: String::from_utf8_lossy(&id).into_owned(),
                offset: data,
                len,
                available: room,
            });
        }
        chunks.push(Chunk {
            id,
            offset: data,
            len,
        });
        // A chunk is padded up to a four-byte boundary. The padding of the last
        // chunk may be cut off by the end of the form, which is not an error:
        // there is nothing after it to be misaligned.
        let end = data + len as usize;
        offset = end
            .checked_next_multiple_of(4)
            .unwrap_or(form_end)
            .min(form_end);
    }

    Ok(chunks)
}

/// The end of the `FOR1 <size>` prefix, which is what the size field counts
/// from.
const SIZE_FIELD_END: usize = 8;

/// The four bytes at `offset`, which the caller has already bounded.
///
/// Every call site has checked that four bytes are there, and the fallback
/// keeps the reader panic-free without making each one carry a match.
fn four(bytes: &[u8], offset: usize) -> [u8; 4] {
    bytes
        .get(offset..offset.saturating_add(4))
        .and_then(|slice| slice.try_into().ok())
        .unwrap_or_default()
}

/// The chunk identifiers of the file at `path`, in file order.
///
/// # Errors
///
/// [`BeamError::Io`] when the file cannot be read, and whatever [`chunks`]
/// reports about its contents. A module that was stripped is unwrapped first,
/// so a stripped `.beam` lists its chunks like any other.
pub fn chunk_ids(path: &Path) -> Result<Vec<String>, BeamError> {
    let bytes = std::fs::read(path).map_err(|source| BeamError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    chunks(&bytes).map(|chunks| chunks.iter().map(Chunk::id_str).collect())
}

/// Whether `bytes` is a BEAM module holding a chunk with this identifier.
///
/// A file that is not a BEAM module at all holds no chunk, so this answers
/// `false` rather than raising: the question "does this still carry debug
/// information" has an answer for every file.
pub fn has_chunk(bytes: &[u8], id: &[u8; 4]) -> bool {
    chunks(bytes).is_ok_and(|chunks| chunks.iter().any(|chunk| chunk.id == *id))
}
