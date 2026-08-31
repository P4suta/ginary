// SPDX-License-Identifier: MIT OR Apache-2.0
//! The SPDX 2.3 software bill of materials for an artifact.
//!
//! An artifact is one file that carries a whole BEAM runtime and a whole
//! application closure, and the people who have to answer "what is in this
//! binary" cannot open it. `ginary build --sbom` and `ginary sbom <exe>` write
//! the answer beside it as [SPDX 2.3][spec] JSON.
//!
//! [spec]: https://spdx.github.io/spdx-spec/v2.3/
//!
//! Two rules shape the module.
//!
//! **Nothing is invented.** The shipment records what an application *is*, not
//! where it came from, so a package whose origin cannot be read is
//! [`NOASSERTION`] rather than a guess at a hex URL. The project's own
//! `manifest.toml` is the one place the origin is written down, and when it is
//! readable its `[[packages]]` supply the version and the download location.
//!
//! **The document is a function of the artifact.** The namespace an SPDX
//! document must be unique under is derived from the payload's SHA-256 rather
//! than from a random UUID and a clock, so two runs over one artifact produce
//! the same bytes and a build that is reproducible has a bill of materials
//! that is reproducible with it.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::inspect::{self, InspectError};
use crate::manifest::Manifest;

/// The SPDX version this document declares.
pub const SPDX_VERSION: &str = "SPDX-2.3";

/// The licence an SPDX document itself is under, which the specification
/// fixes.
pub const DATA_LICENSE: &str = "CC0-1.0";

/// The identifier the document element must carry.
pub const DOCUMENT_SPDX_ID: &str = "SPDXRef-DOCUMENT";

/// The prefix of every document namespace ginary writes.
pub const NAMESPACE_PREFIX: &str = "https://github.com/P4suta/ginary/spdx";

/// What SPDX says instead of guessing.
pub const NOASSERTION: &str = "NOASSERTION";

/// The download location of a package that came from hex.
pub const HEX_PACKAGE_PREFIX: &str = "https://hex.pm/packages/";

/// The `source` a Gleam `manifest.toml` gives a package that came from hex.
pub const HEX_SOURCE: &str = "hex";

/// The name of the package that stands for the bundled runtime.
pub const OTP_PACKAGE_NAME: &str = "erlang-otp";

/// Where the bundled runtime comes from.
pub const OTP_DOWNLOAD_LOCATION: &str = "https://github.com/erlang/otp";

/// The licence the bundled runtime is under.
pub const OTP_LICENCE: &str = "Apache-2.0";

/// The file name the document is written under, beside the artifact.
pub const SBOM_SUFFIX: &str = ".spdx.json";

/// The file a Gleam project locks its dependency versions in.
pub const GLEAM_MANIFEST_NAME: &str = "manifest.toml";

/// The prefix every element identifier this module writes begins with.
pub const PACKAGE_ID_PREFIX: &str = "SPDXRef-Package-";

/// The relationship the document has to the application it is about.
pub const DESCRIBES: &str = "DESCRIBES";

/// The relationship the application has to everything bundled with it.
pub const DEPENDS_ON: &str = "DEPENDS_ON";

/// One package read out of a Gleam `manifest.toml`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HexPackage {
    /// The package name.
    pub name: String,
    /// The locked version.
    pub version: String,
    /// The `source` field, `hex` for a package that came from hex.
    pub source: Option<String>,
}

/// The `creationInfo` member.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreationInfo {
    /// When the artifact was built, copied from the manifest.
    pub created: String,
    /// One entry, `Tool: ginary-<version>`.
    pub creators: Vec<String>,
}

/// One `packages` entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpdxPackage {
    /// The element identifier other elements refer to it by.
    #[serde(rename = "SPDXID")]
    pub spdx_id: String,
    /// The package name.
    pub name: String,
    /// The package version.
    pub version_info: String,
    /// Where the package can be obtained, or [`NOASSERTION`].
    pub download_location: String,
    /// Always `false`: the document describes packages, not their files.
    pub files_analyzed: bool,
    /// The licence, or [`NOASSERTION`].
    pub license_concluded: String,
    /// The licence the package declares, or [`NOASSERTION`].
    pub license_declared: String,
}

/// One `relationships` entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Relationship {
    /// The element the relationship is from.
    #[serde(rename = "spdxElementId")]
    pub spdx_element_id: String,
    /// `DESCRIBES` or `DEPENDS_ON`.
    pub relationship_type: String,
    /// The element the relationship is to.
    pub related_spdx_element: String,
}

