use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::Instant;

use regex::Regex;
use serde::Serialize;
use tokio::sync::Mutex;
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

fn cached_workshop_manifest_matches(workshop_id: u64, expected_gid: u64) -> bool {
    let path = workshop_cache_path(workshop_id);
    path.is_file()
        && std::fs::metadata(&path).map(|metadata| metadata.len() > 0).unwrap_or(false)
        && std::fs::read(path)
            .ok()
            .and_then(|bytes| manifest_identity(&bytes).ok())
            .map(|(_, gid)| gid == expected_gid)
            .unwrap_or(false)
}

/// Serializes the cache/depotcache commit shared by the periodic worker and
/// explicit Workshop actions. Hubcap request deduplication alone is not enough:
/// two callers could otherwise race on the same temporary cache filename.
fn workshop_commit_gate() -> &'static Mutex<()> {
    static GATE: OnceLock<Mutex<()>> = OnceLock::new();
    GATE.get_or_init(|| Mutex::new(()))
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if bytes.is_empty() {
        return Err("Workshop manifest cache cannot be committed because the payload is empty".to_string());
    }
    let parent = path
        .parent()
        .ok_or_else(|| "Workshop manifest cache has no parent directory".to_string())?;
    std::fs::create_dir_all(parent)
        .map_err(|e| format!("Could not create Workshop manifest cache: {e}"))?;
    let temporary = path.with_extension("manifest.tmp");
    std::fs::write(&temporary, bytes)
        .map_err(|e| format!("Could not write Workshop manifest cache: {e}"))?;
    std::fs::rename(&temporary, path)
        .map_err(|e| format!("Could not commit Workshop manifest cache: {e}"))?;
    let cached_len = std::fs::metadata(path)
        .map_err(|e| format!("Could not verify Workshop manifest cache: {e}"))?
        .len();
    if cached_len != bytes.len() as u64 {
        return Err(format!(
            "Workshop manifest cache is incomplete (expected {} bytes, found {})",
            bytes.len(),
            cached_len
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct WorkshopItem {
    app_id: u32,
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
            let Some(app_id) = path
                .file_stem()
                .and_then(|name| name.to_str())
                .and_then(|name| name.strip_prefix("appworkshop_"))
                .and_then(|id| id.parse::<u32>().ok())
                .filter(|id| *id > 0)
            else { continue };
            let Ok(content) = std::fs::read_to_string(path) else { continue };
            for captures in item_re.captures_iter(&content) {
                let Ok(workshop_id) = captures[1].parse::<u64>() else { continue };
                let Ok(manifest_gid) = captures[2].parse::<u64>() else { continue };
                if workshop_id != 0 && manifest_gid != 0 {
                    items.push(WorkshopItem { app_id, workshop_id, manifest_gid });
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
    Generated { content_present: bool },
    Cached { content_present: bool },
    AlreadyLocal { content_present: bool },
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

/// A staged `.manifest` is not proof that Steam has materialized the Workshop
/// payload. Keep this check separate so reports never claim a fully available
/// item based solely on the manifest file.
fn has_workshop_content(steam_path: &str, app_id: u32, workshop_id: u64) -> bool {
    crate::steam::library::SteamLibraryScanner::new(steam_path)
        .discover_library_paths()
        .into_iter()
        .map(|library| {
            library
                .join("steamapps")
                .join("workshop")
                .join("content")
                .join(app_id.to_string())
                .join(workshop_id.to_string())
        })
        .any(|path| path.is_dir())
}

/// A manifest stages the CDN metadata, not the Workshop payload itself. Ask
/// the running Steam client to perform the actual item download when the
/// content directory is still absent. This is best-effort and intentionally
/// does not turn a failed launch into a false "downloaded" result.
fn request_workshop_content(
    steam_path: &str,
    app_id: u32,
    workshop_id: u64,
) -> Result<(), String> {
    let uri = format!("steam://workshop/downloaditem/{app_id}/{workshop_id}");
    let executable = if cfg!(windows) {
        PathBuf::from(steam_path).join("steam.exe")
    } else {
        PathBuf::from("steam")
    };
    Command::new(&executable)
        .arg(uri)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("Could not request Steam Workshop download: {error}"))
}

async fn sync_one_workshop_item(
    client: HubcapClient,
    steam_path: String,
    item: WorkshopItem,
) -> Result<SyncOrigin, String> {
    let started = Instant::now();
    crate::desk_log_info!(
        "workshop",
        "Sync start workshop_id={} expected_manifest_gid={}",
        item.workshop_id,
        item.manifest_gid
    );
    if steam_path.trim().is_empty() {
        return Err("Workshop sync cannot install manifests because the Steam path is empty".to_string());
    }
    let _commit_guard = workshop_commit_gate().lock().await;
    if has_local_manifest_gid(&steam_path, item.manifest_gid) {
        let content_present = has_workshop_content(&steam_path, item.app_id, item.workshop_id);
        crate::desk_log_debug!(
            "workshop",
            "Sync manifest local workshop_id={} manifest_gid={} content_present={}",
            item.workshop_id,
            item.manifest_gid,
            content_present
        );
        if !content_present {
            if let Err(error) = request_workshop_content(&steam_path, item.app_id, item.workshop_id) {
                crate::desk_log_warn!("workshop", "Workshop content request unavailable item_id={}: {}", item.workshop_id, error);
            }
        }
        return Ok(SyncOrigin::AlreadyLocal { content_present });
    }
    let cache_path = workshop_cache_path(item.workshop_id);
    if cache_path.is_file() {
        match std::fs::read(&cache_path) {
            Ok(bytes) => match manifest_identity(&bytes) {
                Ok((depot_id, gid)) if gid == item.manifest_gid => {
                    crate::desk_log_info!(
                        "workshop",
                        "Sync using validated cache workshop_id={} bytes={} depot_id={} manifest_gid={}",
                        item.workshop_id,
                        bytes.len(),
                        depot_id,
                        gid
                    );
                    install_to_depotcache(&steam_path, depot_id, gid, &bytes)?;
                    let content_present = has_workshop_content(&steam_path, item.app_id, item.workshop_id);
                    if !content_present {
                        if let Err(error) = request_workshop_content(&steam_path, item.app_id, item.workshop_id) {
                            crate::desk_log_warn!("workshop", "Workshop content request unavailable item_id={}: {}", item.workshop_id, error);
                        }
                    }
                    crate::desk_log_info!(
                        "workshop",
                        "Sync complete origin=cache workshop_id={} content_present={} elapsed_ms={}",
                        item.workshop_id,
                        content_present,
                        started.elapsed().as_millis()
                    );
                    return Ok(SyncOrigin::Cached { content_present });
                }
                Ok((_depot_id, gid)) => crate::desk_log_warn!(
                    "workshop",
                    "Ignoring stale Workshop cache workshop_id={} expected_manifest_gid={} cached_manifest_gid={}",
                    item.workshop_id,
                    item.manifest_gid,
                    gid
                ),
                Err(error) => crate::desk_log_warn!(
                    "workshop",
                    "Ignoring invalid Workshop cache workshop_id={}: {}",
                    item.workshop_id,
                    error
                ),
            },
            Err(error) => crate::desk_log_warn!(
                "workshop",
                "Workshop cache read failed workshop_id={} path={}: {}",
                item.workshop_id,
                cache_path.display(),
                error
            ),
        }
    }
    crate::desk_log_info!(
        "workshop",
        "Sync requesting authenticated Hubcap Workshop manifest workshop_id={}",
        item.workshop_id
    );
    let bytes = client.generate_workshop_manifest(item.workshop_id).await?;
    let (depot_id, gid) = manifest_identity(&bytes)?;
    crate::desk_log_debug!(
        "workshop",
        "Hubcap Workshop manifest validated workshop_id={} bytes={} depot_id={} manifest_gid={}",
        item.workshop_id,
        bytes.len(),
        depot_id,
        gid
    );
    if gid != item.manifest_gid {
        return Err(format!(
            "Hubcap Workshop manifest mismatch for item {} (expected {}, received {})",
            item.workshop_id, item.manifest_gid, gid
        ));
    }
    write_atomic(&cache_path, &bytes)?;
    install_to_depotcache(&steam_path, depot_id, gid, &bytes)?;
    let content_present = has_workshop_content(&steam_path, item.app_id, item.workshop_id);
    if !content_present {
        if let Err(error) = request_workshop_content(&steam_path, item.app_id, item.workshop_id) {
            crate::desk_log_warn!("workshop", "Workshop content request unavailable item_id={}: {}", item.workshop_id, error);
        }
    }
    crate::desk_log_info!(
        "workshop",
        "Sync complete origin=generated workshop_id={} bytes={} content_present={} elapsed_ms={}",
        item.workshop_id,
        bytes.len(),
        content_present,
        started.elapsed().as_millis()
    );
    Ok(SyncOrigin::Generated { content_present })
}

fn install_to_depotcache(
    steam_path: &str,
    depot_id: u32,
    manifest_gid: u64,
    bytes: &[u8],
) -> Result<(), String> {
    if bytes.is_empty() {
        return Err("Workshop manifest cannot be installed because the payload is empty".to_string());
    }
    let directory = PathBuf::from(steam_path).join("depotcache");
    std::fs::create_dir_all(&directory)
        .map_err(|e| format!("Could not create Steam depotcache for Workshop: {e}"))?;
    let path = directory.join(format!("{depot_id}_{manifest_gid}.manifest"));
    if path.is_file() {
        let existing_len = std::fs::metadata(&path).map(|metadata| metadata.len()).unwrap_or(0);
        if existing_len > 0 && existing_len == bytes.len() as u64 {
            crate::desk_log_debug!(
                "workshop",
                "Install verified existing depotcache manifest depot_id={} manifest_gid={} bytes={}",
                depot_id,
                manifest_gid,
                existing_len
            );
            return Ok(());
        }
        crate::desk_log_warn!(
            "workshop",
            "Replacing incomplete or stale depotcache manifest depot_id={} manifest_gid={} existing_bytes={} expected_bytes={}",
            depot_id,
            manifest_gid,
            existing_len,
            bytes.len()
        );
    }
    let temporary = path.with_extension("manifest.tmp");
    std::fs::write(&temporary, bytes)
        .map_err(|e| format!("Could not stage Workshop manifest: {e}"))?;
    std::fs::rename(&temporary, &path)
        .map_err(|e| format!("Could not install Workshop manifest: {e}"))?;
    let installed_len = std::fs::metadata(&path)
        .map_err(|e| format!("Could not verify installed Workshop manifest: {e}"))?
        .len();
    if installed_len != bytes.len() as u64 || installed_len == 0 {
        return Err(format!(
            "Installed Workshop manifest is incomplete (expected {} bytes, found {})",
            bytes.len(),
            installed_len
        ));
    }
    crate::desk_log_info!(
        "workshop",
        "Workshop manifest installed path={} bytes={} depot_id={} manifest_gid={}",
        path.display(),
        installed_len,
        depot_id,
        manifest_gid
    );
    Ok(())
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct WorkshopSyncReport {
    pub discovered: usize,
    pub generated: usize,
    pub restored_from_cache: usize,
    pub already_local: usize,
    /// Items with a valid staged manifest but without Steam's actual Workshop
    /// content directory. They are intentionally not counted as complete.
    pub content_missing: usize,
    pub failed: usize,
}

/// Scans Steam's appworkshop ACF files and proactively stages missing
/// Workshop manifests through authenticated Hubcap. The coordinator invokes it
/// only after an ACF fingerprint change; exact local files and validated caches
/// are skipped before any provider validation or generation request.
#[tauri::command]
pub async fn sync_hubcap_workshop_manifests(
    app: tauri::AppHandle,
) -> Result<WorkshopSyncReport, String> {
    let started = Instant::now();
    let settings = SettingsManager::new(&app).load();
    crate::desk_log_info!(
        "workshop",
        "Workshop sync start key_configured={} steam_path_configured={}",
        !settings.hubcap_api_key.trim().is_empty(),
        !settings.steam_path.trim().is_empty()
    );
    if settings.steam_path.trim().is_empty() {
        crate::desk_log_error!("workshop", "Workshop sync cannot run because the Steam path is empty");
        return Err("Workshop sync cannot run because the Steam path is empty".to_string());
    }
    let steam_path = settings.steam_path;
    let items = discover_workshop_items(&steam_path);
    crate::desk_log_info!("workshop", "Workshop discovery complete discovered={}", items.len());
    let mut report = WorkshopSyncReport { discovered: items.len(), ..Default::default() };
    if items.is_empty() {
        crate::desk_log_debug!("workshop", "Workshop sync complete no installed Workshop items");
        return Ok(report);
    }
    let needs_remote_generation = items.iter().any(|item| {
        !has_local_manifest_gid(&steam_path, item.manifest_gid)
            && !cached_workshop_manifest_matches(item.workshop_id, item.manifest_gid)
    });
    crate::desk_log_debug!(
        "workshop",
        "Workshop sync local/cache gate discovered={} needs_remote_generation={}",
        items.len(),
        needs_remote_generation
    );
    if needs_remote_generation && settings.hubcap_api_key.trim().is_empty() {
        return Err("A valid authenticated Hubcap API key is required for missing Workshop manifests.".to_string());
    }
    let client = HubcapClient::new(settings.hubcap_api_key);
    if needs_remote_generation && !client.validate_api_key().await? {
        crate::desk_log_error!("workshop", "Workshop sync stopped because the Hubcap API key is not valid or not allowed");
        return Err("Hubcap API key is not valid or is not allowed to make requests.".to_string());
    }
    let mut tasks = JoinSet::new();
    let mut pending = items.into_iter();
    // The provider rate-limits generation separately from the daily quota.
    // Keep the worker's queue shallow; the process-wide scheduler also
    // serializes generation traffic shared with store/versioning actions.
    for _ in 0..1 {
        let Some(item) = pending.next() else { break };
        tasks.spawn(sync_one_workshop_item(client.clone(), steam_path.clone(), item));
    }
    while let Some(joined) = tasks.join_next().await {
        match joined {
            Ok(Ok(SyncOrigin::Generated { content_present })) => {
                report.generated += 1;
                if !content_present { report.content_missing += 1; }
            }
            Ok(Ok(SyncOrigin::Cached { content_present })) => {
                report.restored_from_cache += 1;
                if !content_present { report.content_missing += 1; }
            }
            Ok(Ok(SyncOrigin::AlreadyLocal { content_present })) => {
                if content_present {
                    report.already_local += 1;
                } else {
                    report.content_missing += 1;
                }
            }
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
    crate::desk_log_info!(
        "workshop",
        "Workshop sync complete discovered={} generated={} cached={} already_local={} content_missing={} failed={} elapsed_ms={}",
        report.discovered,
        report.generated,
        report.restored_from_cache,
        report.already_local,
        report.content_missing,
        report.failed,
        started.elapsed().as_millis()
    );
    Ok(report)
}

/// Explicit single-item entry point used by future Workshop UI flows. It uses
/// the same cache and generation scheduler as the periodic synchronizer.
#[tauri::command]
pub async fn generate_hubcap_workshop_manifest(
    app: tauri::AppHandle,
    workshop_id: u64,
) -> Result<String, String> {
    let started = Instant::now();
    let _commit_guard = workshop_commit_gate().lock().await;
    if workshop_id == 0 {
        return Err("A valid Workshop item ID is required".to_string());
    }
    let path = workshop_cache_path(workshop_id);
    let settings = SettingsManager::new(&app).load();
    crate::desk_log_info!(
        "workshop",
        "Explicit Workshop generation start workshop_id={} key_configured={} steam_path_configured={}",
        workshop_id,
        !settings.hubcap_api_key.trim().is_empty(),
        !settings.steam_path.trim().is_empty()
    );
    if settings.steam_path.trim().is_empty() {
        return Err("Workshop generation cannot complete because the Steam path is empty".to_string());
    }
    if path.is_file()
        && std::fs::metadata(&path)
            .map(|metadata| metadata.len() > 0)
            .unwrap_or(false)
    {
        match std::fs::read(&path) {
            Ok(bytes) => match manifest_identity(&bytes) {
                Ok((depot_id, manifest_gid)) => {
                    install_to_depotcache(&settings.steam_path, depot_id, manifest_gid, &bytes)?;
                    crate::desk_log_info!(
                        "workshop",
                        "Explicit Workshop generation restored validated cache workshop_id={} path={} elapsed_ms={}",
                        workshop_id,
                        path.display(),
                        started.elapsed().as_millis()
                    );
                    return Ok(path.display().to_string());
                }
                Err(error) => crate::desk_log_warn!(
                    "workshop",
                    "Ignoring invalid explicit Workshop cache workshop_id={}: {}",
                    workshop_id,
                    error
                ),
            },
            Err(error) => crate::desk_log_warn!(
                "workshop",
                "Explicit Workshop cache read failed workshop_id={}: {}",
                workshop_id,
                error
            ),
        }
    }
    if settings.hubcap_api_key.trim().is_empty() {
        return Err("A valid Hubcap API key is required for Workshop manifest generation.".to_string());
    }
    let hubcap = HubcapClient::new(settings.hubcap_api_key);
    if !hubcap.validate_api_key().await? {
        return Err("Hubcap API key is not valid or is not allowed to make requests.".to_string());
    }
    let bytes = hubcap.generate_workshop_manifest(workshop_id).await?;
    // Validate before caching so a provider error or wrong content type can
    // never become a persistent fake Workshop manifest.
    let (depot_id, manifest_gid) = manifest_identity(&bytes)?;
    write_atomic(&path, &bytes)?;
    install_to_depotcache(&settings.steam_path, depot_id, manifest_gid, &bytes)?;
    crate::desk_log_info!(
        "workshop",
        "Explicit Workshop generation complete workshop_id={} bytes={} depot_id={} manifest_gid={} path={} elapsed_ms={}",
        workshop_id,
        bytes.len(),
        depot_id,
        manifest_gid,
        path.display(),
        started.elapsed().as_millis()
    );
    Ok(path.display().to_string())
}
