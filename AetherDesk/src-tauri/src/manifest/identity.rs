//! Minimal reader of the Steam depot-manifest container, shared by every path
//! that receives manifest bytes from a provider (Hubcap Workshop generation,
//! LuaTools per-depot fetches) and must prove they are the manifest they claim
//! to be BEFORE anything reaches depotcache. A wrong or corrupt file that lands
//! there is sticky: Steam keeps failing on it and the local-first resolver keeps
//! "finding" it.
//!
//! Steam manifests are four little-endian framed protobuf sections. Only the
//! first two matter here: the payload (magic `0x71F617D0`) and the metadata
//! (magic `0x1F4812BE`) whose fields 1 (`depot_id`) and 2 (`gid_manifest`) give
//! the identity. A tiny wire reader avoids coupling the codebase to generated
//! protobufs.

use std::io::{Cursor, Read};

use zip::ZipArchive;

const PAYLOAD_MAGIC: u32 = 0x71F6_17D0;
const METADATA_MAGIC: u32 = 0x1F48_12BE;

/// Provider responses are sometimes a ZIP wrapping the single manifest file
/// (LuaTools has been observed returning one entry literally named `z`;
/// Hubcap Workshop generation does the same). The caller only ever wants the
/// inner bytes, so a ZIP signature unwraps the first non-empty entry and
/// anything else is returned as-is.
pub fn unwrap_manifest_payload(bytes: &[u8]) -> Result<Vec<u8>, String> {
    if bytes.len() >= 4 && &bytes[..4] == b"PK\x03\x04" {
        let mut archive = ZipArchive::new(Cursor::new(bytes))
            .map_err(|e| format!("Manifest ZIP is invalid: {e}"))?;
        if archive.len() == 0 {
            return Err("Manifest ZIP is empty".to_string());
        }
        for index in 0..archive.len() {
            let mut file = archive
                .by_index(index)
                .map_err(|e| format!("Could not read manifest ZIP entry {index}: {e}"))?;
            if file.is_dir() || file.size() == 0 {
                continue;
            }
            let mut payload = Vec::with_capacity(file.size() as usize);
            file.read_to_end(&mut payload)
                .map_err(|e| format!("Could not decompress manifest ZIP entry: {e}"))?;
            return Ok(payload);
        }
        return Err("Manifest ZIP contains no file entry".to_string());
    }
    Ok(bytes.to_vec())
}

/// True when the bytes start with the Steam manifest payload magic
/// (`D0 17 F6 71` on disk). Cheap pre-check; use [`manifest_identity`] for
/// the authoritative depot/GID verification.
pub fn has_manifest_magic(bytes: &[u8]) -> bool {
    bytes.len() >= 4 && u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) == PAYLOAD_MAGIC
}

fn read_u32_le(bytes: &[u8], offset: &mut usize) -> Result<u32, String> {
    if bytes.len().saturating_sub(*offset) < 4 {
        return Err("Manifest header is truncated".to_string());
    }
    let value = u32::from_le_bytes(bytes[*offset..*offset + 4].try_into().unwrap());
    *offset += 4;
    Ok(value)
}

fn read_varint(bytes: &[u8], offset: &mut usize) -> Result<u64, String> {
    let mut value = 0u64;
    for shift in (0..70).step_by(7) {
        let byte = *bytes
            .get(*offset)
            .ok_or_else(|| "Manifest metadata is truncated".to_string())?;
        *offset += 1;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err("Manifest metadata contains an invalid varint".to_string())
}

/// `(depot_id, manifest_gid)` declared by the manifest's own metadata section.
/// Accepts raw bytes or a single-entry ZIP wrapper (see
/// [`unwrap_manifest_payload`]).
pub fn manifest_identity(bytes: &[u8]) -> Result<(u32, u64), String> {
    let payload = unwrap_manifest_payload(bytes)?;
    let mut offset = 0usize;
    let payload_magic = read_u32_le(&payload, &mut offset)?;
    let payload_len = read_u32_le(&payload, &mut offset)? as usize;
    if payload_magic != PAYLOAD_MAGIC || payload.len().saturating_sub(offset) < payload_len {
        return Err("Manifest payload section has an unexpected format".to_string());
    }
    offset += payload_len;
    let metadata_magic = read_u32_le(&payload, &mut offset)?;
    let metadata_len = read_u32_le(&payload, &mut offset)? as usize;
    if metadata_magic != METADATA_MAGIC || payload.len().saturating_sub(offset) < metadata_len {
        return Err("Manifest metadata section has an unexpected format".to_string());
    }
    let metadata = &payload[offset..offset + metadata_len];
    let mut cursor = 0usize;
    let mut depot_id = 0u32;
    let mut manifest_gid = 0u64;
    while cursor < metadata.len() {
        let tag = read_varint(metadata, &mut cursor)?;
        let field = tag >> 3;
        match tag & 7 {
            0 => {
                let value = read_varint(metadata, &mut cursor)?;
                if field == 1 {
                    depot_id = value as u32;
                }
                if field == 2 {
                    manifest_gid = value;
                }
            }
            1 => cursor = cursor.saturating_add(8),
            2 => {
                let length = read_varint(metadata, &mut cursor)? as usize;
                cursor = cursor.saturating_add(length);
            }
            5 => cursor = cursor.saturating_add(4),
            _ => return Err("Manifest metadata has an unsupported protobuf wire type".to_string()),
        }
        if cursor > metadata.len() {
            return Err("Manifest metadata is truncated".to_string());
        }
    }
    if depot_id == 0 || manifest_gid == 0 {
        return Err("Manifest carries no depot/GID metadata".to_string());
    }
    Ok((depot_id, manifest_gid))
}

/// Verifies that `bytes` are a Steam manifest whose own metadata declares
/// exactly `depot_id` / `manifest_gid`, and returns the unwrapped bytes ready
/// to be written as `<depot_id>_<manifest_gid>.manifest`.
pub fn verify_manifest_bytes(
    bytes: &[u8],
    depot_id: u32,
    manifest_gid: u64,
) -> Result<Vec<u8>, String> {
    let payload = unwrap_manifest_payload(bytes)?;
    if !has_manifest_magic(&payload) {
        let preview: String = String::from_utf8_lossy(&payload[..payload.len().min(96)])
            .chars()
            .map(|character| if character.is_control() { ' ' } else { character })
            .collect();
        return Err(format!(
            "Response is not a Steam manifest ({} bytes, starts with {:?})",
            payload.len(),
            preview.trim()
        ));
    }
    let (found_depot, found_gid) = manifest_identity(&payload)?;
    if found_depot != depot_id || found_gid != manifest_gid {
        return Err(format!(
            "Manifest identity mismatch: expected {depot_id}_{manifest_gid}, file declares {found_depot}_{found_gid}"
        ));
    }
    Ok(payload)
}