/// A whole SPDX 2.3 document.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SbomDocument {
    /// [`SPDX_VERSION`].
    pub spdx_version: String,
    /// [`DATA_LICENSE`].
    pub data_license: String,
    /// [`DOCUMENT_SPDX_ID`].
    #[serde(rename = "SPDXID")]
    pub spdx_id: String,
    /// The document name, `<app>-<version>`.
    pub name: String,
    /// The namespace [`namespace`] derived from the payload digest.
    pub document_namespace: String,
    /// Who wrote the document and when.
    pub creation_info: CreationInfo,
    /// The application, every dependency, and the runtime.
    pub packages: Vec<SpdxPackage>,
    /// One `DESCRIBES` and one `DEPENDS_ON` per dependency.
    pub relationships: Vec<Relationship>,
}

/// The RFC 4122 spelling of the first sixteen bytes of a payload digest.
///
/// Not a random UUID: the same artifact must produce the same document, and a
/// version 4 UUID would make the SBOM the one part of a reproducible build
/// that is not reproducible. The digest is already unique per artifact, so the
/// bytes are taken from it and the version and variant nibbles are set to
/// those of a version 4 UUID, which is what a reader's parser expects to see.
pub fn uuid_from_sha256(digest: &[u8; 32]) -> String {
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    // The version and variant nibbles RFC 4122 fixes. The bytes underneath
    // them are still the digest's, so two different artifacts still produce
    // two different identifiers; what this buys is a string every UUID parser
    // in a consumer's toolchain will accept.
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;

    let hex = hex::encode(bytes);
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

/// The document namespace for one artifact.
///
/// `<prefix>/<app>-<version>-<uuid>`, with the UUID from [`uuid_from_sha256`].
pub fn namespace(app: &str, version: &str, digest: &[u8; 32]) -> String {
    format!(
        "{NAMESPACE_PREFIX}/{app}-{version}-{}",
        uuid_from_sha256(digest)
    )
}

/// One `[[packages]]` table of a Gleam `manifest.toml`.
///
/// Only the three fields that reach the document. `gleam` writes several more
/// — `build_tools`, `requirements`, `otp_app`, `outer_checksum` — and none of
/// them is refused, because this file is not ginary's to define and a key
/// Gleam adds tomorrow must not stop a bill of materials from being written.
#[derive(Debug, Deserialize)]
struct LockedPackage {
    /// The package name.
    name: String,
    /// The locked version.
    version: String,
    /// Where the package came from, `hex` for one that came from hex.
    #[serde(default)]
    source: Option<String>,
}

/// A Gleam `manifest.toml`.
#[derive(Debug, Deserialize)]
struct GleamManifest {
    /// The locked packages, absent in a project that locks none.
    #[serde(default)]
    packages: Vec<LockedPackage>,
}

/// Reads the `[[packages]]` of a Gleam `manifest.toml`.
///
/// # Errors
///
/// [`SbomError::Manifest`] when the file cannot be read and
/// [`SbomError::ManifestFormat`] when it is not the TOML Gleam writes.
pub fn read_manifest_toml(path: &Path) -> Result<Vec<HexPackage>, SbomError> {
    let text = std::fs::read_to_string(path).map_err(|source| SbomError::Manifest {
        path: path.to_path_buf(),
        source,
    })?;
    let manifest: GleamManifest =
        toml::from_str(&text).map_err(|error| SbomError::ManifestFormat {
            path: path.to_path_buf(),
            message: error.to_string().trim_end().to_owned(),
        })?;
    Ok(manifest
        .packages
        .into_iter()
        .map(|package| HexPackage {
            name: package.name,
            version: package.version,
            source: package.source,
        })
        .collect())
}

/// The element identifier one package name gets.
///
/// SPDX identifiers are `SPDXRef-` followed by letters, digits, `.` and `-`,
/// and a Gleam package name is snake case, so every other character becomes a
/// `-`. Two names that differ only in punctuation would collide, which no hex
/// namespace produces: hex names are `[a-z0-9_]`.
pub fn package_id(name: &str) -> String {
    let sanitised: String = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '.' || character == '-' {
                character
            } else {
                '-'
            }
        })
        .collect();
    format!("{PACKAGE_ID_PREFIX}{sanitised}")
}

/// One package with nothing asserted about its licence.
///
/// A shipment records what an application *is*, never what it is licensed
/// under, and SPDX has a word for that: guessing would be the one thing a bill
/// of materials may not do.
fn package(name: &str, version: &str, download: String, licence: &str) -> SpdxPackage {
    SpdxPackage {
        spdx_id: package_id(name),
        name: name.to_owned(),
        version_info: version.to_owned(),
        download_location: download,
        files_analyzed: false,
        license_concluded: licence.to_owned(),
        license_declared: licence.to_owned(),
    }
}

