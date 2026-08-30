// SPDX-License-Identifier: MIT OR Apache-2.0
//! The payload: a deterministic tar archive inside a single zstd stream.
//!
//! ```text
//! ginary.json         entry 0, the manifest
//! ginary.index.json   entry 1, path/size/mode/sha256 of everything else
//! <the staging root>  entry 2 onwards, sorted by path
//! ```
//!
//! Two properties are the reason this module exists rather than a call to
//! `tar` at each end.
//!
//! **Packing is deterministic.** Entries are written in path order with
//! `HeaderMode::Deterministic`, `mtime` 0 and uid/gid 0, and the zstd encoder
//! is single-threaded, so the same staging root and the same manifest bytes
//! produce the same payload bytes on every machine. `ginary.stage.json` is not
//! packed at all: [`Index`] supersedes it, and a file describing a tree it is
//! itself inside cannot be reproduced.
//!
//! **Unpacking is hostile to its input.** The bytes come from a file somebody
//! else may have edited, so only `Regular` and `Directory` entries are legal,
//! a path with a `..`, a root or a tar prefix component is refused before it
//! is used, an entry the tar crate declines to unpack is
//! [`PayloadError::PathEscape`] rather than a silent skip, and the stream's
//! SHA-256 is compared against the trailer's after the last entry. Nothing is
//! written outside the destination, whatever the archive says, nothing already
//! there is overwritten, and `ginary.json` — the file a cache entry's
//! completeness is judged by — is written last, once the digest has matched.

use std::collections::BTreeSet;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::assemble::{LISTING_NAME, StageListing};
use crate::manifest::{INDEX_NAME, Index, IndexError, MANIFEST_NAME, Manifest, ManifestError};

/// The largest `ginary.json` or `ginary.index.json` a reader will hold in
/// memory.
///
/// The two front entries are read whole because they are parsed, and a payload
/// is bytes somebody else may have edited: a few kilobytes of zstd can claim a
/// terabyte of tar entry, and a launcher that tried to allocate it would be
/// killed rather than report anything. The index of a full OTP tree is a few
/// hundred kilobytes, so this is two orders of magnitude of headroom.
pub const MAX_FRONT_ENTRY_BYTES: u64 = 8 * 1024 * 1024;

/// The names the format fixes at the front of the payload, with the position
/// each is fixed at.
///
/// Both are read rather than unpacked, at either end: `pack` writes them from
/// the manifest and the index it built, and `unpack` holds entry 0 back until
/// the digest matches. So neither the packer's file walk nor the unpacker's
/// `set_overwrite(false)` stands between a repeat of one of these names and
/// the destination, and both ends refuse a repeat explicitly.
const RESERVED_NAMES: [(&str, usize); 2] = [(MANIFEST_NAME, 0), (INDEX_NAME, 1)];

/// What [`pack`] wrote.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Packed {
    /// How many bytes were written to the output.
    pub len: u64,
    /// The SHA-256 of exactly those bytes.
    pub sha256: [u8; 32],
}

