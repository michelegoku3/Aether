use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

use regex::Regex;
use serde::Serialize;
use tokio::task::JoinSet;
use zip::ZipArchive;

use crate::core::paths::LocalAppPaths;
use crate::core::settings::SettingsManager;
use crate::providers::hubcap::HubcapClient;

fn workshop_cache_path(workshop_id: u64) -> PathBuf {
    LocalAppPaths::temp_dir()
        .join("hubcap")
        .join("workshop")
        .join(format!("{workshop_id}.manifest"))
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "Workshop manifest cache has no parent directory".to_string())?;
    std::fs::create_dir_all(parent)
        .map_err(|e| format!("Could not create Workshop manifest cache: {e}"))?;
    let temporary = path.with_extension("manifest.tmp");
    std::fs::write(&temporary, bytes)
        .map_err(|e| format!("Could not write Workshop manifest cache: {e}"))?;
    std::fs::rename(&temporary, path)
        .map_err(|e| format!("Could not commit Workshop manifest cache: {e}"))
}

#[derive(Debug, Clone, Copy)]
struct WorkshopItem {
    workshop_id: u64,
    manifest_gid: u64,
}

fn discover_workshop_items(steam_path: &str) -> Vec<WorkshopItem> {
    let libraries = crate::steam::library::SteamLibraryScanner::new(steam_path)
        .discover_library_paths();
    let item_re = Regex::new(r#"(?s)"(\d+)"\s*\{.*?"manifest"\s*"(\d+)""#)
        .expect("static Workshop ACF regex");
    let mut items = Vec::new();
    for library in libraries {
        let workshop_dir = library.join("steamapps").join("workshop");
        let Ok(entries) = std::fs::read_dir(workshop_dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            let is_workshop_acf = path
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.starts_with("appworkshop_") && name.ends_with(".acf"))
                .unwrap_or(false);
            if !is_workshop_acf { continue; }
            let Ok(content) = std::fs::read_to_string(path) else { continue };
            for captures in item_re.captures_iter(&content) {
                let Ok(workshop_id) = captures[1].parse::<u64>() else { continue };
                let Ok(manifest_gid) = captures[2].parse::<u64>() else { continue };
                if workshop_id != 0 && manifest_gid != 0 {
                    items.push(WorkshopItem { workshop_id, manifest_gid });
                }
            }
        }
    }
    items.sort_by_key(|item| (item.workshop_id, item.manifest_gid));
    items.dedup_by_key(|item| item.workshop_id);
    items
}

fn read_manifest_payload(bytes: &[u8]) -> Result<Vec<u8>, String> {
    if bytes.len() >= 4 && &bytes[..4] == b"PK\x03\x04" {
        let mut archive = ZipArchive::new(Cursor::new(bytes))
            .map_err(|e| format!("Workshop manifest ZIP is invalid: {e}"))?;
        if archive.len() == 0 {
            return Err("Workshop manifest ZIP is empty".to_string());
        }
        let mut file = archive
            .by_index(0)
            .map_err(|e| format!("Could not read Workshop manifest ZIP: {e}"))?;
        let mut payload = Vec::new();
        file.read_to_end(&mut payload)
            .map_err(|e| format!("Could not decompress Workshop manifest: {e}"))?;
        return Ok(payload);
    }
    Ok(bytes.to_vec())
}

fn read_u32_le(bytes: &[u8], offset: &mut usize) -> Result<u32, String> {
    if bytes.len().saturating_sub(*offset) < 4 {
        return Err("Workshop manifest header is truncated".to_string());
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
            .ok_or_else(|| "Workshop manifest metadata is truncated".to_string())?;
        *offset += 1;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 { return Ok(value); }
    }
    Err("Workshop manifest metadata contains an invalid varint".to_string())
}