/// Builds the document for one artifact.
///
/// `packages` is what the project's `manifest.toml` gave, and it may be empty:
/// a shipment records no origin, so a dependency nothing described becomes a
/// package with [`NOASSERTION`] for its download location rather than a
/// guessed hex URL.
pub fn build(manifest: &Manifest, digest: &[u8; 32], packages: &[HexPackage]) -> SbomDocument {
    let application = package(
        &manifest.app,
        &manifest.app_version,
        NOASSERTION.to_owned(),
        NOASSERTION,
    );

    let mut elements = vec![application.clone()];
    elements.extend(packages.iter().map(|locked| {
        let download = if locked.source.as_deref() == Some(HEX_SOURCE) {
            format!("{HEX_PACKAGE_PREFIX}{}", locked.name)
        } else {
            NOASSERTION.to_owned()
        };
        package(&locked.name, &locked.version, download, NOASSERTION)
    }));
    elements.push(package(
        OTP_PACKAGE_NAME,
        &manifest.otp_version,
        OTP_DOWNLOAD_LOCATION.to_owned(),
        OTP_LICENCE,
    ));

    let mut relationships = vec![Relationship {
        spdx_element_id: DOCUMENT_SPDX_ID.to_owned(),
        relationship_type: DESCRIBES.to_owned(),
        related_spdx_element: application.spdx_id.clone(),
    }];
    relationships.extend(elements.iter().skip(1).map(|element| Relationship {
        spdx_element_id: application.spdx_id.clone(),
        relationship_type: DEPENDS_ON.to_owned(),
        related_spdx_element: element.spdx_id.clone(),
    }));

    SbomDocument {
        spdx_version: SPDX_VERSION.to_owned(),
        data_license: DATA_LICENSE.to_owned(),
        spdx_id: DOCUMENT_SPDX_ID.to_owned(),
        name: format!("{}-{}", manifest.app, manifest.app_version),
        document_namespace: namespace(&manifest.app, &manifest.app_version, digest),
        creation_info: CreationInfo {
            // The manifest's own timestamp and the ginary that wrote it, not a
            // clock and not the build reading the artifact: the document is a
            // function of the artifact and of nothing else.
            created: manifest.created_at.clone(),
            creators: vec![format!("Tool: ginary-{}", manifest.ginary_version)],
        },
        packages: elements,
        relationships,
    }
}

/// The application the document describes.
///
/// The first package, which is the one the `DESCRIBES` relationship points at.
/// `ginary build --sbom` needs it to name the file it writes, and the
/// document's own `name` carries the version as well.
pub fn application_name(document: &SbomDocument) -> &str {
    document
        .packages
        .first()
        .map_or(document.name.as_str(), |package| package.name.as_str())
}

/// Reads an artifact and builds its document.
///
/// `project` is the directory holding the Gleam `manifest.toml`, when there is
/// one to read; `None` is the `ginary sbom <exe>` case, where the artifact is
/// all there is and every download location is [`NOASSERTION`].
///
/// # Errors
///
/// [`SbomError::Artifact`] when the file is not a packaged application, and
/// the `manifest.toml` variants when a project was named and its manifest is
/// unreadable.
pub fn for_artifact(artifact: &Path, project: Option<&Path>) -> Result<SbomDocument, SbomError> {
    let info = inspect::open(artifact)?;
    let packages = match project {
        Some(root) => locked_packages(&root.join(GLEAM_MANIFEST_NAME))?,
        None => Vec::new(),
    };
    Ok(build(
        &info.manifest,
        &info.trailer.payload_sha256,
        &packages,
    ))
}

/// The packages a project's `manifest.toml` locks, or none when it has none.
///
/// A project that has never resolved its dependencies has no `manifest.toml`,
/// and that is not a failure: it is the [`NOASSERTION`] case, the same one
/// `ginary sbom <exe>` is in. Every other reason the file cannot be read *is*
/// a failure, because a manifest that is there and unreadable is a document
/// this run was supposed to use.
fn locked_packages(path: &Path) -> Result<Vec<HexPackage>, SbomError> {
    match read_manifest_toml(path) {
        Err(SbomError::Manifest { source, .. })
            if source.kind() == std::io::ErrorKind::NotFound =>
        {
            Ok(Vec::new())
        }
        other => other,
    }
}