/// Why a payload could not be written or read.
#[derive(Debug, thiserror::Error)]
pub enum PayloadError {
    /// The staging listing could not be read.
    #[error("reading the staging listing `{path}` failed")]
    Listing {
        /// The listing that could not be read.
        path: PathBuf,
        /// What the read failed with.
        #[source]
        source: std::io::Error,
    },
    /// The staging listing is not the JSON `assemble::stage` writes.
    #[error("the staging listing `{path}` is not a listing this ginary wrote")]
    ListingFormat {
        /// The listing that could not be parsed.
        path: PathBuf,
        /// What the parse failed with.
        #[source]
        source: serde_json::Error,
    },
    /// A file named by the listing could not be hashed.
    #[error("the artifact index could not be built")]
    Index(#[from] IndexError),
    /// The manifest is unusable, most often a format version this build does
    /// not read.
    #[error("the payload's manifest cannot be used by this ginary")]
    Manifest(#[from] ManifestError),
    /// Entry 0 is not JSON.
    #[error("the payload's first entry is not a ginary manifest")]
    ManifestFormat {
        /// What the parse failed with.
        #[source]
        source: serde_json::Error,
    },
    /// Entry 1 is not JSON.
    #[error("the payload's second entry is not a ginary index")]
    IndexFormat {
        /// What the parse failed with.
        #[source]
        source: serde_json::Error,
    },
    /// The archive ended before an entry the format requires.
    #[error("the payload ends before entry {position}, which must be `{expected}`")]
    MissingEntry {
        /// The zero-based entry position that is not there.
        position: usize,
        /// The name the format requires at that position.
        expected: String,
    },
    /// An entry the format fixes by name is something else.
    #[error("entry {position} of the payload must be `{expected}` and is `{found}`")]
    UnexpectedEntry {
        /// The zero-based entry position.
        position: usize,
        /// The name the format requires there.
        expected: String,
        /// The name the archive has there.
        found: String,
    },
    /// An entry after the front matter lands on a path whose first component
    /// is a name the format reserves for entry 0 or entry 1.
    ///
    /// Those two entries are read rather than unpacked, so a repeat of either
    /// name is the one path in the archive that `set_overwrite(false)` does
    /// not already stand in front of. A path *under* a reserved name is the
    /// same rejection arriving one directory later: unpacking it would create
    /// `<dest>/ginary.json` as a directory, and the manifest's own
    /// `create_new` would then fail on it with an unattributed `AlreadyExists`.
    #[error(
        "entry {position} of the payload lands on `{name}`, whose first path component the \
         format reserves for entry {fixed}"
    )]
    DuplicateEntry {
        /// The zero-based position of the repeat.
        position: usize,
        /// The path the entry would land on, relative to the destination.
        name: String,
        /// The position the format fixes the reserved first component at.
        fixed: usize,
    },
    /// The staging root holds a file whose first path component is a name the
    /// payload format reserves.
    ///
    /// Packing it would produce an artifact whose entry 2 or later lands on a
    /// front-matter name — as the file itself, or as a directory holding it —
    /// which this ginary's own reader refuses.
    #[error(
        "the staging root holds `{path}`, whose first path component the payload format reserves \
         for its entry {fixed}"
    )]
    ReservedName {
        /// The staged file whose first path component is reserved.
        path: String,
        /// The position the format fixes that component at.
        fixed: usize,
    },
    /// An entry is neither a regular file nor a directory.
    ///
    /// `kind` is one of `contiguous file`, `symlink`, `hardlink`,
    /// `character device`, `block device`, `fifo`, `gnu long name`,
    /// `gnu long link name`, `gnu sparse`, `pax` or `other`.
    #[error("the payload holds `{path}`, which is a {kind} and not a file or a directory")]
    UnsupportedEntry {
        /// The entry's path, lossily converted.
        path: String,
        /// What the entry is instead.
        kind: String,
    },
    /// An entry's path is absolute, holds a `..`, holds a tar prefix or names
    /// nothing at all.
    #[error("the payload holds the path `{path}`, which does not stay under the extracted root")]
    UnsafePath {
        /// The entry's path, lossily converted.
        path: String,
    },
    /// The tar crate declined to unpack an entry into the destination.
    ///
    /// It answers `false` rather than failing, and a skipped file is exactly
    /// the outcome this format may not have, so `false` is this error.
    #[error("the payload's entry `{path}` was refused by the destination and was not written")]
    PathEscape {
        /// The entry's path, lossily converted.
        path: String,
    },
    /// The payload does not hash to what the trailer says.
    #[error("the payload hashes to {actual} and the trailer says {expected}")]
    ChecksumMismatch {
        /// The digest the trailer carries, in lower-case hexadecimal.
        expected: String,
        /// The digest the bytes actually have.
        actual: String,
    },
    /// One of the two front entries is larger than a reader will hold.
    #[error(
        "the payload's `{name}` entry is larger than the {limit} bytes this ginary will read into \
         memory"
    )]
    FrontEntryTooLarge {
        /// The entry that is too large.
        name: String,
        /// [`MAX_FRONT_ENTRY_BYTES`].
        limit: u64,
    },
    /// The staging root holds a file its own listing does not name.
    ///
    /// Packing it would put a file in the artifact that `ginary.index.json`
    /// does not describe, and leaving it out would drop a file silently.
    #[error(
        "the staging root holds `{path}`, which `{listing}` does not name; run the staging step \
         again so that the listing describes the tree"
    )]
    Unlisted {
        /// The file that is in the tree and not in the listing.
        path: String,
        /// The name of the listing that does not name it.
        listing: String,
    },
    /// A document ginary writes itself could not be serialised.
    #[error("writing the payload's `{name}` entry as JSON failed")]
    Serialise {
        /// The entry that could not be written.
        name: String,
        /// What the serialisation failed with.
        #[source]
        source: serde_json::Error,
    },
    /// Reading or writing the payload failed.
    #[error("reading or writing the payload failed")]
    Io(#[from] std::io::Error),
}

