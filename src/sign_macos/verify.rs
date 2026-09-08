// SPDX-License-Identifier: MIT OR Apache-2.0
//! Independent reader for the ad-hoc artifact profile emitted by the writer.

use sha2::{Digest as _, Sha256};

use super::SignMacosError;

fn invalid(message: impl Into<String>) -> SignMacosError {
    SignMacosError::InvalidSignature {
        message: message.into(),
    }
}

fn field<const N: usize>(bytes: &[u8], at: usize) -> Result<[u8; N], SignMacosError> {
    let end = at
        .checked_add(N)
        .ok_or_else(|| invalid("field offset overflow"))?;
    let source = bytes
        .get(at..end)
        .ok_or_else(|| invalid("truncated field"))?;
    let mut value = [0; N];
    value.copy_from_slice(source);
    Ok(value)
}

fn le32(bytes: &[u8], at: usize) -> Result<u32, SignMacosError> {
    Ok(u32::from_le_bytes(field(bytes, at)?))
}

fn le64(bytes: &[u8], at: usize) -> Result<u64, SignMacosError> {
    Ok(u64::from_le_bytes(field(bytes, at)?))
}

fn be32(bytes: &[u8], at: usize) -> Result<u32, SignMacosError> {
    Ok(u32::from_be_bytes(field(bytes, at)?))
}

fn be64(bytes: &[u8], at: usize) -> Result<u64, SignMacosError> {
    Ok(u64::from_be_bytes(field(bytes, at)?))
}

fn bounded(start: u64, len: u64, limit: usize) -> Result<std::ops::Range<usize>, SignMacosError> {
    let end = start
        .checked_add(len)
        .ok_or_else(|| invalid("region length overflow"))?;
    let start = usize::try_from(start).map_err(|_| invalid("region offset exceeds this host"))?;
    let end = usize::try_from(end).map_err(|_| invalid("region length exceeds this host"))?;
    if end > limit {
        return Err(invalid("region extends past its container"));
    }
    Ok(start..end)
}

/// Verifies a finished ginary macOS artifact before publishing its file.
///
/// This independently reads the thin Mach-O commands and the emitted ad-hoc
/// SHA-256 CodeDirectory profile, rehashes every code page, and checks the
/// payload's trailer, bounds and digest. It needs no macOS tools and claims no
/// signing identity or Gatekeeper acceptance. Native CI still runs strict codesign.
///
/// # Errors
/// Returns [`SignMacosError::InvalidSignature`] for malformed geometry, unsupported
/// signature fields, missing or duplicate commands, changed pages, or a bad payload.
pub fn verify_ad_hoc(bytes: &[u8]) -> Result<(), SignMacosError> {
    if le32(bytes, 0)? != 0xfeed_facf || le32(bytes, 12)? != 2 {
        return Err(invalid(
            "expected a thin little-endian 64-bit Mach-O executable",
        ));
    }
    let commands = bounded(32, u64::from(le32(bytes, 20)?), bytes.len())?;
    let mut at = commands.start;
    let mut signature = None;
    let mut text = None;
    let mut linkedit = None;
    for _ in 0..le32(bytes, 16)? {
        let header = bytes
            .get(at..commands.end)
            .ok_or_else(|| invalid("load command offset escaped header"))?;
        let kind = le32(header, 0)?;
        let length = le32(header, 4)?;
        if length < 8 || length % 8 != 0 {
            return Err(invalid("invalid load command length"));
        }
        let command = bounded(at as u64, u64::from(length), commands.end)?;
        let raw = &bytes[command.clone()];
        at = command.end;
        if kind == 0x1d {
            if length != 16 || signature.is_some() {
                return Err(invalid("invalid or duplicate code signature command"));
            }
            signature = Some(bounded(
                u64::from(le32(raw, 8)?),
                u64::from(le32(raw, 12)?),
                bytes.len(),
            )?);
        } else if kind == 0x19 {
            let sections = u64::from(le32(raw, 64)?);
            if u64::from(length) != 72 + sections * 80 {
                return Err(invalid("segment section count disagrees with command size"));
            }
            let region = bounded(le64(raw, 40)?, le64(raw, 48)?, bytes.len())?;
            if le64(raw, 32)? < le64(raw, 48)? {
                return Err(invalid("segment VM size is smaller than its file size"));
            }
            let name = field::<16>(raw, 8)?;
            if &name == b"__TEXT\0\0\0\0\0\0\0\0\0\0" {
                if text.replace(region.clone()).is_some() {
                    return Err(invalid("duplicate __TEXT segment"));
                }
            } else if &name == b"__LINKEDIT\0\0\0\0\0\0"
                && linkedit.replace(region.clone()).is_some()
            {
                return Err(invalid("duplicate __LINKEDIT segment"));
            }
            for section in raw[72..].as_chunks::<80>().0 {
                let flags = le32(section, 64)? & 0xff;
                if [1, 0x0c, 0x12].contains(&flags) {
                    continue;
                }
                let offset = le32(section, 48)?;
                if offset == 0 {
                    continue;
                }
                let section = bounded(u64::from(offset), le64(section, 40)?, region.end)?;
                if section.start < region.start {
                    return Err(invalid("section begins before its segment"));
                }
            }
        }
    }
    if at != commands.end {
        return Err(invalid("load command count disagrees with header size"));
    }
    let signature = signature.ok_or_else(|| invalid("missing code signature"))?;
    let text = text.ok_or_else(|| invalid("missing __TEXT segment"))?;
    let linkedit = linkedit.ok_or_else(|| invalid("missing __LINKEDIT segment"))?;
    if signature.start < commands.end
        || signature.start % 16 != 0
        || signature.end != bytes.len()
        || linkedit.end != bytes.len()
        || signature.start < linkedit.start
        || text.end > signature.start
    {
        return Err(invalid("signature is not the aligned end of __LINKEDIT"));
    }
    verify_directory(bytes, &signature, &text)?;
    let trailer_at = signature
        .start
        .checked_sub(64)
        .ok_or_else(|| invalid("missing payload trailer"))?;
    let trailer =
        crate::trailer::Trailer::parse(&field(bytes, trailer_at)?, signature.start as u64)
            .map_err(|error| invalid(format!("payload trailer: {error}")))?
            .ok_or_else(|| invalid("missing payload trailer magic"))?;
    let payload = bounded(trailer.payload_offset, trailer.payload_len, trailer_at)?;
    if payload.start < linkedit.start || payload.start < commands.end {
        return Err(invalid("payload begins outside __LINKEDIT"));
    }
    let digest: [u8; 32] = Sha256::digest(&bytes[payload]).into();
    if digest != trailer.payload_sha256 {
        return Err(invalid("payload digest does not match trailer"));
    }
    Ok(())
}

