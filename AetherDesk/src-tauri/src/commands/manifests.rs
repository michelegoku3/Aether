use serde::Serialize;
use std::path::PathBuf;
use std::time::Instant;

use crate::core::settings::SettingsManager;
use crate::manifest::pins::{DepotManifestPin, LuaManifestPins};
use crate::providers::hubcap::HubcapClient;
use crate::steam::compat::SteamCompat;

/// Result of an explicit, app-scoped manifest repair. Automatic package
/// updates use the ACF-driven coordinator instead of calling this for every
/// game on a timer.
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GameManifestSyncReport {
    pub app_id: u32,
    pub pins: usize,
    pub missing: usize,
    pub generated: usize,
    pub installed: usize,
    pub skipped: bool,
}

/// Local-first repair for one managed game. It never discovers the whole
/// library, never uses the obsolete request-code path, and asks Hubcap only
/// for exact manifest files that are missing from Steam's depotcache.
#[tauri::command]
pub async fn sync_hubcap_game_manifest(
    app: tauri::AppHandle,
    app_id: u32,
) -> Result<GameManifestSyncReport, String> {
    if app_id == 0 {
        return Err("A valid AppID is required".to_string());
    }

    let settings = SettingsManager::new(&app).load();
    if settings.steam_path.trim().is_empty() {
        return Err("Manifest repair cannot run because the Steam path is empty".to_string());
    }

    let started = Instant::now();
    let lua = LuaManifestPins::new(settings.steam_path.clone(), app_id);
    let content = std::fs::read_to_string(lua.lua_path())
        .map_err(|error| format!("Could not read Lua for app {app_id}: {error}"))?;
    LuaManifestPins::validate_content(&content)?;
    let pins: Vec<DepotManifestPin> = LuaManifestPins::rows_for_manifest_sync(&content)
        .into_iter()
        .map(|row| DepotManifestPin {
            depot_id: row.app_id,
            manifest_id: row.manifest_id,
        })
        .collect();

    let mut report = GameManifestSyncReport {
        app_id,
        pins: pins.len(),
        ..Default::default()
    };
    if pins.is_empty() {
        report.skipped = true;
        return Ok(report);
    }

    let local_path = settings.steam_path.clone();
    let verify_pins = pins.clone();
    let local_pins = pins;
    let missing = tauri::async_runtime::spawn_blocking(move || {
        crate::versioning::apply::prepare_local_manifests(&local_path, app_id, &local_pins)
    })
    .await
    .map_err(|error| format!("Local manifest preparation failed for app {app_id}: {error}"))??;
    report.missing = missing.len();
    if missing.is_empty() {
        crate::desk_log_debug!(
            "hubcap",
            "Explicit game manifest repair already local app_id={} pins={} elapsed_ms={}",
            app_id,
            report.pins,
            started.elapsed().as_millis()
        );
        return Ok(report);
    }
    if settings.hubcap_api_key.trim().is_empty() {
        return Err("A valid authenticated Hubcap API key is required for missing manifests".to_string());
    }

    let expected = missing.len();
    let client = HubcapClient::new(settings.hubcap_api_key);
    if !client.validate_api_key().await? {
        return Err("Hubcap API key is not valid or is not allowed to make requests.".to_string());
    }
    let generated = crate::commands::versioning::generate_missing_manifests(client, missing).await?;
    report.generated = generated.len();
    report.installed = SteamCompat::new(settings.steam_path.clone())
        .install_manifest_files(&generated)?;
    if report.installed != expected {
        return Err(format!(
            "Manifest completion gate failed for app {app_id}: expected {expected} files, installed {}",
            report.installed
        ));
    }

    let verify_path = settings.steam_path.clone();
    let remaining = tauri::async_runtime::spawn_blocking(move || {
        crate::versioning::apply::prepare_local_manifests(&verify_path, app_id, &verify_pins)
    })
    .await
    .map_err(|error| format!("Manifest install verification task failed for app {app_id}: {error}"))??;
    if !remaining.is_empty() {
        return Err(format!(
            "Manifest completion gate failed for app {app_id}: {} manifest(s) remain missing after install",
            remaining.len()
        ));
    }

    if let Ok(backup) = crate::core::backup::GameBackup::for_app(app_id) {
        let depotcache = PathBuf::from(&settings.steam_path).join("depotcache");
        let _ = backup.backup_referenced_manifests(&content, &depotcache);
    }
    crate::desk_log_info!(
        "hubcap",
        "Explicit game manifest repair complete app_id={} generated={} installed={} elapsed_ms={}",
        app_id,
        report.generated,
        report.installed,
        started.elapsed().as_millis()
    );
    Ok(report)
}