/// Writes the payload for a staging root.
///
/// The staging root must hold the `ginary.stage.json` that `assemble::stage`
/// wrote: it is where the per-file categories of [`Index`] come from. It is
/// the one file that is not packed.
///
/// # Errors
///
/// [`PayloadError`] when the listing is missing or unreadable, when a file it
/// names cannot be hashed, or when the output cannot be written.
pub fn pack(
    staging: &Path,
    manifest: &Manifest,
    level: i32,
    out: &mut impl Write,
) -> Result<Packed, PayloadError> {
    let listing = read_listing(staging)?;
    check_no_reserved_names(&listing)?;
    let index = Index::from_staged(staging, &listing.files)?;
    let manifest_bytes = to_json(MANIFEST_NAME, manifest)?;
    let index_bytes = to_json(INDEX_NAME, &index)?;

    // The index is what the artifact says it holds, so the tree may not hold
    // anything else: a file the listing does not name would either be packed
    // and undescribed, or dropped without a word.
    check_tree_is_listed(staging, &index)?;

    let mut writer = HashingWriter::new(out);
    {
        let encoder = zstd::stream::write::Encoder::new(&mut writer, level)?;
        let mut builder = tar::Builder::new(encoder);
        append_bytes(&mut builder, MANIFEST_NAME, &manifest_bytes)?;
        append_bytes(&mut builder, INDEX_NAME, &index_bytes)?;
        for file in &index.files {
            append_staged(&mut builder, staging, &file.path)?;
        }
        builder.into_inner()?.finish()?;
    }
    Ok(writer.finish())
}

/// Extracts a payload into `dest` and returns its manifest.
///
/// `src` is read for at most `expected_len` bytes, and every byte read is
/// hashed; after the last entry the remainder of the stream is consumed so
/// that the digest covers the whole payload rather than the part tar happened
/// to want.
///
/// # Errors
///
/// [`PayloadError`] for an illegal entry, a path that does not stay under
/// `dest`, a digest that does not match `expected_sha`, or an I/O failure.
pub fn unpack(
    src: impl Read,
    expected_len: u64,
    expected_sha: &[u8; 32],
    dest: &Path,
) -> Result<Manifest, PayloadError> {
    std::fs::create_dir_all(dest)?;
    let mut hashing = HashingReader::new(src.take(expected_len));

    let front = {
        let mut archive = tar::Archive::new(zstd::stream::read::Decoder::new(&mut hashing)?);
        archive.set_preserve_permissions(true);
        archive.set_preserve_mtime(false);
        archive.set_unpack_xattrs(false);
        archive.set_overwrite(false);

        let mut front: Option<FrontMatter> = None;
        let mut index_seen = false;
        for (position, entry) in archive.entries()?.enumerate() {
            let mut entry = entry?;
            let name = entry_name(&entry);
            check_entry_type(&entry, &name)?;
            let destined = check_entry_path(&entry, &name)?;

            if position == 0 {
                expect_name(0, MANIFEST_NAME, &name)?;
                let mode = entry.header().mode().unwrap_or(0o644);
                let bytes = read_front_entry(&mut entry, MANIFEST_NAME)?;
                let manifest = parse_manifest(&bytes)?;
                // Entry 0 is read rather than unpacked, and it is not written
                // here: see the end of the function.
                front = Some(FrontMatter {
                    manifest,
                    bytes,
                    mode,
                });
            } else {
                // The front of the payload is fixed by the format for every
                // reader, not only for the streaming ones: a cache directory
                // without an index is a directory `ginary verify` cannot read.
                if position == 1 {
                    expect_name(1, INDEX_NAME, &name)?;
                    index_seen = true;
                } else {
                    // Entries 0 and 1 never reach `unpack_in`, so a later
                    // entry carrying one of their names is the one path in the
                    // archive that nothing else refuses.
                    check_not_reserved(position, &destined)?;
                }
                unpack_entry(&mut entry, dest, &name)?;
            }
        }
        let front = front.ok_or_else(|| PayloadError::MissingEntry {
            position: 0,
            expected: MANIFEST_NAME.to_owned(),
        })?;
        if !index_seen {
            return Err(PayloadError::MissingEntry {
                position: 1,
                expected: INDEX_NAME.to_owned(),
            });
        }
        front
    };

    // The digest covers the whole payload, not the part tar happened to want,
    // so whatever is left of the stream is read before it is compared.
    std::io::copy(&mut hashing, &mut std::io::sink())?;
    let actual = hashing.finish();
    if &actual != expected_sha {
        return Err(PayloadError::ChecksumMismatch {
            expected: hex::encode(expected_sha),
            actual: hex::encode(actual),
        });
    }

    // `<key>/ginary.json` being a regular file is what the cache treats as
    // proof that an entry is complete, so it is written last and only once the
    // digest has matched: a marker that preceded a tree which was never
    // finished would be a cache hit on a half-extracted directory. `create_new`
    // is the rule the rest of the entries follow through `set_overwrite(false)`.
    create_file(&dest.join(MANIFEST_NAME), &front.bytes, front.mode)?;
    Ok(front.manifest)
}

