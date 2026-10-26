use std::io::Write;

use crate::manifest::identity::{
    has_manifest_magic, manifest_content_size, manifest_identity, unwrap_manifest_payload,
    verify_manifest_bytes,
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

/// Frames one payload body with a metadata section carrying `depot_id`
/// (field 1) and `gid_manifest` (field 2), plus an unrelated length-delimited
/// field the readers must skip.
fn framed_manifest(payload: &[u8], depot_id: u32, manifest_gid: u64) -> Vec<u8> {
    let mut metadata = Vec::new();
    varint(1 << 3, &mut metadata);
    varint(u64::from(depot_id), &mut metadata);
    varint(2 << 3, &mut metadata);
    varint(manifest_gid, &mut metadata);
    // field 7, wire type 2 (bytes): must be skipped without affecting identity
    varint((7 << 3) | 2, &mut metadata);
    varint(3, &mut metadata);
    metadata.extend_from_slice(b"abc");

    let mut bytes = Vec::new();
    bytes.extend_from_slice(&0x71F6_17D0u32.to_le_bytes());
    bytes.extend_from_slice(
        &(u32::try_from(payload.len()).expect("test payload fits in u32")).to_le_bytes(),
    );
    bytes.extend_from_slice(payload);
    bytes.extend_from_slice(&0x1F48_12BEu32.to_le_bytes());
    bytes.extend_from_slice(
        &(u32::try_from(metadata.len()).expect("test metadata fits in u32")).to_le_bytes(),
    );
    bytes.extend_from_slice(&metadata);
    bytes
}

/// Minimal Steam manifest container with an opaque payload body.
fn synthetic_manifest(depot_id: u32, manifest_gid: u64) -> Vec<u8> {
    framed_manifest(b"payload-bytes", depot_id, manifest_gid)
}

/// Payload of `ContentManifestPayload`: one `ContentManifestFile` per size,
/// each carrying a filename, its size (field 2) and a chunk field to skip.
fn payload_with_files(file_sizes: &[u64]) -> Vec<u8> {
    let mut payload = Vec::new();
    for (index, size) in file_sizes.iter().enumerate() {
        let name = format!("file{index}.bin");
        let mut file = Vec::new();
        varint((1 << 3) | 2, &mut file);
        varint(name.len() as u64, &mut file);
        file.extend_from_slice(name.as_bytes());
        varint(2 << 3, &mut file);
        varint(*size, &mut file);
        varint((3 << 3) | 2, &mut file);
        varint(2, &mut file);
        file.extend_from_slice(&[0, 0]);
        varint((1 << 3) | 2, &mut payload);
        varint(file.len() as u64, &mut payload);
        payload.extend_from_slice(&file);
    }
    payload
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
    assert_eq!(
        manifest_identity(&bytes).unwrap(),
        (489831, 4940892828028256588)
    );
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

#[test]
fn content_size_sums_the_file_sizes() {
    let manifest = framed_manifest(
        &payload_with_files(&[10, 25, 0, 4_000]),
        4656002,
        5722240766019054112,
    );
    assert_eq!(manifest_content_size(&manifest).unwrap(), 4_035);
    // Depot and GID come from the metadata, not from the payload.
    assert_eq!(
        manifest_identity(&manifest).unwrap(),
        (4656002, 5722240766019054112)
    );
}

#[test]
fn content_size_of_an_empty_payload_is_zero() {
    let manifest = framed_manifest(&payload_with_files(&[]), 1, 2);
    assert_eq!(manifest_content_size(&manifest).unwrap(), 0);
}

#[test]
fn content_size_rejects_bodies_that_are_not_manifests() {
    assert!(manifest_content_size(b"{\"error\":\"Manifest not found\"}").is_err());
    assert!(manifest_content_size(&[]).is_err());
    // A payload whose declared length runs past the buffer is truncated.
    let mut bytes = synthetic_manifest(1, 2);
    bytes[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(manifest_content_size(&bytes).is_err());
}
