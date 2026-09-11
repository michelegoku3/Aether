use std::path::PathBuf;

use serde::Serialize;
use tokio::task::JoinSet;

use crate::core::settings::SettingsManager;
use crate::manifest::pins::{DepotManifestPin, LuaManifestPins};
use crate::providers::hubcap::HubcapClient;
use crate::steam::compat::SteamCompat;

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GameManifestSyncReport {
    pub games_scanned: usize,
    pub generated: usize,
    pub already_local: usize,
    pub failed: usize,
}

fn discover_lua_apps(steam_path: &str) -> Vec<u32> {
    let directory = PathBuf::from(steam_path).join("config").join("stplug-in");
    let Ok(entries) = std::fs::read_dir(directory) else { return Vec::new() };
    let mut apps = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("lua") {
                return None;
            }
            path.file_stem()?.to_str()?.parse::<u32>().ok()
        })
        .filter(|app_id| *app_id != 0)
        .collect::<Vec<_>>();
    apps.sort_unstable();
    apps.dedup();
    apps
}

async fn sync_one_game(
    client: HubcapClient,
    steam_path: String,
    app_id: u32,
) -> Result<usize, String> {
    let lua = LuaManifestPins::new(steam_path.clone(), app_id);
    let content = std::fs::read_to_string(lua.lua_path())
        .map_err(|error| format!("Could not read Lua for app {app_id}: {error}"))?;
    let pins: Vec<DepotManifestPin> = LuaManifestPins::rows_from_content(&content)
        .into_iter()
        .filter(|row| row.enabled)
        .map(|row| DepotManifestPin {
            depot_id: row.app_id,
            manifest_id: row.manifest_id,
        })
        .collect();
    if pins.is_empty() { return Ok(0); }

    let local_path = steam_path.clone();
    let local_pins = pins;
    let missing = tauri::async_runtime::spawn_blocking(move || {
        crate::versioning::apply::prepare_local_manifests(&local_path, app_id, &local_pins)
    })
    .await
    .map_err(|error| format!("Local manifest preparation failed for app {app_id}: {error}"))??;
    if missing.is_empty() { return Ok(0); }

    let generated = crate::commands::versioning::generate_missing_manifests(client, missing).await?;
    SteamCompat::new(steam_path.clone()).install_manifest_files(&generated)?;
    if let Ok(backup) = crate::core::backup::GameBackup::for_app(app_id) {
        let depotcache = PathBuf::from(&steam_path).join("depotcache");
        let _ = backup.backup_referenced_manifests(&content, &depotcache);
    }
    Ok(generated.len())
}

/// Periodic local-first update synchronizer. When a Lua points at a new
/// manifest after a game update, this path generates only the missing exact
/// files through Hubcap; the archived Steam request-code bridge is never used.
#[tauri::command]
pub async fn sync_hubcap_game_manifests(
    app: tauri::AppHandle,
) -> Result<GameManifestSyncReport, String> {
    let settings = SettingsManager::new(&app).load();
    if !settings.download_games_with_updates_on || settings.hubcap_api_key.trim().is_empty() {
        return Ok(GameManifestSyncReport::default());
    }
    let steam_path = settings.steam_path;
    let apps = discover_lua_apps(&steam_path);
    let mut report = GameManifestSyncReport {
        games_scanned: apps.len(),
        ..Default::default()
    };
    let client = HubcapClient::new(settings.hubcap_api_key);
    let mut tasks = JoinSet::new();
    let mut pending = apps.into_iter();
    for _ in 0..2 {
        let Some(app_id) = pending.next() else { break };
        tasks.spawn(sync_one_game(client.clone(), steam_path.clone(), app_id));
    }
    while let Some(joined) = tasks.join_next().await {
        match joined {
            Ok(Ok(count)) => {
                if count == 0 { report.already_local += 1; }
                else { report.generated += count; }
            }
            Ok(Err(error)) => {
                report.failed += 1;
                crate::desk_log_warn!("hubcap", "Game manifest synchronization failed: {}", error);
            }
            Err(error) => {
                report.failed += 1;
                crate::desk_log_warn!("hubcap", "Game manifest task failed: {}", error);
            }
        }
        if let Some(app_id) = pending.next() {
            tasks.spawn(sync_one_game(client.clone(), steam_path.clone(), app_id));
        }
    }
    Ok(report)
}