/// Entry 0, held until the digest has been checked.
struct FrontMatter {
    /// The parsed manifest, which is what `unpack` returns.
    manifest: Manifest,
    /// The exact bytes to write to `<dest>/ginary.json`.
    bytes: Vec<u8>,
    /// The permission bits the entry carried.
    mode: u32,
}

/// Reads entry 0 and stops.
///
/// This is what `ginary inspect` and the launcher's cache lookup use, and it
/// is why the manifest is the first entry: answering takes a few kilobytes of
/// a payload that may be tens of megabytes.
///
/// # Errors
///
/// [`PayloadError`] when entry 0 is absent, is not `ginary.json`, or does not
/// parse.
pub fn read_manifest(src: impl Read) -> Result<Manifest, PayloadError> {
    let mut archive = tar::Archive::new(zstd::stream::read::Decoder::new(src)?);
    let mut entries = archive.entries()?;
    let bytes = front_entry(&mut entries, 0, MANIFEST_NAME)?;
    parse_manifest(&bytes)
}

/// Reads entries 0 and 1 and stops.
///
/// # Errors
///
/// [`PayloadError`] when either entry is absent, is misnamed, or does not
/// parse.
pub fn read_index(src: impl Read) -> Result<(Manifest, Index), PayloadError> {
    let mut archive = tar::Archive::new(zstd::stream::read::Decoder::new(src)?);
    let mut entries = archive.entries()?;
    let manifest = parse_manifest(&front_entry(&mut entries, 0, MANIFEST_NAME)?)?;
    let index_bytes = front_entry(&mut entries, 1, INDEX_NAME)?;
    let index = serde_json::from_slice(&index_bytes)
        .map_err(|source| PayloadError::IndexFormat { source })?;
    Ok((manifest, index))
}

// ------------------------------------------------------------- packing --

/// Reads and parses the staging listing at the root of `staging`.
fn read_listing(staging: &Path) -> Result<StageListing, PayloadError> {
    let path = staging.join(LISTING_NAME);
    let bytes = std::fs::read(&path).map_err(|source| PayloadError::Listing {
        path: path.clone(),
        source,
    })?;
    serde_json::from_slice(&bytes).map_err(|source| PayloadError::ListingFormat { path, source })
}

/// Serialises one of the two documents ginary writes into the payload.
fn to_json<T: serde::Serialize>(name: &str, value: &T) -> Result<Vec<u8>, PayloadError> {
    serde_json::to_vec(value).map_err(|source| PayloadError::Serialise {
        name: name.to_owned(),
        source,
    })
}