/// Steam manifests are four little-endian framed protobuf sections. We only
/// need ContentManifestMetadata fields 1 (depot_id) and 2 (gid_manifest), so a
/// small wire reader avoids coupling the Workshop path to generated protobufs.
fn manifest_identity(bytes: &[u8]) -> Result<(u32, u64), String> {
    let payload = read_manifest_payload(bytes)?;
    let mut offset = 0usize;
    let payload_magic = read_u32_le(&payload, &mut offset)?;
    let payload_len = read_u32_le(&payload, &mut offset)? as usize;
    if payload_magic != 0x71F6_17D0 || payload.len().saturating_sub(offset) < payload_len {
        return Err("Workshop manifest payload section has an unexpected format".to_string());
    }
    offset += payload_len;
    let metadata_magic = read_u32_le(&payload, &mut offset)?;
    let metadata_len = read_u32_le(&payload, &mut offset)? as usize;
    if metadata_magic != 0x1F48_12BE || payload.len().saturating_sub(offset) < metadata_len {
        return Err("Workshop manifest metadata section has an unexpected format".to_string());
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
                if field == 1 { depot_id = value as u32; }
                if field == 2 { manifest_gid = value; }
            }
            1 => cursor = cursor.saturating_add(8),
            2 => {
                let length = read_varint(metadata, &mut cursor)? as usize;
                cursor = cursor.saturating_add(length);
            }
            5 => cursor = cursor.saturating_add(4),
            _ => return Err("Workshop manifest metadata has an unsupported protobuf wire type".to_string()),
        }
        if cursor > metadata.len() { return Err("Workshop manifest metadata is truncated".to_string()); }
    }
    if depot_id == 0 || manifest_gid == 0 {
        return Err("Hubcap returned a Workshop manifest without depot/GID metadata".to_string());
    }
    Ok((depot_id, manifest_gid))
}

#[derive(Debug, Clone, Copy)]
enum SyncOrigin {
    Generated,
    Cached,
    AlreadyLocal,
}

fn has_local_manifest_gid(steam_path: &str, manifest_gid: u64) -> bool {
    let file_name_suffix = format!("_{manifest_gid}.manifest");
    [
        PathBuf::from(steam_path).join("depotcache"),
        PathBuf::from(steam_path).join("config").join("depotcache"),
    ]
    .into_iter()
    .filter_map(|directory| std::fs::read_dir(directory).ok())
    .flat_map(|entries| entries.flatten())
    .map(|entry| entry.path())
    .any(|path| {
        path.is_file()
            && path
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.ends_with(&file_name_suffix))
                .unwrap_or(false)
            && std::fs::metadata(path).map(|metadata| metadata.len() > 0).unwrap_or(false)
    })
}

async fn sync_one_workshop_item(
    client: HubcapClient,
    steam_path: String,
    item: WorkshopItem,
) -> Result<SyncOrigin, String> {
    if has_local_manifest_gid(&steam_path, item.manifest_gid) {
        return Ok(SyncOrigin::AlreadyLocal);
    }
    let cache_path = workshop_cache_path(item.workshop_id);
    if cache_path.is_file() {
        if let Ok(bytes) = std::fs::read(&cache_path) {
            if let Ok((depot_id, gid)) = manifest_identity(&bytes) {
                if gid == item.manifest_gid {
                    install_to_depotcache(&steam_path, depot_id, gid, &bytes)?;
                    return Ok(SyncOrigin::Cached);
                }
            }
        }
    }
    let bytes = client.generate_workshop_manifest(item.workshop_id).await?;
    let (depot_id, gid) = manifest_identity(&bytes)?;
    if gid != item.manifest_gid {
        return Err(format!(
            "Hubcap Workshop manifest mismatch for item {} (expected {}, received {})",
            item.workshop_id, item.manifest_gid, gid
        ));
    }
    write_atomic(&cache_path, &bytes)?;
    install_to_depotcache(&steam_path, depot_id, gid, &bytes)?;
    Ok(SyncOrigin::Generated)
}