fn verify_directory(
    bytes: &[u8],
    signature: &std::ops::Range<usize>,
    text: &std::ops::Range<usize>,
) -> Result<(), SignMacosError> {
    let blob = &bytes[signature.clone()];
    if be32(blob, 0)? != 0xfade_0cc0
        || be32(blob, 4)? as usize != blob.len()
        || be32(blob, 8)? != 1
        || be32(blob, 12)? != 0
        || be32(blob, 16)? != 20
    {
        return Err(invalid("unsupported ad-hoc signature superblob"));
    }
    let directory = blob
        .get(20..)
        .ok_or_else(|| invalid("missing CodeDirectory"))?;
    if be32(directory, 0)? != 0xfade_0c02
        || be32(directory, 4)? as usize != directory.len()
        || be32(directory, 8)? != 0x0002_0400
        || be32(directory, 12)? != 2
        || be32(directory, 24)? != 0
        || field::<4>(directory, 36)? != [32, 2, 0, 12]
        || be32(directory, 40)? != 0
        || be32(directory, 44)? != 0
        || be32(directory, 48)? != 0
        || be32(directory, 52)? != 0
    {
        return Err(invalid("unsupported CodeDirectory profile"));
    }
    if u64::from(be32(directory, 32)?) != signature.start as u64
        || be64(directory, 56)? != signature.start as u64
        || be64(directory, 64)? != text.start as u64
        || be64(directory, 72)? != text.len() as u64
        || be64(directory, 80)? != 1
    {
        return Err(invalid("CodeDirectory limits disagree with the executable"));
    }
    let hashes = be32(directory, 16)? as usize;
    let identifier = be32(directory, 20)? as usize;
    if identifier < 88
        || identifier >= hashes
        || hashes > directory.len()
        || !directory[identifier..hashes].contains(&0)
    {
        return Err(invalid("invalid CodeDirectory identifier bounds"));
    }
    let slots = be32(directory, 28)? as usize;
    if slots != signature.start.div_ceil(4096)
        || slots
            .checked_mul(32)
            .and_then(|size| hashes.checked_add(size))
            != Some(directory.len())
    {
        return Err(invalid(
            "CodeDirectory slot count or hash bounds disagree with code pages",
        ));
    }
    for (slot, page) in bytes[..signature.start].chunks(4096).enumerate() {
        let digest: [u8; 32] = Sha256::digest(page).into();
        if directory[hashes + slot * 32..hashes + (slot + 1) * 32] != digest {
            return Err(invalid(format!("code page {slot} digest mismatch")));
        }
    }
    Ok(())
}