/// Checks that the staging tree holds nothing the index does not describe.
///
/// `ginary.stage.json` is the one exception, because it is the file the index
/// was built from and the one file that is never packed. An empty directory is
/// passed over: it holds nothing that could be lost, and the format has no
/// entry for one that assembly did not create.
fn check_tree_is_listed(staging: &Path, index: &Index) -> Result<(), PayloadError> {
    let listed: BTreeSet<&str> = index.files.iter().map(|file| file.path.as_str()).collect();
    let mut found = Vec::new();
    walk(staging, staging, &mut found)?;
    for path in found {
        if path != LISTING_NAME && !listed.contains(path.as_str()) {
            return Err(PayloadError::Unlisted {
                path,
                listing: LISTING_NAME.to_owned(),
            });
        }
    }
    Ok(())
}

/// Collects every non-directory under `dir`, as a root-relative
/// `/`-separated path.
fn walk(root: &Path, dir: &Path, found: &mut Vec<String>) -> Result<(), PayloadError> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            walk(root, &path, found)?;
        } else {
            found.push(
                path.strip_prefix(root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        }
    }
    Ok(())
}

/// Appends one of the two front entries, which are bytes rather than files.
fn append_bytes<W: Write>(
    builder: &mut tar::Builder<W>,
    name: &str,
    bytes: &[u8],
) -> Result<(), PayloadError> {
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(tar::EntryType::Regular);
    header.set_size(bytes.len() as u64);
    header.set_mode(0o644);
    header.set_mtime(0);
    header.set_uid(0);
    header.set_gid(0);
    builder.append_data(&mut header, name, bytes)?;
    Ok(())
}

/// Appends one staged file, with the deterministic header its metadata gives.
fn append_staged<W: Write>(
    builder: &mut tar::Builder<W>,
    staging: &Path,
    relative: &str,
) -> Result<(), PayloadError> {
    let path = staging.join(relative);
    let file = std::fs::File::open(&path)?;
    let mut header = tar::Header::new_gnu();
    // `HeaderMode::Deterministic` is where the artifact stops carrying the
    // build machine: it zeroes the owner and reduces the mode to the user
    // execute bit, so neither a umask, an ACL nor a set-user-ID bit reaches the
    // artifact. It does *not* zero the mtime — tar-rs writes a fixed non-zero
    // timestamp there, to work around tools that mishandle a zero one — and
    // `docs/format.md` fixes the payload's at 0, so that one field is set
    // afterwards.
    header.set_metadata_in_mode(&file.metadata()?, tar::HeaderMode::Deterministic);
    header.set_mtime(0);
    builder.append_data(&mut header, relative, file)?;
    Ok(())
}

// ----------------------------------------------------------- unpacking --

/// The path of an entry, lossily converted, for a message.
fn entry_name<R: Read>(entry: &tar::Entry<'_, R>) -> String {
    String::from_utf8_lossy(&entry.path_bytes()).into_owned()
}

/// Refuses everything that is neither a regular file nor a directory.
fn check_entry_type<R: Read>(entry: &tar::Entry<'_, R>, name: &str) -> Result<(), PayloadError> {
    use tar::EntryType;

    let kind = match entry.header().entry_type() {
        EntryType::Regular | EntryType::Directory => return Ok(()),
        EntryType::Continuous => "contiguous file",
        EntryType::Symlink => "symlink",
        EntryType::Link => "hardlink",
        EntryType::Char => "character device",
        EntryType::Block => "block device",
        EntryType::Fifo => "fifo",
        EntryType::GNULongName => "gnu long name",
        EntryType::GNULongLink => "gnu long link name",
        EntryType::GNUSparse => "gnu sparse",
        EntryType::XGlobalHeader | EntryType::XHeader => "pax",
        _ => "other",
    };
    Err(PayloadError::UnsupportedEntry {
        path: name.to_owned(),
        kind: kind.to_owned(),
    })
}