fn install_to_depotcache(
    steam_path: &str,
    depot_id: u32,
    manifest_gid: u64,
    bytes: &[u8],
) -> Result<(), String> {
    let directory = PathBuf::from(steam_path).join("depotcache");
    std::fs::create_dir_all(&directory)
        .map_err(|e| format!("Could not create Steam depotcache for Workshop: {e}"))?;
    let path = directory.join(format!("{depot_id}_{manifest_gid}.manifest"));
    if path.is_file() && std::fs::metadata(&path).map(|metadata| metadata.len() > 0).unwrap_or(false) {
        return Ok(());
    }
    let temporary = path.with_extension("manifest.tmp");
    std::fs::write(&temporary, bytes)
        .map_err(|e| format!("Could not stage Workshop manifest: {e}"))?;
    std::fs::rename(&temporary, &path)
        .map_err(|e| format!("Could not install Workshop manifest: {e}"))
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct WorkshopSyncReport {
    pub discovered: usize,
    pub generated: usize,
    pub restored_from_cache: usize,
    pub already_local: usize,
    pub failed: usize,
}

/// Scans Steam's appworkshop ACF files and proactively stages missing
/// Workshop manifests through authenticated Hubcap. It is safe to call from a
/// periodic worker: exact local files and cached identities are skipped, while
/// identical concurrent calls are coalesced by the Hubcap scheduler.
#[tauri::command]
pub async fn sync_hubcap_workshop_manifests(
    app: tauri::AppHandle,
) -> Result<WorkshopSyncReport, String> {
    let settings = SettingsManager::new(&app).load();
    if settings.hubcap_api_key.trim().is_empty() {
        return Ok(WorkshopSyncReport::default());
    }
    let steam_path = settings.steam_path;
    let items = discover_workshop_items(&steam_path);
    let mut report = WorkshopSyncReport { discovered: items.len(), ..Default::default() };
    let client = HubcapClient::new(settings.hubcap_api_key);
    let mut tasks = JoinSet::new();
    let mut pending = items.into_iter();
    for _ in 0..4 {
        let Some(item) = pending.next() else { break };
        tasks.spawn(sync_one_workshop_item(client.clone(), steam_path.clone(), item));
    }
    while let Some(joined) = tasks.join_next().await {
        match joined {
            Ok(Ok(SyncOrigin::Generated)) => report.generated += 1,
            Ok(Ok(SyncOrigin::Cached)) => report.restored_from_cache += 1,
            Ok(Ok(SyncOrigin::AlreadyLocal)) => report.already_local += 1,
            Ok(Err(error)) => {
                report.failed += 1;
                crate::desk_log_warn!("hubcap", "Workshop manifest preparation failed: {}", error);
            }
            Err(error) => {
                report.failed += 1;
                crate::desk_log_warn!("hubcap", "Workshop manifest task failed: {}", error);
            }
        }
        if let Some(item) = pending.next() {
            tasks.spawn(sync_one_workshop_item(client.clone(), steam_path.clone(), item));
        }
    }
    Ok(report)
}

/// Explicit single-item entry point used by future Workshop UI flows. It uses
/// the same cache and generation scheduler as the periodic synchronizer.
#[tauri::command]
pub async fn generate_hubcap_workshop_manifest(
    app: tauri::AppHandle,
    workshop_id: u64,
) -> Result<String, String> {
    if workshop_id == 0 {
        return Err("A valid Workshop item ID is required".to_string());
    }
    let path = workshop_cache_path(workshop_id);
    let settings = SettingsManager::new(&app).load();
    if path.is_file()
        && std::fs::metadata(&path)
            .map(|metadata| metadata.len() > 0)
            .unwrap_or(false)
    {
        if let Ok(bytes) = std::fs::read(&path) {
            if let Ok((depot_id, manifest_gid)) = manifest_identity(&bytes) {
                if !settings.steam_path.trim().is_empty() {
                    install_to_depotcache(&settings.steam_path, depot_id, manifest_gid, &bytes)?;
                }
            }
        }
        return Ok(path.display().to_string());
    }
    if settings.hubcap_api_key.trim().is_empty() {
        return Err("A valid Hubcap API key is required for Workshop manifest generation.".to_string());
    }
    let bytes = HubcapClient::new(settings.hubcap_api_key)
        .generate_workshop_manifest(workshop_id)
        .await?;
    // Validate before caching so a provider error or wrong content type can
    // never become a persistent fake Workshop manifest.
    let (depot_id, manifest_gid) = manifest_identity(&bytes)?;
    write_atomic(&path, &bytes)?;
    if !settings.steam_path.trim().is_empty() {
        install_to_depotcache(&settings.steam_path, depot_id, manifest_gid, &bytes)?;
    }
    Ok(path.display().to_string())
}
