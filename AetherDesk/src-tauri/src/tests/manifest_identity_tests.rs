use std::io::Write;

use crate::manifest::identity::{
    has_manifest_magic, manifest_identity, unwrap_manifest_payload, verify_manifest_bytes,
};

fn varint(mut value: u64, out: &mut Vec<u8>) {
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

/// Minimal Steam manifest container: payload section + metadata section with
/// `depot_id` (field 1) and `gid_manifest` (field 2), plus an unrelated
/// length-delimited field the reader must skip.
fn synthetic_manifest(depot_id: u32, manifest_gid: u64) -> Vec<u8> {
    let payload = b"payload-bytes".to_vec();
    let mut metadata = Vec::new();
    varint((1 << 3) | 0, &mut metadata);
    varint(u64::from(depot_id), &mut metadata);
    varint((2 << 3) | 0, &mut metadata);
    varint(manifest_gid, &mut metadata);
    // field 7, wire type 2 (bytes): must be skipped without affecting identity
    varint((7 << 3) | 2, &mut metadata);
    varint(3, &mut metadata);
    metadata.extend_from_slice(b"abc");

    let mut bytes = Vec::new();
    bytes.extend_from_slice(&0x71F6_17D0u32.to_le_bytes());
    bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&payload);
    bytes.extend_from_slice(&0x1F48_12BEu32.to_le_bytes());
    bytes.extend_from_slice(&(metadata.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&metadata);
    bytes
}

fn zipped(entry_name: &str, content: &[u8]) -> Vec<u8> {
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut writer = zip::ZipWriter::new(&mut cursor);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        writer.start_file(entry_name, options).unwrap();
        writer.write_all(content).unwrap();
        writer.finish().unwrap();
    }
    cursor.into_inner()
}

#[test]
fn identity_is_read_from_the_metadata_section() {
    let bytes = synthetic_manifest(489831, 4940892828028256588);
    assert!(has_manifest_magic(&bytes));
    assert_eq!(manifest_identity(&bytes).unwrap(), (489831, 4940892828028256588));
}

#[test]
fn verify_accepts_matching_identity_and_returns_raw_bytes() {
    let bytes = synthetic_manifest(489831, 4940892828028256588);
    let verified = verify_manifest_bytes(&bytes, 489831, 4940892828028256588).unwrap();
    assert_eq!(verified, bytes);
}

#[test]
fn verify_rejects_a_manifest_for_another_depot_or_gid() {
    let bytes = synthetic_manifest(489831, 4940892828028256588);
    let error = verify_manifest_bytes(&bytes, 489832, 4940892828028256588).unwrap_err();
    assert!(error.contains("identity mismatch"), "{error}");
    let error = verify_manifest_bytes(&bytes, 489831, 1).unwrap_err();
    assert!(error.contains("identity mismatch"), "{error}");
}

#[test]
fn verify_rejects_non_manifest_bodies_with_a_preview() {
    let error = verify_manifest_bytes(b"{\"error\":\"Manifest not found\"}", 1, 2).unwrap_err();
    assert!(error.contains("not a Steam manifest"), "{error}");
    assert!(error.contains("Manifest not found"), "{error}");
    assert!(verify_manifest_bytes(b"", 1, 2).is_err());
}

#[test]
fn single_entry_zip_wrapper_is_unwrapped() {
    // lua.tools has been observed wrapping the raw bytes in a ZIP whose only
    // entry is literally named "z".
    let raw = synthetic_manifest(544860, 7);
    let wrapped = zipped("z", &raw);
    assert!(!has_manifest_magic(&wrapped));
    assert_eq!(unwrap_manifest_payload(&wrapped).unwrap(), raw);
    assert_eq!(verify_manifest_bytes(&wrapped, 544860, 7).unwrap(), raw);
    assert!(unwrap_manifest_payload(&zipped("empty", b"")).is_err());
}

#[test]
fn truncated_or_foreign_containers_are_rejected() {
    let mut bytes = synthetic_manifest(1, 2);
    bytes.truncate(bytes.len() - 4);
    assert!(manifest_identity(&bytes).is_err());
    assert!(manifest_identity(b"PK\x03\x04 not really a zip").is_err());
    assert!(manifest_identity(&[0xD0, 0x17, 0xF6, 0x71]).is_err());
}