/// Refuses a path that does not stay under the extracted root, and answers
/// with the path the entry would land on, relative to that root.
///
/// The answer is the `/`-joined `Normal` components, which is what the tar
/// crate itself extracts to: it drops a `.` component silently, so
/// `./ginary.json` and `ginary.json` are one destination and a caller
/// comparing names has to compare this and not the raw header field.
///
/// This is the check made *before* the path is used at all; the tar crate's
/// own refusal, which it reports by answering `false`, is
/// [`PayloadError::PathEscape`] and is a second line rather than this one.
fn check_entry_path<R: Read>(
    entry: &tar::Entry<'_, R>,
    name: &str,
) -> Result<String, PayloadError> {
    let unsafe_path = || PayloadError::UnsafePath {
        path: name.to_owned(),
    };

    let Ok(path) = entry.path() else {
        return Err(unsafe_path());
    };
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(unsafe_path());
            }
        }
    }
    if parts.is_empty() {
        Err(unsafe_path())
    } else {
        Ok(parts.join("/"))
    }
}

/// The position the format fixes `path`'s first component at, when that
/// component is a reserved name.
///
/// The comparison is on the first component and not on the whole path, so a
/// *directory* named `ginary.json` is refused with the file it would hold: the
/// tar crate creates the parents of every entry, so `ginary.json/nested.txt`
/// occupies the manifest's path just as surely as a repeat of the name does.
fn reserved_first_component(path: &str) -> Option<usize> {
    let first = path.split('/').next().unwrap_or(path);
    RESERVED_NAMES
        .into_iter()
        .find(|(name, _)| first == *name)
        .map(|(_, fixed)| fixed)
}

/// Refuses an entry after the front matter that lands under a reserved name.
fn check_not_reserved(position: usize, destined: &str) -> Result<(), PayloadError> {
    match reserved_first_component(destined) {
        Some(fixed) => Err(PayloadError::DuplicateEntry {
            position,
            name: destined.to_owned(),
            fixed,
        }),
        None => Ok(()),
    }
}

/// Refuses a staging listing that names a file under a reserved name.
///
/// `pack` would otherwise write it as entry 2 or later under a front-matter
/// name and produce an artifact [`unpack`] refuses, which is a build failure
/// deferred to the machine that runs the binary.
fn check_no_reserved_names(listing: &StageListing) -> Result<(), PayloadError> {
    for file in &listing.files {
        if let Some(fixed) = reserved_first_component(&file.path) {
            return Err(PayloadError::ReservedName {
                path: file.path.clone(),
                fixed,
            });
        }
    }
    Ok(())
}

/// Unpacks one entry into `dest`, refusing a skip.
fn unpack_entry<R: Read>(
    entry: &mut tar::Entry<'_, R>,
    dest: &Path,
    name: &str,
) -> Result<(), PayloadError> {
    refuse_skip(entry.unpack_in(dest)?, name)
}

/// Turns the tar crate's "I declined to write this" answer into an error.
///
/// It answers `false` rather than failing, and a silently skipped file is
/// exactly the outcome this format may not have. The answer is `false` only
/// for a `..` component or a destination with no parent, both of which
/// [`check_entry_path`] has already refused by the time an entry reaches here,
/// so [`PayloadError::PathEscape`] is defence in depth against a future
/// version of the tar crate declining for a reason ginary did not anticipate —
/// `docs/format.md` says so, and the unit test below pins the mapping that
/// makes it a report rather than a skip.
fn refuse_skip(unpacked: bool, name: &str) -> Result<(), PayloadError> {
    if unpacked {
        Ok(())
    } else {
        Err(PayloadError::PathEscape {
            path: name.to_owned(),
        })
    }
}

/// Checks the name the format fixes at a position.
fn expect_name(position: usize, expected: &str, found: &str) -> Result<(), PayloadError> {
    if found == expected {
        Ok(())
    } else {
        Err(PayloadError::UnexpectedEntry {
            position,
            expected: expected.to_owned(),
            found: found.to_owned(),
        })
    }
}