/// Where the document goes when `--sbom-out` did not say.
///
/// `<the artifact's directory>/<app>.spdx.json`, so a build writes its bill of
/// materials beside the file it describes.
pub fn out_path(artifact: &Path, app: &str) -> PathBuf {
    let name = format!("{app}{SBOM_SUFFIX}");
    match artifact.parent() {
        Some(parent) => parent.join(name),
        None => PathBuf::from(name),
    }
}

/// The document as pretty JSON, ending in a newline.
///
/// # Errors
///
/// [`SbomError::Json`] when the document cannot be serialised.
pub fn to_json(document: &SbomDocument) -> Result<String, SbomError> {
    let mut json = serde_json::to_string_pretty(document)?;
    json.push('\n');
    Ok(json)
}

/// Writes the document to `path`.
///
/// # Errors
///
/// [`SbomError::Write`] when the file cannot be written, and
/// [`SbomError::Json`] when the document cannot be serialised.
pub fn write(document: &SbomDocument, path: &Path) -> Result<(), SbomError> {
    let json = to_json(document)?;
    std::fs::write(path, json).map_err(|source| SbomError::Write {
        path: path.to_path_buf(),
        source,
    })
}

/// Why a bill of materials could not be produced.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SbomError {
    /// The artifact is not a packaged application, or cannot be read.
    #[error(transparent)]
    Artifact(#[from] InspectError),
    /// The Gleam manifest could not be opened.
    #[error("cannot read the Gleam manifest {path}")]
    Manifest {
        /// The file that could not be read.
        path: PathBuf,
        /// What the operating system said.
        #[source]
        source: std::io::Error,
    },
    /// The Gleam manifest is not the TOML `gleam` writes.
    #[error("{path}: {message}")]
    ManifestFormat {
        /// The file the error is in.
        path: PathBuf,
        /// What the parser said, without its trailing newline.
        message: String,
    },
    /// The document could not be written.
    #[error("cannot write the SBOM to {path}")]
    Write {
        /// The file that could not be written.
        path: PathBuf,
        /// What the operating system said.
        #[source]
        source: std::io::Error,
    },
    /// The document could not be serialised.
    #[error("the SBOM cannot be serialised")]
    Json(#[from] serde_json::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A digest whose every byte is different, so a slice of it is visible.
    fn digest() -> [u8; 32] {
        std::array::from_fn(|index| index as u8)
    }

    #[test]
    fn the_identifier_is_a_version_four_uuid_over_the_digests_own_bytes() {
        let uuid = uuid_from_sha256(&digest());
        assert_eq!(uuid, "00010203-0405-4607-8809-0a0b0c0d0e0f");
        assert_eq!(uuid.len(), 36);
        assert_eq!(
            uuid.chars().nth(14),
            Some('4'),
            "the version nibble RFC 4122 fixes"
        );
        assert!(
            matches!(uuid.chars().nth(19), Some('8' | '9' | 'a' | 'b')),
            "the variant nibble RFC 4122 fixes: {uuid}"
        );
    }

    #[test]
    fn two_digests_that_differ_inside_their_first_sixteen_bytes_differ_here_too() {
        let mut other = digest();
        other[15] = 0xff;
        assert_ne!(uuid_from_sha256(&digest()), uuid_from_sha256(&other));
    }

    #[test]
    fn nothing_past_the_sixteenth_byte_reaches_the_identifier() {
        // Deliberate, and stated because a reader will wonder. A UUID is 128
        // bits and a SHA-256 is 256, so half the digest is what fits; two
        // artifacts colliding here would first have to collide across sixteen
        // bytes of SHA-256, which is the same assumption every content address
        // in this repository already rests on.
        let mut other = digest();
        other[16] = 0xff;
        other[31] = 0xff;
        assert_eq!(uuid_from_sha256(&digest()), uuid_from_sha256(&other));
    }

    #[test]
    fn a_snake_case_name_becomes_a_legal_element_identifier() {
        assert_eq!(package_id("gleam_stdlib"), "SPDXRef-Package-gleam-stdlib");
        assert_eq!(package_id("erlang-otp"), "SPDXRef-Package-erlang-otp");
        assert_eq!(package_id("a b/c"), "SPDXRef-Package-a-b-c");
    }

    #[test]
    fn an_artifact_at_the_root_still_has_somewhere_to_put_its_document() {
        assert_eq!(
            out_path(Path::new("/hello"), "hello"),
            PathBuf::from("/hello.spdx.json")
        );
        assert_eq!(
            out_path(Path::new("hello"), "hello"),
            PathBuf::from("hello.spdx.json")
        );
    }
}