/// Reads one front entry, refusing one that claims more than
/// [`MAX_FRONT_ENTRY_BYTES`].
fn read_front_entry<R: Read>(entry: &mut R, name: &str) -> Result<Vec<u8>, PayloadError> {
    let mut bytes = Vec::new();
    let read = entry
        .take(MAX_FRONT_ENTRY_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if read as u64 > MAX_FRONT_ENTRY_BYTES {
        return Err(PayloadError::FrontEntryTooLarge {
            name: name.to_owned(),
            limit: MAX_FRONT_ENTRY_BYTES,
        });
    }
    Ok(bytes)
}

/// Reads the entry the format fixes at `position`, and nothing after it.
fn front_entry<R: Read>(
    entries: &mut tar::Entries<'_, R>,
    position: usize,
    expected: &str,
) -> Result<Vec<u8>, PayloadError> {
    let Some(entry) = entries.next() else {
        return Err(PayloadError::MissingEntry {
            position,
            expected: expected.to_owned(),
        });
    };
    let mut entry = entry?;
    let name = entry_name(&entry);
    check_entry_type(&entry, &name)?;
    check_entry_path(&entry, &name)?;
    expect_name(position, expected, &name)?;
    read_front_entry(&mut entry, expected)
}

/// Parses entry 0 and checks that this build can act on it.
fn parse_manifest(bytes: &[u8]) -> Result<Manifest, PayloadError> {
    let manifest: Manifest =
        serde_json::from_slice(bytes).map_err(|source| PayloadError::ManifestFormat { source })?;
    manifest.check_version()?;
    Ok(manifest)
}

/// Creates one file with the permission bits an entry carried.
///
/// `create_new` rather than a plain write: a destination that already holds
/// the file is refused, which is the rule `set_overwrite(false)` applies to
/// every other entry, and an unpack into a populated directory must fail the
/// same way wherever it lands.
fn create_file(path: &Path, bytes: &[u8], mode: u32) -> Result<(), PayloadError> {
    use std::os::unix::fs::PermissionsExt;

    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(bytes)?;
    file.flush()?;
    drop(file);
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode & 0o7777))?;
    Ok(())
}

// -------------------------------------------------------------- hashing --

/// A writer that hashes and counts everything that passes through it.
struct HashingWriter<W> {
    /// Where the bytes go.
    inner: W,
    /// The digest of everything written so far.
    hasher: Sha256,
    /// How many bytes have been written.
    len: u64,
}

impl<W: Write> HashingWriter<W> {
    /// Wraps `inner`.
    fn new(inner: W) -> Self {
        Self {
            inner,
            hasher: Sha256::new(),
            len: 0,
        }
    }

    /// What was written: the length and the digest.
    fn finish(self) -> Packed {
        Packed {
            len: self.len,
            sha256: self.hasher.finalize().into(),
        }
    }
}

impl<W: Write> Write for HashingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let written = self.inner.write(buf)?;
        self.hasher.update(&buf[..written]);
        self.len = self.len.saturating_add(written as u64);
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// A reader that hashes everything that passes through it.
struct HashingReader<R> {
    /// Where the bytes come from.
    inner: R,
    /// The digest of everything read so far.
    hasher: Sha256,
}

impl<R: Read> HashingReader<R> {
    /// Wraps `inner`.
    fn new(inner: R) -> Self {
        Self {
            inner,
            hasher: Sha256::new(),
        }
    }

    /// The digest of everything that was read.
    fn finish(self) -> [u8; 32] {
        self.hasher.finalize().into()
    }
}

impl<R: Read> Read for HashingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let read = self.inner.read(buf)?;
        self.hasher.update(&buf[..read]);
        Ok(read)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `PayloadError::PathEscape` cannot be produced by any archive: every
    /// path that would make `unpack_in` answer `false` has already been
    /// refused as [`PayloadError::UnsafePath`] by the time the entry is
    /// unpacked. It is kept anyway, because the alternative to reporting a
    /// `false` is skipping the entry silently, and the format may not do that.
    /// The mapping is therefore pinned here rather than through an archive, so
    /// that a later reordering of the checks — or a tar crate that starts
    /// declining for a new reason — still ends as an error and not as a
    /// missing file.
    #[test]
    fn a_declined_entry_is_an_error_and_not_a_skip() {
        let error = refuse_skip(false, "lib/hello/ebin/hello.beam")
            .expect_err("`false` is the tar crate declining to write the entry");

        match error {
            PayloadError::PathEscape { path } => assert_eq!(path, "lib/hello/ebin/hello.beam"),
            other => panic!("expected PathEscape, got {other:?}"),
        }
    }

    #[test]
    fn an_unpacked_entry_is_not_an_error() {
        refuse_skip(true, "lib/hello/ebin/hello.beam").expect("`true` is the entry being written");
    }
}
